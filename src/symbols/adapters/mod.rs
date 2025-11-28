mod go;
mod helpers;
mod python;
mod rust_lang;
mod typescript;

use std::path::Path;
use std::sync::Arc;

use tree_sitter::Parser;

use crate::symbols::Symbol;

pub use go::GoAdapter;
pub use python::PythonAdapter;
pub use rust_lang::RustAdapter;
pub use typescript::TypeScriptAdapter;

pub trait LanguageAdapter: Send + Sync {
    fn language_id(&self) -> &str;
    fn file_extensions(&self) -> &[&str];

    /// Extract symbols using a pre-configured parser (avoids re-creating the parser).
    fn extract_symbols_with_parser(
        &self,
        path: &Path,
        content: &str,
        parser: &mut Parser,
    ) -> Vec<Symbol>;

    /// Extract symbols, creating a parser internally. Convenience method
    /// that delegates to `extract_symbols_with_parser`.
    fn extract_symbols(&self, path: &Path, content: &str) -> Vec<Symbol> {
        let ts_lang = match self.language_id() {
            "go" => tree_sitter_go::LANGUAGE.into(),
            "rust" => tree_sitter_rust::LANGUAGE.into(),
            "python" => tree_sitter_python::LANGUAGE.into(),
            "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            _ => return Vec::new(),
        };
        let mut parser = Parser::new();
        if parser.set_language(&ts_lang).is_err() {
            return Vec::new();
        }
        self.extract_symbols_with_parser(path, content, &mut parser)
    }
}

pub struct AdapterRegistry {
    adapters: Vec<Arc<dyn LanguageAdapter>>,
}

impl Default for AdapterRegistry {
    fn default() -> Self {
        Self {
            adapters: vec![
                Arc::new(GoAdapter),
                Arc::new(RustAdapter),
                Arc::new(PythonAdapter),
                Arc::new(TypeScriptAdapter),
            ],
        }
    }
}

impl AdapterRegistry {
    pub fn adapter_for_path(&self, path: &Path) -> Option<&dyn LanguageAdapter> {
        let ext = path.extension()?.to_string_lossy().to_lowercase();
        self.adapters
            .iter()
            .find(|adapter| adapter.file_extensions().iter().any(|item| *item == ext))
            .map(|adapter| adapter.as_ref())
    }

    /// Create a pre-configured parser for the given adapter's language.
    pub fn create_parser_for(&self, adapter: &dyn LanguageAdapter) -> Option<Parser> {
        let ts_lang = match adapter.language_id() {
            "go" => tree_sitter_go::LANGUAGE.into(),
            "rust" => tree_sitter_rust::LANGUAGE.into(),
            "python" => tree_sitter_python::LANGUAGE.into(),
            "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            _ => return None,
        };
        let mut parser = Parser::new();
        parser.set_language(&ts_lang).ok()?;
        Some(parser)
    }
}
