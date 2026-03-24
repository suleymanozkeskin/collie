mod common;

use anyhow::Result;
use collie_search::config::CollieConfig;
use collie_search::indexer::IndexBuilder;
use collie_search::storage::generation::GenerationManager;
use common::*;
use serde_json::json;
use std::fs;
use std::thread;
use std::time::Duration;

extern crate libc;

#[test]
fn watch_starts_daemon_and_returns_when_ready() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn warm_index() {}")?;

    let output = run_collie(worktree.path(), &["watch", "."])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    wait_for_running(worktree.path())?;

    let search = run_collie(worktree.path(), &["-s", "warm_index"])?;
    assert!(search.status.success(), "stderr: {}", stderr(&search));
    assert!(stdout(&search).contains("Found 1 results for pattern: warm_index"));

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn watch_reports_already_running_for_same_worktree() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn first() {}")?;

    let first = run_collie(worktree.path(), &["watch", "."])?;
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    wait_for_running(worktree.path())?;

    let pid_before = fs::read_to_string(pid_path(worktree.path()))?;
    let second = run_collie(worktree.path(), &["watch", "."])?;
    assert!(second.status.success(), "stderr: {}", stderr(&second));
    let pid_after = fs::read_to_string(pid_path(worktree.path()))?;

    assert_eq!(pid_before, pid_after);
    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn stop_terminates_running_daemon() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn stop_me() {}")?;
    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;

    let stop = run_collie(worktree.path(), &["stop", "."])?;
    assert!(stop.status.success(), "stderr: {}", stderr(&stop));
    wait_for_stopped(worktree.path())?;
    Ok(())
}

#[test]
fn status_reports_running_daemon() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn status_running() {}")?;
    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;

    let status = run_collie(worktree.path(), &["status", "."])?;
    assert!(status.status.success(), "stderr: {}", stderr(&status));
    assert!(stdout(&status).contains("Collie daemon status: running"));

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn status_reports_stopped_for_missing_or_dead_pid() -> Result<()> {
    let worktree = create_worktree()?;
    fs::create_dir_all(collie_dir(worktree.path()))?;
    fs::write(pid_path(worktree.path()), "999999")?;
    fs::write(
        state_path(worktree.path()),
        serde_json::to_string_pretty(&json!({
            "worktree_root": canonical_root(worktree.path()),
            "index_path": index_path(worktree.path()),
            "pid": 999999,
            "status": "Running",
            "started_at_unix_ms": 0,
            "last_event_at_unix_ms": null,
            "last_save_at_unix_ms": null,
            "total_files": 0,
            "total_terms": 0,
            "total_postings": 0,
            "trigram_entries": 0,
            "last_error": null,
        }))?,
    )?;

    let status = run_collie(worktree.path(), &["status", "."])?;
    assert!(status.status.success(), "stderr: {}", stderr(&status));
    assert!(stdout(&status).contains("Collie daemon status: stopped"));
    Ok(())
}

#[test]
fn watch_cleans_stale_pid_and_restarts() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn restart_me() {}")?;
    fs::create_dir_all(collie_dir(worktree.path()))?;
    fs::write(pid_path(worktree.path()), "999999")?;
    fs::write(
        state_path(worktree.path()),
        serde_json::to_string_pretty(&json!({
            "worktree_root": canonical_root(worktree.path()),
            "index_path": index_path(worktree.path()),
            "pid": 999999,
            "status": "Running",
            "started_at_unix_ms": 0,
            "last_event_at_unix_ms": null,
            "last_save_at_unix_ms": null,
            "total_files": 0,
            "total_terms": 0,
            "total_postings": 0,
            "trigram_entries": 0,
            "last_error": null,
        }))?,
    )?;

    let output = run_collie(worktree.path(), &["watch", "."])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    wait_for_running(worktree.path())?;

    let pid = fs::read_to_string(pid_path(worktree.path()))?;
    assert_ne!(pid.trim(), "999999");
    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn different_worktrees_get_different_daemons() -> Result<()> {
    let one = create_worktree()?;
    let two = create_worktree()?;
    write_file(one.path(), "src/lib.rs", "fn one() {}")?;
    write_file(two.path(), "src/lib.rs", "fn two() {}")?;

    run_collie(one.path(), &["watch", "."])?;
    run_collie(two.path(), &["watch", "."])?;
    wait_for_running(one.path())?;
    wait_for_running(two.path())?;

    let pid_one = fs::read_to_string(pid_path(one.path()))?;
    let pid_two = fs::read_to_string(pid_path(two.path()))?;
    assert_ne!(pid_one.trim(), pid_two.trim());

    ensure_stopped(one.path());
    ensure_stopped(two.path());
    Ok(())
}

