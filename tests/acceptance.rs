mod common;

use anyhow::Result;
use std::fs;
use std::thread;
use std::time::Duration;

use common::{
    create_worktree, ensure_stopped, run_collie, wait_for_condition, wait_for_running, write_file,
};

fn search_output(root: &std::path::Path, pattern: &str) -> Result<String> {
    let output = run_collie(root, &["-s", pattern])?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[test]
fn daemon_updates_index_and_search_reads_latest_persisted_state() -> Result<()> {
    let worktree = create_worktree()?;
    let file = write_file(worktree.path(), "src/lib.rs", "fn old_name() {}")?;

    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;

    wait_for_condition(Duration::from_secs(5), || {
        Ok(search_output(worktree.path(), "old_name")?.contains("Found 1 results"))
    })?;

    fs::write(&file, "fn new_name() {}")?;
    wait_for_condition(Duration::from_secs(10), || {
        Ok(
            search_output(worktree.path(), "new_name")?.contains("Found 1 results")
                && search_output(worktree.path(), "old_name")?
                    == "No results found for pattern: old_name",
        )
    })?;

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn same_worktree_second_agent_reuses_existing_daemon() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn shared() {}")?;

    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;

    let second = run_collie(worktree.path(), &["watch", "."])?;
    assert_eq!(
        String::from_utf8_lossy(&second.stdout).trim(),
        format!(
            "Collie daemon already running for {}",
            common::canonical_root(worktree.path()).display()
        )
    );

    wait_for_condition(Duration::from_secs(5), || {
        Ok(search_output(worktree.path(), "shared")?.contains("Found 1 results"))
    })?;

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn different_worktrees_do_not_share_results() -> Result<()> {
    let one = create_worktree()?;
    let two = create_worktree()?;
    write_file(one.path(), "src/lib.rs", "fn alpha_agent() {}")?;
    write_file(two.path(), "src/lib.rs", "fn beta_agent() {}")?;

    run_collie(one.path(), &["watch", "."])?;
    run_collie(two.path(), &["watch", "."])?;
    wait_for_running(one.path())?;
    wait_for_running(two.path())?;

    wait_for_condition(Duration::from_secs(5), || {
        Ok(
            search_output(one.path(), "alpha_agent")?.contains("Found 1 results")
                && search_output(one.path(), "beta_agent")?
                    == "No results found for pattern: beta_agent"
                && search_output(two.path(), "beta_agent")?.contains("Found 1 results")
                && search_output(two.path(), "alpha_agent")?
                    == "No results found for pattern: alpha_agent",
        )
    })?;

    ensure_stopped(one.path());
    ensure_stopped(two.path());
    Ok(())
}

#[test]
fn stop_then_search_reads_last_persisted_state_without_reindex() -> Result<()> {
    let worktree = create_worktree()?;
    let file = write_file(worktree.path(), "src/lib.rs", "fn session_token() {}")?;

    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;
    wait_for_condition(Duration::from_secs(5), || {
        Ok(search_output(worktree.path(), "session_token")?.contains("Found 1 results"))
    })?;

    fs::write(&file, "fn persisted_after_stop() {}")?;
    wait_for_condition(Duration::from_secs(10), || {
        Ok(search_output(worktree.path(), "persisted_after_stop")?.contains("Found 1 results"))
    })?;

    run_collie(worktree.path(), &["stop", "."])?;
    thread::sleep(Duration::from_millis(500));

    let output = search_output(worktree.path(), "persisted_after_stop")?;
    assert!(output.contains("Found 1 results"));
    Ok(())
}
