use std::path::Path;

use tree_sitter::{Node, Parser};

use super::helpers::{for_each_child, make_symbol, text};
use crate::symbols::adapters::LanguageAdapter;
use crate::symbols::{Symbol, SymbolKind};

pub struct KotlinAdapter;

impl LanguageAdapter for KotlinAdapter {
    fn language_id(&self) -> &str {
        "kotlin"
    }

    fn file_extensions(&self) -> &[&str] {
        &["kt", "kts"]
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
        walk_kotlin(tree.root_node(), content.as_bytes(), path, &mut symbols, None);
        symbols
    }
}

fn walk_kotlin(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    symbols: &mut Vec<Symbol>,
    container: Option<&str>,
) {
    match node.kind() {
        "class_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                symbols.push(make_symbol(
                    SymbolKind::Class, name,
                    path, node, source, container, None, "kotlin",
                ));
                for_each_child(node, |child| {
                    walk_kotlin(child, source, path, symbols, Some(name));
                });
                return;
            }
        }
        "object_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                symbols.push(make_symbol(
                    SymbolKind::Class, name,
                    path, node, source, container, None, "kotlin",
                ));
                for_each_child(node, |child| {
                    walk_kotlin(child, source, path, symbols, Some(name));
                });
                return;
            }
        }
        "interface_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                symbols.push(make_symbol(
                    SymbolKind::Interface, name,
                    path, node, source, container, None, "kotlin",
                ));
                for_each_child(node, |child| {
                    walk_kotlin(child, source, path, symbols, Some(name));
                });
                return;
            }
        }
        "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let kind = if container.is_some() {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                };
                symbols.push(make_symbol(
                    kind, text(name_node, source),
                    path, node, source, container, None, "kotlin",
                ));
            }
        }
        "property_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(make_symbol(
                    SymbolKind::Property, text(name_node, source),
                    path, node, source, container, None, "kotlin",
                ));
            }
        }
        "enum_class_body" => {
            // Walk enum entries
        }
        "type_alias" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(make_symbol(
                    SymbolKind::TypeAlias, text(name_node, source),
                    path, node, source, container, None, "kotlin",
                ));
            }
        }
        _ => {}
    }

    for_each_child(node, |child| walk_kotlin(child, source, path, symbols, container));
}