#[test]
fn search_reads_existing_index_without_talking_to_daemon() -> Result<()> {
    let worktree = create_worktree()?;
    build_index(worktree.path(), &[("src/lib.rs", "fn read_only() {}")])?;

    let output = run_collie(worktree.path(), &["-s", "read_only"])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(stdout(&output).contains("Found 1 results for pattern: read_only"));
    Ok(())
}

#[test]
fn watch_writes_pid_and_state_files() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn files() {}")?;

    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;

    assert!(pid_path(worktree.path()).exists());
    assert!(state_path(worktree.path()).exists());
    assert!(log_path(worktree.path()).exists());

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn watch_initial_build_makes_search_ready_before_return() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn ready_on_return() {}")?;

    let watch = run_collie(worktree.path(), &["watch", "."])?;
    assert!(watch.status.success(), "stderr: {}", stderr(&watch));

    let search = run_collie(worktree.path(), &["-s", "ready_on_return"])?;
    assert!(search.status.success(), "stderr: {}", stderr(&search));
    assert!(stdout(&search).contains("Found 1 results for pattern: ready_on_return"));

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn stop_removes_or_invalidates_stale_pid_state() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn stop_state() {}")?;
    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;

    let stop = run_collie(worktree.path(), &["stop", "."])?;
    assert!(stop.status.success(), "stderr: {}", stderr(&stop));
    wait_for_stopped(worktree.path())?;

    let pid_exists = pid_path(worktree.path()).exists();
    let state = read_state(worktree.path())?;
    assert!(!pid_exists || state["status"] == "Stopped" || state["status"] == "stopped");
    Ok(())
}

#[test]
fn clean_stops_running_daemon_before_removing_runtime_state() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn clean_state() {}")?;
    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;

    let clean = run_collie(worktree.path(), &["clean", "."])?;
    assert!(clean.status.success(), "stderr: {}", stderr(&clean));
    assert!(
        !collie_dir(worktree.path()).exists(),
        "runtime state should be removed after clean"
    );

    let status = run_collie(worktree.path(), &["status", "."])?;
    assert!(status.status.success(), "stderr: {}", stderr(&status));
    assert!(stdout(&status).contains("Collie daemon status: stopped"));
    Ok(())
}

#[test]
fn foreground_watch_uses_same_state_layout() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn foreground() {}")?;
    let mut child = spawn_collie(worktree.path(), &["watch", ".", "--foreground"])?;
    wait_for_running(worktree.path())?;

    assert!(pid_path(worktree.path()).exists());
    assert!(state_path(worktree.path()).exists());
    assert!(index_path(worktree.path()).exists());

    let stop = run_collie(worktree.path(), &["stop", "."])?;
    assert!(stop.status.success(), "stderr: {}", stderr(&stop));
    wait_for_condition(Duration::from_secs(10), || Ok(child.try_wait()?.is_some()))?;
    Ok(())
}

#[test]
fn search_uses_existing_index_when_daemon_is_not_running() -> Result<()> {
    let worktree = create_worktree()?;
    build_index(worktree.path(), &[("src/lib.rs", "fn persisted() {}")])?;

    let output = run_collie(worktree.path(), &["-s", "persisted"])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(stdout(&output).contains("Found 1 results for pattern: persisted"));
    Ok(())
}

