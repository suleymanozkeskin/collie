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
) {
    if node.kind() == "export_statement" {
        helpers::for_each_child(node, |child| {
            walk_typescript(child, source, path, symbols, true);
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
                    exported,
                ));
            }
        }
        "class_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(make_ts_symbol(
                    SymbolKind::Class,
                    helpers::text(name_node, source),
                    path,
                    node,
                    source,
                    exported,
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
                    exported,
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
                    exported,
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
                    exported,
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
                    exported,
                ));
            }
        }
        _ => {}
    }

    helpers::for_each_child(node, |child| {
        walk_typescript(child, source, path, symbols, exported);
    });
}

fn make_ts_symbol(
    kind: SymbolKind,
    name: &str,
    path: &Path,
    node: Node<'_>,
    source: &[u8],
    exported: bool,
) -> Symbol {
    helpers::make_symbol(
        kind,
        name,
        path,
        node,
        source,
        None,
        Some(if exported { "pub" } else { "private" }),
        "typescript",
    )
}
