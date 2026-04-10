mod common;

use anyhow::Result;
use collie_search::config::CollieConfig;
use collie_search::indexer::IndexBuilder;
use collie_search::storage::generation::GenerationManager;
use collie_search::symbols::query::parse_query;
use std::fs;
use std::path::{Path, PathBuf};

use common::{collie_dir, create_worktree, write_file};

fn setup_builder(root: &Path) -> Result<(PathBuf, IndexBuilder)> {
    let collie = collie_dir(root);
    let mgr = GenerationManager::new(&collie);
    let gen_dir = mgr.create_generation()?;
    let mut builder = IndexBuilder::new(&gen_dir, &CollieConfig::default())?;
    builder.set_worktree_root(common::canonical_root(root));
    Ok((gen_dir, builder))
}

#[test]
fn index_go_file_and_search_by_kind() -> Result<()> {
    let worktree = create_worktree()?;
    let file = write_file(
        worktree.path(),
        "pkg/api/handler.go",
        "package api\n\ntype Config struct {}\nfunc handle() {}\nfunc initServer() {}\n",
    )?;
    let (_gen_dir, mut builder) = setup_builder(worktree.path())?;

    builder.index_file(&file)?;
    builder.save()?;

    let results = builder.search_symbols(&parse_query("kind:fn"), 0)?;
    assert_eq!(results.len(), 2);
    assert_eq!(
        results[0].kind,
        collie_search::symbols::SymbolKind::Function
    );
    Ok(())
}

#[test]
fn index_rust_file_and_search_by_name() -> Result<()> {
    let worktree = create_worktree()?;
    let file = write_file(
        worktree.path(),
        "src/config.rs",
        "pub struct Config;\nfn new_config() -> Config { Config }\n",
    )?;
    let (_gen_dir, mut builder) = setup_builder(worktree.path())?;

    builder.index_file(&file)?;
    builder.save()?;

    let results = builder.search_symbols(&parse_query("kind:struct Config"), 0)?;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Config");
    Ok(())
}

#[test]
fn index_mixed_languages_filter_by_lang() -> Result<()> {
    let worktree = create_worktree()?;
    let go_file = write_file(
        worktree.path(),
        "pkg/api/handler.go",
        "package api\n\nfunc handler() {}\n",
    )?;
    let rs_file = write_file(worktree.path(), "src/lib.rs", "fn handler() {}\n")?;
    let (_gen_dir, mut builder) = setup_builder(worktree.path())?;

    builder.index_file(&go_file)?;
    builder.index_file(&rs_file)?;
    builder.save()?;

    let results = builder.search_symbols(&parse_query("kind:fn lang:go handler"), 0)?;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].language, "go");
    Ok(())
}

#[test]
fn search_with_path_prefix() -> Result<()> {
    let worktree = create_worktree()?;
    let file_a = write_file(
        worktree.path(),
        "pkg/api/handler.go",
        "package api\n\nfunc init() {}\n",
    )?;
    let file_b = write_file(
        worktree.path(),
        "cmd/main.go",
        "package main\n\nfunc init() {}\n",
    )?;
    let (_gen_dir, mut builder) = setup_builder(worktree.path())?;

    builder.index_file(&file_a)?;
    builder.index_file(&file_b)?;
    builder.save()?;

    let results = builder.search_symbols(&parse_query("kind:fn path:pkg/api/ init"), 0)?;
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].repo_rel_path,
        PathBuf::from("pkg/api/handler.go")
    );
    Ok(())
}

#[test]
fn search_with_name_wildcard() -> Result<()> {
    let worktree = create_worktree()?;
    let file = write_file(
        worktree.path(),
        "pkg/api/handler.go",
        "package api\n\nfunc handleRequest() {}\nfunc handleError() {}\nfunc processData() {}\n",
    )?;
    let (_gen_dir, mut builder) = setup_builder(worktree.path())?;

    builder.index_file(&file)?;
    builder.save()?;

    let results = builder.search_symbols(&parse_query("kind:fn handle%"), 0)?;
    assert_eq!(results.len(), 2);
    Ok(())
}

