#!/usr/bin/env python3
"""
Production-style Collie benchmark runner.

Measures:
- release binary size
- repo size / tracked file count
- cold rebuild wall/user/sys/max RSS
- watch readiness wall/user/sys/max RSS
- lexical query latency + optional rg baseline
- symbol query latency
- incremental watcher propagation latency
- index size and daemon status snapshots

Usage:
    python3 scripts/benchmark_production.py /path/to/repo
    python3 scripts/benchmark_production.py /path/to/repo --profile kubernetes
    python3 scripts/benchmark_production.py /path/to/repo --runs-per-query 10 --warmups 1
"""

from __future__ import annotations

import argparse
import json
import math
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
import uuid
from datetime import datetime, timezone
from pathlib import Path


COMMENT_BY_SUFFIX = {
    ".py": "#",
    ".sh": "#",
    ".bash": "#",
    ".zsh": "#",
    ".yaml": "#",
    ".yml": "#",
    ".toml": "#",
    ".rb": "#",
    ".rs": "//",
    ".go": "//",
    ".c": "//",
    ".cc": "//",
    ".cpp": "//",
    ".h": "//",
    ".hpp": "//",
    ".java": "//",
    ".js": "//",
    ".ts": "//",
    ".tsx": "//",
    ".jsx": "//",
    ".kt": "//",
    ".swift": "//",
    ".php": "//",
    ".scss": "//",
    ".sass": "//",
    ".css": "/*",
    ".html": "<!--",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run a production-style Collie benchmark and write a JSON artifact."
    )
    parser.add_argument(
        "repo",
        nargs="?",
        help="Path to the target repository. If omitted, --profile may supply a default repo.",
    )
    parser.add_argument(
        "--profile",
        help="Benchmark profile key from benchmark-data/production_profiles.json. "
        "If omitted, infer from repo name/origin/size.",
    )
    parser.add_argument(
        "--profiles",
        help="Path to the production benchmark profiles JSON. "
        "Default: benchmark-data/production_profiles.json",
    )
    parser.add_argument(
        "--binary",
        help="Path to a prebuilt collie binary. Default: target/release/collie",
    )
    parser.add_argument(
        "--skip-build",
        action="store_true",
        help="Skip cargo build --release and use the existing binary.",
    )
    parser.add_argument(
        "--runs-per-query",
        type=int,
        default=5,
        help="Measured runs per query. Default: 5.",
    )
    parser.add_argument(
        "--warmups",
        type=int,
        default=1,
        help="Warmup runs before measured runs for each query. Default: 1.",
    )
    parser.add_argument(
        "--poll-interval-ms",
        type=int,
        default=50,
        help="Polling interval for watcher propagation checks. Default: 50.",
    )
    parser.add_argument(
        "--timeout-seconds",
        type=float,
        default=30.0,
        help="Timeout for watcher propagation checks. Default: 30.",
    )
    parser.add_argument(
        "--skip-watch",
        action="store_true",
        help="Skip watch readiness and incremental update checks.",
    )
    parser.add_argument(
        "--skip-incremental",
        action="store_true",
        help="Skip incremental add/remove latency checks.",
    )
    parser.add_argument(
        "--skip-rg",
        action="store_true",
        help="Skip ripgrep baselines for lexical queries.",
    )
    parser.add_argument(
        "--keep-daemon-running",
        action="store_true",
        help="Leave the daemon running after the benchmark completes.",
    )
    parser.add_argument(
        "--output",
        help="Where to write the JSON result. Default: benchmark-results/<timestamp>-production-<repo>.json",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    script_path = Path(__file__).resolve()
    collie_root = script_path.parent.parent
    profiles_path = (
        Path(args.profiles).expanduser().resolve()
        if args.profiles
        else collie_root / "benchmark-data" / "production_profiles.json"
    )
    profiles_doc = json.loads(profiles_path.read_text(encoding="utf-8"))

    if args.runs_per_query < 1:
        print("error: --runs-per-query must be >= 1", file=sys.stderr)
        return 1
    if args.warmups < 0:
        print("error: --warmups must be >= 0", file=sys.stderr)
        return 1

    repo = resolve_target_repo(collie_root, args.repo, args.profile, profiles_doc["profiles"])
    if not repo.exists():
        print(f"error: repo not found: {repo}", file=sys.stderr)
        return 1
    if not repo.joinpath(".git").exists():
        print(f"error: repo is not a git repository: {repo}", file=sys.stderr)
        return 1

    binary = resolve_binary(collie_root, args.binary, args.skip_build)
    time_tool = detect_time_tool()
    rg_path = shutil.which("rg")

    tracked_files = count_files_with_rg(repo)
    repo_origin = git_remote_origin(repo)
    if args.profile:
        profile = find_profile(profiles_doc["profiles"], args.profile)
        profile_selection = {
            "mode": "requested",
            "requested_key": args.profile,
            "compatible": True,
        }
    else:
        profile, profile_selection = select_profile(
            profiles_doc["profiles"],
            repo_name=repo.name,
            repo_origin=repo_origin,
            tracked_files=tracked_files,
        )

    output_path = resolve_output_path(collie_root, repo, args.output)
    report = {
        "timestamp_utc": datetime.now(timezone.utc).isoformat(),
        "benchmark_version": 1,
        "environment": {
            "platform": platform.platform(),
            "python": sys.version.split()[0],
            "time_tool": time_tool["mode"],
        },
        "collie_repo": {
            "path": str(collie_root),
            "git_commit": git_rev_parse_short(collie_root),
            "binary_path": str(binary),
            "binary_size_bytes": binary.stat().st_size,
        },
        "target_repo": {
            "path": str(repo),
            "name": repo.name,
            "git_commit": git_rev_parse_short(repo),
            "origin": repo_origin,
            "tracked_files": tracked_files,
            "size_bytes": directory_size_bytes(repo),
        },
        "profile": profile,
        "profile_selection": profile_selection,
        "metrics": {},
    }

    exit_code = 0
    try:
        stop_daemon(binary, repo, collie_root)
        reset_collie_state(binary, repo, collie_root)

        rebuild = timed_run(
            [str(binary), "rebuild", str(repo)],
            cwd=collie_root,
            time_tool=time_tool,
        )
        if rebuild["returncode"] != 0:
            raise RuntimeError(rebuild["stderr"].strip() or "collie rebuild failed")

        rebuild_status = load_status(binary, repo, collie_root)
        report["metrics"]["rebuild"] = enrich_command_metric(
            rebuild,
            index_size_bytes=state_dir_size_bytes(repo, rebuild_status),
            status=rebuild_status,
        )

        if not args.skip_watch:
            watch = timed_run(
                [str(binary), "watch", str(repo)],
                cwd=collie_root,
                time_tool=time_tool,
            )
            if watch["returncode"] != 0:
                raise RuntimeError(watch["stderr"].strip() or "collie watch failed")
            watch_status = load_status(binary, repo, collie_root)
            report["metrics"]["watch_ready"] = enrich_command_metric(
                watch,
                index_size_bytes=state_dir_size_bytes(repo, watch_status),
                status=watch_status,
            )
        else:
            report["metrics"]["watch_ready"] = None

        artifacts_status = (
            report["metrics"]["watch_ready"]["status"]
            if report["metrics"]["watch_ready"]
            else report["metrics"]["rebuild"]["status"]
        )
        report["metrics"]["artifacts"] = {
            "binary_size_bytes": binary.stat().st_size,
            "collie_dir_size_bytes": state_dir_size_bytes(repo, artifacts_status),
            "active_generation_tantivy_size_bytes": active_generation_tantivy_size_bytes(
                repo, artifacts_status
            ),
        }

        report["metrics"]["queries"] = measure_queries(
            binary=binary,
            repo=repo,
            profile=profile,
            runs_per_query=args.runs_per_query,
            warmups=args.warmups,
            time_tool=time_tool,
            rg_path=rg_path if not args.skip_rg else None,
        )

        if not args.skip_watch and not args.skip_incremental:
            report["metrics"]["incremental_update"] = measure_incremental(
                binary=binary,
                collie_root=collie_root,
                repo=repo,
                profile=profile,
                poll_interval_ms=args.poll_interval_ms,
                timeout_seconds=args.timeout_seconds,
            )
        else:
            report["metrics"]["incremental_update"] = None

    except Exception as exc:
        exit_code = 1
        report["error"] = str(exc)
    finally:
        if not args.keep_daemon_running:
            stop_daemon(binary, repo, collie_root)

    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print_summary(output_path, report)
    return exit_code


def resolve_binary(collie_root: Path, binary_arg: str | None, skip_build: bool) -> Path:
    binary = (
        Path(binary_arg).expanduser().resolve()
        if binary_arg
        else collie_root / "target" / "release" / "collie"
    )
    if not skip_build:
        bin_name = binary.stem or binary.name
        subprocess.run(
            ["cargo", "build", "--release", "--bin", bin_name],
            cwd=collie_root,
            check=True,
        )
    if not binary.exists():
        raise SystemExit(f"error: collie binary not found at {binary}")
    return binary


def resolve_output_path(collie_root: Path, repo: Path, output_arg: str | None) -> Path:
    if output_arg:
        return Path(output_arg).expanduser().resolve()
    slug = re.sub(r"[^a-zA-Z0-9._-]+", "-", repo.name).strip("-") or "repo"
    timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    return collie_root / "benchmark-results" / f"{timestamp}-production-{slug}.json"


def find_profile(profiles: list[dict], key: str) -> dict:
    for profile in profiles:
        if profile["key"] == key:
            return profile
    raise SystemExit(f"error: unknown benchmark profile key: {key}")


def resolve_target_repo(
    collie_root: Path,
    repo_arg: str | None,
    requested_key: str | None,
    profiles: list[dict],
) -> Path:
    explicit_repo = Path(repo_arg).expanduser().resolve() if repo_arg else None
    if not requested_key:
        if explicit_repo is None:
            raise SystemExit("error: repo is required when --profile is omitted")
        return explicit_repo

    profile = find_profile(profiles, requested_key)
    default_rel = profile.get("default_repo_relpath")
    default_repo = (collie_root / default_rel).resolve() if default_rel else None

    if explicit_repo is not None:
        if default_repo is not None and explicit_repo != default_repo:
            raise SystemExit(
                f"error: profile '{requested_key}' targets {default_repo}, "
                f"but repo argument points to {explicit_repo}. "
                "Omit the repo argument or pass the matching repo."
            )
        return explicit_repo

    if default_repo is not None:
        return default_repo

    raise SystemExit(
        f"error: profile '{requested_key}' does not define a default repo; pass a repo path explicitly"
    )


def select_profile(
    profiles: list[dict],
    repo_name: str,
    repo_origin: str,
    tracked_files: int,
) -> tuple[dict, dict]:
    for profile in profiles:
        if repo_name in profile.get("repo_names", []):
            return profile, {
                "mode": "inferred",
                "requested_key": None,
                "compatible": True,
                "matched_by": "repo_name",
            }
        if any(fragment in repo_origin for fragment in profile.get("repo_origin_substrings", [])):
            return profile, {
                "mode": "inferred",
                "requested_key": None,
                "compatible": True,
                "matched_by": "repo_origin",
            }

    for profile in profiles:
        min_files = profile.get("min_tracked_files")
        max_files = profile.get("max_tracked_files")
        if min_files is not None and tracked_files < min_files:
            continue
        if max_files is not None and tracked_files > max_files:
            continue
        if not profile.get("repo_names") and not profile.get("repo_origin_substrings"):
            return profile, {
                "mode": "inferred",
                "requested_key": None,
                "compatible": True,
                "matched_by": "tracked_files",
            }

    raise SystemExit("error: could not infer a production benchmark profile")


def detect_time_tool() -> dict:
    time_path = Path("/usr/bin/time")
    if time_path.exists():
        if sys.platform == "darwin":
            return {"mode": "bsd", "path": str(time_path)}
        probe = subprocess.run(
            [str(time_path), "--version"],
            capture_output=True,
            text=True,
        )
        if probe.returncode == 0:
            return {"mode": "gnu", "path": str(time_path)}
    return {"mode": "builtin", "path": None}


def timed_run(cmd: list[str], cwd: Path, time_tool: dict) -> dict:
    start = time.perf_counter()
    if time_tool["mode"] == "bsd":
        with tempfile.NamedTemporaryFile("w+", delete=False) as stats_file:
            stats_path = Path(stats_file.name)
        try:
            proc = subprocess.run(
                [time_tool["path"], "-l", "-o", str(stats_path), *cmd],
                cwd=cwd,
                capture_output=True,
                text=True,
            )
            wall_ms = round((time.perf_counter() - start) * 1000, 2)
            stats = parse_bsd_time(stats_path.read_text(encoding="utf-8"))
        finally:
            stats_path.unlink(missing_ok=True)
        if should_fallback_from_time_wrapper(proc):
            return timed_run(cmd, cwd, {"mode": "builtin", "path": None})
        return {
            "command": cmd,
            "returncode": proc.returncode,
            "stdout": proc.stdout,
            "stderr": proc.stderr,
            "wall_ms": wall_ms,
            "user_ms": stats.get("user_ms"),
            "sys_ms": stats.get("sys_ms"),
            "max_rss_bytes": stats.get("max_rss_bytes"),
        }

    if time_tool["mode"] == "gnu":
        with tempfile.NamedTemporaryFile("w+", delete=False) as stats_file:
            stats_path = Path(stats_file.name)
        try:
            proc = subprocess.run(
                [time_tool["path"], "-v", "-o", str(stats_path), *cmd],
                cwd=cwd,
                capture_output=True,
                text=True,
            )
            wall_ms = round((time.perf_counter() - start) * 1000, 2)
            stats = parse_gnu_time(stats_path.read_text(encoding="utf-8"))
        finally:
            stats_path.unlink(missing_ok=True)
        if should_fallback_from_time_wrapper(proc):
            return timed_run(cmd, cwd, {"mode": "builtin", "path": None})
        return {
            "command": cmd,
            "returncode": proc.returncode,
            "stdout": proc.stdout,
            "stderr": proc.stderr,
            "wall_ms": wall_ms,
            "user_ms": stats.get("user_ms"),
            "sys_ms": stats.get("sys_ms"),
            "max_rss_bytes": stats.get("max_rss_bytes"),
        }

    proc = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True)
    wall_ms = round((time.perf_counter() - start) * 1000, 2)
    return {
        "command": cmd,
        "returncode": proc.returncode,
        "stdout": proc.stdout,
        "stderr": proc.stderr,
        "wall_ms": wall_ms,
        "user_ms": None,
        "sys_ms": None,
        "max_rss_bytes": None,
    }