#[test]
fn status_detects_crashed_daemon() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn crash_test() {}")?;

    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;

    let pid_str = fs::read_to_string(pid_path(worktree.path()))?
        .trim()
        .to_string();
    let pid: i32 = pid_str.parse()?;

    // Kill the daemon with SIGKILL (no cleanup)
    unsafe { libc::kill(pid, libc::SIGKILL) };

    wait_for_condition(Duration::from_secs(5), || {
        Ok(unsafe { libc::kill(pid, 0) } != 0)
    })?;

    let status = run_collie(worktree.path(), &["status", "."])?;
    assert!(status.status.success(), "stderr: {}", stderr(&status));
    let output_text = stdout(&status);
    assert!(
        output_text.contains("crashed"),
        "status should mention 'crashed' when daemon died uncleanly, got: {}",
        output_text
    );
    Ok(())
}

#[test]
fn search_uses_clean_index_without_warning_when_daemon_is_not_running() -> Result<()> {
    let worktree = create_worktree()?;
    build_index(worktree.path(), &[("src/lib.rs", "fn stale_result() {}")])?;

    let output = run_collie(worktree.path(), &["-s", "stale_result"])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stderr(&output).is_empty(),
        "search should not warn for a clean persisted index, got stderr: '{}'",
        stderr(&output)
    );
    assert!(
        stdout(&output).contains("stale_result"),
        "search results should still be returned, got: {}",
        stdout(&output)
    );
    Ok(())
}

#[test]
fn search_migrates_legacy_repo_local_runtime_state() -> Result<()> {
    let worktree = create_worktree()?;
    let legacy_dir = worktree.path().join(".collie");
    fs::create_dir_all(&legacy_dir)?;
    fs::write(legacy_dir.join("config.toml"), "# keep me\n")?;

    let source = write_file(worktree.path(), "src/lib.rs", "fn migrated_result() {}")?;
    let mgr = GenerationManager::new(&legacy_dir);
    let gen_dir = mgr.create_generation()?;
    let mut builder = IndexBuilder::new(&gen_dir, &CollieConfig::default())?;
    builder.index_file(&source)?;
    builder.save()?;
    mgr.activate(&gen_dir)?;

    let output = run_collie(worktree.path(), &["-s", "migrated_result"])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(stdout(&output).contains("migrated_result"));

    let external_dir = collie_dir(worktree.path());
    assert!(external_dir.join("CURRENT").exists());
    assert!(external_dir.join("generations").exists());
    assert!(!legacy_dir.join("CURRENT").exists());
    assert!(!legacy_dir.join("generations").exists());
    assert_eq!(
        fs::read_to_string(legacy_dir.join("config.toml"))?,
        "# keep me\n"
    );
    Ok(())
}

#[test]
fn restart_on_crash_respawns_daemon_after_sigkill() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn restart_test() {}")?;

    let mut supervisor = spawn_collie(worktree.path(), &["watch", ".", "--restart-on-crash"])?;
    wait_for_running(worktree.path())?;

    let pid_str = fs::read_to_string(pid_path(worktree.path()))?
        .trim()
        .to_string();
    let initial_pid: i32 = pid_str.parse()?;

    unsafe { libc::kill(initial_pid, libc::SIGKILL) };

    wait_for_condition(Duration::from_secs(10), || {
        Ok(unsafe { libc::kill(initial_pid, 0) } != 0)
    })?;

    // Wait for the supervisor to detect crash and respawn (2s poll + rebuild time)
    wait_for_condition(Duration::from_secs(30), || {
        if !pid_path(worktree.path()).exists() {
            return Ok(false);
        }
        let new_pid_str = fs::read_to_string(pid_path(worktree.path()))?
            .trim()
            .to_string();
        let new_pid: i32 = new_pid_str.parse().unwrap_or(0);
        Ok(new_pid != initial_pid && new_pid != 0 && unsafe { libc::kill(new_pid, 0) } == 0)
    })?;

    wait_for_running(worktree.path())?;
    let search = run_collie(worktree.path(), &["-s", "restart_test"])?;
    assert!(
        stdout(&search).contains("restart_test"),
        "new daemon should serve search, got: {}",
        stdout(&search)
    );

    ensure_stopped(worktree.path());
    let _ = supervisor.kill();
    let _ = supervisor.wait();
    Ok(())
}

