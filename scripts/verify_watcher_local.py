#!/usr/bin/env python3
"""
Verify Collie's watcher behavior from a normal local terminal.

This script is meant to be run by a human outside the agent environment.
It automates:
1. create or use a git repo
2. start `collie watch`
3. mutate a source file
4. poll `status --json` and `search`
5. report whether the watcher observed the edit

Usage:
    python3 scripts/verify_watcher_local.py
    python3 scripts/verify_watcher_local.py --repo /path/to/repo
    python3 scripts/verify_watcher_local.py --repo /path/to/repo --file src/main.rs
"""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import tempfile
import time
import uuid
from dataclasses import dataclass
from pathlib import Path


DEFAULT_SETTLE_SECONDS = 1.0
DEFAULT_TIMEOUT_SECONDS = 15.0
DEFAULT_POLL_MS = 100

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


@dataclass
class RepoSetup:
    repo: Path
    file_path: Path
    created_temp_repo: bool
    temp_dir: tempfile.TemporaryDirectory[str] | None


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Verify Collie's watcher behavior outside the agent environment."
    )
    parser.add_argument(
        "--repo",
        help="Existing git repo to use. If omitted, create a temporary repo.",
    )
    parser.add_argument(
        "--file",
        help="Relative file to mutate inside --repo. If omitted, choose a sensible default.",
    )
    parser.add_argument(
        "--binary",
        help="Path to collie-search binary. Default: target/release/collie-search",
    )
    parser.add_argument(
        "--timeout-seconds",
        type=float,
        default=DEFAULT_TIMEOUT_SECONDS,
        help=f"Polling timeout after mutation. Default: {DEFAULT_TIMEOUT_SECONDS}",
    )
    parser.add_argument(
        "--poll-ms",
        type=int,
        default=DEFAULT_POLL_MS,
        help=f"Polling interval in milliseconds. Default: {DEFAULT_POLL_MS}",
    )
    parser.add_argument(
        "--settle-seconds",
        type=float,
        default=DEFAULT_SETTLE_SECONDS,
        help=f"Extra delay after `watch` returns before mutating. Default: {DEFAULT_SETTLE_SECONDS}",
    )
    parser.add_argument(
        "--keep-running",
        action="store_true",
        help="Leave the daemon running after the script completes.",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Print the full JSON result instead of the human summary.",
    )
    parser.add_argument(
        "--output",
        help="Optional path to write the JSON result.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    collie_root = Path(__file__).resolve().parent.parent
    binary = (
        Path(args.binary).expanduser().resolve()
        if args.binary
        else collie_root / "target" / "release" / "collie-search"
    )
    if not binary.exists():
        print(f"error: collie binary not found at {binary}", file=sys.stderr)
        print("build it first with: cargo build --release", file=sys.stderr)
        return 1

    setup = prepare_repo(args.repo, args.file)
    repo = setup.repo
    file_path = setup.file_path
    token = f"manual_watch_probe_{uuid.uuid4().hex}"

    original_bytes = file_path.read_bytes()
    result = {
        "binary": str(binary),
        "repo": str(repo),
        "file": str(file_path),
        "token": token,
        "created_temp_repo": setup.created_temp_repo,
        "watch_started": False,
        "watch_returncode": None,
        "status_before": None,
        "status_after": None,
        "search_found": False,
        "last_event_changed": False,
        "add_detected_ms": None,
        "daemon_log_tail": None,
        "error": None,
    }

    try:
        stop_daemon(binary, repo, collie_root)

        watch = subprocess.run(
            [str(binary), "watch", str(repo)],
            cwd=collie_root,
            capture_output=True,
            text=True,
        )
        result["watch_returncode"] = watch.returncode
        result["watch_stdout"] = watch.stdout
        result["watch_stderr"] = watch.stderr
        if watch.returncode != 0:
            result["error"] = watch.stderr.strip() or "collie watch failed"
            return finish(result, args, setup)

        result["watch_started"] = True
        status_before = load_status(binary, repo, collie_root)
        result["status_before"] = status_before

        time.sleep(args.settle_seconds)
        append_token(file_path, token)

        initial_last_event = status_before.get("last_event_at_unix_ms") if status_before else None
        start = time.perf_counter()
        deadline = start + args.timeout_seconds

        while time.perf_counter() < deadline:
            status_now = load_status(binary, repo, collie_root)
            search_now = subprocess.run(
                [str(binary), "search", token, "--no-snippets", "-n", "5"],
                cwd=repo,
                capture_output=True,
                text=True,
            )
            found = parse_collie_count(search_now.stdout) >= 1
            if found:
                result["search_found"] = True
                result["add_detected_ms"] = round((time.perf_counter() - start) * 1000, 2)
                result["search_stdout"] = search_now.stdout
                result["search_stderr"] = search_now.stderr
                result["status_after"] = status_now
                result["last_event_changed"] = (
                    status_now.get("last_event_at_unix_ms") != initial_last_event
                )
                break
            result["status_after"] = status_now
            time.sleep(args.poll_ms / 1000.0)

        if not result["search_found"]:
            result["daemon_log_tail"] = read_log_tail(repo)
            result["last_event_changed"] = (
                (result["status_after"] or {}).get("last_event_at_unix_ms") != initial_last_event
            )
    except Exception as exc:
        result["error"] = str(exc)
    finally:
        file_path.write_bytes(original_bytes)
        if not args.keep_running:
            stop_daemon(binary, repo, collie_root)

    return finish(result, args, setup)


def finish(result: dict, args: argparse.Namespace, setup: RepoSetup) -> int:
    if args.output:
        output_path = Path(args.output).expanduser().resolve()
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")

    if args.json:
        print(json.dumps(result, indent=2))
    else:
        print_summary(result)

    if setup.temp_dir is not None:
        setup.temp_dir.cleanup()

    if result.get("error"):
        return 1
    return 0 if result.get("search_found") else 2


def prepare_repo(repo_arg: str | None, file_arg: str | None) -> RepoSetup:
    if repo_arg:
        repo = Path(repo_arg).expanduser().resolve()
        if not repo.exists():
            raise SystemExit(f"error: repo does not exist: {repo}")
        if not repo.joinpath(".git").exists():
            raise SystemExit(f"error: repo is not a git repository: {repo}")
        file_path = resolve_existing_file(repo, file_arg)
        return RepoSetup(repo=repo, file_path=file_path, created_temp_repo=False, temp_dir=None)

    temp_dir = tempfile.TemporaryDirectory()
    repo = Path(temp_dir.name)
    subprocess.run(["git", "init"], cwd=repo, check=True, capture_output=True, text=True)
    file_path = repo / "a.rs"
    file_path.write_text("fn hello() {}\n", encoding="utf-8")
    return RepoSetup(repo=repo, file_path=file_path, created_temp_repo=True, temp_dir=temp_dir)


def resolve_existing_file(repo: Path, file_arg: str | None) -> Path:
    if file_arg:
        path = repo / file_arg
        if not path.is_file():
            raise SystemExit(f"error: file does not exist: {path}")
        return path

    candidates = [
        "src/main.rs",
        "src/lib.rs",
        "main.go",
        "pkg/probe/dialer_others.go",
        "pkg/probe/exec/exec.go",
        "app/main.py",
    ]
    for rel in candidates:
        path = repo / rel
        if path.is_file():
            return path

    rg = shutil.which("rg")
    if not rg:
        raise SystemExit("error: rg is required to auto-discover a file")

    for ext in [".rs", ".go", ".py", ".js", ".ts"]:
        proc = subprocess.run(
            [rg, "--files", "-g", f"*{ext}", "--max-depth", "4", "."],
            cwd=repo,
            capture_output=True,
            text=True,
        )
        if proc.returncode != 0:
            continue
        files = [line.strip().removeprefix("./") for line in proc.stdout.splitlines() if line.strip()]
        if files:
            return repo / files[0]

    raise SystemExit("error: could not find a suitable source file to mutate")


def stop_daemon(binary: Path, repo: Path, collie_root: Path) -> None:
    subprocess.run(
        [str(binary), "stop", str(repo)],
        cwd=collie_root,
        capture_output=True,
        text=True,
    )


def load_status(binary: Path, repo: Path, collie_root: Path) -> dict:
    proc = subprocess.run(
        [str(binary), "status", str(repo), "--json"],
        cwd=collie_root,
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(proc.stdout)


def append_token(file_path: Path, token: str) -> None:
    suffix = file_path.suffix.lower()
    comment = COMMENT_BY_SUFFIX.get(suffix, "//")
    if comment == "/*":
        marker = f"\n/* {token} */\n"
    elif comment == "<!--":
        marker = f"\n<!-- {token} -->\n"
    else:
        marker = f"\n{comment} {token}\n"
    with file_path.open("a", encoding="utf-8") as handle:
        handle.write(marker)


def parse_collie_count(stdout: str) -> int:
    for line in stdout.splitlines():
        if line.startswith("Found "):
            parts = line.split()
            if len(parts) > 1 and parts[1].isdigit():
                return int(parts[1])
    return 0


def read_log_tail(repo: Path, max_chars: int = 2000) -> str | None:
    log_path = repo / ".collie" / "daemon.log"
    if not log_path.exists():
        return None
    text = log_path.read_text(encoding="utf-8", errors="replace")
    return text[-max_chars:] if text else ""


def print_summary(result: dict) -> None:
    print(f"Repo: {result['repo']}")
    print(f"File: {result['file']}")
    print(f"Token: {result['token']}")
    print(f"Watch return code: {result['watch_returncode']}")
    print(f"Search found token: {result['search_found']}")
    print(f"Detection time ms: {result['add_detected_ms']}")
    print(f"last_event_at changed: {result['last_event_changed']}")
    before = result.get("status_before") or {}
    after = result.get("status_after") or {}
    print(f"Status before: {before.get('status')}")
    print(f"Status after: {after.get('status')}")
    print(f"last_event_at before: {before.get('last_event_at_unix_ms')}")
    print(f"last_event_at after: {after.get('last_event_at_unix_ms')}")
    if result.get("error"):
        print(f"Error: {result['error']}")
    if result.get("daemon_log_tail") is not None:
        print("\nDaemon log tail:")
        print(result["daemon_log_tail"] or "<empty>")


if __name__ == "__main__":
    raise SystemExit(main())
