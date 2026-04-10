use std::path::Path;

use tree_sitter::{Node, Parser};

use super::helpers;
use crate::symbols::adapters::LanguageAdapter;
use crate::symbols::{Symbol, SymbolKind};

pub struct GoAdapter;

impl LanguageAdapter for GoAdapter {
    fn language_id(&self) -> &str {
        "go"
    }

    fn file_extensions(&self) -> &[&str] {
        &["go"]
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
        walk_go(
            tree.root_node(),
            content.as_bytes(),
            path,
            &mut symbols,
            None,
        );
        symbols
    }
}

fn walk_go(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    symbols: &mut Vec<Symbol>,
    container: Option<&str>,
) {
    match node.kind() {
        "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = helpers::text(name_node, source);
                symbols.push(helpers::make_symbol(
                    SymbolKind::Function,
                    name,
                    path,
                    node,
                    source,
                    container,
                    Some(helpers::go_visibility(name)),
                    "go",
                ));
            }
        }
        "method_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = helpers::text(name_node, source);
                let receiver = node
                    .child_by_field_name("receiver")
                    .map(|n| clean_type_name(helpers::text(n, source)));
                symbols.push(helpers::make_symbol(
                    SymbolKind::Method,
                    name,
                    path,
                    node,
                    source,
                    receiver.as_deref(),
                    Some(helpers::go_visibility(name)),
                    "go",
                ));
            }
        }
        "type_spec" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = helpers::text(name_node, source);
                if let Some(type_node) = node.child_by_field_name("type") {
                    match type_node.kind() {
                        "struct_type" => {
                            let mut sym = helpers::make_symbol(
                                SymbolKind::Struct,
                                name,
                                path,
                                node,
                                source,
                                None,
                                Some(helpers::go_visibility(name)),
                                "go",
                            );
                            sym.signature =
                                Some(helpers::extract_header_before_brace(node, source));
                            symbols.push(sym);
                            extract_go_struct_fields(type_node, source, path, symbols, name);
                            return;
                        }
                        "interface_type" => {
                            let mut sym = helpers::make_symbol(
                                SymbolKind::Interface,
                                name,
                                path,
                                node,
                                source,
                                None,
                                Some(helpers::go_visibility(name)),
                                "go",
                            );
                            sym.signature =
                                Some(helpers::extract_header_before_brace(node, source));
                            symbols.push(sym);
                        }
                        _ => symbols.push(helpers::make_symbol(
                            SymbolKind::TypeAlias,
                            name,
                            path,
                            node,
                            source,
                            None,
                            Some(helpers::go_visibility(name)),
                            "go",
                        )),
                    }
                }
            }
        }
        "const_spec" => {
            extract_go_identifiers(node, source, path, symbols, SymbolKind::Constant);
            return;
        }
        "var_spec" => {
            extract_go_identifiers(node, source, path, symbols, SymbolKind::Variable);
            return;
        }
        _ => {}
    }

    helpers::for_each_child(node, |child| {
        walk_go(child, source, path, symbols, container);
    });
}

fn extract_go_struct_fields(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    symbols: &mut Vec<Symbol>,
    container: &str,
) {
    helpers::for_each_descendant_of_kind(node, "field_identifier", &mut |child| {
        let name = helpers::text(child, source);
        symbols.push(helpers::make_symbol(
            SymbolKind::Field,
            name,
            path,
            child,
            source,
            Some(container),
            None,
            "go",
        ));
    });
}

fn extract_go_identifiers(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    symbols: &mut Vec<Symbol>,
    kind: SymbolKind,
) {
    helpers::for_each_descendant_of_kind(node, "identifier", &mut |ident| {
        let name = helpers::text(ident, source);
        symbols.push(helpers::make_symbol(
            kind, name, path, ident, source, None, None, "go",
        ));
    });
}

fn clean_type_name(raw: &str) -> String {
    raw.trim_matches(|c: char| c == '(' || c == ')' || c == '*' || c == '&' || c.is_whitespace())
        .split_whitespace()
        .last()
        .unwrap_or("")
        .trim_start_matches('*')
        .to_string()
}
