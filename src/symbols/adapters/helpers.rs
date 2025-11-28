use std::path::{Path, PathBuf};

use tree_sitter::Node;

use crate::symbols::{Symbol, SymbolKind};

/// Extract text from a node without allocating when possible.
/// Returns a &str borrowing from source.
pub fn text<'a>(node: Node<'_>, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

/// Iterate direct children of a node without allocating a Vec.
pub fn for_each_child(node: Node<'_>, mut f: impl FnMut(Node<'_>)) {
    let count = node.child_count();
    for i in 0..count {
        if let Some(child) = node.child(i) {
            f(child);
        }
    }
}

/// Find descendants of a specific kind using a cursor, calling `f` for each.
/// Avoids the recursive Vec<Node> allocation of the old `descendants_of_kind`.
pub fn for_each_descendant_of_kind<'a>(
    node: Node<'a>,
    kind: &str,
    f: &mut impl FnMut(Node<'a>),
) {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return;
    }
    loop {
        let child = cursor.node();
        if child.kind() == kind {
            f(child);
        }
        for_each_descendant_of_kind(child, kind, f);
        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// Check if a Rust node starts with `pub` by inspecting the first child,
/// avoiding materializing the entire node text.
pub fn rust_visibility_from_node(node: Node<'_>) -> Option<&'static str> {
    let first_child = node.child(0)?;
    Some(if first_child.kind() == "visibility_modifier" {
        "pub"
    } else {
        "private"
    })
}

/// Go visibility: uppercase first letter = pub.
pub fn go_visibility(name: &str) -> &'static str {
    name.chars()
        .next()
        .map(|ch| if ch.is_uppercase() { "pub" } else { "private" })
        .unwrap_or("private")
}

pub fn make_symbol(
    kind: SymbolKind,
    name: &str,
    path: &Path,
    node: Node<'_>,
    source: &[u8],
    container_name: Option<&str>,
    visibility: Option<&'static str>,
    language: &'static str,
) -> Symbol {
    let start = node.start_position();
    let end = node.end_position();
    Symbol {
        qualified_name: container_name.map(|c| format!("{c}::{name}")),
        name: name.to_string(),
        kind,
        language: language.to_string(),
        repo_rel_path: PathBuf::from(path),
        container_name: container_name.map(|s| s.to_string()),
        visibility: visibility.map(|s| s.to_string()),
        signature: Some(text(node, source).to_string()),
        line_start: (start.row + 1) as u32,
        line_end: (end.row + 1) as u32,
        byte_start: node.start_byte() as u32,
        byte_end: node.end_byte() as u32,
        doc: None,
    }
}
