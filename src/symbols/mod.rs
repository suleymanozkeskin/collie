pub mod adapters;
pub mod query;

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Interface,
    Trait,
    Variable,
    Field,
    Property,
    Constant,
    Module,
    TypeAlias,
    Import,
}

impl SymbolKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Method => "method",
            Self::Class => "class",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Interface => "interface",
            Self::Trait => "trait",
            Self::Variable => "variable",
            Self::Field => "field",
            Self::Property => "property",
            Self::Constant => "constant",
            Self::Module => "module",
            Self::TypeAlias => "type_alias",
            Self::Import => "import",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub kind: SymbolKind,
    pub name: String,
    pub qualified_name: Option<String>,
    pub language: String,
    pub repo_rel_path: PathBuf,
    pub container_name: Option<String>,
    pub visibility: Option<String>,
    pub signature: Option<String>,
    pub line_start: u32,
    pub line_end: u32,
    pub byte_start: u32,
    pub byte_end: u32,
    pub doc: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolResult {
    pub kind: SymbolKind,
    pub name: String,
    pub qualified_name: Option<String>,
    pub language: String,
    pub repo_rel_path: PathBuf,
    pub container_name: Option<String>,
    pub visibility: Option<String>,
    pub signature: Option<String>,
    pub line_start: u32,
    pub line_end: u32,
    pub byte_start: u32,
    pub byte_end: u32,
    pub doc: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SymbolQuery {
    /// One or more kinds to match (OR). Empty means "any kind".
    pub kinds: Vec<SymbolKind>,
    pub language: Option<String>,
    pub path_prefix: Option<String>,
    pub name_pattern: String,
    pub qualified_name_pattern: Option<String>,
    pub invalid_filter: Option<String>,
}

impl SymbolQuery {
    pub fn has_filters(&self) -> bool {
        !self.kinds.is_empty()
            || self.language.is_some()
            || self.path_prefix.is_some()
            || self.qualified_name_pattern.is_some()
    }

    pub fn invalid_filter(&self) -> Option<&str> {
        self.invalid_filter.as_deref()
    }
}
