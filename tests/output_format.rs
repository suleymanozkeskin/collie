mod common;

use anyhow::Result;
use common::*;

#[test]
fn plain_format_shows_path_colon_line_colon_content() -> Result<()> {
    let worktree = create_worktree()?;
    let content = "line1\nfn target_func() {}\nline3\n";
    write_file(worktree.path(), "src/lib.rs", content)?;
    build_index(worktree.path(), &[("src/lib.rs", content)])?;

    let output = run_collie(
        worktree.path(),
        &["search", "target_func", "--format", "plain", "-C", "0"],
    )?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);

    let has_plain_line = text.lines().any(|line| {
        let parts: Vec<&str> = line.splitn(3, ':').collect();
        parts.len() == 3 && parts[0].contains("src/lib.rs") && parts[1].parse::<usize>().is_ok()
    });
    assert!(
        has_plain_line,
        "should have path:line:content format, got: {}",
        text
    );
    Ok(())
}

#[test]
fn plain_format_no_header() -> Result<()> {
    let worktree = create_worktree()?;
    let content = "fn rg_test() {}\n";
    write_file(worktree.path(), "src/lib.rs", content)?;
    build_index(worktree.path(), &[("src/lib.rs", content)])?;

    let output = run_collie(worktree.path(), &["search", "rg_test", "--format", "plain"])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(
        !text.contains("Found"),
        "plain format should not have 'Found' header, got: {}",
        text
    );
    Ok(())
}

#[test]
fn plain_format_context_lines_use_dash_separator() -> Result<()> {
    let worktree = create_worktree()?;
    let content = "ctx_before\nfn rg_ctx_match() {}\nctx_after\n";
    write_file(worktree.path(), "src/lib.rs", content)?;
    build_index(worktree.path(), &[("src/lib.rs", content)])?;

    let output = run_collie(
        worktree.path(),
        &["search", "rg_ctx_match", "--format", "plain", "-C", "1"],
    )?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);

    // Match line uses : separator
    assert!(
        text.contains("src/lib.rs:2:"),
        "match line should use ':', got: {}",
        text
    );
    // Context lines use - separator
    let has_context_dash = text.lines().any(|l| l.starts_with("src/lib.rs-"));
    assert!(
        has_context_dash,
        "context lines should use '-' separator, got: {}",
        text
    );
    Ok(())
}

#[test]
fn plain_format_group_separator_between_non_contiguous() -> Result<()> {
    let worktree = create_worktree()?;
    let content =
        "fn match_one() {}\nfiller1\nfiller2\nfiller3\nfiller4\nfiller5\nfn match_two() {}\n";
    write_file(worktree.path(), "src/lib.rs", content)?;
    build_index(worktree.path(), &[("src/lib.rs", content)])?;

    let output = run_collie(
        worktree.path(),
        &["search", "%match_%", "--format", "plain", "-C", "0"],
    )?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("--"),
        "should have group separator, got: {}",
        text
    );
    Ok(())
}

#[test]
fn plain_format_no_snippets_shows_paths_only() -> Result<()> {
    let worktree = create_worktree()?;
    let content = "fn rg_paths_only() {}\n";
    write_file(worktree.path(), "src/lib.rs", content)?;
    build_index(worktree.path(), &[("src/lib.rs", content)])?;

    let output = run_collie(
        worktree.path(),
        &[
            "search",
            "rg_paths_only",
            "--format",
            "plain",
            "--no-snippets",
        ],
    )?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("src/lib.rs"),
        "should show path, got: {}",
        text
    );
    assert!(
        !text.lines().any(|l| l.contains(":1:")),
        "no-snippets should not show line numbers, got: {}",
        text
    );
    Ok(())
}

#[test]
fn plain_format_with_regex_flag() -> Result<()> {
    let worktree = create_worktree()?;
    let content = "fn hello_world() {}\nfn goodbye_world() {}\n";
    write_file(worktree.path(), "src/lib.rs", content)?;
    build_index(worktree.path(), &[("src/lib.rs", content)])?;

    let output = run_collie(
        worktree.path(),
        &[
            "search",
            "-e",
            "hello.*world",
            "--format",
            "plain",
            "-C",
            "0",
        ],
    )?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);

    let has_plain_line = text.lines().any(|line| {
        let parts: Vec<&str> = line.splitn(3, ':').collect();
        parts.len() == 3 && parts[0].contains("src/lib.rs") && parts[1] == "1"
    });
    assert!(
        has_plain_line,
        "regex + plain format should work together, got: {}",
        text
    );
    Ok(())
}

#[test]
fn plain_format_no_results_prints_nothing() -> Result<()> {
    let worktree = create_worktree()?;
    let content = "fn unrelated() {}\n";
    write_file(worktree.path(), "src/lib.rs", content)?;
    build_index(worktree.path(), &[("src/lib.rs", content)])?;

    let output = run_collie(
        worktree.path(),
        &["search", "nonexistent", "--format", "plain"],
    )?;
    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.is_empty(),
        "plain format with no results should print nothing, got: {}",
        text
    );
    Ok(())
}
