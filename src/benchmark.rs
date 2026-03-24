use anyhow::Result;
use anyhow::{Context, bail};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::CollieConfig;
use crate::indexer::IndexBuilder;

pub struct BenchmarkSetup {
    pub corpus_path: PathBuf,
    pub index_path: PathBuf,
    pub builder: IndexBuilder,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct AgenticBenchmarkSuite {
    pub version: u32,
    pub tasks: Vec<AgenticBenchmarkTask>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct AgenticBenchmarkTask {
    pub id: String,
    pub repo: String,
    pub prompt: String,
    pub expected_paths: Vec<PathBuf>,
    pub collie_symbol_queries: Vec<String>,
    pub collie_lexical_queries: Vec<String>,
    pub rg_regex_queries: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ProductionBenchmarkProfiles {
    pub version: u32,
    pub profiles: Vec<ProductionBenchmarkProfile>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ProductionBenchmarkProfile {
    pub key: String,
    pub description: String,
    #[serde(default)]
    pub default_repo_relpath: Option<PathBuf>,
    #[serde(default)]
    pub repo_names: Vec<String>,
    #[serde(default)]
    pub repo_origin_substrings: Vec<String>,
    #[serde(default)]
    pub min_tracked_files: Option<usize>,
    #[serde(default)]
    pub max_tracked_files: Option<usize>,
    pub lexical_queries: Vec<String>,
    pub symbol_queries: Vec<String>,
    pub incremental_candidates: Vec<PathBuf>,
}

pub fn default_agentic_tasks_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("benchmark-data")
        .join("agentic_tasks.json")
}

pub fn default_production_profiles_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("benchmark-data")
        .join("production_profiles.json")
}

pub fn load_agentic_benchmark_suite(path: &Path) -> Result<AgenticBenchmarkSuite> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read benchmark task file {:?}", path))?;
    let suite: AgenticBenchmarkSuite = serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse benchmark task file {:?}", path))?;
    validate_agentic_benchmark_suite(&suite)?;
    Ok(suite)
}

pub fn load_production_benchmark_profiles(path: &Path) -> Result<ProductionBenchmarkProfiles> {
    let contents = fs::read_to_string(path).with_context(|| {
        format!(
            "failed to read production benchmark profile file {:?}",
            path
        )
    })?;
    let profiles: ProductionBenchmarkProfiles =
        serde_json::from_str(&contents).with_context(|| {
            format!(
                "failed to parse production benchmark profile file {:?}",
                path
            )
        })?;
    validate_production_benchmark_profiles(&profiles)?;
    Ok(profiles)
}

pub fn validate_agentic_benchmark_suite(suite: &AgenticBenchmarkSuite) -> Result<()> {
    if suite.version == 0 {
        bail!("benchmark suite version must be >= 1");
    }
    if suite.tasks.is_empty() {
        bail!("benchmark suite must contain at least one task");
    }

    let mut seen_ids = BTreeSet::new();
    for task in &suite.tasks {
        if task.id.trim().is_empty() {
            bail!("benchmark task id must not be empty");
        }
        if !seen_ids.insert(task.id.clone()) {
            bail!("duplicate benchmark task id: {}", task.id);
        }
        if task.repo.trim().is_empty() {
            bail!("benchmark task {} must set repo", task.id);
        }
        if task.prompt.trim().is_empty() {
            bail!("benchmark task {} must set prompt", task.id);
        }
        if task.expected_paths.is_empty() {
            bail!("benchmark task {} must list expected_paths", task.id);
        }
        if task.collie_symbol_queries.is_empty() {
            bail!(
                "benchmark task {} must define collie_symbol_queries",
                task.id
            );
        }
        if task.collie_lexical_queries.is_empty() {
            bail!(
                "benchmark task {} must define collie_lexical_queries",
                task.id
            );
        }
        if task.rg_regex_queries.is_empty() {
            bail!("benchmark task {} must define rg_regex_queries", task.id);
        }

        for path in &task.expected_paths {
            if path.as_os_str().is_empty() || path.is_absolute() {
                bail!(
                    "benchmark task {} must use non-empty repo-relative expected_paths",
                    task.id
                );
            }
        }

        for query in task
            .collie_symbol_queries
            .iter()
            .chain(task.collie_lexical_queries.iter())
            .chain(task.rg_regex_queries.iter())
        {
            if query.trim().is_empty() {
                bail!("benchmark task {} contains an empty query", task.id);
            }
        }
    }

    Ok(())
}

pub fn validate_production_benchmark_profiles(
    profiles: &ProductionBenchmarkProfiles,
) -> Result<()> {
    if profiles.version == 0 {
        bail!("production benchmark profile version must be >= 1");
    }
    if profiles.profiles.is_empty() {
        bail!("production benchmark profiles must contain at least one profile");
    }

    let mut seen_keys = BTreeSet::new();
    for profile in &profiles.profiles {
        if profile.key.trim().is_empty() {
            bail!("production benchmark profile key must not be empty");
        }
        if !seen_keys.insert(profile.key.clone()) {
            bail!(
                "duplicate production benchmark profile key: {}",
                profile.key
            );
        }
        if profile.description.trim().is_empty() {
            bail!(
                "production benchmark profile {} must set description",
                profile.key
            );
        }
        if let Some(default_repo_relpath) = &profile.default_repo_relpath {
            if default_repo_relpath.as_os_str().is_empty() || default_repo_relpath.is_absolute() {
                bail!(
                    "production benchmark profile {} must use a non-empty relative default_repo_relpath",
                    profile.key
                );
            }
        }
        if profile.lexical_queries.is_empty() {
            bail!(
                "production benchmark profile {} must define lexical_queries",
                profile.key
            );
        }
        if profile.symbol_queries.is_empty() {
            bail!(
                "production benchmark profile {} must define symbol_queries",
                profile.key
            );
        }
        if profile.incremental_candidates.is_empty() {
            bail!(
                "production benchmark profile {} must define incremental_candidates",
                profile.key
            );
        }

        if let (Some(min), Some(max)) = (profile.min_tracked_files, profile.max_tracked_files) {
            if min > max {
                bail!(
                    "production benchmark profile {} has min_tracked_files > max_tracked_files",
                    profile.key
                );
            }
        }

        if profile.repo_names.is_empty()
            && profile.repo_origin_substrings.is_empty()
            && profile.min_tracked_files.is_none()
            && profile.max_tracked_files.is_none()
        {
            bail!(
                "production benchmark profile {} must have at least one matching rule",
                profile.key
            );
        }

        if (!profile.repo_names.is_empty() || !profile.repo_origin_substrings.is_empty())
            && profile.default_repo_relpath.is_none()
        {
            bail!(
                "production benchmark profile {} must set default_repo_relpath for specific repos",
                profile.key
            );
        }

        for query in profile
            .lexical_queries
            .iter()
            .chain(profile.symbol_queries.iter())
        {
            if query.trim().is_empty() {
                bail!(
                    "production benchmark profile {} contains an empty query",
                    profile.key
                );
            }
        }

        for path in &profile.incremental_candidates {
            if path.as_os_str().is_empty() || path.is_absolute() {
                bail!(
                    "production benchmark profile {} must use non-empty repo-relative incremental_candidates",
                    profile.key
                );
            }
        }
    }

    Ok(())
}

pub fn generate_corpus(output_dir: &Path) -> Result<()> {
    fs::create_dir_all(output_dir)?;

    for dir_index in 0..50 {
        let dir_path = output_dir.join(format!("dir_{dir_index:02}"));
        fs::create_dir_all(&dir_path)?;

        for file_index in 0..20 {
            let flat_index = dir_index * 20 + file_index;
            let file_path = dir_path.join(format!("file_{file_index:02}.rs"));
            let mut lines = Vec::with_capacity(100);

            for line_index in 0..100 {
                let mut line =
                    format!("fn func_{file_index}_{line_index}() {{ let x = {line_index}; }}");

                if line_index == 50 && flat_index % 100 < 10 {
                    line = "fn initialize_connection() { let status = ready; }".to_string();
                }
                if line_index == 51 && flat_index % 100 < 5 {
                    line = "fn connect_database() { let pool = active; }".to_string();
                }
                if line_index == 52 && flat_index % 100 < 20 {
                    line = "fn handle_request() { let response = ok; }".to_string();
                }

                lines.push(line);
            }

            fs::write(file_path, lines.join("\n"))?;
        }
    }

    Ok(())
}

pub fn build_benchmark_setup(base_dir: &Path) -> Result<BenchmarkSetup> {
    let corpus_path = base_dir.join("corpus");
    let index_path = base_dir.join(".collie");
    generate_corpus(&corpus_path)?;

    let config = CollieConfig::default();
    let mut builder = IndexBuilder::new(&index_path, &config)?;
    builder.set_worktree_root(corpus_path.clone());
    for entry in ignore::WalkBuilder::new(&corpus_path).build() {
        let entry = entry?;
        if entry.path().is_file() {
            builder.index_file(entry.path())?;
        }
    }
    builder.save()?;

    Ok(BenchmarkSetup {
        corpus_path,
        index_path,
        builder,
    })
}

pub fn command_available(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}
