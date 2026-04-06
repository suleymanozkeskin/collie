#!/usr/bin/env python3
"""
Collie vs ripgrep search benchmark.

Runs diverse queries in randomized order, collects statistics, and prints
a full report with tables. Designed for repeated use to track regressions.

Usage:
    python3 scripts/benchmark_search.py /path/to/repo
    python3 scripts/benchmark_search.py /path/to/repo --runs-per-query 20
    python3 scripts/benchmark_search.py /path/to/repo --queries small
"""

import argparse
import json
import math
import os
import random
import shutil
import statistics
import subprocess
import sys
import time
import uuid
from datetime import datetime, timezone
from pathlib import Path


# ---------------------------------------------------------------------------
# Query sets
# ---------------------------------------------------------------------------

LARGE_REPO_QUERIES = [
    # Exact
    "kubelet", "context", "handler", "client",
    "config", "server", "controller", "request",
    # Prefix
    "kube%", "handle%", "init%", "config%",
    # Suffix
    "%handler", "%client", "%error", "%config",
    # Substring
    "%inform%", "%context%", "%request%", "%server%",
]

SMALL_REPO_QUERIES = [
    # Exact
    "index", "search", "builder", "config",
    "watcher", "daemon", "token", "tantivy",
    # Prefix
    "search%", "index%", "build%", "config%",
    # Suffix
    "%index", "%builder", "%config", "%path",
    # Substring
    "%search%", "%index%", "%file%", "%test%",
]

COMMENT_BY_SUFFIX = {
    ".py": "#", ".sh": "#", ".bash": "#", ".zsh": "#",
    ".rs": "//", ".go": "//", ".c": "//", ".cpp": "//",
    ".h": "//", ".hpp": "//", ".java": "//", ".js": "//",
    ".ts": "//", ".tsx": "//", ".jsx": "//", ".kt": "//",
    ".swift": "//", ".php": "//", ".scss": "//", ".sass": "//",
    ".rb": "#", ".toml": "#", ".yaml": "#", ".yml": "#",
    ".css": "/*", ".html": "<!--", ".md": "<!--",
}


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def collie_query_to_rg(query: str) -> str:
    """Convert a collie % pattern to an rg regex."""
    if query.startswith("%") and query.endswith("%"):
        return query.strip("%")
    elif query.endswith("%"):
        return r"\b" + query.rstrip("%")
    elif query.startswith("%"):
        return query.lstrip("%") + r"\b"
    else:
        return r"\b" + query + r"\b"


def query_type(query: str) -> str:
    if query.startswith("%") and query.endswith("%"):
        return "substring"
    elif query.endswith("%"):
        return "prefix"
    elif query.startswith("%"):
        return "suffix"
    else:
        return "exact"


def run_collie(binary: str, repo: Path, query: str) -> tuple[float, int]:
    start = time.perf_counter()
    result = subprocess.run(
        [binary, "search", query, "--no-snippets", "-n", "20"],
        cwd=repo, capture_output=True, text=True,
    )
    elapsed = (time.perf_counter() - start) * 1000
    count = 0
    if result.returncode == 0:
        for line in result.stdout.splitlines():
            if line.startswith("Found "):
                try:
                    count = int(line.split()[1])
                except (IndexError, ValueError):
                    pass
    return round(elapsed, 2), count


def run_rg(repo: Path, pattern: str) -> tuple[float, int]:
    start = time.perf_counter()
    result = subprocess.run(
        ["rg", "-il", pattern, "."],
        cwd=repo, capture_output=True, text=True,
    )
    elapsed = (time.perf_counter() - start) * 1000
    count = len([l for l in result.stdout.splitlines() if l.strip()]) if result.returncode == 0 else 0
    return round(elapsed, 2), count


def count_files(repo: Path) -> int:
    result = subprocess.run(
        ["rg", "--files", "."], cwd=repo, capture_output=True, text=True
    )
    return len(result.stdout.splitlines()) if result.returncode == 0 else 0


