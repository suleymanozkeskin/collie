mod common;

use anyhow::Result;
use common::*;
use std::fs;

#[test]
fn gitignore_prevents_indexing_of_matched_files() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), ".gitignore", "secret.yaml\n")?;
    write_file(worktree.path(), "secret.yaml", "password: hunter2")?;
    write_file(worktree.path(), "src/lib.rs", "fn public_api() {}")?;

    let output = run_collie(worktree.path(), &["watch", "."])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    wait_for_running(worktree.path())?;

    let search = run_collie(worktree.path(), &["-s", "hunter2"])?;
    assert!(
        stdout(&search).contains("No results found"),
        "gitignored file should not be indexed, got: {}",
        stdout(&search)
    );

    let search = run_collie(worktree.path(), &["-s", "public_api"])?;
    assert!(
        stdout(&search).contains("Found 1 results"),
        "non-ignored file should be indexed, got: {}",
        stdout(&search)
    );

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn collieignore_prevents_indexing_of_matched_files() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), ".collieignore", "generated.rs\n")?;
    write_file(worktree.path(), "generated.rs", "fn generated_code() {}")?;
    write_file(worktree.path(), "src/lib.rs", "fn real_code() {}")?;

    let output = run_collie(worktree.path(), &["watch", "."])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    wait_for_running(worktree.path())?;

    let search = run_collie(worktree.path(), &["-s", "generated_code"])?;
    assert!(
        stdout(&search).contains("No results found"),
        ".collieignore should prevent indexing, got: {}",
        stdout(&search)
    );

    let search = run_collie(worktree.path(), &["-s", "real_code"])?;
    assert!(
        stdout(&search).contains("Found 1 results"),
        "non-ignored file should be indexed, got: {}",
        stdout(&search)
    );

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn gitignore_negation_allows_specific_file() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), ".gitignore", "*.log\n!important.log\n")?;
    write_file(worktree.path(), "debug.log", "debug_token_abc")?;
    write_file(worktree.path(), "important.log", "important_token_xyz")?;
    let config_dir = worktree.path().join(".collie");
    fs::create_dir_all(&config_dir)?;
    fs::write(
        config_dir.join("config.toml"),
        "[index]\nextra_extensions = [\"log\"]\n",
    )?;

    let output = run_collie(worktree.path(), &["watch", "."])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    wait_for_running(worktree.path())?;

    let search = run_collie(worktree.path(), &["-s", "debug_token_abc"])?;
    assert!(
        stdout(&search).contains("No results found"),
        "gitignored .log file should not be indexed, got: {}",
        stdout(&search)
    );

    let search = run_collie(worktree.path(), &["-s", "important_token_xyz"])?;
    assert!(
        stdout(&search).contains("Found 1 results"),
        "negated .log file should be indexed, got: {}",
        stdout(&search)
    );

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn hidden_directories_are_not_indexed() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), ".hidden/secret.rs", "fn hidden_fn() {}")?;
    write_file(worktree.path(), "src/lib.rs", "fn visible_fn() {}")?;

    let output = run_collie(worktree.path(), &["watch", "."])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    wait_for_running(worktree.path())?;

    let search = run_collie(worktree.path(), &["-s", "hidden_fn"])?;
    assert!(
        stdout(&search).contains("No results found"),
        "hidden dir should not be indexed, got: {}",
        stdout(&search)
    );

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn extra_extensions_from_config_are_indexed() -> Result<()> {
    let worktree = create_worktree()?;
    let config_dir = worktree.path().join(".collie");
    fs::create_dir_all(&config_dir)?;
    fs::write(
        config_dir.join("config.toml"),
        "[index]\nextra_extensions = [\"sql\"]\n",
    )?;
    write_file(worktree.path(), "schema.sql", "CREATE TABLE users")?;

    let output = run_collie(worktree.path(), &["watch", "."])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    wait_for_running(worktree.path())?;

    let search = run_collie(worktree.path(), &["-s", "users"])?;
    assert!(
        stdout(&search).contains("Found 1 results"),
        ".sql file should be indexed with extra_extensions config, got: {}",
        stdout(&search)
    );

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn exclude_extensions_from_config_are_skipped() -> Result<()> {
    let worktree = create_worktree()?;
    let config_dir = worktree.path().join(".collie");
    fs::create_dir_all(&config_dir)?;
    fs::write(
        config_dir.join("config.toml"),
        "[index]\nexclude_extensions = [\"md\"]\n",
    )?;
    write_file(worktree.path(), "readme.md", "fn markdown_token() {}")?;
    write_file(worktree.path(), "src/lib.rs", "fn rust_token() {}")?;

    let output = run_collie(worktree.path(), &["watch", "."])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    wait_for_running(worktree.path())?;

    let search = run_collie(worktree.path(), &["-s", "markdown_token"])?;
    assert!(
        stdout(&search).contains("No results found"),
        ".md should be excluded, got: {}",
        stdout(&search)
    );

    let search = run_collie(worktree.path(), &["-s", "rust_token"])?;
    assert!(
        stdout(&search).contains("Found 1 results"),
        ".rs should still be indexed, got: {}",
        stdout(&search)
    );

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn max_file_size_skips_large_files() -> Result<()> {
    let worktree = create_worktree()?;
    let config_dir = worktree.path().join(".collie");
    fs::create_dir_all(&config_dir)?;
    fs::write(
        config_dir.join("config.toml"),
        "[index]\nmax_file_size = 50\n",
    )?;
    write_file(
        worktree.path(),
        "big.rs",
        "fn a_very_long_function_name_that_exceeds_fifty_bytes_for_sure() {}",
    )?;
    write_file(worktree.path(), "small.rs", "fn tiny() {}")?;

    let output = run_collie(worktree.path(), &["watch", "."])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    wait_for_running(worktree.path())?;

    let search = run_collie(
        worktree.path(),
        &[
            "-s",
            "a_very_long_function_name_that_exceeds_fifty_bytes_for_sure",
        ],
    )?;
    assert!(
        stdout(&search).contains("No results found"),
        "oversized file should be skipped, got: {}",
        stdout(&search)
    );

    let search = run_collie(worktree.path(), &["-s", "tiny"])?;
    assert!(
        stdout(&search).contains("Found 1 results"),
        "small file should be indexed, got: {}",
        stdout(&search)
    );

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn watcher_respects_gitignore_for_new_files() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), ".gitignore", "ignored_*.rs\n")?;
    write_file(worktree.path(), "src/lib.rs", "fn initial() {}")?;

    let output = run_collie(worktree.path(), &["watch", "."])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    wait_for_running(worktree.path())?;

    write_file(
        worktree.path(),
        "ignored_secret.rs",
        "fn watcher_should_skip_me() {}",
    )?;
    write_file(
        worktree.path(),
        "src/new.rs",
        "fn watcher_should_index_me() {}",
    )?;

    wait_for_condition(std::time::Duration::from_secs(5), || {
        let search = run_collie(worktree.path(), &["-s", "watcher_should_index_me"])?;
        Ok(stdout(&search).contains("Found 1 results"))
    })?;

    let search = run_collie(worktree.path(), &["-s", "watcher_should_skip_me"])?;
    assert!(
        stdout(&search).contains("No results found"),
        "watcher should respect gitignore for new files, got: {}",
        stdout(&search)
    );

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn watcher_respects_nested_gitignore() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/lib.rs", "fn root_fn() {}")?;
    // Nested .gitignore that ignores *.generated.rs in subdir/
    write_file(worktree.path(), "subdir/.gitignore", "*.generated.rs\n")?;

    let output = run_collie(worktree.path(), &["watch", "."])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    wait_for_running(worktree.path())?;

    // Create a file matching the nested .gitignore pattern after daemon is running
    write_file(
        worktree.path(),
        "subdir/auto.generated.rs",
        "fn nested_ignored_fn() {}",
    )?;
    // Also create a non-ignored file to confirm the watcher is processing events
    write_file(worktree.path(), "subdir/real.rs", "fn nested_real_fn() {}")?;

    // Wait for the watcher to process the non-ignored file
    wait_for_condition(std::time::Duration::from_secs(5), || {
        let search = run_collie(worktree.path(), &["-s", "nested_real_fn"])?;
        Ok(stdout(&search).contains("Found 1 results"))
    })?;

    // The file matching subdir/.gitignore should NOT be indexed
    let search = run_collie(worktree.path(), &["-s", "nested_ignored_fn"])?;
    assert!(
        stdout(&search).contains("No results found"),
        "watcher should respect nested .gitignore, got: {}",
        stdout(&search)
    );

    ensure_stopped(worktree.path());
    Ok(())
}

