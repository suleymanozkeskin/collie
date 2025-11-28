use anyhow::Result;
use collie_search::storage::tantivy_index::TantivyIndex;
use collie_search::symbols::{Symbol, SymbolKind, SymbolQuery};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn open_index() -> Result<(TempDir, TantivyIndex)> {
    let temp = TempDir::new()?;
    let index = TantivyIndex::open(&temp.path().join("tantivy"))?;
    Ok((temp, index))
}

fn sym(kind: SymbolKind, name: &str, language: &str, rel_path: &str) -> Symbol {
    Symbol {
        kind,
        name: name.to_string(),
        qualified_name: None,
        language: language.to_string(),
        repo_rel_path: PathBuf::from(rel_path),
        container_name: None,
        visibility: None,
        signature: None,
        line_start: 1,
        line_end: 1,
        byte_start: 0,
        byte_end: name.len() as u32,
        doc: None,
    }
}

#[test]
fn index_and_search_symbol_by_name() -> Result<()> {
    let (_temp, mut index) = open_index()?;
    index.index_symbols(
        Path::new("/a.go"),
        &[
            sym(
                SymbolKind::Function,
                "handleRequest",
                "go",
                "pkg/api/handler.go",
            ),
            sym(SymbolKind::Struct, "Config", "go", "pkg/api/handler.go"),
            sym(SymbolKind::Variable, "timeout", "go", "pkg/api/handler.go"),
        ],
    )?;
    index.commit()?;

    let results = index.search_symbols(
        &SymbolQuery {
            name_pattern: "handlerequest".to_string(),
            ..SymbolQuery::default()
        },
        0,
    )?;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].kind, SymbolKind::Function);
    assert!(results[0].name.contains("handleRequest"));
    Ok(())
}

#[test]
fn search_symbols_by_kind() -> Result<()> {
    let (_temp, mut index) = open_index()?;
    index.index_symbols(
        Path::new("/a.go"),
        &[
            sym(SymbolKind::Function, "foo", "go", "a.go"),
            sym(SymbolKind::Struct, "Bar", "go", "a.go"),
            sym(SymbolKind::Function, "baz", "go", "a.go"),
        ],
    )?;
    index.commit()?;

    let results = index.search_symbols(
        &SymbolQuery {
            kind: Some(SymbolKind::Function),
            ..SymbolQuery::default()
        },
        0,
    )?;
    assert_eq!(results.len(), 2);
    Ok(())
}

#[test]
fn search_symbols_by_language() -> Result<()> {
    let (_temp, mut index) = open_index()?;
    index.index_symbols(
        Path::new("/a.go"),
        &[sym(SymbolKind::Function, "go_fn", "go", "a.go")],
    )?;
    index.index_symbols(
        Path::new("/b.rs"),
        &[sym(SymbolKind::Function, "rs_fn", "rust", "b.rs")],
    )?;
    index.commit()?;

    let results = index.search_symbols(
        &SymbolQuery {
            language: Some("go".to_string()),
            ..SymbolQuery::default()
        },
        0,
    )?;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].language, "go");
    Ok(())
}

#[test]
fn search_symbols_by_path_prefix() -> Result<()> {
    let (_temp, mut index) = open_index()?;
    index.index_symbols(
        Path::new("/pkg/api/handler.go"),
        &[sym(
            SymbolKind::Function,
            "handler",
            "go",
            "pkg/api/handler.go",
        )],
    )?;
    index.index_symbols(
        Path::new("/cmd/main.go"),
        &[sym(SymbolKind::Function, "main", "go", "cmd/main.go")],
    )?;
    index.commit()?;

    let results = index.search_symbols(
        &SymbolQuery {
            path_prefix: Some("pkg/api/".to_string()),
            ..SymbolQuery::default()
        },
        0,
    )?;
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].repo_rel_path,
        PathBuf::from("pkg/api/handler.go")
    );
    Ok(())
}

#[test]
fn remove_by_path_clears_symbols_and_tokens() -> Result<()> {
    let (_temp, mut index) = open_index()?;

    // Index file content
    index.index_file_content(Path::new("/a.go"), "handler other")?;
    // Index symbols
    index.index_symbols(
        Path::new("/a.go"),
        &[sym(SymbolKind::Function, "handler", "go", "a.go")],
    )?;
    index.commit()?;

    // Remove by path
    index.remove_by_path(Path::new("/a.go"))?;
    index.commit()?;

    // Both should be empty
    assert!(index.search_exact("handler").is_empty());
    let sym_results = index.search_symbols(
        &SymbolQuery {
            name_pattern: "handler".to_string(),
            ..SymbolQuery::default()
        },
        0,
    )?;
    assert!(sym_results.is_empty());
    Ok(())
}

#[test]
fn symbol_name_prefix_search() -> Result<()> {
    let (_temp, mut index) = open_index()?;
    index.index_symbols(
        Path::new("/a.go"),
        &[
            sym(SymbolKind::Function, "handleRequest", "go", "a.go"),
            sym(SymbolKind::Function, "handleError", "go", "a.go"),
            sym(SymbolKind::Function, "processData", "go", "a.go"),
        ],
    )?;
    index.commit()?;

    let results = index.search_symbols(
        &SymbolQuery {
            name_pattern: "handle%".to_string(),
            ..SymbolQuery::default()
        },
        0,
    )?;
    assert_eq!(results.len(), 2);
    Ok(())
}