#[test]
fn search_rejects_unsupported_language_filter() -> Result<()> {
    let worktree = create_worktree()?;
    let file = write_file(
        worktree.path(),
        "pkg/api/handler.go",
        "package api\n\nfunc handler() {}\n",
    )?;
    let (_gen_dir, mut builder) = setup_builder(worktree.path())?;

    builder.index_file(&file)?;
    builder.save()?;

    let err = builder
        .search_symbols(&parse_query("kind:fn lang:js handler"), 0)
        .unwrap_err();
    assert!(
        err.to_string().contains("unsupported language filter: js"),
        "unexpected error: {err}"
    );
    Ok(())
}

#[test]
fn search_with_qualified_name_filter() -> Result<()> {
    let worktree = create_worktree()?;
    let file = write_file(
        worktree.path(),
        "src/server.rs",
        "struct Server;\nimpl Server { fn start(&self) {} }\n",
    )?;
    let (_gen_dir, mut builder) = setup_builder(worktree.path())?;

    builder.index_file(&file)?;
    builder.save()?;

    let results = builder.search_symbols(&parse_query("kind:method qname:Server::start%"), 0)?;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].container_name.as_deref(), Some("Server"));
    Ok(())
}

#[test]
fn reindex_updates_symbols() -> Result<()> {
    let worktree = create_worktree()?;
    let file = write_file(worktree.path(), "src/lib.rs", "fn old_func() {}\n")?;
    let (_gen_dir, mut builder) = setup_builder(worktree.path())?;

    builder.index_file(&file)?;
    builder.save()?;

    fs::write(&file, "fn new_func() {}\n")?;
    builder.index_file(&file)?;
    builder.save()?;

    assert!(
        builder
            .search_symbols(&parse_query("kind:fn old_func"), 0)?
            .is_empty()
    );
    let results = builder.search_symbols(&parse_query("kind:fn new_func"), 0)?;
    assert_eq!(results.len(), 1);
    Ok(())
}

#[test]
fn remove_file_clears_symbols() -> Result<()> {
    let worktree = create_worktree()?;
    let file = write_file(
        worktree.path(),
        "pkg/api/config.go",
        "package api\n\ntype Config struct {}\n",
    )?;
    let (_gen_dir, mut builder) = setup_builder(worktree.path())?;

    builder.index_file(&file)?;
    builder.save()?;
    builder.remove_file(&file);
    builder.save()?;

    assert!(
        builder
            .search_symbols(&parse_query("kind:struct Config"), 0)?
            .is_empty()
    );
    Ok(())
}

#[test]
fn unsupported_language_still_has_lexical_index() -> Result<()> {
    let worktree = create_worktree()?;
    let file = write_file(
        worktree.path(),
        "db/query.sql",
        "SELECT * FROM users WHERE id = 1;\n",
    )?;
    let (_gen_dir, mut builder) = setup_builder(worktree.path())?;

    builder.index_file(&file)?;
    builder.save()?;

    assert!(
        builder
            .search_symbols(&parse_query("kind:fn"), 0)?
            .is_empty()
    );
    let lexical = builder.search_pattern("SELECT");
    assert_eq!(lexical.len(), 1);
    Ok(())
}

#[test]
fn symbol_results_include_location() -> Result<()> {
    let worktree = create_worktree()?;
    let file = write_file(
        worktree.path(),
        "pkg/api/handler.go",
        "package api\n\nfunc handleRequest() {\n    println(\"ok\")\n}\n",
    )?;
    let (_gen_dir, mut builder) = setup_builder(worktree.path())?;

    builder.index_file(&file)?;
    builder.save()?;

    let results = builder.search_symbols(&parse_query("kind:fn"), 0)?;
    assert_eq!(results.len(), 1);
    let symbol = &results[0];
    assert_eq!(symbol.line_start, 3);
    assert_eq!(symbol.line_end, 5);
    assert!(symbol.byte_end > symbol.byte_start);
    Ok(())
}

#[test]
fn plain_search_without_filters_uses_lexical_index() -> Result<()> {
    let worktree = create_worktree()?;
    let file = write_file(
        worktree.path(),
        "pkg/api/handler.go",
        "package api\n\nfunc handler() {}\n",
    )?;
    let (_gen_dir, mut builder) = setup_builder(worktree.path())?;

    builder.index_file(&file)?;
    builder.save()?;

    let results = builder.search_pattern("handler");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].file_path, file);
    Ok(())
}
