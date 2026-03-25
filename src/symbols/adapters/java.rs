use std::path::Path;

use tree_sitter::{Node, Parser};

use super::helpers::{for_each_child, make_symbol, text};
use crate::symbols::adapters::LanguageAdapter;
use crate::symbols::{Symbol, SymbolKind};

pub struct JavaAdapter;

impl LanguageAdapter for JavaAdapter {
    fn language_id(&self) -> &str {
        "java"
    }

    fn file_extensions(&self) -> &[&str] {
        &["java"]
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
        walk_java(
            tree.root_node(),
            content.as_bytes(),
            path,
            &mut symbols,
            None,
        );
        symbols
    }
}

fn walk_java(
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
                let vis = java_visibility(node, source);
                symbols.push(make_symbol(
                    SymbolKind::Class,
                    name,
                    path,
                    node,
                    source,
                    container,
                    vis,
                    "java",
                ));
                for_each_child(node, |child| {
                    walk_java(child, source, path, symbols, Some(name));
                });
                return;
            }
        }
        "interface_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let vis = java_visibility(node, source);
                symbols.push(make_symbol(
                    SymbolKind::Interface,
                    name,
                    path,
                    node,
                    source,
                    container,
                    vis,
                    "java",
                ));
                for_each_child(node, |child| {
                    walk_java(child, source, path, symbols, Some(name));
                });
                return;
            }
        }
        "enum_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let vis = java_visibility(node, source);
                symbols.push(make_symbol(
                    SymbolKind::Enum,
                    name,
                    path,
                    node,
                    source,
                    container,
                    vis,
                    "java",
                ));
                for_each_child(node, |child| {
                    walk_java(child, source, path, symbols, Some(name));
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
                let vis = java_visibility(node, source);
                symbols.push(make_symbol(
                    kind,
                    text(name_node, source),
                    path,
                    node,
                    source,
                    container,
                    vis,
                    "java",
                ));
            }
        }
        "field_declaration" => {
            if let Some(declarator) = node.child_by_field_name("declarator") {
                if let Some(name_node) = declarator.child_by_field_name("name") {
                    let vis = java_visibility(node, source);
                    symbols.push(make_symbol(
                        SymbolKind::Field,
                        text(name_node, source),
                        path,
                        node,
                        source,
                        container,
                        vis,
                        "java",
                    ));
                }
            }
        }
        "constant_declaration" => {
            if let Some(declarator) = node.child_by_field_name("declarator") {
                if let Some(name_node) = declarator.child_by_field_name("name") {
                    symbols.push(make_symbol(
                        SymbolKind::Constant,
                        text(name_node, source),
                        path,
                        node,
                        source,
                        container,
                        Some("pub"),
                        "java",
                    ));
                }
            }
        }
        "import_declaration" => {
            let import_text = text(node, source);
            let name = import_text
                .trim_start_matches("import ")
                .trim_end_matches(';')
                .trim()
                .rsplit('.')
                .next()
                .unwrap_or("")
                .trim();
            if !name.is_empty() && name != "*" {
                symbols.push(make_symbol(
                    SymbolKind::Import,
                    name,
                    path,
                    node,
                    source,
                    None,
                    None,
                    "java",
                ));
            }
        }
        _ => {}
    }

    for_each_child(node, |child| {
        walk_java(child, source, path, symbols, container)
    });
}

fn java_visibility(node: Node<'_>, source: &[u8]) -> Option<&'static str> {
    let count = node.child_count();
    for i in 0..count {
        if let Some(child) = node.child(i) {
            if child.kind() == "modifiers" {
                let mod_text = text(child, source);
                if mod_text.contains("public") {
                    return Some("pub");
                }
                if mod_text.contains("private") {
                    return Some("private");
                }
                if mod_text.contains("protected") {
                    return Some("protected");
                }
            }
        }
    }
    None
}