#[test]
fn symbol_name_suffix_search() -> Result<()> {
    let (_temp, mut index) = open_index()?;
    index.index_symbols(
        Path::new("/a.go"),
        &[
            sym(SymbolKind::Function, "handleRequest", "go", "a.go"),
            sym(SymbolKind::Function, "processRequest", "go", "a.go"),
            sym(SymbolKind::Function, "getData", "go", "a.go"),
        ],
    )?;
    index.commit()?;

    let results = index.search_symbols(
        &SymbolQuery {
            name_pattern: "%request".to_string(),
            ..SymbolQuery::default()
        },
        0,
    )?;
    assert_eq!(results.len(), 2);
    Ok(())
}

#[test]
fn symbol_name_substring_search() -> Result<()> {
    let (_temp, mut index) = open_index()?;
    index.index_symbols(
        Path::new("/a.go"),
        &[
            sym(SymbolKind::Function, "handleRequest", "go", "a.go"),
            sym(SymbolKind::Function, "myRequestHandler", "go", "a.go"),
            sym(SymbolKind::Function, "getData", "go", "a.go"),
        ],
    )?;
    index.commit()?;

    let results = index.search_symbols(
        &SymbolQuery {
            name_pattern: "%request%".to_string(),
            ..SymbolQuery::default()
        },
        0,
    )?;
    assert_eq!(results.len(), 2);
    Ok(())
}

#[test]
fn combined_symbol_filters() -> Result<()> {
    let (_temp, mut index) = open_index()?;
    index.index_symbols(
        Path::new("/a.go"),
        &[sym(SymbolKind::Function, "handleRequest", "go", "a.go")],
    )?;
    index.index_symbols(
        Path::new("/b.rs"),
        &[sym(SymbolKind::Struct, "HandleRequest", "rust", "b.rs")],
    )?;
    index.index_symbols(
        Path::new("/c.go"),
        &[sym(SymbolKind::Function, "handleError", "go", "c.go")],
    )?;
    index.commit()?;

    let results = index.search_symbols(
        &SymbolQuery {
            kind: Some(SymbolKind::Function),
            language: Some("go".to_string()),
            name_pattern: "handlerequest".to_string(),
            ..SymbolQuery::default()
        },
        0,
    )?;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "handleRequest");
    Ok(())
}

#[test]
fn qualified_name_search() -> Result<()> {
    let (_temp, mut index) = open_index()?;
    let mut symbol = sym(SymbolKind::Method, "handleRequest", "rust", "a.rs");
    symbol.qualified_name = Some("Server::handleRequest".to_string());
    index.index_symbols(Path::new("/a.rs"), &[symbol])?;
    index.commit()?;

    let results = index.search_symbols(
        &SymbolQuery {
            qualified_name_pattern: Some("server::handlerequest".to_string()),
            ..SymbolQuery::default()
        },
        0,
    )?;
    assert_eq!(results.len(), 1);
    Ok(())
}

#[test]
fn symbol_results_include_all_fields() -> Result<()> {
    let (_temp, mut index) = open_index()?;
    let mut symbol = sym(
        SymbolKind::Function,
        "handleRequest",
        "go",
        "pkg/api/handler.go",
    );
    symbol.qualified_name = Some("Server::handleRequest".to_string());
    symbol.container_name = Some("Server".to_string());
    symbol.visibility = Some("pub".to_string());
    symbol.signature = Some("func handleRequest()".to_string());
    symbol.line_start = 10;
    symbol.line_end = 12;
    symbol.byte_start = 100;
    symbol.byte_end = 140;
    symbol.doc = Some("docs".to_string());
    index.index_symbols(Path::new("/pkg/api/handler.go"), &[symbol])?;
    index.commit()?;

    let result = index.search_symbols(
        &SymbolQuery {
            name_pattern: "handlerequest".to_string(),
            ..SymbolQuery::default()
        },
        0,
    )?;
    assert_eq!(result.len(), 1);
    let result = &result[0];
    assert_eq!(result.kind, SymbolKind::Function);
    assert_eq!(result.name, "handleRequest");
    assert_eq!(
        result.qualified_name.as_deref(),
        Some("Server::handleRequest")
    );
    assert_eq!(result.language, "go");
    assert_eq!(result.repo_rel_path, PathBuf::from("pkg/api/handler.go"));
    assert_eq!(result.container_name.as_deref(), Some("Server"));
    assert_eq!(result.visibility.as_deref(), Some("pub"));
    assert_eq!(result.signature.as_deref(), Some("func handleRequest()"));
    assert_eq!(result.line_start, 10);
    assert_eq!(result.line_end, 12);
    assert_eq!(result.byte_start, 100);
    assert_eq!(result.byte_end, 140);
    assert_eq!(result.doc.as_deref(), Some("docs"));
    Ok(())
}

