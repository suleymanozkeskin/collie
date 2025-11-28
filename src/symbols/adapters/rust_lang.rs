use std::path::Path;

use tree_sitter::{Node, Parser};

use super::helpers;
use crate::symbols::adapters::LanguageAdapter;
use crate::symbols::{Symbol, SymbolKind};

pub struct RustAdapter;

impl LanguageAdapter for RustAdapter {
    fn language_id(&self) -> &str {
        "rust"
    }

    fn file_extensions(&self) -> &[&str] {
        &["rs"]
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
        walk_rust(
            tree.root_node(),
            content.as_bytes(),
            path,
            &mut symbols,
            None,
        );
        symbols
    }
}

fn walk_rust(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    symbols: &mut Vec<Symbol>,
    current_impl: Option<&str>,
) {
    match node.kind() {
        "function_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = helpers::text(name_node, source);
                let kind = if current_impl.is_some() {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                };
                symbols.push(helpers::make_symbol(
                    kind,
                    name,
                    path,
                    node,
                    source,
                    current_impl,
                    helpers::rust_visibility_from_node(node),
                    "rust",
                ));
            }
        }
        "struct_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = helpers::text(name_node, source);
                symbols.push(helpers::make_symbol(
                    SymbolKind::Struct,
                    name,
                    path,
                    node,
                    source,
                    None,
                    helpers::rust_visibility_from_node(node),
                    "rust",
                ));
                helpers::for_each_descendant_of_kind(
                    node,
                    "field_declaration",
                    &mut |field| {
                        if let Some(field_name) = field.child_by_field_name("name") {
                            symbols.push(helpers::make_symbol(
                                SymbolKind::Field,
                                helpers::text(field_name, source),
                                path,
                                field,
                                source,
                                Some(name),
                                helpers::rust_visibility_from_node(field),
                                "rust",
                            ));
                        }
                    },
                );
                return;
            }
        }
        "enum_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(helpers::make_symbol(
                    SymbolKind::Enum,
                    helpers::text(name_node, source),
                    path,
                    node,
                    source,
                    None,
                    helpers::rust_visibility_from_node(node),
                    "rust",
                ));
            }
        }
        "trait_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(helpers::make_symbol(
                    SymbolKind::Trait,
                    helpers::text(name_node, source),
                    path,
                    node,
                    source,
                    None,
                    helpers::rust_visibility_from_node(node),
                    "rust",
                ));
            }
        }
        "const_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(helpers::make_symbol(
                    SymbolKind::Constant,
                    helpers::text(name_node, source),
                    path,
                    node,
                    source,
                    None,
                    helpers::rust_visibility_from_node(node),
                    "rust",
                ));
            }
        }
        "mod_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(helpers::make_symbol(
                    SymbolKind::Module,
                    helpers::text(name_node, source),
                    path,
                    node,
                    source,
                    None,
                    helpers::rust_visibility_from_node(node),
                    "rust",
                ));
            }
        }
        "type_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(helpers::make_symbol(
                    SymbolKind::TypeAlias,
                    helpers::text(name_node, source),
                    path,
                    node,
                    source,
                    None,
                    helpers::rust_visibility_from_node(node),
                    "rust",
                ));
            }
        }
        "impl_item" => {
            let impl_target = node
                .child_by_field_name("type")
                .map(|n| clean_type_name(helpers::text(n, source)));
            helpers::for_each_child(node, |child| {
                walk_rust(child, source, path, symbols, impl_target.as_deref());
            });
            return;
        }
        _ => {}
    }

    helpers::for_each_child(node, |child| {
        walk_rust(child, source, path, symbols, current_impl);
    });
}

fn clean_type_name(raw: &str) -> String {
    raw.split_whitespace()
        .last()
        .unwrap_or("")
        .trim()
        .to_string()
}