#[test]
fn restart_purges_deleted_files_from_index() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/keep.rs", "fn keep_me() {}")?;
    write_file(worktree.path(), "src/remove.rs", "fn remove_me() {}")?;

    // Start daemon, both files indexed
    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;

    let search = run_collie(worktree.path(), &["-s", "remove_me"])?;
    assert!(
        stdout(&search).contains("remove_me"),
        "remove_me should be indexed initially, got: {}",
        stdout(&search)
    );

    // Stop daemon
    ensure_stopped(worktree.path());

    // Delete the file while daemon is stopped
    fs::remove_file(worktree.path().join("src/remove.rs"))?;

    // Restart daemon — should rebuild from clean filesystem state
    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;

    // The deleted file's tokens must NOT be searchable
    let search = run_collie(worktree.path(), &["-s", "remove_me"])?;
    assert!(
        stdout(&search).contains("No results found"),
        "deleted file should not be searchable after restart, got: {}",
        stdout(&search)
    );

    // The kept file should still be searchable
    let search = run_collie(worktree.path(), &["-s", "keep_me"])?;
    assert!(
        stdout(&search).contains("keep_me"),
        "kept file should still be searchable, got: {}",
        stdout(&search)
    );

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn restart_on_crash_does_not_respawn_after_intentional_stop() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn stop_test() {}")?;

    // Start with --restart-on-crash
    let mut supervisor = spawn_collie(worktree.path(), &["watch", ".", "--restart-on-crash"])?;
    wait_for_running(worktree.path())?;

    // Intentionally stop the daemon via `collie stop`
    let stop_output = run_collie(worktree.path(), &["stop", "."])?;
    assert!(
        stop_output.status.success(),
        "stop should succeed, stderr: {}",
        stderr(&stop_output)
    );

    // Wait long enough for the supervisor poll (2s) to fire and potentially respawn
    std::thread::sleep(Duration::from_secs(5));

    // The daemon should NOT be running — stop was intentional
    let status = run_collie(worktree.path(), &["status", "."])?;
    let text = stdout(&status);
    assert!(
        text.contains("stopped"),
        "daemon should stay stopped after intentional stop, got: {}",
        text
    );

    let _ = supervisor.kill();
    let _ = supervisor.wait();
    Ok(())
}

