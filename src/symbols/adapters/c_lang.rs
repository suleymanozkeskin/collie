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
        "function_definition" => {
            if let Some(name) = extract_declarator_name(node, source) {
                symbols.push(make_symbol(
                    SymbolKind::Function, &name,
                    path, node, source, None, None, "c",
                ));
            }
        }
        "struct_specifier" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(make_symbol(
                    SymbolKind::Struct, text(name_node, source),
                    path, node, source, None, None, "c",
                ));
            }
        }
        "enum_specifier" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(make_symbol(
                    SymbolKind::Enum, text(name_node, source),
                    path, node, source, None, None, "c",
                ));
            }
        }
        "type_definition" => {
            if let Some(name) = extract_declarator_name(node, source) {
                symbols.push(make_symbol(
                    SymbolKind::TypeAlias, &name,
                    path, node, source, None, None, "c",
                ));
            }
        }
        "declaration" => {
            if node.parent().map_or(false, |p| p.kind() == "translation_unit") {
                if let Some(name) = extract_declarator_name(node, source) {
                    symbols.push(make_symbol(
                        SymbolKind::Variable, &name,
                        path, node, source, None, None, "c",
                    ));
                }
            }
        }
        "preproc_def" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                symbols.push(make_symbol(
                    SymbolKind::Constant, text(name_node, source),
                    path, node, source, None, None, "c",
                ));
            }
        }
        _ => {}
    }

    for_each_child(node, |child| walk_c(child, source, path, symbols));
}

/// Walk down through declarator/function_declarator/pointer_declarator chains
/// to find the actual identifier name.
fn extract_declarator_name<'a>(node: Node<'a>, source: &'a [u8]) -> Option<&'a str> {
    let mut current = node.child_by_field_name("declarator")?;
    // Walk through nested declarators: pointer_declarator → function_declarator → identifier
    loop {
        match current.kind() {
            "identifier" | "type_identifier" | "field_identifier" => {
                return Some(text(current, source));
            }
            _ => {
                // Try the "declarator" field, or "name" field
                if let Some(inner) = current.child_by_field_name("declarator") {
                    current = inner;
                } else if let Some(inner) = current.child_by_field_name("name") {
                    current = inner;
                } else {
                    // Last resort: find first identifier child
                    let count = current.child_count();
                    for i in 0..count {
                        if let Some(child) = current.child(i) {
                            if child.kind() == "identifier" {
                                return Some(text(child, source));
                            }
                        }
                    }
                    return None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbols::adapters::LanguageAdapter;

    #[test]
    fn c_extracts_functions() {
        let adapter = CAdapter;
        let content = "void handle_request(int fd) { }\nint main() { return 0; }";
        let symbols = adapter.extract_symbols(Path::new("main.c"), content);
        let fns: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Function).collect();
        assert!(!fns.is_empty(), "should find at least one function, got: {:?}",
            symbols.iter().map(|s| format!("{:?} {}", s.kind, s.name)).collect::<Vec<_>>());
    }

    #[test]
    fn c_extracts_structs() {
        let adapter = CAdapter;
        let symbols = adapter.extract_symbols(
            Path::new("main.c"),
            "struct Config { int port; };",
        );
        let structs: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Struct).collect();
        assert_eq!(structs.len(), 1);
        assert_eq!(structs[0].name, "Config");
    }
}
