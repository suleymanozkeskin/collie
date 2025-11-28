use anyhow::Result;
use collie_search::config::CollieConfig;
use collie_search::indexer::IndexBuilder;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn setup() -> Result<(TempDir, IndexBuilder)> {
    let temp = TempDir::new()?;
    let index_path = temp.path().join(".collie");
    let config = CollieConfig::default();
    let builder = IndexBuilder::new(&index_path, &config)?;
    Ok((temp, builder))
}

fn write_file(root: &Path, relative: &str, content: &str) -> Result<PathBuf> {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, content)?;
    Ok(path)
}

#[test]
fn reindex_removes_old_tokens() -> Result<()> {
    let (temp, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "fn old_func() {}")?;
    builder.index_file(&a)?;

    fs::write(&a, "fn new_func() {}")?;
    builder.index_file(&a)?;
    builder.save()?;

    assert!(builder.search_pattern("old_func").is_empty());
    assert_eq!(builder.search_pattern("new_func").len(), 1);
    Ok(())
}

#[test]
fn reindex_preserves_other_files() -> Result<()> {
    let (temp, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "alpha")?;
    let b = write_file(temp.path(), "b.rs", "beta")?;
    builder.index_file(&a)?;
    builder.index_file(&b)?;

    fs::write(&a, "gamma")?;
    builder.index_file(&a)?;
    builder.save()?;

    assert!(builder.search_pattern("alpha").is_empty());
    assert_eq!(builder.search_pattern("beta").len(), 1);
    assert_eq!(builder.search_pattern("gamma").len(), 1);
    Ok(())
}

#[test]
fn delete_file_cleans_all_indexes() -> Result<()> {
    let (temp, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "initialize connect")?;
    builder.index_file(&a)?;
    builder.remove_file(&a);
    builder.save()?;

    assert!(builder.search_pattern("initialize").is_empty());
    assert!(builder.search_pattern("init%").is_empty());
    assert!(builder.search_pattern("%ize").is_empty());
    assert!(builder.search_pattern("%niti%").is_empty());
    assert_eq!(builder.stats().total_files, 0);
    Ok(())
}

#[test]
fn rapid_reindex_same_file() -> Result<()> {
    let (temp, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "version1")?;
    builder.index_file(&a)?;

    for version in ["version2", "version3", "version4"] {
        fs::write(&a, version)?;
        builder.index_file(&a)?;
    }
    builder.save()?;

    assert!(builder.search_pattern("version1").is_empty());
    assert!(builder.search_pattern("version2").is_empty());
    assert!(builder.search_pattern("version3").is_empty());
    assert_eq!(builder.search_pattern("version4").len(), 1);
    assert_eq!(builder.stats().total_files, 1);
    Ok(())
}

#[test]
fn reindex_does_not_leak_file_ids() -> Result<()> {
    let (temp, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "version0")?;
    builder.index_file(&a)?;

    for index in 1..=10 {
        fs::write(&a, format!("version{index}"))?;
        builder.index_file(&a)?;
    }
    builder.save()?;

    assert_eq!(builder.stats().total_files, 1);
    Ok(())
}

#[test]
fn delete_nonexistent_file_is_noop() -> Result<()> {
    let (_temp, mut builder) = setup()?;
    builder.remove_file(Path::new("nonexistent.rs"));
    assert_eq!(builder.stats().total_files, 0);
    Ok(())
}

#[test]
fn reindex_all_query_types_consistent() -> Result<()> {
    let (temp, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "initialization")?;
    builder.index_file(&a)?;

    fs::write(&a, "finalization")?;
    builder.index_file(&a)?;
    builder.save()?;

    for pattern in [
        "initialization",
        "initialization%",
        "%initialization",
        "%initializ%",
    ] {
        assert!(
            builder.search_pattern(pattern).is_empty(),
            "expected old pattern {pattern} to be removed"
        );
    }

    for pattern in [
        "finalization",
        "finalization%",
        "%finalization",
        "%finaliz%",
    ] {
        assert_eq!(
            builder.search_pattern(pattern).len(),
            1,
            "expected new pattern {pattern} to be indexed"
        );
    }

    Ok(())
}
