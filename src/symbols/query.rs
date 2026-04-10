use crate::symbols::{SymbolKind, SymbolQuery};

pub fn parse_query(input: &str) -> SymbolQuery {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    let mut query = SymbolQuery::default();
    let mut name_start = tokens.len();

    for (idx, token) in tokens.iter().enumerate() {
        let Some((key, value)) = token.split_once(':') else {
            name_start = idx;
            break;
        };

        match key {
            "kind" => {
                let kinds = normalize_kinds(value);
                if kinds.is_empty() {
                    query.invalid_filter = Some(format!("unsupported kind filter: {value}"));
                    return query;
                }
                query.kinds = kinds;
            }
            "lang" => {
                if let Some(language) = normalize_language(value) {
                    query.language = Some(language);
                } else {
                    query.invalid_filter = Some(format!("unsupported language filter: {value}"));
                    return query;
                }
            }
            "path" => {
                query.path_prefix = Some(value.to_string());
            }
            "qname" => {
                query.qualified_name_pattern = Some(value.to_string());
            }
            _ => {
                name_start = idx;
                break;
            }
        }
    }

    if name_start < tokens.len() {
        query.name_pattern = tokens[name_start..].join(" ");
    }

    query
}

/// Normalize a kind filter value into one or more SymbolKinds.
///
/// `kind:fn` expands to both Function and Method because in most languages
/// (Rust, Go, Python, etc.) the `fn`/`func`/`def` keyword is used for both
/// freestanding functions and methods on types.
pub fn normalize_kinds(value: &str) -> Vec<SymbolKind> {
    let value = value.trim().to_lowercase();
    match value.as_str() {
        "function" | "fn" => vec![SymbolKind::Function, SymbolKind::Method],
        "method" => vec![SymbolKind::Method],
        "class" => vec![SymbolKind::Class],
        "struct" => vec![SymbolKind::Struct],
        "enum" => vec![SymbolKind::Enum],
        "interface" => vec![SymbolKind::Interface],
        "trait" => vec![SymbolKind::Trait],
        "variable" | "var" => vec![SymbolKind::Variable],
        "field" => vec![SymbolKind::Field],
        "property" | "prop" => vec![SymbolKind::Property],
        "constant" | "const" => vec![SymbolKind::Constant],
        "module" | "mod" => vec![SymbolKind::Module],
        "type" => vec![SymbolKind::TypeAlias],
        "import" => vec![SymbolKind::Import],
        _ => Vec::new(),
    }
}

pub fn normalize_language(value: &str) -> Option<String> {
    let value = value.trim().to_lowercase();
    match value.as_str() {
        "go" => Some("go".to_string()),
        "rust" | "rs" => Some("rust".to_string()),
        "python" | "py" => Some("python".to_string()),
        "typescript" | "ts" => Some("typescript".to_string()),
        "java" => Some("java".to_string()),
        "c" => Some("c".to_string()),
        "cpp" => Some("cpp".to_string()),
        "ruby" | "rb" => Some("ruby".to_string()),
        "csharp" | "cs" => Some("csharp".to_string()),
        "zig" => Some("zig".to_string()),
        _ => None,
    }
}