#[test]
fn symbol_search_results_ordered() -> Result<()> {
    let (_temp, mut index) = open_index()?;

    let mut s1 = sym(SymbolKind::Function, "bbb", "go", "b.go");
    s1.line_start = 20;
    let mut s2 = sym(SymbolKind::Function, "aaa", "go", "a.go");
    s2.line_start = 20;
    let mut s3 = sym(SymbolKind::Method, "ccc", "go", "a.go");
    s3.line_start = 10;
    let mut s4 = sym(SymbolKind::Function, "ddd", "go", "a.go");
    s4.line_start = 10;

    index.index_symbols(Path::new("/b.go"), &[s1])?;
    index.index_symbols(Path::new("/a.go"), &[s2, s3, s4])?;
    index.commit()?;

    let results = index.search_symbols(&SymbolQuery::default(), 0)?;
    let names: Vec<_> = results.into_iter().map(|s| s.name).collect();
    assert_eq!(names, vec!["ddd", "ccc", "aaa", "bbb"]);
    Ok(())
}

#[test]
fn case_insensitive_symbol_name() -> Result<()> {
    let (_temp, mut index) = open_index()?;
    index.index_symbols(
        Path::new("/a.go"),
        &[sym(
            SymbolKind::Function,
            "SharedInformerFactory",
            "go",
            "a.go",
        )],
    )?;
    index.commit()?;

    let results = index.search_symbols(
        &SymbolQuery {
            name_pattern: "sharedinformerfactory".to_string(),
            ..SymbolQuery::default()
        },
        0,
    )?;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "SharedInformerFactory");
    Ok(())
}

#[test]
fn short_substring_query_rejected() -> Result<()> {
    let (_temp, mut index) = open_index()?;
    index.index_symbols(
        Path::new("/a.go"),
        &[sym(SymbolKind::Function, "cat", "go", "a.go")],
    )?;
    index.commit()?;

    let error = index
        .search_symbols(
            &SymbolQuery {
                name_pattern: "%at%".to_string(),
                ..SymbolQuery::default()
            },
            0,
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("symbol substring search requires at least 3 chars")
    );
    Ok(())
}

#[test]
fn symbol_name_parts_camel_case() -> Result<()> {
    let (_temp, mut index) = open_index()?;
    index.index_symbols(
        Path::new("/a.go"),
        &[sym(SymbolKind::Function, "getPayingUsers", "go", "a.go")],
    )?;
    index.commit()?;
    let results = index.search_symbols(
        &SymbolQuery {
            kind: Some(SymbolKind::Function),
            name_pattern: "paying users".to_string(),
            ..SymbolQuery::default()
        },
        0,
    )?;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "getPayingUsers");
    Ok(())
}

#[test]
fn symbol_name_parts_snake_case() -> Result<()> {
    let (_temp, mut index) = open_index()?;
    index.index_symbols(
        Path::new("/a.go"),
        &[sym(SymbolKind::Function, "new_webhook_token", "go", "a.go")],
    )?;
    index.commit()?;
    let results = index.search_symbols(
        &SymbolQuery {
            kind: Some(SymbolKind::Function),
            name_pattern: "webhook token".to_string(),
            ..SymbolQuery::default()
        },
        0,
    )?;
    assert_eq!(results.len(), 1);
    Ok(())
}

#[test]
fn symbol_name_parts_pascal_case() -> Result<()> {
    let (_temp, mut index) = open_index()?;
    index.index_symbols(
        Path::new("/a.go"),
        &[sym(
            SymbolKind::Struct,
            "SharedInformerFactory",
            "go",
            "a.go",
        )],
    )?;
    index.commit()?;
    let results = index.search_symbols(
        &SymbolQuery {
            kind: Some(SymbolKind::Struct),
            name_pattern: "informer factory".to_string(),
            ..SymbolQuery::default()
        },
        0,
    )?;
    assert_eq!(results.len(), 1);
    Ok(())
}

#[test]
fn symbol_name_parts_no_false_match() -> Result<()> {
    let (_temp, mut index) = open_index()?;
    index.index_symbols(
        Path::new("/a.go"),
        &[sym(SymbolKind::Function, "getPayingUsers", "go", "a.go")],
    )?;
    index.commit()?;
    let results = index.search_symbols(
        &SymbolQuery {
            kind: Some(SymbolKind::Function),
            name_pattern: "billing users".to_string(),
            ..SymbolQuery::default()
        },
        0,
    )?;
    assert_eq!(results.len(), 0);
    Ok(())
}

#[test]
fn qualified_name_parts_search() -> Result<()> {
    let (_temp, mut index) = open_index()?;
    let mut symbol = sym(SymbolKind::Method, "handleRequest", "rust", "a.rs");
    symbol.qualified_name = Some("Server::handleRequest".to_string());
    index.index_symbols(Path::new("/a.rs"), &[symbol])?;
    index.commit()?;
    let results = index.search_symbols(
        &SymbolQuery {
            kind: Some(SymbolKind::Method),
            qualified_name_pattern: Some("server handle request".to_string()),
            ..SymbolQuery::default()
        },
        0,
    )?;
    assert_eq!(results.len(), 1);
    Ok(())
}
