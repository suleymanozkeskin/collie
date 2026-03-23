use std::path::Path;

use tree_sitter::{Node, Parser};

use super::helpers::{for_each_child, make_symbol, text};
use crate::symbols::adapters::LanguageAdapter;
use crate::symbols::{Symbol, SymbolKind};

pub struct CAdapter;

impl LanguageAdapter for CAdapter {
    fn language_id(&self) -> &str {
        "c"
    }

    fn file_extensions(&self) -> &[&str] {
        &["c", "h"]
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
        walk_c(tree.root_node(), content.as_bytes(), path, &mut symbols);
        symbols
    }
}

fn walk_c(node: Node<'_>, source: &[u8], path: &Path, symbols: &mut Vec<Symbol>) {
    match node.kind() {
        "function_definition" | "function_declarator" => {
            if node.kind() == "function_definition" {
                if let Some(declarator) = node.child_by_field_name("declarator") {
                    if let Some(name_node) = find_identifier(&declarator) {
                        symbols.push(make_symbol(
                            SymbolKind::Function,
                            text(name_node, source),
                            path, node, source, None, None, "c",
                        ));
                    }
                }
            }
        }
        "struct_specifier" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(make_symbol(
                    SymbolKind::Struct,
                    text(name_node, source),
                    path, node, source, None, None, "c",
                ));
            }
        }
        "enum_specifier" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(make_symbol(
                    SymbolKind::Enum,
                    text(name_node, source),
                    path, node, source, None, None, "c",
                ));
            }
        }
        "type_definition" => {
            if let Some(declarator) = node.child_by_field_name("declarator") {
                if let Some(name_node) = find_identifier(&declarator) {
                    symbols.push(make_symbol(
                        SymbolKind::TypeAlias,
                        text(name_node, source),
                        path, node, source, None, None, "c",
                    ));
                }
            }
        }
        "declaration" => {
            // Top-level variable/constant declarations
            if node.parent().map_or(false, |p| p.kind() == "translation_unit") {
                if let Some(declarator) = node.child_by_field_name("declarator") {
                    if let Some(name_node) = find_identifier(&declarator) {
                        symbols.push(make_symbol(
                            SymbolKind::Variable,
                            text(name_node, source),
                            path, node, source, None, None, "c",
                        ));
                    }
                }
            }
        }
        "preproc_def" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(make_symbol(
                    SymbolKind::Constant,
                    text(name_node, source),
                    path, node, source, None, None, "c",
                ));
            }
        }
        _ => {}
    }

    for_each_child(node, |child| walk_c(child, source, path, symbols));
}

fn find_identifier<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    if node.kind() == "identifier" {
        return Some(*node);
    }
    let count = node.child_count();
    for i in 0..count {
        if let Some(child) = node.child(i) {
            if let Some(found) = find_identifier(&child) {
                return Some(found);
            }
        }
    }
    None
}
