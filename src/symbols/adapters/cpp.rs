use std::path::Path;

use tree_sitter::{Node, Parser};

use super::helpers::{for_each_child, make_symbol, text};
use crate::symbols::adapters::LanguageAdapter;
use crate::symbols::{Symbol, SymbolKind};

pub struct CppAdapter;

impl LanguageAdapter for CppAdapter {
    fn language_id(&self) -> &str {
        "cpp"
    }

    fn file_extensions(&self) -> &[&str] {
        &["cpp", "cc", "cxx", "hpp", "hxx", "hh"]
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
        walk_cpp(
            tree.root_node(),
            content.as_bytes(),
            path,
            &mut symbols,
            None,
            false,
        );
        symbols
    }
}

fn walk_cpp(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    symbols: &mut Vec<Symbol>,
    container: Option<&str>,
    in_type_scope: bool,
) {
    match node.kind() {
        "function_definition" => {
            if let Some(declarator) = node.child_by_field_name("declarator") {
                if let Some(name_node) = find_identifier(&declarator) {
                    let kind = if in_type_scope {
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
                        "cpp",
                    ));
                }
            }
        }
        "class_specifier" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                symbols.push(make_symbol(
                    SymbolKind::Class,
                    name,
                    path,
                    node,
                    source,
                    None,
                    None,
                    "cpp",
                ));
                for_each_child(node, |child| {
                    walk_cpp(child, source, path, symbols, Some(name), true);
                });
                return;
            }
        }
        "struct_specifier" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                symbols.push(make_symbol(
                    SymbolKind::Struct,
                    name,
                    path,
                    node,
                    source,
                    None,
                    None,
                    "cpp",
                ));
                for_each_child(node, |child| {
                    walk_cpp(child, source, path, symbols, Some(name), true);
                });
                return;
            }
        }
        "enum_specifier" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(make_symbol(
                    SymbolKind::Enum,
                    text(name_node, source),
                    path,
                    node,
                    source,
                    None,
                    None,
                    "cpp",
                ));
            }
        }
        "namespace_definition" => {
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
                    "cpp",
                ));
                for_each_child(node, |child| {
                    walk_cpp(child, source, path, symbols, Some(name), false);
                });
                return;
            }
        }
        "type_definition" | "alias_declaration" => {
            if let Some(declarator) = node.child_by_field_name("declarator") {
                if let Some(name_node) = find_identifier(&declarator) {
                    symbols.push(make_symbol(
                        SymbolKind::TypeAlias,
                        text(name_node, source),
                        path,
                        node,
                        source,
                        container,
                        None,
                        "cpp",
                    ));
                }
            } else if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(make_symbol(
                    SymbolKind::TypeAlias,
                    text(name_node, source),
                    path,
                    node,
                    source,
                    container,
                    None,
                    "cpp",
                ));
            }
        }
        "field_declaration" => {
            if let Some(declarator) = node.child_by_field_name("declarator") {
                if let Some(name_node) = find_identifier(&declarator) {
                    symbols.push(make_symbol(
                        SymbolKind::Field,
                        text(name_node, source),
                        path,
                        node,
                        source,
                        container,
                        None,
                        "cpp",
                    ));
                }
            }
        }
        "template_declaration" => {
            // Walk into the template body to find the actual declaration
        }
        _ => {}
    }

    for_each_child(node, |child| {
        walk_cpp(child, source, path, symbols, container, in_type_scope)
    });
}

fn find_identifier<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    if node.kind() == "identifier" || node.kind() == "field_identifier" {
        return Some(*node);
    }
    // For qualified names like Foo::bar, take the last identifier
    if node.kind() == "qualified_identifier" || node.kind() == "destructor_name" {
        let count = node.child_count();
        for i in (0..count).rev() {
            if let Some(child) = node.child(i) {
                if child.kind() == "identifier" || child.kind() == "destructor_name" {
                    return Some(child);
                }
            }
        }
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