#[test]
fn restart_preserves_old_index_when_rebuild_has_unreadable_files() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn surviving_token() {}")?;

    // Start daemon, index the file
    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;

    let search = run_collie(worktree.path(), &["-s", "surviving_token"])?;
    assert!(stdout(&search).contains("surviving_token"));

    ensure_stopped(worktree.path());

    // Write a file with invalid UTF-8 — fs::read_to_string will fail on it
    let bad_path = worktree.path().join("src/bad.rs");
    fs::write(&bad_path, &[0xFF, 0xFE, 0x00, 0x80])?;

    // Restart daemon — bulk_rebuild should skip the bad file
    // but still index the good file and start successfully
    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;

    // The good file should be freshly indexed (clean rebuild)
    let search = run_collie(worktree.path(), &["-s", "surviving_token"])?;
    assert!(
        stdout(&search).contains("surviving_token"),
        "good file should be indexed despite bad file, got: {}",
        stdout(&search)
    );

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn invalid_utf8_file_is_partially_searchable() -> Result<()> {
    let worktree = create_worktree()?;

    // Write a file with a valid ASCII identifier followed by invalid UTF-8 bytes
    let mut content = b"fn recoverable_token() {}\n".to_vec();
    content.extend_from_slice(&[0xFF, 0xFE, 0x00, 0x80]);
    content.extend_from_slice(b"\nfn another_valid_token() {}\n");
    let file_path = worktree.path().join("src/mixed.rs");
    fs::create_dir_all(file_path.parent().unwrap())?;
    fs::write(&file_path, &content)?;

    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;

    // Both valid tokens should be searchable despite invalid bytes in the file
    let search = run_collie(worktree.path(), &["-s", "recoverable_token"])?;
    assert!(
        stdout(&search).contains("Found 1 results"),
        "valid tokens in files with invalid UTF-8 should be searchable, got: {}",
        stdout(&search)
    );

    let search = run_collie(worktree.path(), &["-s", "another_valid_token"])?;
    assert!(
        stdout(&search).contains("Found 1 results"),
        "second valid token should also be searchable, got: {}",
        stdout(&search)
    );

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn status_shows_skipped_files_count() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn normal() {}")?;

    // Create a file that cannot be read (permission denied)
    let unreadable = worktree.path().join("src/secret.rs");
    fs::write(&unreadable, "fn secret_token() {}")?;
    // Remove read permission
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000))?;

    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;

    let status = run_collie(worktree.path(), &["status", "."])?;
    let text = stdout(&status);
    assert!(
        text.contains("Skipped:"),
        "status should show skipped files info, got: {}",
        text
    );

    // Restore permissions for cleanup
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644))?;
    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn runtime_size_limit_skip_shows_in_status() -> Result<()> {
    let worktree = create_worktree()?;
    let config_dir = worktree.path().join(".collie");
    fs::create_dir_all(&config_dir)?;
    fs::write(
        config_dir.join("config.toml"),
        "[index]\nmax_file_size = 50\n",
    )?;
    write_file(worktree.path(), "src/lib.rs", "fn small() {}")?;

    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;

    // Create a file that exceeds max_file_size after daemon is running
    let big_content = format!("fn runtime_big_token() {{}}\n// {}", "x".repeat(200));
    write_file(worktree.path(), "src/big.rs", &big_content)?;

    // Wait for the watcher to process it
    std::thread::sleep(Duration::from_secs(2));

    let status = run_collie(worktree.path(), &["status", "."])?;
    let text = stdout(&status);
    assert!(
        text.contains("Skipped:"),
        "status should show runtime size-limit skip, got: {}",
        text
    );
    assert!(
        text.contains("size limit"),
        "skip reason should mention size limit, got: {}",
        text
    );

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn rebuild_command_creates_fresh_index() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn rebuild_target() {}")?;

    let output = run_collie(worktree.path(), &["rebuild", "."])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let out = stdout(&output);
    assert!(
        out.contains("Rebuilt index"),
        "should print rebuild summary, got: {}",
        out
    );
    assert!(out.contains("Files indexed:"), "should show files indexed");
    assert!(out.contains("Generation:"), "should show generation name");

    // Search should work after rebuild (no daemon needed)
    let search = run_collie(worktree.path(), &["-s", "rebuild_target"])?;
    assert!(search.status.success(), "stderr: {}", stderr(&search));
    assert!(stdout(&search).contains("Found 1 results"));

    Ok(())
}

#[test]
fn rebuild_command_stops_running_daemon() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn daemon_running() {}")?;

    // Start daemon
    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;

    // Rebuild should stop the daemon
    let output = run_collie(worktree.path(), &["rebuild", "."])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    // Daemon should be stopped
    let status = run_collie(worktree.path(), &["status", "."])?;
    assert!(stdout(&status).contains("stopped") || !pid_path(worktree.path()).exists());

    // Search should still work from rebuilt index
    let search = run_collie(worktree.path(), &["-s", "daemon_running"])?;
    assert!(search.status.success(), "stderr: {}", stderr(&search));
    assert!(stdout(&search).contains("Found 1 results"));

    Ok(())
}

