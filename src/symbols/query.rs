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
                if let Some(kind) = normalize_kind(value) {
                    query.kind = Some(kind);
                } else {
                    name_start = idx;
                    break;
                }
            }
            "lang" => {
                if let Some(language) = normalize_language(value) {
                    query.language = Some(language);
                } else {
                    name_start = idx;
                    break;
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

pub fn normalize_kind(value: &str) -> Option<SymbolKind> {
    let value = value.trim().to_lowercase();
    match value.as_str() {
        "function" | "fn" => Some(SymbolKind::Function),
        "method" => Some(SymbolKind::Method),
        "class" => Some(SymbolKind::Class),
        "struct" => Some(SymbolKind::Struct),
        "enum" => Some(SymbolKind::Enum),
        "interface" => Some(SymbolKind::Interface),
        "trait" => Some(SymbolKind::Trait),
        "variable" | "var" => Some(SymbolKind::Variable),
        "field" => Some(SymbolKind::Field),
        "property" | "prop" => Some(SymbolKind::Property),
        "constant" | "const" => Some(SymbolKind::Constant),
        "module" | "mod" => Some(SymbolKind::Module),
        "type" => Some(SymbolKind::TypeAlias),
        "import" => Some(SymbolKind::Import),
        _ => None,
    }
}

pub fn normalize_language(value: &str) -> Option<String> {
    let value = value.trim().to_lowercase();
    match value.as_str() {
        "go" => Some("go".to_string()),
        "rust" | "rs" => Some("rust".to_string()),
        "python" | "py" => Some("python".to_string()),
        "typescript" | "ts" => Some("typescript".to_string()),
        "javascript" | "js" => Some("javascript".to_string()),
        "java" => Some("java".to_string()),
        "c" => Some("c".to_string()),
        "cpp" => Some("cpp".to_string()),
        "ruby" | "rb" => Some("ruby".to_string()),
        "php" => Some("php".to_string()),
        "swift" => Some("swift".to_string()),
        "kotlin" | "kt" => Some("kotlin".to_string()),
        _ => None,
    }
}
