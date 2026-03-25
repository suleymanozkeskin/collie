use std::path::Path;

use tree_sitter::{Node, Parser};

use super::helpers::{for_each_child, make_symbol, text};
use crate::symbols::adapters::LanguageAdapter;
use crate::symbols::{Symbol, SymbolKind};

pub struct CSharpAdapter;

impl LanguageAdapter for CSharpAdapter {
    fn language_id(&self) -> &str {
        "csharp"
    }

    fn file_extensions(&self) -> &[&str] {
        &["cs"]
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
        walk_csharp(
            tree.root_node(),
            content.as_bytes(),
            path,
            &mut symbols,
            None,
        );
        symbols
    }
}

fn walk_csharp(
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
                    SymbolKind::Class,
                    name,
                    path,
                    node,
                    source,
                    container,
                    None,
                    "csharp",
                ));
                for_each_child(node, |child| {
                    walk_csharp(child, source, path, symbols, Some(name));
                });
                return;
            }
        }
        "struct_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                symbols.push(make_symbol(
                    SymbolKind::Struct,
                    name,
                    path,
                    node,
                    source,
                    container,
                    None,
                    "csharp",
                ));
                for_each_child(node, |child| {
                    walk_csharp(child, source, path, symbols, Some(name));
                });
                return;
            }
        }
        "interface_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                symbols.push(make_symbol(
                    SymbolKind::Interface,
                    name,
                    path,
                    node,
                    source,
                    container,
                    None,
                    "csharp",
                ));
                for_each_child(node, |child| {
                    walk_csharp(child, source, path, symbols, Some(name));
                });
                return;
            }
        }
        "enum_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(make_symbol(
                    SymbolKind::Enum,
                    text(name_node, source),
                    path,
                    node,
                    source,
                    container,
                    None,
                    "csharp",
                ));
            }
        }
        "namespace_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                symbols.push(make_symbol(
                    SymbolKind::Module,
                    name,
                    path,
                    node,
                    source,
                    None,
                    None,
                    "csharp",
                ));
                for_each_child(node, |child| {
                    walk_csharp(child, source, path, symbols, Some(name));
                });
                return;
            }
        }
        "method_declaration" | "constructor_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let kind = if container.is_some() {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                };
                symbols.push(make_symbol(
                    kind,
                    text(name_node, source),
                    path,
                    node,
                    source,
                    container,
                    None,
                    "csharp",
                ));
            }
        }
        "property_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(make_symbol(
                    SymbolKind::Property,
                    text(name_node, source),
                    path,
                    node,
                    source,
                    container,
                    None,
                    "csharp",
                ));
            }
        }
        "field_declaration" => {
            if let Some(declarator) = node.child_by_field_name("declarator") {
                let count = declarator.child_count();
                for i in 0..count {
                    if let Some(child) = declarator.child(i) {
                        if child.kind() == "variable_declarator" {
                            if let Some(name_node) = child.child_by_field_name("name") {
                                symbols.push(make_symbol(
                                    SymbolKind::Field,
                                    text(name_node, source),
                                    path,
                                    node,
                                    source,
                                    container,
                                    None,
                                    "csharp",
                                ));
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }

    for_each_child(node, |child| {
        walk_csharp(child, source, path, symbols, container)
    });
}
