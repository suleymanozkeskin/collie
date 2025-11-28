mod common;

use anyhow::Result;
use common::*;
use std::fs;

#[test]
fn search_shows_line_based_snippets_by_default() -> Result<()> {
    let worktree = create_worktree()?;
    let content = "line1\nline2\nfn target_function() {}\nline4\nline5\n";
    write_file(worktree.path(), "src/lib.rs", content)?;
    build_index(worktree.path(), &[("src/lib.rs", content)])?;

    let output = run_collie(worktree.path(), &["search", "target_function"])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);

    assert!(
        text.contains("target_function"),
        "snippet should show the matching line, got: {}",
        text
    );
    assert!(
        text.contains(" 3"),
        "snippet should show line number 3, got: {}",
        text
    );
    assert!(
        !text.contains("Positions:"),
        "default output should use snippets not positions, got: {}",
        text
    );
    Ok(())
}

#[test]
fn search_no_snippets_flag_shows_positions() -> Result<()> {
    let worktree = create_worktree()?;
    let content = "fn old_format() {}";
    write_file(worktree.path(), "src/lib.rs", content)?;
    build_index(worktree.path(), &[("src/lib.rs", content)])?;

    let output = run_collie(worktree.path(), &["search", "old_format", "--no-snippets"])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);

    assert!(
        !text.contains("Positions:"),
        "--no-snippets should not show positions, got: {}",
        text
    );
    Ok(())
}

#[test]
fn search_context_flag_controls_surrounding_lines() -> Result<()> {
    let worktree = create_worktree()?;
    let content =
        "line1\nline2\nline3\nline4\nfn context_target() {}\nline6\nline7\nline8\nline9\nline10\n";
    write_file(worktree.path(), "src/lib.rs", content)?;
    build_index(worktree.path(), &[("src/lib.rs", content)])?;

    let output = run_collie(worktree.path(), &["search", "context_target", "-C", "0"])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("context_target"));
    assert!(
        !text.contains("line1"),
        "-C 0 should not show line1, got: {}",
        text
    );
    assert!(
        !text.contains("line2"),
        "-C 0 should not show line2, got: {}",
        text
    );
    Ok(())
}

#[test]
fn search_context_merges_adjacent_matches() -> Result<()> {
    let worktree = create_worktree()?;
    let content = "line1\nfn match_a() {}\nfn match_b() {}\nline4\nline5\n";
    write_file(worktree.path(), "src/lib.rs", content)?;
    build_index(worktree.path(), &[("src/lib.rs", content)])?;

    let output = run_collie(worktree.path(), &["search", "%match_%", "-C", "1"])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);

    assert!(
        text.contains("match_a"),
        "should show match_a, got: {}",
        text
    );
    assert!(
        text.contains("match_b"),
        "should show match_b, got: {}",
        text
    );
    assert!(
        text.contains("line1"),
        "should show context above, got: {}",
        text
    );
    assert!(
        text.contains("line4"),
        "should show context below, got: {}",
        text
    );
    Ok(())
}

#[test]
fn search_snippet_file_not_found_shows_warning() -> Result<()> {
    let worktree = create_worktree()?;
    let content = "fn ephemeral() {}";
    let file = write_file(worktree.path(), "src/lib.rs", content)?;
    build_index(worktree.path(), &[("src/lib.rs", content)])?;

    fs::remove_file(&file)?;

    let output = run_collie(worktree.path(), &["search", "ephemeral"])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);

    assert!(
        text.contains("file not found") || text.contains("not found"),
        "should indicate the file no longer exists, got: {}",
        text
    );
    Ok(())
}

#[test]
fn search_shows_relative_paths() -> Result<()> {
    let worktree = create_worktree()?;
    let content = "fn relative_path_test() {}";
    write_file(worktree.path(), "src/deep/nested/module.rs", content)?;
    build_index(worktree.path(), &[("src/deep/nested/module.rs", content)])?;

    let output = run_collie(worktree.path(), &["search", "relative_path_test"])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);

    assert!(
        text.contains("src/deep/nested/module.rs"),
        "should show relative path, got: {}",
        text
    );
    Ok(())
}

#[test]
fn search_snippet_shows_pipe_separator() -> Result<()> {
    let worktree = create_worktree()?;
    let content = "fn pipe_test() {}";
    write_file(worktree.path(), "src/lib.rs", content)?;
    build_index(worktree.path(), &[("src/lib.rs", content)])?;

    let output = run_collie(worktree.path(), &["search", "pipe_test"])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);

    assert!(
        text.contains(" | "),
        "snippet lines should use ' | ' separator, got: {}",
        text
    );
    Ok(())
}