#[test]
fn file_growing_past_max_size_removes_stale_postings() -> Result<()> {
    let worktree = create_worktree()?;
    let config_dir = worktree.path().join(".collie");
    fs::create_dir_all(&config_dir)?;
    // Set max_file_size to 100 bytes
    fs::write(
        config_dir.join("config.toml"),
        "[index]\nmax_file_size = 100\n",
    )?;
    // Start with a small file (under 100 bytes)
    write_file(worktree.path(), "src/lib.rs", "fn small_token() {}")?;

    let output = run_collie(worktree.path(), &["watch", "."])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    wait_for_running(worktree.path())?;

    // Verify the small file is indexed
    let search = run_collie(worktree.path(), &["-s", "small_token"])?;
    assert!(
        stdout(&search).contains("Found 1 results"),
        "small file should be indexed, got: {}",
        stdout(&search)
    );

    // Now grow the file past max_file_size
    let big_content = format!("fn small_token() {{}}\n// {}", "x".repeat(200));
    write_file(worktree.path(), "src/lib.rs", &big_content)?;

    // Wait for the watcher to process the modification
    std::thread::sleep(std::time::Duration::from_secs(2));

    // The file is now too large — its old postings should be removed
    let search = run_collie(worktree.path(), &["-s", "small_token"])?;
    assert!(
        stdout(&search).contains("No results found"),
        "file that grew past max_file_size should have stale postings removed, got: {}",
        stdout(&search)
    );

    ensure_stopped(worktree.path());
    Ok(())
}