#[test]
fn rebuild_replaces_stale_data() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/old.rs", "fn stale_function() {}")?;

    // First rebuild
    let output = run_collie(worktree.path(), &["rebuild", "."])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    // Delete the file, add a new one
    fs::remove_file(worktree.path().join("src/old.rs"))?;
    write_file(worktree.path(), "src/new.rs", "fn fresh_function() {}")?;

    // Second rebuild
    let output = run_collie(worktree.path(), &["rebuild", "."])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    // Old file should be gone, new file should be present
    let old_search = run_collie(worktree.path(), &["-s", "stale_function"])?;
    assert!(stdout(&old_search).contains("No results found"));

    let new_search = run_collie(worktree.path(), &["-s", "fresh_function"])?;
    assert!(stdout(&new_search).contains("Found 1 results"));

    Ok(())
}

#[test]
fn rebuild_cleans_stale_daemon_metadata() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn rebuilt_cleanly() {}")?;
    fs::create_dir_all(collie_dir(worktree.path()))?;
    fs::write(pid_path(worktree.path()), "999999")?;
    fs::write(
        state_path(worktree.path()),
        serde_json::to_string_pretty(&json!({
            "worktree_root": canonical_root(worktree.path()),
            "index_path": index_path(worktree.path()),
            "pid": 999999,
            "status": "Running",
            "started_at_unix_ms": 0,
            "last_event_at_unix_ms": null,
            "last_save_at_unix_ms": null,
            "total_files": 0,
            "total_terms": 0,
            "total_postings": 0,
            "trigram_entries": 0,
            "last_error": null,
        }))?,
    )?;

    let rebuild = run_collie(worktree.path(), &["rebuild", "."])?;
    assert!(rebuild.status.success(), "stderr: {}", stderr(&rebuild));

    let status = run_collie(worktree.path(), &["status", "."])?;
    assert!(status.status.success(), "stderr: {}", stderr(&status));
    let text = stdout(&status);
    assert!(text.contains("Collie daemon status: stopped"));
    assert!(text.contains("PID: missing"), "status should not retain stale pid: {}", text);
    assert!(
        !text.contains("daemon crashed"),
        "status should not report a crash after rebuilding from stale metadata: {}",
        text
    );
    Ok(())
}

