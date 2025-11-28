#!/usr/bin/env python3

import argparse
import json
import os
import re
import shutil
import statistics
import subprocess
import sys
import time
import uuid
from datetime import datetime, timezone
from pathlib import Path


DEFAULT_QUERIES = [
    "kubelet",
    "SharedInformerFactory",
    "CustomResourceDefinition",
    "kube%",
    "%informer%",
]

RESET_STATE_FILES = [
    "index.mmap",
    "index.mmap.tmp",
    "index.mmap.rebuild",
    "index.rebuild",
    "collie.pid",
    "daemon-state.json",
    "daemon-state.json.tmp",
    "daemon.log",
    "CURRENT",
    "CURRENT.tmp",
    "tantivy",
    "generations",
]

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
        description="Run a repeatable Collie benchmark against a repository."
    )
    parser.add_argument("repo", help="Path to the repository to benchmark.")
    parser.add_argument(
        "--query",
        dest="queries",
        action="append",
        default=[],
        help="Search query to benchmark. May be passed multiple times.",
    )
    parser.add_argument(
        "--search-runs",
        type=int,
        default=3,
        help="How many times to run each query. Default: 3.",
    )
    parser.add_argument(
        "--poll-interval-ms",
        type=int,
        default=200,
        help="Polling interval for watcher propagation checks. Default: 200.",
    )
    parser.add_argument(
        "--timeout-seconds",
        type=float,
        default=120.0,
        help="Timeout for watcher propagation checks. Default: 120.",
    )
    parser.add_argument(
        "--incremental-file",
        help="Relative path of the file to mutate for watcher latency checks.",
    )
    parser.add_argument(
        "--binary",
        help="Path to a prebuilt collie-search binary. Default: target/release/collie-search.",
    )
    parser.add_argument(
        "--skip-build",
        action="store_true",
        help="Skip cargo build --release and use the existing binary.",
    )
    parser.add_argument(
        "--keep-daemon-running",
        action="store_true",
        help="Leave the daemon running after the benchmark completes.",
    )
    parser.add_argument(
        "--output",
        help="Where to write the JSON result. Default: benchmark-results/<timestamp>-<repo>.json",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    script_path = Path(__file__).resolve()
    collie_root = script_path.parent.parent
    target_repo = Path(args.repo).expanduser().resolve()
    if not target_repo.exists():
        print(f"error: repo does not exist: {target_repo}", file=sys.stderr)
        return 1
    if not target_repo.joinpath(".git").exists():
        print(f"error: repo is not a git repository: {target_repo}", file=sys.stderr)
        return 1
    if args.search_runs < 1:
        print("error: --search-runs must be >= 1", file=sys.stderr)
        return 1

    queries = args.queries or list(DEFAULT_QUERIES)
    binary = resolve_binary(collie_root, args.binary, args.skip_build)
    output_path = resolve_output_path(collie_root, target_repo, args.output)

    stop_daemon(binary, target_repo)
    reset_collie_state(target_repo)

    result = {
        "timestamp_utc": datetime.now(timezone.utc).isoformat(),
        "collie_repo": {
            "path": str(collie_root),
            "git_commit": git_rev_parse_short(collie_root),
        },
        "target_repo": {
            "path": str(target_repo),
            "git_commit": git_rev_parse_short(target_repo),
            "origin": git_remote_origin(target_repo),
            "tracked_files": count_files_with_rg(target_repo),
            "size_bytes": directory_size_bytes(target_repo),
        },
        "config": {
            "queries": queries,
            "search_runs": args.search_runs,
            "poll_interval_ms": args.poll_interval_ms,
            "timeout_seconds": args.timeout_seconds,
        },
        "metrics": {},
    }

    exit_code = 0
    try:
        cold_start = timed_run(
            [str(binary), "watch", str(target_repo)],
            cwd=collie_root,
        )
        status = load_status(binary, target_repo, collie_root)
        collie_dir = target_repo / ".collie"
        index_size = directory_size_bytes(collie_dir)
        result["metrics"] = {
            "cold_start_ms": cold_start["elapsed_ms"],
            "watch_stdout": cold_start["stdout"],
            "watch_stderr": cold_start["stderr"],
            "index_size_bytes": index_size,
            "status": status,
            "files_per_second": round(
                status.get("total_files", 0) / max(cold_start["elapsed_ms"] / 1000.0, 0.001),
                2,
            ),
            "searches": [],
        }

        search_metrics = []
        for query in queries:
            durations = []
            result_count = None
            for _ in range(args.search_runs):
                run = timed_run(
                    [str(binary), "search", query, "--no-snippets", "-n", "20"],
                    cwd=target_repo,
                )
                durations.append(run["elapsed_ms"])
                result_count = parse_search_result_count(run["stdout"])
            search_metrics.append(
                {
                    "query": query,
                    "runs_ms": durations,
                    "avg_ms": round(statistics.fmean(durations), 2),
                    "min_ms": min(durations),
                    "max_ms": max(durations),
                    "result_count": result_count,
                }
            )
        result["metrics"]["searches"] = search_metrics

        incremental_file = resolve_incremental_file(target_repo, args.incremental_file)
        incremental = measure_incremental_latency(
            binary=binary,
            collie_root=collie_root,
            target_repo=target_repo,
            relative_file=incremental_file,
            poll_interval_ms=args.poll_interval_ms,
            timeout_seconds=args.timeout_seconds,
        )
        result["metrics"]["incremental_update"] = incremental
    except Exception as exc:
        exit_code = 1
        result["error"] = str(exc)
    finally:
        if not args.keep_daemon_running:
            stop_daemon(binary, target_repo)

    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print_summary(output_path, result)
    return exit_code


def resolve_binary(collie_root: Path, binary_arg: str | None, skip_build: bool) -> Path:
    if binary_arg:
        binary = Path(binary_arg).expanduser().resolve()
    else:
        binary = collie_root / "target" / "release" / "collie-search"
    if not skip_build:
        subprocess.run(
            ["cargo", "build", "--release", "--bin", "collie-search"],
            cwd=collie_root,
            check=True,
        )
    if not binary.exists():
        raise SystemExit(f"error: collie binary not found at {binary}")
    return binary


def resolve_output_path(collie_root: Path, target_repo: Path, output_arg: str | None) -> Path:
    if output_arg:
        return Path(output_arg).expanduser().resolve()
    slug = re.sub(r"[^a-zA-Z0-9._-]+", "-", target_repo.name).strip("-") or "repo"
    timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    return collie_root / "benchmark-results" / f"{timestamp}-{slug}.json"


def timed_run(cmd: list[str], cwd: Path) -> dict[str, object]:
    start = time.perf_counter()
    completed = subprocess.run(
        cmd,
        cwd=cwd,
        check=True,
        capture_output=True,
        text=True,
    )
    elapsed_ms = round((time.perf_counter() - start) * 1000, 2)
    return {
        "elapsed_ms": elapsed_ms,
        "stdout": completed.stdout.strip(),
        "stderr": completed.stderr.strip(),
    }


def load_status(binary: Path, target_repo: Path, collie_root: Path) -> dict[str, object]:
    completed = subprocess.run(
        [str(binary), "status", str(target_repo), "--json"],
        cwd=collie_root,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(completed.stdout)


def stop_daemon(binary: Path, target_repo: Path) -> None:
    subprocess.run(
        [str(binary), "stop", str(target_repo)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )


def reset_collie_state(target_repo: Path) -> None:
    collie_dir = target_repo / ".collie"
    if not collie_dir.exists():
        return
    for name in RESET_STATE_FILES:
        path = collie_dir / name
        if path.exists():
            if path.is_dir():
                shutil.rmtree(path, ignore_errors=True)
            else:
                path.unlink(missing_ok=True)


def count_files_with_rg(repo: Path) -> int:
    rg = shutil.which("rg")
    if rg:
        completed = subprocess.run(
            [rg, "--files", str(repo)],
            check=True,
            capture_output=True,
            text=True,
        )
        return sum(1 for line in completed.stdout.splitlines() if line.strip())
    count = 0
    for root, dirs, files in os.walk(repo):
        if ".git" in dirs:
            dirs.remove(".git")
        count += len(files)
    return count


def directory_size_bytes(repo: Path) -> int:
    total = 0
    for root, dirs, files in os.walk(repo):
        if ".git" in dirs:
            dirs.remove(".git")
        for file_name in files:
            path = Path(root) / file_name
            try:
                total += path.stat().st_size
            except OSError:
                pass
    return total


def git_rev_parse_short(repo: Path) -> str | None:
    completed = subprocess.run(
        ["git", "-C", str(repo), "rev-parse", "--short", "HEAD"],
        check=False,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        return None
    return completed.stdout.strip() or None


def git_remote_origin(repo: Path) -> str | None:
    completed = subprocess.run(
        ["git", "-C", str(repo), "remote", "get-url", "origin"],
        check=False,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        return None
    return completed.stdout.strip() or None


def parse_search_result_count(stdout: str) -> int:
    match = re.search(r"Found (\d+) results? for pattern:", stdout)
    if match:
        return int(match.group(1))
    if "No results found for pattern:" in stdout:
        return 0
    return -1


def resolve_incremental_file(repo: Path, file_arg: str | None) -> str:
    if file_arg:
        path = repo / file_arg
        if not path.is_file():
            raise SystemExit(f"error: incremental file does not exist: {path}")
        return file_arg

    rg = shutil.which("rg")
    if rg:
        completed = subprocess.run(
            [
                rg,
                "--files",
                ".",
                "-g",
                "*.rs",
                "-g",
                "*.go",
                "-g",
                "*.py",
                "-g",
                "*.ts",
                "-g",
                "*.tsx",
                "-g",
                "*.js",
                "-g",
                "*.jsx",
            ],
            cwd=repo,
            check=True,
            capture_output=True,
            text=True,
        )
        for line in completed.stdout.splitlines():
            relative = line.strip()
            if relative:
                return relative

    for path in repo.rglob("*"):
        if path.is_file() and path.suffix.lower() in COMMENT_BY_SUFFIX:
            return str(path.relative_to(repo))

    raise SystemExit("error: could not find a file to use for watcher latency checks")


def measure_incremental_latency(
    *,
    binary: Path,
    collie_root: Path,
    target_repo: Path,
    relative_file: str,
    poll_interval_ms: int,
    timeout_seconds: float,
) -> dict[str, object]:
    file_path = target_repo / relative_file
    original_bytes = file_path.read_bytes()
    token = f"collie_benchmark_token_{uuid.uuid4().hex}"
    comment = comment_prefix(file_path.suffix.lower())
    if comment == "/*":
        marker_line = f"\n/* {token} */\n".encode("utf-8")
    elif comment == "<!--":
        marker_line = f"\n<!-- {token} -->\n".encode("utf-8")
    else:
        marker_line = f"\n{comment} {token}\n".encode("utf-8")

    add_visible_ms = None
    remove_visible_ms = None

    try:
        file_path.write_bytes(original_bytes + marker_line)
        add_visible_ms = poll_for_query_count(
            binary=binary,
            collie_root=collie_root,
            target_repo=target_repo,
            query=token,
            expected_min_count=1,
            poll_interval_ms=poll_interval_ms,
            timeout_seconds=timeout_seconds,
        )

        file_path.write_bytes(original_bytes)
        remove_visible_ms = poll_for_query_count(
            binary=binary,
            collie_root=collie_root,
            target_repo=target_repo,
            query=token,
            expected_min_count=0,
            poll_interval_ms=poll_interval_ms,
            timeout_seconds=timeout_seconds,
            exact_zero=True,
        )
    finally:
        if file_path.read_bytes() != original_bytes:
            file_path.write_bytes(original_bytes)

    return {
        "file": relative_file,
        "token": token,
        "add_visible_ms": add_visible_ms,
        "remove_visible_ms": remove_visible_ms,
    }


def poll_for_query_count(
    *,
    binary: Path,
    collie_root: Path,
    target_repo: Path,
    query: str,
    expected_min_count: int,
    poll_interval_ms: int,
    timeout_seconds: float,
    exact_zero: bool = False,
) -> float:
    deadline = time.perf_counter() + timeout_seconds
    start = time.perf_counter()
    while time.perf_counter() < deadline:
        status = load_status(binary, target_repo, collie_root)
        if status.get("status") != "Running":
            raise RuntimeError(
                "daemon stopped during watcher benchmark: "
                f"{status.get('reason', 'unknown reason')}"
            )
        completed = subprocess.run(
            [str(binary), "search", query, "--no-snippets", "-n", "20"],
            cwd=target_repo,
            check=True,
            capture_output=True,
            text=True,
        )
        count = parse_search_result_count(completed.stdout)
        if exact_zero:
            if count == 0:
                return round((time.perf_counter() - start) * 1000, 2)
        elif count >= expected_min_count:
            return round((time.perf_counter() - start) * 1000, 2)
        time.sleep(poll_interval_ms / 1000.0)
    raise RuntimeError(f"timed out waiting for watcher propagation of query {query!r}")


def comment_prefix(suffix: str) -> str:
    return COMMENT_BY_SUFFIX.get(suffix, "//")


def print_summary(output_path: Path, result: dict[str, object]) -> None:
    if "error" in result:
        print(f"Saved benchmark result to {output_path}")
        print(f"Benchmark failed: {result['error']}")
        return

    metrics = result["metrics"]
    target = result["target_repo"]
    status = metrics["status"]

    print(f"Saved benchmark result to {output_path}")
    print(
        "Target repo: "
        f"{target['path']} "
        f"({target.get('git_commit') or 'unknown commit'}, {target['tracked_files']} files)"
    )
    print(
        "Cold start: "
        f"{metrics['cold_start_ms']} ms, "
        f"{status.get('total_files', 0)} indexed files, "
        f"{metrics['index_size_bytes']} byte index"
    )
    if status.get("skipped_files", 0):
        print(f"Skipped files during rebuild: {status['skipped_files']}")
    print("Queries:")
    for search in metrics["searches"]:
        print(
            f"  {search['query']}: "
            f"avg {search['avg_ms']} ms "
            f"(min {search['min_ms']}, max {search['max_ms']}), "
            f"results {search['result_count']}"
        )
    incremental = metrics["incremental_update"]
    print(
        "Incremental watcher: "
        f"{incremental['file']} "
        f"(add {incremental['add_visible_ms']} ms, "
        f"remove {incremental['remove_visible_ms']} ms)"
    )


if __name__ == "__main__":
    sys.exit(main())
