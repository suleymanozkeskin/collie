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
        "FnProto" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let vis = if has_pub_prefix(node, source) {
                    Some("pub")
                } else {
                    Some("private")
                };
                symbols.push(make_symbol(
                    SymbolKind::Function, text(name_node, source),
                    path, node, source, None, vis, "zig",
                ));
            }
        }
        "TestDecl" => {
            // test "name" { ... }
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(make_symbol(
                    SymbolKind::Function, text(name_node, source),
                    path, node, source, None, None, "zig",
                ));
            }
        }
        "VarDecl" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let snippet = text(node, source);
                let kind = if snippet.trim_start().starts_with("const") {
                    SymbolKind::Constant
                } else {
                    SymbolKind::Variable
                };
                let vis = if has_pub_prefix(node, source) {
                    Some("pub")
                } else {
                    Some("private")
                };
                symbols.push(make_symbol(
                    kind, text(name_node, source),
                    path, node, source, None, vis, "zig",
                ));
            }
        }
        "ContainerDecl" | "ContainerDeclType" => {
            // struct, enum, union — the name comes from the parent VarDecl
        }
        _ => {}
    }

    for_each_child(node, |child| walk_zig(child, source, path, symbols));
}

fn has_pub_prefix(node: Node<'_>, source: &[u8]) -> bool {
    let snippet = text(node, source);
    snippet.trim_start().starts_with("pub ")
}
