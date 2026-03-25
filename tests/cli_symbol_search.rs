mod common;

use anyhow::Result;
use common::*;
use std::fs;

#[test]
fn cli_kind_filter_returns_symbols() -> Result<()> {
    let worktree = create_worktree()?;
    build_index(
        worktree.path(),
        &[(
            "pkg/api/handler.go",
            "package api\n\ntype Config struct {}\nfunc handleRequest() {}\nfunc handleError() {}\n",
        )],
    )?;

    let output = run_collie(worktree.path(), &["search", "kind:fn", "--no-snippets"])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);

    assert!(text.contains("Found 2 symbols for: kind:fn"));
    assert!(text.contains("handleRequest (function)"));
    assert!(text.contains("handleError (function)"));
    assert!(!text.contains("Config (struct)"));
    Ok(())
}

#[test]
fn cli_combined_filters() -> Result<()> {
    let worktree = create_worktree()?;
    build_index(
        worktree.path(),
        &[
            ("pkg/api/handler.go", "package api\n\nfunc handler() {}\n"),
            ("src/lib.rs", "fn handler() {}\n"),
        ],
    )?;

    let output = run_collie(
        worktree.path(),
        &["search", "kind:fn lang:go handler", "--no-snippets"],
    )?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);

    assert!(text.contains("Found 1 symbols for: kind:fn lang:go handler"));
    assert!(text.contains("pkg/api/handler.go"));
    assert!(text.contains("lang:go"));
    assert!(!text.contains("src/lib.rs"));
    Ok(())
}

#[test]
fn cli_path_filter() -> Result<()> {
    let worktree = create_worktree()?;
    build_index(
        worktree.path(),
        &[
            ("src/init.go", "package src\n\nfunc init() {}\n"),
            ("cmd/init.go", "package cmd\n\nfunc init() {}\n"),
        ],
    )?;

    let output = run_collie(
        worktree.path(),
        &["search", "kind:fn path:src/ init", "--no-snippets"],
    )?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);

    assert!(text.contains("Found 1 symbols for: kind:fn path:src/ init"));
    assert!(text.contains("src/init.go"));
    assert!(!text.contains("cmd/init.go"));
    Ok(())
}

#[test]
fn cli_plain_search_unchanged() -> Result<()> {
    let worktree = create_worktree()?;
    build_index(
        worktree.path(),
        &[("src/lib.rs", "fn handler() { handler(); }\n")],
    )?;

    let output = run_collie(worktree.path(), &["search", "handler", "--no-snippets"])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let lines: Vec<_> = stdout(&output).lines().map(str::to_string).collect();
    assert_eq!(lines[0], "Found 1 results for pattern: handler");
    assert_eq!(lines[2], "1. src/lib.rs");
    Ok(())
}

#[test]
fn cli_no_results_for_empty_kind() -> Result<()> {
    let worktree = create_worktree()?;
    build_index(
        worktree.path(),
        &[("pkg/api/handler.go", "package api\n\nfunc handler() {}\n")],
    )?;

    let output = run_collie(worktree.path(), &["search", "kind:trait", "--no-snippets"])?;
    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "No symbols found for: kind:trait");
    Ok(())
}

#[test]
fn cli_short_symbol_substring_shows_explicit_error() -> Result<()> {
    let worktree = create_worktree()?;
    build_index(
        worktree.path(),
        &[("pkg/api/handler.go", "package api\n\nfunc handler() {}\n")],
    )?;

    let output = run_collie(
        worktree.path(),
        &["search", "kind:fn %ab%", "--no-snippets"],
    )?;
    assert!(!output.status.success());
    assert_eq!(
        stderr(&output),
        "symbol substring search requires at least 3 chars"
    );
    Ok(())
}

#[test]
fn cli_symbol_regex_searches_full_symbol_result_set() -> Result<()> {
    let worktree = create_worktree()?;

    let mut content = String::from("package api\n\n");
    for idx in 0..600 {
        if idx == 599 {
            content.push_str("func fn599() {\n    println(\"needle\")\n}\n");
        } else {
            content.push_str(&format!("func fn{idx:03}() {{}}\n"));
        }
    }

    build_index(worktree.path(), &[("pkg/api/handlers.go", &content)])?;

    let output = run_collie(
        worktree.path(),
        &[
            "search",
            "kind:fn",
            "--symbol-regex",
            "needle",
            "--no-snippets",
            "-n",
            "1",
        ],
    )?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);

    assert!(text.contains("Found 1 symbols for: kind:fn"));
    assert!(
        text.contains("fn599 (function)"),
        "unexpected output: {}",
        text
    );
    Ok(())
}

#[test]
fn cli_symbol_regex_handles_shortened_source_without_panic() -> Result<()> {
    let worktree = create_worktree()?;
    build_index(
        worktree.path(),
        &[(
            "pkg/api/watched.go",
            "package api\n\nfunc watched() {\n    println(\"before\")\n    println(\"after\")\n}\n",
        )],
    )?;

    fs::write(worktree.path().join("pkg/api/watched.go"), "")?;

    let output = run_collie(
        worktree.path(),
        &[
            "search",
            "kind:fn",
            "--symbol-regex",
            "before",
            "--no-snippets",
        ],
    )?;
    assert_ne!(
        output.status.code(),
        Some(101),
        "stderr: {}",
        stderr(&output)
    );
    assert!(
        !stderr(&output).contains("panicked"),
        "unexpected panic output: {}",
        stderr(&output)
    );
    Ok(())
}
