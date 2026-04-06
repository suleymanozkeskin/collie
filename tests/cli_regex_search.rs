mod common;

use anyhow::Result;
use common::*;
use std::fs;

#[test]
fn regex_search_finds_matching_lines() -> Result<()> {
    let worktree = create_worktree()?;
    let content = "fn hello_world() {}\nfn goodbye_world() {}\n";
    write_file(worktree.path(), "src/lib.rs", content)?;
    build_index(worktree.path(), &[("src/lib.rs", content)])?;

    let output = run_collie(worktree.path(), &["search", "-e", "hello_world"])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("hello_world"),
        "should find the match, got: {}",
        text
    );
    Ok(())
}

#[test]
fn regex_search_with_real_regex_pattern() -> Result<()> {
    let worktree = create_worktree()?;
    let content = "fn hello_world() {}\nfn goodbye_world() {}\n";
    write_file(worktree.path(), "src/lib.rs", content)?;
    build_index(worktree.path(), &[("src/lib.rs", content)])?;

    let output = run_collie(worktree.path(), &["search", "--regex", "hello.*world"])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("hello_world"),
        "should find regex match, got: {}",
        text
    );
    Ok(())
}

#[test]
fn regex_search_with_alternation() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(worktree.path(), "src/a.rs", "// TODO: fix this\n")?;
    write_file(worktree.path(), "src/b.rs", "// FIXME: broken\n")?;
    write_file(worktree.path(), "src/c.rs", "// nothing here\n")?;
    build_index(
        worktree.path(),
        &[
            ("src/a.rs", "// TODO: fix this\n"),
            ("src/b.rs", "// FIXME: broken\n"),
            ("src/c.rs", "// nothing here\n"),
        ],
    )?;

    let output = run_collie(
        worktree.path(),
        &["search", "-e", "TODO|FIXME", "--no-snippets"],
    )?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("a.rs"), "should find a.rs (TODO): {}", text);
    assert!(text.contains("b.rs"), "should find b.rs (FIXME): {}", text);
    assert!(!text.contains("c.rs"), "should not find c.rs: {}", text);
    Ok(())
}

#[test]
fn regex_search_no_match() -> Result<()> {
    let worktree = create_worktree()?;
    let content = "fn unrelated() {}\n";
    write_file(worktree.path(), "src/lib.rs", content)?;
    build_index(worktree.path(), &[("src/lib.rs", content)])?;

    let output = run_collie(worktree.path(), &["search", "-e", "nonexistent_pattern"])?;
    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("No results"),
        "should report no results, got: {}",
        text
    );
    Ok(())
}

#[test]
fn regex_search_invalid_pattern_reports_error() -> Result<()> {
    let worktree = create_worktree()?;
    let content = "fn test() {}\n";
    write_file(worktree.path(), "src/lib.rs", content)?;
    build_index(worktree.path(), &[("src/lib.rs", content)])?;

    let output = run_collie(worktree.path(), &["search", "-e", "[invalid"])?;
    assert!(!output.status.success(), "should fail for invalid regex");
    Ok(())
}

#[test]
fn regex_search_with_no_snippets_shows_paths_only() -> Result<()> {
    let worktree = create_worktree()?;
    let content = "fn target_func() {}\n";
    write_file(worktree.path(), "src/lib.rs", content)?;
    build_index(worktree.path(), &[("src/lib.rs", content)])?;

    let output = run_collie(
        worktree.path(),
        &["search", "-e", "target_func", "--no-snippets"],
    )?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("src/lib.rs"),
        "should show file path, got: {}",
        text
    );
    assert!(
        !text.contains(" | "),
        "should not show snippet lines, got: {}",
        text
    );
    Ok(())
}

#[test]
fn regex_search_excludes_non_matching_files() -> Result<()> {
    let worktree = create_worktree()?;
    write_file(
        worktree.path(),
        "src/a.rs",
        "fn foo() { bar().unwrap(); }\n",
    )?;
    write_file(worktree.path(), "src/b.rs", "fn baz() { qux(); }\n")?;
    build_index(
        worktree.path(),
        &[
            ("src/a.rs", "fn foo() { bar().unwrap(); }\n"),
            ("src/b.rs", "fn baz() { qux(); }\n"),
        ],
    )?;

    let output = run_collie(
        worktree.path(),
        &["search", "-e", r"\.unwrap\(\)", "--no-snippets"],
    )?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("a.rs"), "should find a.rs: {}", text);
    assert!(!text.contains("b.rs"), "should not find b.rs: {}", text);
    Ok(())
}

#[test]
fn regex_search_dot_plus_scans_all_files() -> Result<()> {
    let worktree = create_worktree()?;
    let content = "fn anything() {}\n";
    write_file(worktree.path(), "src/lib.rs", content)?;
    build_index(worktree.path(), &[("src/lib.rs", content)])?;

    let output = run_collie(worktree.path(), &["search", "-e", ".+"])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("Found"), "should find results, got: {}", text);
    Ok(())
}

#[test]
fn regex_search_context_flag_shows_surrounding_lines() -> Result<()> {
    let worktree = create_worktree()?;
    let content = "line1\nline2\nfn target_func() {}\nline4\nline5\n";
    write_file(worktree.path(), "src/lib.rs", content)?;
    build_index(worktree.path(), &[("src/lib.rs", content)])?;

    let output = run_collie(
        worktree.path(),
        &["search", "-e", "target_func", "-n", "0", "-C", "1"],
    )?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("line2"),
        "should show before-context, got: {}",
        text
    );
    assert!(
        text.contains("target_func"),
        "should show match line, got: {}",
        text
    );
    assert!(
        text.contains("line4"),
        "should show after-context, got: {}",
        text
    );
    Ok(())
}

#[test]
fn regex_search_uses_indexed_text_when_source_file_is_missing() -> Result<()> {
    let worktree = create_worktree()?;
    let content = "line1\nline2\nfn target_func() {}\nline4\n";
    let file_path = write_file(worktree.path(), "src/lib.rs", content)?;
    build_index(worktree.path(), &[("src/lib.rs", content)])?;
    fs::remove_file(&file_path)?;

    let output = run_collie(
        worktree.path(),
        &["search", "-e", "target_func", "-n", "0", "-C", "1"],
    )?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("target_func"),
        "should show match from indexed text: {}",
        text
    );
    assert!(
        text.contains("line2"),
        "should show stored before-context: {}",
        text
    );
    assert!(
        text.contains("line4"),
        "should show stored after-context: {}",
        text
    );
    Ok(())
}

#[test]
fn regex_count_ignores_default_limit() -> Result<()> {
    let worktree = create_worktree()?;
    let mut files = Vec::new();
    for i in 0..25 {
        let rel = format!("src/file_{i:02}.rs");
        let content = "fn handle_request() { handle_request(); }\n".to_string();
        files.push((rel, content));
    }

    let tuples: Vec<(&str, &str)> = files
        .iter()
        .map(|(path, content)| (path.as_str(), content.as_str()))
        .collect();
    build_index(worktree.path(), &tuples)?;

    let output = run_collie(
        worktree.path(),
        &["search", "-e", "handle_request", "--count"],
    )?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "25");
    Ok(())
}
