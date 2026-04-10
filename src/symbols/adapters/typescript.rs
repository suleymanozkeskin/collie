use std::path::Path;

use tree_sitter::{Node, Parser};

use super::helpers;
use crate::symbols::adapters::LanguageAdapter;
use crate::symbols::{Symbol, SymbolKind};

pub struct TypeScriptAdapter;

impl LanguageAdapter for TypeScriptAdapter {
    fn language_id(&self) -> &str {
        "typescript"
    }

    fn file_extensions(&self) -> &[&str] {
        &["ts", "tsx"]
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
        walk_typescript(
            tree.root_node(),
            content.as_bytes(),
            path,
            &mut symbols,
            false,
            None,
        );
        symbols
    }
}

fn walk_typescript(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    symbols: &mut Vec<Symbol>,
    exported: bool,
    container: Option<&str>,
) {
    if node.kind() == "export_statement" {
        helpers::for_each_child(node, |child| {
            walk_typescript(child, source, path, symbols, true, container);
        });
        return;
    }

    match node.kind() {
        "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(make_ts_symbol(
                    SymbolKind::Function,
                    helpers::text(name_node, source),
                    path,
                    node,
                    source,
                    container,
                ));
            }
        }
        "class_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = helpers::text(name_node, source);
                symbols.push(make_ts_symbol(
                    SymbolKind::Class,
                    name,
                    path,
                    node,
                    source,
                    None,
                ));
                helpers::for_each_child(node, |child| {
                    walk_typescript(child, source, path, symbols, exported, Some(name));
                });
                return;
            }
        }
        "method_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let kind = if container.is_some() {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                };
                symbols.push(make_ts_symbol(
                    kind,
                    helpers::text(name_node, source),
                    path,
                    node,
                    source,
                    container,
                ));
            }
        }
        "interface_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(make_ts_symbol(
                    SymbolKind::Interface,
                    helpers::text(name_node, source),
                    path,
                    node,
                    source,
                    container,
                ));
            }
        }
        "type_alias_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(make_ts_symbol(
                    SymbolKind::TypeAlias,
                    helpers::text(name_node, source),
                    path,
                    node,
                    source,
                    container,
                ));
            }
        }
        "enum_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(make_ts_symbol(
                    SymbolKind::Enum,
                    helpers::text(name_node, source),
                    path,
                    node,
                    source,
                    container,
                ));
            }
        }
        "variable_declarator" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(make_ts_symbol(
                    SymbolKind::Variable,
                    helpers::text(name_node, source),
                    path,
                    node,
                    source,
                    container,
                ));
            }
        }
        _ => {}
    }

    helpers::for_each_child(node, |child| {
        walk_typescript(child, source, path, symbols, exported, container);
    });
}

fn make_ts_symbol(
    kind: SymbolKind,
    name: &str,
    path: &Path,
    node: Node<'_>,
    source: &[u8],
    container: Option<&str>,
) -> Symbol {
    helpers::make_symbol(kind, name, path, node, source, container, None, "typescript")
}