def should_fallback_from_time_wrapper(proc: subprocess.CompletedProcess[str]) -> bool:
    stderr = proc.stderr.lower()
    return proc.returncode != 0 and (
        "operation not permitted" in stderr
        or "not permitted" in stderr
        or "cannot run" in stderr
    )


def parse_bsd_time(text: str) -> dict:
    stats: dict[str, float | int | None] = {
        "user_ms": None,
        "sys_ms": None,
        "max_rss_bytes": None,
    }
    for line in text.splitlines():
        line = line.strip()
        m = re.search(r"([0-9.]+)\s+real\s+([0-9.]+)\s+user\s+([0-9.]+)\s+sys", line)
        if m:
            stats["user_ms"] = round(float(m.group(2)) * 1000, 2)
            stats["sys_ms"] = round(float(m.group(3)) * 1000, 2)
            continue
        m = re.search(r"(\d+)\s+maximum resident set size", line)
        if m:
            # On macOS/BSD `time -l`, this value is reported in bytes.
            stats["max_rss_bytes"] = int(m.group(1))
    return stats


def parse_gnu_time(text: str) -> dict:
    stats: dict[str, float | int | None] = {
        "user_ms": None,
        "sys_ms": None,
        "max_rss_bytes": None,
    }
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("User time (seconds):"):
            stats["user_ms"] = round(float(line.split(":", 1)[1].strip()) * 1000, 2)
        elif line.startswith("System time (seconds):"):
            stats["sys_ms"] = round(float(line.split(":", 1)[1].strip()) * 1000, 2)
        elif line.startswith("Maximum resident set size"):
            stats["max_rss_bytes"] = int(line.split(":", 1)[1].strip()) * 1024
    return stats


