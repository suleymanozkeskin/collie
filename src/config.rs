use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct CollieConfig {
    pub index: IndexConfig,
    pub watcher: WatcherConfig,
    pub search: SearchConfig,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct IndexConfig {
    /// Deprecated — min_token_length is now hardcoded to 2 in the Tantivy analyzer.
    /// Kept for config file compatibility.
    #[serde(default)]
    pub min_token_length: usize,

    /// Additional file extensions to index beyond the built-in set.
    pub extra_extensions: Vec<String>,

    /// File extensions to exclude from the built-in set.
    pub exclude_extensions: Vec<String>,

    /// Maximum file size in bytes. Files larger than this are skipped.
    pub max_file_size: u64,

    /// Index PDF files by extracting their text layer. Default: false.
    pub include_pdfs: bool,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct WatcherConfig {
    /// Deprecated — debounce is no longer used. Kept for config file compatibility.
    #[serde(default)]
    pub debounce_ms: u64,

    /// Seconds of inactivity (no searches, no file changes) before the daemon
    /// auto-stops. 0 disables idle auto-stop. Default: 1800 (30 minutes).
    pub idle_timeout_secs: u64,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct SearchConfig {
    /// Default maximum number of results to return.
    pub default_limit: usize,

    /// Number of context lines to show around each match in snippet mode.
    pub context_lines: usize,
}

impl Default for CollieConfig {
    fn default() -> Self {
        Self {
            index: IndexConfig::default(),
            watcher: WatcherConfig::default(),
            search: SearchConfig::default(),
        }
    }
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            min_token_length: 2,
            extra_extensions: Vec::new(),
            exclude_extensions: Vec::new(),
            max_file_size: 1_048_576,
            include_pdfs: false,
        }
    }
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            debounce_ms: 300,
            idle_timeout_secs: 1800,
        }
    }
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            default_limit: 20,
            context_lines: 2,
        }
    }
}

impl CollieConfig {
    pub fn load(worktree_root: &Path) -> Self {
        for config_path in crate::paths::config_path_candidates(worktree_root) {
            if !config_path.exists() {
                continue;
            }
            match std::fs::read_to_string(&config_path) {
                Ok(content) => match toml::from_str(&content) {
                    Ok(config) => return config,
                    Err(err) => {
                        eprintln!(
                            "warning: failed to parse {}: {}",
                            config_path.display(),
                            err
                        );
                    }
                },
                Err(err) => {
                    eprintln!("warning: failed to read {}: {}", config_path.display(), err);
                }
            }
        }
        Self::default()
    }
}

/// The example config template written by `collie config --init`.
pub const CONFIG_TEMPLATE: &str = r#"# Collie configuration
# Place this file at <worktree>/.collie.toml
# Legacy <worktree>/.collie/config.toml is still supported for reads.

[index]
# max_file_size = 1048576
# extra_extensions = ["proto", "graphql"]
# exclude_extensions = []
# include_pdfs = false

[watcher]
# idle_timeout_secs = 1800

[search]
# default_limit = 20
# context_lines = 2
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn default_config_has_expected_values() {
        let config = CollieConfig::default();
        assert_eq!(config.index.min_token_length, 2);
        assert_eq!(config.index.max_file_size, 1_048_576);
        assert!(config.index.extra_extensions.is_empty());
        assert!(config.index.exclude_extensions.is_empty());
        assert_eq!(config.watcher.debounce_ms, 300);
        assert_eq!(config.search.default_limit, 20);
        assert_eq!(config.search.context_lines, 2);
    }

    #[test]
    fn load_returns_default_for_nonexistent_dir() {
        let config = CollieConfig::load(Path::new("/nonexistent/path"));
        assert_eq!(config.index.min_token_length, 2);
    }

    #[test]
    fn load_partial_toml_merges_with_defaults() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join(".collie.toml"),
            "[search]\ndefault_limit = 50\n",
        )
        .unwrap();

        let config = CollieConfig::load(tmp.path());
        assert_eq!(config.search.default_limit, 50);
        assert_eq!(config.index.min_token_length, 2);
    }
}