def find_incremental_file(repo: Path) -> str | None:
    """Find a source file suitable for incremental watcher testing.

    Prefers files that are likely to be watched and indexed:
    shallow paths, common extensions, not in vendor/generated dirs.
    """
    preferred = [
        # Well-known paths that tend to exist in Go/Rust repos
        "pkg/probe/dialer_others.go",
        "src/lib.rs",
        "src/main.rs",
        "main.go",
        "cmd/main.go",
    ]
    for rel in preferred:
        if (repo / rel).is_file():
            return "./" + rel

    for ext in [".go", ".rs", ".py", ".js", ".ts"]:
        result = subprocess.run(
            ["rg", "--files", "-g", f"*{ext}", "--max-depth", "4", "."],
            cwd=repo, capture_output=True, text=True,
        )
        if result.returncode == 0:
            files = [l.strip() for l in result.stdout.splitlines() if l.strip()]
            # Skip files in vendor/generated/test paths
            files = [f for f in files if not any(
                d in f for d in ["/vendor/", "/generated/", "/_output/", "/third_party/"]
            )]
            if files:
                return files[len(files) // 2]
    return None


def measure_incremental(binary: str, repo: Path, rel_file: str, poll_ms: int = 50, timeout_s: float = 30) -> dict:
    """Measure add and remove watcher propagation latency."""
    file_path = repo / rel_file
    original = file_path.read_bytes()
    token = f"bench_incr_{uuid.uuid4().hex}"
    suffix = file_path.suffix.lower()
    comment = COMMENT_BY_SUFFIX.get(suffix, "//")
    if comment == "/*":
        marker = f"\n/* {token} */\n".encode()
    elif comment == "<!--":
        marker = f"\n<!-- {token} -->\n".encode()
    else:
        marker = f"\n{comment} {token}\n".encode()

    add_ms = None
    remove_ms = None
    try:
        # Add
        file_path.write_bytes(original + marker)
        start = time.perf_counter()
        deadline = start + timeout_s
        while time.perf_counter() < deadline:
            _, count = run_collie(binary, repo, token)
            if count >= 1:
                add_ms = round((time.perf_counter() - start) * 1000, 2)
                break
            time.sleep(poll_ms / 1000)

        # Remove
        file_path.write_bytes(original)
        start = time.perf_counter()
        deadline = start + timeout_s
        while time.perf_counter() < deadline:
            _, count = run_collie(binary, repo, token)
            if count == 0:
                remove_ms = round((time.perf_counter() - start) * 1000, 2)
                break
            time.sleep(poll_ms / 1000)
    finally:
        if file_path.read_bytes() != original:
            file_path.write_bytes(original)

    return {"file": rel_file, "token": token, "add_ms": add_ms, "remove_ms": remove_ms}


def fmt_ms(val: float | None) -> str:
    return f"{val:.1f}ms" if val is not None else "n/a"


def fmt_speedup(collie: float, rg: float) -> str:
    if collie <= 0:
        return "n/a"
    return f"{rg / collie:.1f}x"


def latency_stats(values: list[float]) -> dict[str, float]:
    sorted_values = sorted(float(v) for v in values)
    return {
        "min": round(sorted_values[0], 2),
        "max": round(sorted_values[-1], 2),
        "mean": round(statistics.fmean(sorted_values), 2),
        "p50": round(percentile(sorted_values, 50), 2),
        "p95": round(percentile(sorted_values, 95), 2),
        "p99": round(percentile(sorted_values, 99), 2),
    }


def percentile(sorted_values: list[float], p: float) -> float:
    if len(sorted_values) == 1:
        return sorted_values[0]
    rank = (len(sorted_values) - 1) * (p / 100.0)
    lower = math.floor(rank)
    upper = math.ceil(rank)
    if lower == upper:
        return sorted_values[lower]
    frac = rank - lower
    return sorted_values[lower] * (1.0 - frac) + sorted_values[upper] * frac


def dir_size_bytes(path: Path) -> int:
    total = 0
    for root, dirs, files in os.walk(path):
        if ".git" in dirs:
            dirs.remove(".git")
        for f in files:
            try:
                total += (Path(root) / f).stat().st_size
            except OSError:
                pass
    return total


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Collie vs ripgrep search benchmark with detailed report."
    )
    parser.add_argument("repo", help="Path to the repository to benchmark.")
    parser.add_argument("--runs-per-query", type=int, default=15)
    parser.add_argument("--binary", help="Path to the collie binary.")
    parser.add_argument("--queries", choices=["large", "small", "auto"], default="auto")
    parser.add_argument("--skip-incremental", action="store_true", help="Skip watcher latency test.")
    parser.add_argument("--output", help="JSON output path.")
    parser.add_argument(
        "--state-dir",
        help="Set COLLIE_STATE_DIR for an isolated benchmark cache/index location.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    repo = Path(args.repo).expanduser().resolve()
    if not repo.exists():
        print(f"error: repo not found: {repo}", file=sys.stderr)
        return 1

    if args.state_dir:
        os.environ["COLLIE_STATE_DIR"] = str(Path(args.state_dir).expanduser().resolve())

    script_dir = Path(__file__).resolve().parent
    collie_root = script_dir.parent
    binary = args.binary or str(collie_root / "target" / "release" / "collie")
    if not Path(binary).exists():
        print(f"error: binary not found: {binary}\nRun: cargo build --release", file=sys.stderr)
        return 1

    rg = shutil.which("rg")
    if not rg:
        print("error: rg (ripgrep) not found in PATH", file=sys.stderr)
        return 1

    # ---- Setup ----
    file_count = count_files(repo)

    if args.queries == "large":
        queries = LARGE_REPO_QUERIES
    elif args.queries == "small":
        queries = SMALL_REPO_QUERIES
    else:
        queries = LARGE_REPO_QUERIES if file_count > 1000 else SMALL_REPO_QUERIES

    # Ensure daemon is running
    status_out = subprocess.run(
        [binary, "status", str(repo), "--json"],
        capture_output=True, text=True,
    )
    daemon_was_running = False
    if status_out.returncode == 0:
        try:
            daemon_was_running = json.loads(status_out.stdout).get("status") == "Running"
        except json.JSONDecodeError:
            pass

    cold_start_ms = None
    if not daemon_was_running:
        # Stop and clean for a fair cold start measurement
        subprocess.run([binary, "stop", str(repo)], capture_output=True)
        subprocess.run([binary, "clean", str(repo)], capture_output=True)

        start = time.perf_counter()
        subprocess.run([binary, "watch", str(repo)], check=True, capture_output=True, text=True)
        cold_start_ms = round((time.perf_counter() - start) * 1000, 2)

    # Read status
    status_out = subprocess.run(
        [binary, "status", str(repo), "--json"],
        capture_output=True, text=True,
    )
    status = {}
    if status_out.returncode == 0:
        try:
            status = json.loads(status_out.stdout)
        except json.JSONDecodeError:
            pass

    index_size = dir_size_bytes(repo / ".collie")
    indexed_files = status.get("total_files", "?")
    segments = status.get("segment_count", "?")
    generation = status.get("generation", "?")

    # ---- Search benchmark ----
    # Build randomized execution plan
    plan: list[tuple[str, str]] = []
    for q in queries:
        rg_pat = collie_query_to_rg(q)
        for _ in range(args.runs_per_query):
            plan.append((q, rg_pat))
    random.shuffle(plan)

    # Warmup
    run_collie(binary, repo, queries[0])
    run_rg(repo, collie_query_to_rg(queries[0]))

    collie_times: dict[str, list[float]] = {q: [] for q in queries}
    rg_times: dict[str, list[float]] = {q: [] for q in queries}
    collie_counts: dict[str, int] = {}
    rg_counts: dict[str, int] = {}

    for q, rg_pat in plan:
        c_ms, c_cnt = run_collie(binary, repo, q)
        collie_times[q].append(c_ms)
        collie_counts[q] = c_cnt

        r_ms, r_cnt = run_rg(repo, rg_pat)
        rg_times[q].append(r_ms)
        rg_counts[q] = r_cnt

    # ---- Incremental watcher benchmark ----
    incr_result = None
    if not args.skip_incremental:
        incr_file = find_incremental_file(repo)
        if incr_file:
            # Try up to 2 times — watcher can miss events under heavy load
            for attempt in range(2):
                incr_result = measure_incremental(binary, repo, incr_file, timeout_s=60)
                if incr_result.get("add_ms") is not None:
                    break
                if attempt == 0:
                    print("  Incremental add timed out, retrying...")

    # ---- Build report ----
    now = datetime.now(timezone.utc)
    print()
    print("=" * 100)
    print(f"  COLLIE SEARCH BENCHMARK REPORT")
    print(f"  {now.strftime('%Y-%m-%d %H:%M:%S UTC')}")
    print("=" * 100)
    print()

    # Repo info
    print(f"  Repository:      {repo}")
    print(f"  Files on disk:   {file_count:,}")
    print(f"  Files indexed:   {indexed_files}")
    print(f"  Index size:      {index_size / 1_048_576:.1f} MB")
    print(f"  Segments:        {segments}")
    print(f"  Generation:      {generation}")
    if cold_start_ms is not None:
        print(f"  Cold start:      {cold_start_ms:.0f} ms")
    print()

    # Per-query table
    print(f"  {'Query':<22} {'Type':<10} {'collie p50':>11} {'rg p50':>11} {'Speedup':>9} {'collie #':>10} {'rg #':>10}")
    print(f"  {'-'*22} {'-'*10} {'-'*11} {'-'*11} {'-'*9} {'-'*10} {'-'*10}")

    by_type: dict[str, list[tuple[float, float]]] = {"exact": [], "prefix": [], "suffix": [], "substring": []}
    all_collie: list[float] = []
    all_rg: list[float] = []

    query_data = []
    for q in queries:
        ct = collie_times[q]
        rt = rg_times[q]
        c_summary = latency_stats(ct)
        r_summary = latency_stats(rt)
        c_p50 = c_summary["p50"]
        r_p50 = r_summary["p50"]
        qt = query_type(q)
        by_type[qt].append((c_p50, r_p50))
        all_collie.extend(ct)
        all_rg.extend(rt)
        sp = fmt_speedup(c_p50, r_p50)
        print(f"  {q:<22} {qt:<10} {fmt_ms(c_p50):>11} {fmt_ms(r_p50):>11} {sp:>9} {collie_counts.get(q,0):>10} {rg_counts.get(q,0):>10}")
        query_data.append({
            "query": q, "type": qt,
            "collie_stats_ms": c_summary,
            "collie_p50_ms": round(c_p50, 2), "collie_avg_ms": round(statistics.fmean(ct), 2),
            "collie_min_ms": round(min(ct), 2), "collie_max_ms": round(max(ct), 2),
            "rg_stats_ms": r_summary,
            "rg_p50_ms": round(r_p50, 2), "rg_avg_ms": round(statistics.fmean(rt), 2),
            "rg_min_ms": round(min(rt), 2), "rg_max_ms": round(max(rt), 2),
            "collie_results": collie_counts.get(q, 0), "rg_results": rg_counts.get(q, 0),
        })

    # Summary by type
    print()
    print(f"  {'Category':<22} {'collie p50':>11} {'rg p50':>11} {'Speedup':>9}")
    print(f"  {'-'*22} {'-'*11} {'-'*11} {'-'*9}")
    type_summary = {}
    for qt in ["exact", "prefix", "suffix", "substring"]:
        pairs = by_type[qt]
        if not pairs:
            continue
        c_med = statistics.median([p[0] for p in pairs])
        r_med = statistics.median([p[1] for p in pairs])
        sp = fmt_speedup(c_med, r_med)
        print(f"  {qt:<22} {fmt_ms(c_med):>11} {fmt_ms(r_med):>11} {sp:>9}")
        type_summary[qt] = {"collie_p50_ms": round(c_med, 2), "rg_p50_ms": round(r_med, 2)}

    c_agg_stats = latency_stats(all_collie)
    r_agg_stats = latency_stats(all_rg)
    c_agg = c_agg_stats["p50"]
    r_agg = r_agg_stats["p50"]
    print(f"  {'-'*22} {'-'*11} {'-'*11} {'-'*9}")
    print(f"  {'OVERALL':<22} {fmt_ms(c_agg):>11} {fmt_ms(r_agg):>11} {fmt_speedup(c_agg, r_agg):>9}")

    # Incremental
    if incr_result:
        print()
        print(f"  Incremental watcher latency:")
        print(f"    File:    {incr_result['file']}")
        print(f"    Add:     {fmt_ms(incr_result['add_ms'])}")
        print(f"    Remove:  {fmt_ms(incr_result['remove_ms'])}")
        if incr_result["add_ms"] is None or incr_result["remove_ms"] is None:
            print(f"    WARNING: propagation timed out")

    print()
    print("=" * 100)

    # Save JSON
    output_data = {
        "timestamp_utc": now.isoformat(),
        "repo": str(repo),
        "collie_state_dir": os.environ.get("COLLIE_STATE_DIR"),
        "file_count": file_count,
        "indexed_files": indexed_files,
        "index_size_bytes": index_size,
        "segments": segments,
        "generation": generation,
        "cold_start_ms": cold_start_ms,
        "runs_per_query": args.runs_per_query,
        "queries": query_data,
        "type_summary": type_summary,
        "aggregate": {
            "collie_stats_ms": c_agg_stats,
            "rg_stats_ms": r_agg_stats,
            "collie_p50_ms": round(c_agg, 2),
            "rg_p50_ms": round(r_agg, 2),
            "speedup": round(r_agg / c_agg, 2) if c_agg > 0 else None,
        },
        "incremental": incr_result,
    }

    if args.output:
        out_path = Path(args.output)
    else:
        repo_name = repo.name
        out_path = (
            collie_root / "benchmark-results"
            / f"search-{now.strftime('%Y%m%dT%H%M%SZ')}-{repo_name}.json"
        )
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(output_data, indent=2))
    print(f"  Results saved to {out_path}")
    print()

    return 0


if __name__ == "__main__":
    sys.exit(main())
