use std::path::{Path, PathBuf};

use tree_sitter::{Node, Parser};

use super::helpers;
use crate::symbols::adapters::LanguageAdapter;
use crate::symbols::{Symbol, SymbolKind};

pub struct PythonAdapter;

impl LanguageAdapter for PythonAdapter {
    fn language_id(&self) -> &str {
        "python"
    }

    fn file_extensions(&self) -> &[&str] {
        &["py"]
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
        walk_python(
            tree.root_node(),
            content.as_bytes(),
            path,
            &mut symbols,
            None,
        );
        for symbol in fallback_assignment_symbols(path, content) {
            let exists = symbols
                .iter()
                .any(|existing| existing.kind == symbol.kind && existing.name == symbol.name);
            if !exists {
                symbols.push(symbol);
            }
        }
        symbols
    }
}

fn walk_python(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    symbols: &mut Vec<Symbol>,
    current_class: Option<&str>,
) {
    match node.kind() {
        "class_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let class_name = helpers::text(name_node, source);
                symbols.push(helpers::make_symbol(
                    SymbolKind::Class,
                    class_name,
                    path,
                    node,
                    source,
                    None,
                    None,
                    "python",
                ));
                helpers::for_each_child(node, |child| {
                    walk_python(child, source, path, symbols, Some(class_name));
                });
                return;
            }
        }
        "function_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let kind = if current_class.is_some() {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                };
                symbols.push(helpers::make_symbol(
                    kind,
                    helpers::text(name_node, source),
                    path,
                    node,
                    source,
                    current_class,
                    None,
                    "python",
                ));
            }
        }
        "assignment" => {
            let left = node
                .child_by_field_name("left")
                .or_else(|| node.child(0))
                .unwrap_or(node);
            helpers::for_each_descendant_of_kind(left, "identifier", &mut |ident| {
                symbols.push(helpers::make_symbol(
                    SymbolKind::Variable,
                    helpers::text(ident, source),
                    path,
                    ident,
                    source,
                    current_class,
                    None,
                    "python",
                ));
            });
        }
        "import_from_statement" => {
            for name in import_names(node, source) {
                symbols.push(helpers::make_symbol(
                    SymbolKind::Import,
                    &name,
                    path,
                    node,
                    source,
                    None,
                    None,
                    "python",
                ));
            }
        }
        _ => {}
    }

    helpers::for_each_child(node, |child| {
        walk_python(child, source, path, symbols, current_class);
    });
}

fn import_names(node: Node<'_>, source: &[u8]) -> Vec<String> {
    let snippet = helpers::text(node, source);
    let Some((_, imported)) = snippet.split_once(" import ") else {
        return Vec::new();
    };
    imported
        .split(',')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            let alias_source = part.split_whitespace().next_back().unwrap_or(part);
            let alias = alias_source.trim_matches(|ch: char| ch == '(' || ch == ')');
            Some(alias.to_string())
        })
        .collect()
}

fn fallback_assignment_symbols(path: &Path, content: &str) -> Vec<Symbol> {
    let mut byte_offset = 0usize;
    let mut out = Vec::new();

    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.starts_with("def ")
            || trimmed.starts_with("class ")
            || trimmed.starts_with("from ")
            || trimmed.starts_with("import ")
            || !trimmed.contains('=')
            || trimmed.contains("==")
        {
            byte_offset += line.len() + 1;
            continue;
        }

        let Some((lhs, _)) = trimmed.split_once('=') else {
            byte_offset += line.len() + 1;
            continue;
        };
        for raw_name in lhs.split(',') {
            let name = raw_name.trim();
            if name.is_empty()
                || !name
                    .chars()
                    .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
            {
                continue;
            }
            let start_in_line = line.find(name).unwrap_or(0);
            out.push(Symbol {
                kind: SymbolKind::Variable,
                name: name.to_string(),
                qualified_name: None,
                language: "python".to_string(),
                repo_rel_path: PathBuf::from(path),
                container_name: None,
                visibility: None,
                signature: Some(trimmed.to_string()),
                line_start: (line_idx + 1) as u32,
                line_end: (line_idx + 1) as u32,
                byte_start: (byte_offset + start_in_line) as u32,
                byte_end: (byte_offset + start_in_line + name.len()) as u32,
                doc: None,
            });
        }

        byte_offset += line.len() + 1;
    }

    out
}
