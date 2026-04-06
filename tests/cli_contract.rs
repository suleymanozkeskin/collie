mod common;

use anyhow::Result;
use common::*;
use std::time::Duration;

#[test]
fn watch_already_running_prints_exact_message() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn once() {}")?;
    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;

    let output = run_collie(worktree.path(), &["watch", "."])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!(
            "Collie daemon already running for {}",
            canonical_root(worktree.path()).display()
        )
    );

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn status_running_shows_human_readable_fields() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn status_check() {}")?;
    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;

    std::thread::sleep(Duration::from_secs(1));

    let output = run_collie(worktree.path(), &["status", "."])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);

    assert!(
        text.contains("Collie daemon status: running"),
        "missing status line"
    );
    assert!(text.contains("Worktree root:"), "missing worktree root");
    assert!(text.contains("PID:"), "missing PID");
    assert!(text.contains("Uptime:"), "missing uptime");
    assert!(text.contains("Index path:"), "missing index path");
    assert!(text.contains("Index size:"), "missing index size");
    assert!(text.contains("Files indexed:"), "missing files indexed");
    assert!(text.contains("Unique terms:"), "missing unique terms");
    assert!(text.contains("Postings:"), "missing postings");
    assert!(text.contains("Trigram entries:"), "missing trigram entries");
    assert!(text.contains("Last save:"), "missing last save");
    assert!(text.contains("Last event:"), "missing last event");

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn status_stopped_shows_reason() -> Result<()> {
    let worktree = create_worktree()?;
    let output = run_collie(worktree.path(), &["status", "."])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("Collie daemon status: stopped"));
    assert!(text.contains("Reason:"));
    Ok(())
}

#[test]
fn status_json_flag_returns_valid_json() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn json_test() {}")?;
    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;

    let output = run_collie(worktree.path(), &["status", ".", "--json"])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let json: serde_json::Value =
        serde_json::from_str(&stdout(&output)).expect("--json output must be valid JSON");

    assert!(json.get("status").is_some(), "missing 'status' key");
    assert!(json.get("pid").is_some(), "missing 'pid' key");
    assert!(
        json.get("total_files").is_some(),
        "missing 'total_files' key"
    );
    assert!(
        json.get("total_terms").is_some(),
        "missing 'total_terms' key"
    );
    assert!(
        json.get("worktree_root").is_some(),
        "missing 'worktree_root' key"
    );

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn status_json_flag_when_stopped() -> Result<()> {
    let worktree = create_worktree()?;
    let output = run_collie(worktree.path(), &["status", ".", "--json"])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let json: serde_json::Value = serde_json::from_str(&stdout(&output))
        .expect("--json output must be valid JSON even when stopped");

    assert_eq!(json["status"], "stopped");
    Ok(())
}

#[test]
fn stop_without_running_daemon_prints_exact_message() -> Result<()> {
    let worktree = create_worktree()?;
    let output = run_collie(worktree.path(), &["stop", "."])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!(
            "Collie daemon is not running for {}",
            canonical_root(worktree.path()).display()
        )
    );
    Ok(())
}

#[test]
fn search_without_index_prints_exact_error() -> Result<()> {
    let worktree = create_worktree()?;
    let output = run_collie(worktree.path(), &["-s", "missing"])?;
    assert!(!output.status.success());
    assert_eq!(
        stderr(&output),
        "No index found. Run 'collie watch .' from the worktree root first."
    );
    Ok(())
}

#[test]
fn search_with_results_prints_expected_header_and_paths() -> Result<()> {
    let worktree = create_worktree()?;
    let file = write_file(
        worktree.path(),
        "src/lib.rs",
        "fn search_hit() { search_hit(); }",
    )?;
    build_index(
        worktree.path(),
        &[("src/lib.rs", "fn search_hit() { search_hit(); }")],
    )?;

    let output = run_collie(worktree.path(), &["search", "search_hit", "--no-snippets"])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let lines: Vec<_> = stdout(&output).lines().map(str::to_string).collect();
    assert_eq!(lines[0], "Found 1 results for pattern: search_hit");
    assert_eq!(lines[1], "");
    assert_eq!(lines[2], "1. src/lib.rs");
    Ok(())
}

#[test]
fn search_no_results_prints_expected_message() -> Result<()> {
    let worktree = create_worktree()?;
    build_index(worktree.path(), &[("src/lib.rs", "fn unrelated() {}")])?;

    let output = run_collie(worktree.path(), &["search", "missing"])?;
    // Exit code 1 = no results found (like grep)
    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "No results found for pattern: missing");
    Ok(())
}

#[test]
fn token_count_ignores_default_limit() -> Result<()> {
    let worktree = create_worktree()?;
    let mut files = Vec::new();
    for i in 0..25 {
        let rel = format!("src/file_{i:02}.rs");
        let content = "fn count_me() { count_me(); }\n".to_string();
        files.push((rel, content));
    }

    let tuples: Vec<(&str, &str)> = files
        .iter()
        .map(|(path, content)| (path.as_str(), content.as_str()))
        .collect();
    build_index(worktree.path(), &tuples)?;

    let output = run_collie(worktree.path(), &["search", "count_me", "--count"])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "25");
    Ok(())
}
