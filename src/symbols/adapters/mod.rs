mod c_lang;
mod cpp;
mod csharp;
mod go;
mod helpers;
mod java;
// mod kotlin;  // tree-sitter-kotlin uses incompatible tree-sitter version
mod python;
mod ruby;
mod rust_lang;
mod typescript;
mod zig;

use std::path::Path;
use std::sync::Arc;

use tree_sitter::Parser;

use crate::symbols::Symbol;

pub use c_lang::CAdapter;
pub use cpp::CppAdapter;
pub use csharp::CSharpAdapter;
pub use go::GoAdapter;
pub use java::JavaAdapter;
// pub use kotlin::KotlinAdapter;
pub use python::PythonAdapter;
pub use ruby::RubyAdapter;
pub use rust_lang::RustAdapter;
pub use typescript::TypeScriptAdapter;
pub use zig::ZigAdapter;

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
    /// Populates signature fields from content for callers that won't
    /// round-trip through Tantivy (tests, one-shot use).
    fn extract_symbols(&self, path: &Path, content: &str) -> Vec<Symbol> {
        let ts_lang = lang_for_id(self.language_id());
        let Some(ts_lang) = ts_lang else {
            return Vec::new();
        };
        let mut parser = Parser::new();
        if parser.set_language(&ts_lang).is_err() {
            return Vec::new();
        }
        let mut symbols = self.extract_symbols_with_parser(path, content, &mut parser);
        // Fill in signatures from byte offsets for non-Tantivy callers
        for sym in &mut symbols {
            if sym.signature.is_none() {
                sym.signature = content
                    .get(sym.byte_start as usize..sym.byte_end as usize)
                    .map(|s| s.to_string());
            }
        }
        symbols
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
                Arc::new(CAdapter),
                Arc::new(CppAdapter),
                Arc::new(JavaAdapter),
                // Arc::new(KotlinAdapter),  // awaiting tree-sitter-kotlin update
                Arc::new(CSharpAdapter),
                Arc::new(RubyAdapter),
                Arc::new(ZigAdapter),
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
        let ts_lang = lang_for_id(adapter.language_id())?;
        let mut parser = Parser::new();
        parser.set_language(&ts_lang).ok()?;
        Some(parser)
    }
}

fn lang_for_id(id: &str) -> Option<tree_sitter::Language> {
    Some(match id {
        "go" => tree_sitter_go::LANGUAGE.into(),
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "python" => tree_sitter_python::LANGUAGE.into(),
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "c" => tree_sitter_c::LANGUAGE.into(),
        "cpp" => tree_sitter_cpp::LANGUAGE.into(),
        "java" => tree_sitter_java::LANGUAGE.into(),
        // "kotlin" => tree_sitter_kotlin::language(),
        "csharp" => tree_sitter_c_sharp::LANGUAGE.into(),
        "ruby" => tree_sitter_ruby::LANGUAGE.into(),
        "zig" => tree_sitter_zig::LANGUAGE.into(),
        _ => return None,
    })
}