#[test]
fn watch_reuses_clean_active_generation() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn reused_generation() {}")?;
    let rebuild = run_collie(worktree.path(), &["rebuild", "."])?;
    assert!(rebuild.status.success(), "stderr: {}", stderr(&rebuild));

    let current_before = fs::read_to_string(collie_dir(worktree.path()).join("CURRENT"))?;

    let output = run_collie(worktree.path(), &["watch", "."])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    wait_for_running(worktree.path())?;

    let current_after = fs::read_to_string(collie_dir(worktree.path()).join("CURRENT"))?;
    assert_eq!(
        current_before.trim(),
        current_after.trim(),
        "watch should reuse the existing clean generation instead of rebuilding"
    );

    let state = read_state(worktree.path())?;
    assert_eq!(state["generation"], json!(current_before.trim()));

    let search = run_collie(worktree.path(), &["-s", "reused_generation"])?;
    assert!(search.status.success(), "stderr: {}", stderr(&search));
    assert!(stdout(&search).contains("Found 1 results"));

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn watch_rebuilds_dirty_active_generation() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn dirty_generation() {}")?;
    let rebuild = run_collie(worktree.path(), &["rebuild", "."])?;
    assert!(rebuild.status.success(), "stderr: {}", stderr(&rebuild));

    let collie = collie_dir(worktree.path());
    let mgr = GenerationManager::new(&collie);
    let active_before = mgr
        .active_generation()?
        .expect("rebuild should activate a generation");
    fs::write(mgr.dirty_marker(&active_before), "")?;
    let current_before = fs::read_to_string(collie.join("CURRENT"))?;

    let output = run_collie(worktree.path(), &["watch", "."])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    wait_for_running(worktree.path())?;

    let current_after = fs::read_to_string(collie.join("CURRENT"))?;
    assert_ne!(
        current_before.trim(),
        current_after.trim(),
        "watch should rebuild when the active generation is marked dirty"
    );

    let search = run_collie(worktree.path(), &["-s", "dirty_generation"])?;
    assert!(search.status.success(), "stderr: {}", stderr(&search));
    assert!(stdout(&search).contains("Found 1 results"));

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn status_shows_segment_count_and_generation() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn segment_test() {}")?;

    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;

    let output = run_collie(worktree.path(), &["status", "."])?;
    let text = stdout(&output);

    assert!(
        text.contains("Segments:"),
        "status should show segment count, got: {}",
        text
    );
    assert!(
        text.contains("Generation:"),
        "status should show generation, got: {}",
        text
    );

    // JSON status should also have these fields
    let json_output = run_collie(worktree.path(), &["status", ".", "--json"])?;
    let json_text = stdout(&json_output);
    let parsed: serde_json::Value = serde_json::from_str(&json_text)?;
    assert!(
        parsed["segment_count"].is_number(),
        "JSON should have segment_count"
    );
    assert!(
        parsed["generation"].is_string(),
        "JSON should have generation"
    );
    assert!(
        parsed["initial_segment_count"].is_number(),
        "JSON should have initial_segment_count"
    );

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn daemon_auto_stops_after_idle_timeout() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn idle_test() {}")?;

    // Write config with a 2-second idle timeout
    let config_dir = worktree.path().join(".collie");
    fs::create_dir_all(&config_dir)?;
    fs::write(
        config_dir.join("config.toml"),
        "[watcher]\nidle_timeout_secs = 2\n",
    )?;

    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;

    // Verify search works while daemon is alive
    let search = run_collie(worktree.path(), &["-s", "idle_test"])?;
    assert!(stdout(&search).contains("Found 1 results"));

    // Wait for idle timeout to expire (2s timeout + margin)
    thread::sleep(Duration::from_secs(4));

    // Daemon should have auto-stopped
    wait_for_stopped(worktree.path())?;

    // State file should show idle timeout as the reason
    let state = read_state(worktree.path())?;
    assert_eq!(state["status"], "Stopped");
    assert_eq!(state["last_error"], "idle timeout");

    Ok(())
}

#[test]
fn search_activity_resets_idle_timer() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn activity_test() {}")?;

    // Write config with a 3-second idle timeout
    let config_dir = worktree.path().join(".collie");
    fs::create_dir_all(&config_dir)?;
    fs::write(
        config_dir.join("config.toml"),
        "[watcher]\nidle_timeout_secs = 3\n",
    )?;

    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;

    // Keep the daemon alive by searching every 2 seconds (within 3s timeout)
    for _ in 0..3 {
        thread::sleep(Duration::from_secs(2));
        let search = run_collie(worktree.path(), &["-s", "activity_test"])?;
        assert!(stdout(&search).contains("Found 1 results"));
        // Daemon should still be running
        let state = read_state(worktree.path())?;
        assert_eq!(state["status"], "Running");
    }

    // Now stop searching and let it idle out
    thread::sleep(Duration::from_secs(5));
    wait_for_stopped(worktree.path())?;

    let state = read_state(worktree.path())?;
    assert_eq!(state["status"], "Stopped");
    assert_eq!(state["last_error"], "idle timeout");

    Ok(())
}

#[test]
fn idle_timeout_zero_disables_auto_stop() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn no_idle_test() {}")?;

    // Write config with idle timeout disabled
    let config_dir = worktree.path().join(".collie");
    fs::create_dir_all(&config_dir)?;
    fs::write(
        config_dir.join("config.toml"),
        "[watcher]\nidle_timeout_secs = 0\n",
    )?;

    run_collie(worktree.path(), &["watch", "."])?;
    wait_for_running(worktree.path())?;

    // Wait longer than any reasonable timeout
    thread::sleep(Duration::from_secs(3));

    // Daemon should still be running
    let state = read_state(worktree.path())?;
    assert_eq!(state["status"], "Running");

    ensure_stopped(worktree.path());
    Ok(())
}
