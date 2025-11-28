#!/usr/bin/env python3
"""
Benchmark agent-style code retrieval tasks.

This script complements the lexical search benchmark. It measures how quickly
different retrieval plans reach a correct file for a natural-language task.

Systems compared:
- collie_symbol: collie search with symbol-aware filters
- collie_lexical: collie search with plain lexical queries
- rg: ripgrep regex/file-search baseline

Usage:
    python3 scripts/benchmark_agentic.py /path/to/repo --repo-key collie-search
    python3 scripts/benchmark_agentic.py /path/to/repo --repo-key kubernetes
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path


@dataclass
class Task:
    id: str
    repo: str
    prompt: str
    expected_paths: list[str]
    collie_symbol_queries: list[str]
    collie_lexical_queries: list[str]
    rg_regex_queries: list[str]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Benchmark agentic retrieval tasks for Collie vs rg."
    )
    parser.add_argument(
        "repo",
        nargs="?",
        help="Path to the repository to benchmark. If omitted, --repo-key may supply a default repo.",
    )
    parser.add_argument(
        "--repo-key",
        help="Task repo key to use from benchmark-data/agentic_tasks.json. "
        "Examples: collie-search, kubernetes. If omitted, try to infer.",
    )
    parser.add_argument(
        "--tasks",
        help="Path to the task JSON file. Default: benchmark-data/agentic_tasks.json",
    )
    parser.add_argument(
        "--binary",
        help="Path to collie-search binary. Default: target/release/collie-search",
    )
    parser.add_argument(
        "--skip-build",
        action="store_true",
        help="Skip cargo build --release and use the existing binary.",
    )
    parser.add_argument(
        "--keep-daemon-running",
        action="store_true",
        help="Leave the Collie daemon running after the benchmark.",
    )
    parser.add_argument(
        "--output",
        help="Write the JSON report here. Default: benchmark-results/<timestamp>-agentic-<repo>.json",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    script_dir = Path(__file__).resolve().parent
    collie_root = script_dir.parent
    repo = resolve_target_repo(collie_root, args.repo, args.repo_key)
    if not repo.exists():
        print(f"error: repo not found: {repo}", file=sys.stderr)
        return 1

    tasks_path = (
        Path(args.tasks).expanduser().resolve()
        if args.tasks
        else collie_root / "benchmark-data" / "agentic_tasks.json"
    )
    suite = load_tasks(tasks_path)

    repo_key = args.repo_key or infer_repo_key(repo)
    if not repo_key:
        print(
            "error: could not infer --repo-key; pass one explicitly",
            file=sys.stderr,
        )
        return 1

    tasks = [task for task in suite if task.repo == repo_key]
    if not tasks:
        print(f"error: no tasks found for repo key {repo_key}", file=sys.stderr)
        return 1

    binary = resolve_binary(collie_root, args.binary, args.skip_build)
    if not shutil.which("rg"):
        print("error: rg (ripgrep) not found in PATH", file=sys.stderr)
        return 1

    ensure_index_ready(binary, repo)

    report = {
        "timestamp_utc": datetime.now(timezone.utc).isoformat(),
        "repo_key": repo_key,
        "repo_path": str(repo),
        "tasks_path": str(tasks_path),
        "systems": ["collie_symbol", "collie_lexical", "rg"],
        "tasks": [],
    }

    for task in tasks:
        report["tasks"].append(run_task(binary, repo, task))

    report["summary"] = summarize(report["tasks"])
    output_path = resolve_output_path(collie_root, repo, args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print_summary(output_path, report)

    if not args.keep_daemon_running:
        subprocess.run(
            [str(binary), "stop", str(repo)],
            cwd=collie_root,
            capture_output=True,
            text=True,
        )

    return 0


def load_tasks(path: Path) -> list[Task]:
    data = json.loads(path.read_text(encoding="utf-8"))
    tasks = []
    for raw in data["tasks"]:
        tasks.append(Task(**raw))
    return tasks


def infer_repo_key(repo: Path) -> str | None:
    if repo.name == "collie-search":
        return "collie-search"
    origin = git_remote_origin(repo)
    if "kubernetes/kubernetes" in origin:
        return "kubernetes"
    if repo.name == "large-repo-for-test":
        return "kubernetes"
    return None


def resolve_target_repo(collie_root: Path, repo_arg: str | None, repo_key: str | None) -> Path:
    explicit_repo = Path(repo_arg).expanduser().resolve() if repo_arg else None
    if repo_key is None:
        if explicit_repo is None:
            raise SystemExit("error: repo is required when --repo-key is omitted")
        return explicit_repo

    default_repo = default_repo_for_key(collie_root, repo_key)
    if explicit_repo is not None:
        if default_repo is not None and explicit_repo != default_repo:
            raise SystemExit(
                f"error: repo key '{repo_key}' targets {default_repo}, "
                f"but repo argument points to {explicit_repo}. "
                "Omit the repo argument or pass the matching repo."
            )
        return explicit_repo

    if default_repo is not None:
        return default_repo

    raise SystemExit(
        f"error: repo key '{repo_key}' does not define a default repo; pass a repo path explicitly"
    )


def default_repo_for_key(collie_root: Path, repo_key: str) -> Path | None:
    if repo_key == "collie-search":
        return collie_root.resolve()
    if repo_key == "kubernetes":
        return (collie_root.parent / "large-repo-for-test").resolve()
    return None


def resolve_binary(collie_root: Path, binary_arg: str | None, skip_build: bool) -> Path:
    binary = (
        Path(binary_arg).expanduser().resolve()
        if binary_arg
        else collie_root / "target" / "release" / "collie-search"
    )
    if not skip_build:
        subprocess.run(
            ["cargo", "build", "--release", "--bin", "collie-search"],
            cwd=collie_root,
            check=True,
        )
    if not binary.exists():
        raise SystemExit(f"error: collie binary not found at {binary}")
    return binary


def ensure_index_ready(binary: Path, repo: Path) -> None:
    # Rebuild to ensure the latest indexer code (including symbol extraction)
    # is used. No daemon needed — search works from the persisted index.
    subprocess.run(
        [str(binary), "rebuild", str(repo)],
        cwd=repo,
        capture_output=True,
        text=True,
        check=True,
    )


def run_task(binary: Path, repo: Path, task: Task) -> dict:
    systems = {
        "collie_symbol": run_collie_plan(binary, repo, task.collie_symbol_queries, task.expected_paths),
        "collie_lexical": run_collie_plan(binary, repo, task.collie_lexical_queries, task.expected_paths),
        "rg": run_rg_plan(repo, task.rg_regex_queries, task.expected_paths),
    }
    return {
        "id": task.id,
        "repo": task.repo,
        "prompt": task.prompt,
        "expected_paths": task.expected_paths,
        "systems": systems,
    }


def run_collie_plan(binary: Path, repo: Path, queries: list[str], expected_paths: list[str]) -> dict:
    results = []
    elapsed_total = 0.0
    candidates_total = 0
    hit_query_index = None

    for idx, query in enumerate(queries, start=1):
        start = time.perf_counter()
        proc = subprocess.run(
            [str(binary), "search", query, "--no-snippets", "-n", "50"],
            cwd=repo,
            capture_output=True,
            text=True,
        )
        elapsed_ms = round((time.perf_counter() - start) * 1000, 2)
        elapsed_total += elapsed_ms
        candidate_count = parse_collie_count(proc.stdout)
        candidates_total += candidate_count
        hit = output_contains_expected_path(proc.stdout, expected_paths)
        results.append(
            {
                "query": query,
                "elapsed_ms": elapsed_ms,
                "candidate_count": candidate_count,
                "hit": hit,
                "returncode": proc.returncode,
            }
        )
        if hit and hit_query_index is None:
            hit_query_index = idx
            break

    return finalize_plan(results, elapsed_total, candidates_total, hit_query_index)


def run_rg_plan(repo: Path, patterns: list[str], expected_paths: list[str]) -> dict:
    results = []
    elapsed_total = 0.0
    candidates_total = 0
    hit_query_index = None

    for idx, pattern in enumerate(patterns, start=1):
        start = time.perf_counter()
        proc = subprocess.run(
            ["rg", "-l", "-i", "-e", pattern, "."],
            cwd=repo,
            capture_output=True,
            text=True,
        )
        elapsed_ms = round((time.perf_counter() - start) * 1000, 2)
        elapsed_total += elapsed_ms
        lines = [line.strip().removeprefix("./") for line in proc.stdout.splitlines() if line.strip()]
        candidate_count = len(lines)
        candidates_total += candidate_count
        hit = any(expected in lines for expected in expected_paths)
        results.append(
            {
                "query": pattern,
                "elapsed_ms": elapsed_ms,
                "candidate_count": candidate_count,
                "hit": hit,
                "returncode": proc.returncode,
            }
        )
        if hit and hit_query_index is None:
            hit_query_index = idx
            break

    return finalize_plan(results, elapsed_total, candidates_total, hit_query_index)


def finalize_plan(results: list[dict], elapsed_total: float, candidates_total: int, hit_query_index: int | None) -> dict:
    return {
        "success": hit_query_index is not None,
        "queries_run": len(results),
        "first_hit_query_index": hit_query_index,
        "time_to_first_hit_ms": round(elapsed_total, 2) if hit_query_index is not None else None,
        "candidates_until_hit": candidates_total if hit_query_index is not None else None,
        "queries": results,
    }


def parse_collie_count(stdout: str) -> int:
    for line in stdout.splitlines():
        match = re.match(r"Found (\d+) (results|symbols) for(?::| pattern:)", line)
        if match:
            return int(match.group(1))
    return 0


def output_contains_expected_path(stdout: str, expected_paths: list[str]) -> bool:
    return any(expected in stdout for expected in expected_paths)


def summarize(task_results: list[dict]) -> dict:
    systems = ["collie_symbol", "collie_lexical", "rg"]
    summary = {}
    for system in systems:
        successes = 0
        total_time = 0.0
        total_candidates = 0
        completed = 0
        for task in task_results:
            result = task["systems"][system]
            if result["success"]:
                successes += 1
                total_time += result["time_to_first_hit_ms"]
                total_candidates += result["candidates_until_hit"]
                completed += 1
        summary[system] = {
            "successes": successes,
            "task_count": len(task_results),
            "avg_time_to_first_hit_ms": round(total_time / completed, 2) if completed else None,
            "avg_candidates_until_hit": round(total_candidates / completed, 2) if completed else None,
        }
    return summary


def resolve_output_path(collie_root: Path, repo: Path, output_arg: str | None) -> Path:
    if output_arg:
        return Path(output_arg).expanduser().resolve()
    timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    slug = re.sub(r"[^a-zA-Z0-9._-]+", "-", repo.name).strip("-") or "repo"
    return collie_root / "benchmark-results" / f"{timestamp}-agentic-{slug}.json"


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


def print_summary(output_path: Path, report: dict) -> None:
    print("=" * 100)
    print("COLLIE AGENTIC RETRIEVAL BENCHMARK")
    print(report["timestamp_utc"])
    print("=" * 100)
    print(f"Repo: {report['repo_path']} ({report['repo_key']})")
    print(f"Tasks: {len(report['tasks'])}")
    print()
    print(f"{'System':<18} {'Success':>9} {'Avg first hit':>15} {'Avg candidates':>17}")
    for system, summary in report["summary"].items():
        avg_time = (
            f"{summary['avg_time_to_first_hit_ms']:.2f}ms"
            if summary["avg_time_to_first_hit_ms"] is not None
            else "n/a"
        )
        avg_candidates = (
            f"{summary['avg_candidates_until_hit']:.2f}"
            if summary["avg_candidates_until_hit"] is not None
            else "n/a"
        )
        print(
            f"{system:<18} "
            f"{summary['successes']:>2}/{summary['task_count']:<6} "
            f"{avg_time:>15} "
            f"{avg_candidates:>17}"
        )
    print()
    print(f"Saved benchmark result to {output_path}")


if __name__ == "__main__":
    raise SystemExit(main())
