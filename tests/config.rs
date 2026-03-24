mod common;

use anyhow::Result;
use collie_search::config::CollieConfig;
use common::*;
use std::fs;

#[test]
fn config_default_when_no_file_exists() -> Result<()> {
    let worktree = create_worktree()?;
    let config = CollieConfig::load(worktree.path());

    assert_eq!(config.index.min_token_length, 2);
    assert_eq!(config.index.max_file_size, 1_048_576);
    assert!(config.index.extra_extensions.is_empty());
    assert!(config.index.exclude_extensions.is_empty());
    assert_eq!(config.watcher.debounce_ms, 300);
    assert_eq!(config.search.default_limit, 20);
    assert_eq!(config.search.context_lines, 2);
    Ok(())
}

#[test]
fn config_partial_override_uses_defaults_for_rest() -> Result<()> {
    let worktree = create_worktree()?;
    let config_dir = worktree.path().join(".collie");
    fs::create_dir_all(&config_dir)?;
    fs::write(
        config_dir.join("config.toml"),
        "[watcher]\ndebounce_ms = 500\n",
    )?;

    let config = CollieConfig::load(worktree.path());
    assert_eq!(config.watcher.debounce_ms, 500);
    assert_eq!(config.index.min_token_length, 2);
    assert_eq!(config.index.max_file_size, 1_048_576);
    assert_eq!(config.search.default_limit, 20);
    Ok(())
}

#[test]
fn config_invalid_toml_falls_back_to_defaults() -> Result<()> {
    let worktree = create_worktree()?;
    let config_dir = worktree.path().join(".collie");
    fs::create_dir_all(&config_dir)?;
    fs::write(config_dir.join("config.toml"), "this is not valid toml {{{")?;

    let config = CollieConfig::load(worktree.path());
    assert_eq!(config.index.min_token_length, 2);
    assert_eq!(config.watcher.debounce_ms, 300);
    assert_eq!(config.search.default_limit, 20);
    Ok(())
}

#[test]
fn config_extra_extensions_are_loaded() -> Result<()> {
    let worktree = create_worktree()?;
    let config_dir = worktree.path().join(".collie");
    fs::create_dir_all(&config_dir)?;
    fs::write(
        config_dir.join("config.toml"),
        "[index]\nextra_extensions = [\"sql\", \"proto\"]\n",
    )?;

    let config = CollieConfig::load(worktree.path());
    assert_eq!(config.index.extra_extensions, vec!["sql", "proto"]);
    Ok(())
}

#[test]
fn config_exclude_extensions_are_loaded() -> Result<()> {
    let worktree = create_worktree()?;
    let config_dir = worktree.path().join(".collie");
    fs::create_dir_all(&config_dir)?;
    fs::write(
        config_dir.join("config.toml"),
        "[index]\nexclude_extensions = [\"md\", \"txt\"]\n",
    )?;

    let config = CollieConfig::load(worktree.path());
    assert_eq!(config.index.exclude_extensions, vec!["md", "txt"]);
    Ok(())
}

#[test]
fn config_max_file_size_override() -> Result<()> {
    let worktree = create_worktree()?;
    let config_dir = worktree.path().join(".collie");
    fs::create_dir_all(&config_dir)?;
    fs::write(
        config_dir.join("config.toml"),
        "[index]\nmax_file_size = 512\n",
    )?;

    let config = CollieConfig::load(worktree.path());
    assert_eq!(config.index.max_file_size, 512);
    Ok(())
}

#[test]
fn config_prefers_new_path_over_legacy() -> Result<()> {
    let worktree = create_worktree()?;
    let config_dir = worktree.path().join(".collie");
    fs::create_dir_all(&config_dir)?;
    fs::write(
        config_dir.join("config.toml"),
        "[search]\ndefault_limit = 10\n",
    )?;
    fs::write(
        worktree.path().join(".collie.toml"),
        "[search]\ndefault_limit = 70\n",
    )?;

    let config = CollieConfig::load(worktree.path());
    assert_eq!(config.search.default_limit, 70);
    Ok(())
}

#[test]
fn config_init_creates_example_file() -> Result<()> {
    let worktree = create_worktree()?;
    let output = run_collie(worktree.path(), &["config", "--init"])?;
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let config_path = worktree.path().join(".collie.toml");
    assert!(config_path.exists());

    let content = fs::read_to_string(&config_path)?;
    assert!(content.contains("[index]"));
    assert!(content.contains("[watcher]"));
    assert!(content.contains("[search]"));
    assert!(content.contains("max_file_size"));
    assert!(content.contains("idle_timeout_secs"));
    assert!(content.contains("context_lines"));
    Ok(())
}

#[test]
fn config_init_does_not_overwrite_existing() -> Result<()> {
    let worktree = create_worktree()?;
    let config_path = worktree.path().join(".collie.toml");
    fs::write(&config_path, "# my custom config\n")?;

    let output = run_collie(worktree.path(), &["config", "--init"])?;
    assert!(output.status.success());

    let content = fs::read_to_string(&config_path)?;
    assert_eq!(content, "# my custom config\n");

    assert!(stdout(&output).contains("config already exists"));
    Ok(())
}
