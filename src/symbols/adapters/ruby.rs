use std::path::Path;

use tree_sitter::{Node, Parser};

use super::helpers::{for_each_child, make_symbol, text};
use crate::symbols::adapters::LanguageAdapter;
use crate::symbols::{Symbol, SymbolKind};

pub struct RubyAdapter;

impl LanguageAdapter for RubyAdapter {
    fn language_id(&self) -> &str {
        "ruby"
    }

    fn file_extensions(&self) -> &[&str] {
        &["rb"]
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
        walk_ruby(
            tree.root_node(),
            content.as_bytes(),
            path,
            &mut symbols,
            None,
        );
        symbols
    }
}

fn walk_ruby(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    symbols: &mut Vec<Symbol>,
    container: Option<&str>,
) {
    match node.kind() {
        "class" => {
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
                    "ruby",
                ));
                for_each_child(node, |child| {
                    walk_ruby(child, source, path, symbols, Some(name));
                });
                return;
            }
        }
        "module" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                symbols.push(make_symbol(
                    SymbolKind::Module,
                    name,
                    path,
                    node,
                    source,
                    container,
                    None,
                    "ruby",
                ));
                for_each_child(node, |child| {
                    walk_ruby(child, source, path, symbols, Some(name));
                });
                return;
            }
        }
        "method" => {
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
                    "ruby",
                ));
            }
        }
        "singleton_method" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(make_symbol(
                    SymbolKind::Method,
                    text(name_node, source),
                    path,
                    node,
                    source,
                    container,
                    Some("pub"),
                    "ruby",
                ));
            }
        }
        "assignment" => {
            if let Some(left) = node.child_by_field_name("left") {
                if left.kind() == "constant" {
                    symbols.push(make_symbol(
                        SymbolKind::Constant,
                        text(left, source),
                        path,
                        node,
                        source,
                        container,
                        None,
                        "ruby",
                    ));
                }
            }
        }
        _ => {}
    }

    for_each_child(node, |child| {
        walk_ruby(child, source, path, symbols, container)
    });
}
