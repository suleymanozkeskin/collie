use std::path::Path;

use tree_sitter::{Node, Parser};

use super::helpers::{for_each_child, make_symbol, text};
use crate::symbols::adapters::LanguageAdapter;
use crate::symbols::{Symbol, SymbolKind};

pub struct ZigAdapter;

impl LanguageAdapter for ZigAdapter {
    fn language_id(&self) -> &str {
        "zig"
    }

    fn file_extensions(&self) -> &[&str] {
        &["zig"]
    }

    fn extract_symbols_with_parser(
        &self,
        path: &Path,
        content: &str,
        parser: &mut Parser,
    ) -> Vec<Symbol> {
        let Some(tree) = parser.parse(content, None) else {
            return Vec::new();
        };
        let mut symbols = Vec::new();
        walk_zig(tree.root_node(), content.as_bytes(), path, &mut symbols);
        symbols
    }
}

fn walk_zig(node: Node<'_>, source: &[u8], path: &Path, symbols: &mut Vec<Symbol>) {
    match node.kind() {
        "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let vis = if is_pub(node, source) {
                    Some("pub")
                } else {
                    Some("private")
                };
                symbols.push(make_symbol(
                    SymbolKind::Function,
                    text(name_node, source),
                    path,
                    node,
                    source,
                    None,
                    vis,
                    "zig",
                ));
            }
        }
        "test_declaration" => {
            // test "name" { ... } — find the string child
            let count = node.child_count();
            for i in 0..count {
                if let Some(child) = node.child(i) {
                    if child.kind() == "string" {
                        let name = text(child, source).trim_matches('"');
                        if !name.is_empty() {
                            symbols.push(make_symbol(
                                SymbolKind::Function,
                                name,
                                path,
                                node,
                                source,
                                None,
                                None,
                                "zig",
                            ));
                        }
                        break;
                    }
                }
            }
        }
        "variable_declaration" => {
            // Find the identifier child (not a named field)
            if let Some(name) = find_child_identifier(node, source) {
                let snippet = text(node, source);
                let (kind, vis) = if snippet.trim_start().starts_with("pub ") {
                    if snippet.contains("const ") {
                        (SymbolKind::Constant, Some("pub"))
                    } else {
                        (SymbolKind::Variable, Some("pub"))
                    }
                } else if snippet.trim_start().starts_with("const ") || snippet.contains(" const ")
                {
                    (SymbolKind::Constant, Some("private"))
                } else {
                    (SymbolKind::Variable, Some("private"))
                };
                symbols.push(make_symbol(
                    kind, name, path, node, source, None, vis, "zig",
                ));
            }
        }
        _ => {}
    }

    for_each_child(node, |child| walk_zig(child, source, path, symbols));
}

fn is_pub(node: Node<'_>, source: &[u8]) -> bool {
    text(node, source).trim_start().starts_with("pub ")
}

fn find_child_identifier<'a>(node: Node<'a>, source: &'a [u8]) -> Option<&'a str> {
    let count = node.child_count();
    for i in 0..count {
        if let Some(child) = node.child(i) {
            if child.kind() == "identifier" {
                return Some(text(child, source));
            }
        }
    }
    None
}