def git_rev_parse_short(repo: Path) -> str | None:
    proc = subprocess.run(
        ["git", "rev-parse", "--short", "HEAD"],
        cwd=repo,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        return None
    return proc.stdout.strip() or None


def git_remote_origin(repo: Path) -> str:
    proc = subprocess.run(
        ["git", "remote", "get-url", "origin"],
        cwd=repo,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        return ""
    return proc.stdout.strip()


def count_files_with_rg(repo: Path) -> int:
    rg = shutil.which("rg")
    if not rg:
        raise SystemExit("error: rg (ripgrep) not found in PATH")
    proc = subprocess.run([rg, "--files", "."], cwd=repo, capture_output=True, text=True)
    if proc.returncode != 0:
        return 0
    return len([line for line in proc.stdout.splitlines() if line.strip()])


def directory_size_bytes(path: Path) -> int:
    if not path.exists():
        return 0
    total = 0
    for root, dirs, files in os.walk(path):
        if ".git" in dirs:
            dirs.remove(".git")
        for name in files:
            try:
                total += (Path(root) / name).stat().st_size
            except OSError:
                pass
    return total


def stop_daemon(binary: Path, repo: Path, collie_root: Path) -> None:
    subprocess.run(
        [str(binary), "stop", str(repo)],
        cwd=collie_root,
        capture_output=True,
        text=True,
    )


def reset_collie_state(binary: Path, repo: Path, collie_root: Path) -> None:
    subprocess.run(
        [str(binary), "clean", str(repo)],
        cwd=collie_root,
        capture_output=True,
        text=True,
    )


def active_tantivy_dir(collie_dir: Path) -> Path:
    current = collie_dir / "CURRENT"
    if current.is_file():
        gen_name = current.read_text(encoding="utf-8").strip()
        if gen_name:
            gen_tantivy = collie_dir / "generations" / gen_name / "tantivy"
            if gen_tantivy.is_dir():
                return gen_tantivy
    return collie_dir / "tantivy"


def status_index_dir(repo: Path, status: dict | None) -> Path:
    if status:
        index_path = status.get("index_path")
        if index_path:
            return Path(index_path)
    return repo / ".collie"


def state_dir_size_bytes(repo: Path, status: dict | None) -> int:
    return directory_size_bytes(status_index_dir(repo, status))


def active_generation_tantivy_size_bytes(repo: Path, status: dict | None) -> int:
    return directory_size_bytes(active_tantivy_dir(status_index_dir(repo, status)))


def load_status(binary: Path, repo: Path, collie_root: Path) -> dict | None:
    proc = subprocess.run(
        [str(binary), "status", str(repo), "--json"],
        cwd=collie_root,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        return None
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError:
        return None


def enrich_command_metric(metric: dict, index_size_bytes: int, status: dict | None) -> dict:
    enriched = dict(metric)
    enriched["index_size_bytes"] = index_size_bytes
    enriched["status"] = status
    return enriched


def measure_queries(
    binary: Path,
    repo: Path,
    profile: dict,
    runs_per_query: int,
    warmups: int,
    time_tool: dict,
    rg_path: str | None,
) -> dict:
    lexical = []
    for query in profile["lexical_queries"]:
        for _ in range(warmups):
            subprocess.run(
                [str(binary), "search", query, "--no-snippets", "-n", "50"],
                cwd=repo,
                capture_output=True,
                text=True,
            )
            if rg_path:
                subprocess.run(
                    [rg_path, "-il", collie_query_to_rg(query), "."],
                    cwd=repo,
                    capture_output=True,
                    text=True,
                )

        collie_runs = [
            summarize_run(
                timed_run(
                    [str(binary), "search", query, "--no-snippets", "-n", "50"],
                    cwd=repo,
                    time_tool=time_tool,
                ),
                parse_collie_count,
            )
            for _ in range(runs_per_query)
        ]

        rg_runs = None
        if rg_path:
            rg_runs = [
                summarize_run(
                    timed_run(
                        [rg_path, "-il", collie_query_to_rg(query), "."],
                        cwd=repo,
                        time_tool=time_tool,
                    ),
                    parse_rg_count,
                )
                for _ in range(runs_per_query)
            ]

        lexical.append(
            {
                "query": query,
                "query_type": query_type(query),
                "collie": summarize_runs(collie_runs),
                "rg": summarize_runs(rg_runs) if rg_runs is not None else None,
            }
        )

    symbol = []
    for query in profile["symbol_queries"]:
        for _ in range(warmups):
            subprocess.run(
                [str(binary), "search", query, "--no-snippets", "-n", "50"],
                cwd=repo,
                capture_output=True,
                text=True,
            )

        runs = [
            summarize_run(
                timed_run(
                    [str(binary), "search", query, "--no-snippets", "-n", "50"],
                    cwd=repo,
                    time_tool=time_tool,
                ),
                parse_collie_count,
            )
            for _ in range(runs_per_query)
        ]
        symbol.append(
            {
                "query": query,
                "collie": summarize_runs(runs),
            }
        )

    regex = []
    for query in profile.get("regex_queries", []):
        for _ in range(warmups):
            subprocess.run(
                [str(binary), "search", query, "--regex", "--no-snippets", "-n", "50"],
                cwd=repo,
                capture_output=True,
                text=True,
            )
            if rg_path:
                subprocess.run(
                    [rg_path, "-l", query, "."],
                    cwd=repo,
                    capture_output=True,
                    text=True,
                )

        collie_runs = [
            summarize_run(
                timed_run(
                    [str(binary), "search", query, "--regex", "--no-snippets", "-n", "50"],
                    cwd=repo,
                    time_tool=time_tool,
                ),
                parse_collie_count,
            )
            for _ in range(runs_per_query)
        ]

        rg_runs = None
        if rg_path:
            rg_runs = [
                summarize_run(
                    timed_run(
                        [rg_path, "-l", query, "."],
                        cwd=repo,
                        time_tool=time_tool,
                    ),
                    parse_rg_count,
                )
                for _ in range(runs_per_query)
            ]

        regex.append(
            {
                "query": query,
                "collie": summarize_runs(collie_runs),
                "rg": summarize_runs(rg_runs) if rg_runs is not None else None,
            }
        )

    return {
        "lexical": lexical,
        "symbol": symbol,
        "regex": regex,
    }


def summarize_run(metric: dict, count_parser) -> dict:
    return {
        "wall_ms": metric["wall_ms"],
        "user_ms": metric["user_ms"],
        "sys_ms": metric["sys_ms"],
        "max_rss_bytes": metric["max_rss_bytes"],
        "returncode": metric["returncode"],
        "result_count": count_parser(metric["stdout"]),
    }


def summarize_runs(runs: list[dict] | None) -> dict | None:
    if runs is None:
        return None
    walls = [run["wall_ms"] for run in runs]
    users = [run["user_ms"] for run in runs if run["user_ms"] is not None]
    systems = [run["sys_ms"] for run in runs if run["sys_ms"] is not None]
    rss = [run["max_rss_bytes"] for run in runs if run["max_rss_bytes"] is not None]
    counts = [run["result_count"] for run in runs]
    return {
        "runs": runs,
        "summary": {
            "count": len(runs),
            "wall_ms": basic_stats(walls),
            "user_ms": basic_stats(users) if users else None,
            "sys_ms": basic_stats(systems) if systems else None,
            "max_rss_bytes": basic_stats(rss) if rss else None,
            "result_count_min": min(counts) if counts else None,
            "result_count_max": max(counts) if counts else None,
        },
    }


def basic_stats(values: list[float | int]) -> dict:
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


def collie_query_to_rg(query: str) -> str:
    if query.startswith("%") and query.endswith("%"):
        return query.strip("%")
    if query.endswith("%"):
        return r"\b" + query.rstrip("%")
    if query.startswith("%"):
        return query.lstrip("%") + r"\b"
    return r"\b" + query + r"\b"


def query_type(query: str) -> str:
    if query.startswith("%") and query.endswith("%"):
        return "substring"
    if query.endswith("%"):
        return "prefix"
    if query.startswith("%"):
        return "suffix"
    return "exact"


def parse_collie_count(stdout: str) -> int:
    for line in stdout.splitlines():
        match = re.match(r"Found (\d+) (results?|symbols|file\(s\) with matches) for", line)
        if match:
            return int(match.group(1))
    return 0


def parse_rg_count(stdout: str) -> int:
    return len([line for line in stdout.splitlines() if line.strip()])


def measure_incremental(
    binary: Path,
    collie_root: Path,
    repo: Path,
    profile: dict,
    poll_interval_ms: int,
    timeout_seconds: float,
) -> dict:
    relative_file = resolve_incremental_file(repo, profile["incremental_candidates"])
    if relative_file is None:
        return {"available": False, "reason": "no incremental candidate file found"}

    file_path = repo / relative_file
    original = file_path.read_bytes()
    token = f"bench_prod_{uuid.uuid4().hex}"
    marker = build_marker(file_path.suffix.lower(), token)

    add_ms = None
    remove_ms = None
    error = None
    try:
        file_path.write_bytes(original + marker)
        add_ms = poll_for_query(
            binary=binary,
            repo=repo,
            query=token,
            want_present=True,
            poll_interval_ms=poll_interval_ms,
            timeout_seconds=timeout_seconds,
        )

        file_path.write_bytes(original)
        remove_ms = poll_for_query(
            binary=binary,
            repo=repo,
            query=token,
            want_present=False,
            poll_interval_ms=poll_interval_ms,
            timeout_seconds=timeout_seconds,
        )
    except Exception as exc:
        error = str(exc)
    finally:
        if file_path.exists() and file_path.read_bytes() != original:
            file_path.write_bytes(original)

    status = load_status(binary, repo, collie_root)
    return {
        "available": error is None,
        "file": str(relative_file),
        "token": token,
        "add_ms": add_ms,
        "remove_ms": remove_ms,
        "status_after": status,
        "error": error,
    }


def resolve_incremental_file(repo: Path, candidates: list[str]) -> Path | None:
    for candidate in candidates:
        path = repo / candidate
        if path.is_file():
            return Path(candidate)

    rg = shutil.which("rg")
    if not rg:
        return None

    for ext in [".rs", ".go", ".py", ".js", ".ts"]:
        proc = subprocess.run(
            [rg, "--files", "-g", f"*{ext}", "--max-depth", "4", "."],
            cwd=repo,
            capture_output=True,
            text=True,
        )
        if proc.returncode != 0:
            continue
        files = [
            line.strip().removeprefix("./")
            for line in proc.stdout.splitlines()
            if line.strip()
        ]
        files = [
            rel
            for rel in files
            if not any(
                blocked in rel
                for blocked in ["/vendor/", "/generated/", "/_output/", "/third_party/"]
            )
        ]
        if files:
            return Path(files[len(files) // 2])
    return None


def build_marker(suffix: str, token: str) -> bytes:
    comment = COMMENT_BY_SUFFIX.get(suffix, "//")
    if comment == "/*":
        return f"\n/* {token} */\n".encode()
    if comment == "<!--":
        return f"\n<!-- {token} -->\n".encode()
    return f"\n{comment} {token}\n".encode()


def poll_for_query(
    binary: Path,
    repo: Path,
    query: str,
    want_present: bool,
    poll_interval_ms: int,
    timeout_seconds: float,
) -> float | None:
    deadline = time.perf_counter() + timeout_seconds
    start = time.perf_counter()
    while time.perf_counter() < deadline:
        proc = subprocess.run(
            [str(binary), "search", query, "--no-snippets", "-n", "5"],
            cwd=repo,
            capture_output=True,
            text=True,
        )
        count = parse_collie_count(proc.stdout)
        if want_present and count >= 1:
            return round((time.perf_counter() - start) * 1000, 2)
        if not want_present and count == 0:
            return round((time.perf_counter() - start) * 1000, 2)
        time.sleep(poll_interval_ms / 1000.0)
    return None


def print_summary(output_path: Path, report: dict) -> None:
    print("=" * 100)
    print("COLLIE PRODUCTION BENCHMARK")
    print(report["timestamp_utc"])
    print("=" * 100)
    print_kv("Artifact", str(output_path))

    target = report["target_repo"]
    print_section("Target")
    print_kv("Repo", target["path"])
    print_kv("Name", target["name"])
    print_kv("Tracked files", f"{target['tracked_files']:,}")
    print_kv("Repo size", format_bytes(target["size_bytes"]))

    profile = report["profile"]
    selection = report.get("profile_selection", {})
    print_section("Profile")
    print_kv("Key", profile["key"])
    print_kv("Description", profile["description"])
    selection_mode = selection.get("mode", "unknown")
    if selection_mode == "requested":
        print_kv("Selection", "requested")
    else:
        matched_by = selection.get("matched_by", "heuristic")
        print_kv("Selection", f"inferred via {matched_by}")

    metrics = report.get("metrics", {})
    rebuild = metrics.get("rebuild")
    print_section("Indexing")
    if rebuild:
        print_metric_row("Rebuild", rebuild)
    watch = metrics.get("watch_ready")
    if watch:
        print_metric_row("Watch ready", watch)
    artifacts = metrics.get("artifacts", {})
    if artifacts:
        print_section("Artifacts")
        print_kv("Binary", format_bytes(artifacts.get("binary_size_bytes", 0)))
        print_kv("Index", format_bytes(artifacts.get("collie_dir_size_bytes", 0)))
        print_kv(
            "Active tantivy",
            format_bytes(artifacts.get("active_generation_tantivy_size_bytes", 0)),
        )
    incremental = metrics.get("incremental_update")
    if incremental:
        print_section("Incremental")
        if incremental.get("available", True):
            print_kv("File", incremental.get("file", "n/a"))
            print_kv("Add", fmt_optional(incremental.get("add_ms")))
            print_kv("Remove", fmt_optional(incremental.get("remove_ms")))
            if incremental.get("error"):
                print_kv("Error", incremental["error"])
        else:
            print_kv("Status", incremental.get("reason", "unavailable"))

    query_metrics = metrics.get("queries")
    if query_metrics:
        print_section("Query snapshot")
        print_query_snapshot("Lexical", query_metrics.get("lexical", []), include_rg=True)
        print_query_snapshot("Symbol", query_metrics.get("symbol", []), include_rg=False)
        print_query_snapshot("Regex", query_metrics.get("regex", []), include_rg=True)

    if report.get("error"):
        print_section("Error")
        print(report["error"])


def print_section(title: str) -> None:
    print()
    print(title)
    print("-" * len(title))


def print_kv(label: str, value: str) -> None:
    print(f"{label:<14} {value}")


def print_metric_row(label: str, metric: dict) -> None:
    print_kv(label, f"{metric['wall_ms']:.2f}ms")
    print_kv(f"{label} user", fmt_optional(metric.get("user_ms")))
    print_kv(f"{label} sys", fmt_optional(metric.get("sys_ms")))
    print_kv(f"{label} rss", fmt_optional_bytes(metric.get("max_rss_bytes")))


def print_query_snapshot(title: str, entries: list[dict], include_rg: bool) -> None:
    if not entries:
        print_kv(title, "n/a")
        return

    print_kv(title, "")
    for entry in entries[:3]:
        query = entry.get("query", "")
        collie_p50 = summary_p50(entry.get("collie"))
        summary = f"{query} -> collie p50 {collie_p50}"
        if include_rg and entry.get("rg") is not None:
            summary += f", rg p50 {summary_p50(entry.get('rg'))}"
        print_kv("", summary)


def summary_p50(summary_block: dict | None) -> str:
    if not summary_block:
        return "n/a"
    summary = summary_block.get("summary", {})
    wall = summary.get("wall_ms", {})
    p50 = wall.get("p50")
    if p50 is None:
        return "n/a"
    return f"{p50}ms"


def fmt_optional(value, suffix: str = "ms") -> str:
    if value is None:
        return "n/a"
    if suffix:
        return f"{value}{suffix}"
    return str(value)


def format_bytes(num_bytes: int) -> str:
    if num_bytes >= 1024**3:
        return f"{num_bytes / (1024**3):.2f}GiB"
    if num_bytes >= 1024**2:
        return f"{num_bytes / (1024**2):.2f}MiB"
    if num_bytes >= 1024:
        return f"{num_bytes / 1024:.2f}KiB"
    return f"{num_bytes}B"


def fmt_optional_bytes(value) -> str:
    if value is None:
        return "n/a"
    return format_bytes(int(value))


if __name__ == "__main__":
    raise SystemExit(main())
