use anyhow::Result;
use collie_search::storage::tantivy_index::{SearchResult, TantivyIndex};
use std::collections::BTreeSet;
use std::path::PathBuf;
use tempfile::TempDir;

fn setup() -> Result<(TempDir, TantivyIndex)> {
    let temp = TempDir::new()?;
    let index_dir = temp.path().join("tantivy");
    let index = TantivyIndex::open(&index_dir)?;
    Ok((temp, index))
}

fn result_paths(results: &[SearchResult]) -> BTreeSet<PathBuf> {
    results.iter().map(|r| r.file_path.clone()).collect()
}

#[test]
fn index_and_search_exact() -> Result<()> {
    let (_temp, mut index) = setup()?;

    index.index_file_content("/a.rs".as_ref(), "hello other_token hello")?;
    index.commit()?;

    let results = index.search_exact("hello");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].file_path, PathBuf::from("/a.rs"));

    Ok(())
}

#[test]
fn search_exact_no_match() -> Result<()> {
    let (_temp, mut index) = setup()?;

    index.index_file_content("/a.rs".as_ref(), "hello")?;
    index.commit()?;

    assert!(index.search_exact("nonexistent").is_empty());

    Ok(())
}

#[test]
fn search_prefix() -> Result<()> {
    let (_temp, mut index) = setup()?;

    index.index_file_content("/a.rs".as_ref(), "initialize initialization final")?;
    index.commit()?;

    assert_eq!(index.search_prefix("init").len(), 1);
    assert_eq!(index.search_prefix("fin").len(), 1);
    assert!(index.search_prefix("xyz").is_empty());

    Ok(())
}

#[test]
fn search_suffix() -> Result<()> {
    let (_temp, mut index) = setup()?;

    index.index_file_content("/a.rs".as_ref(), "hello jello world")?;
    index.commit()?;

    assert_eq!(index.search_suffix("llo").len(), 1);
    assert!(index.search_suffix("xyz").is_empty());

    Ok(())
}

#[test]
fn search_substring() -> Result<()> {
    let (_temp, mut index) = setup()?;

    index.index_file_content("/a.rs".as_ref(), "initialize initialization final")?;
    index.commit()?;

    assert_eq!(index.search_substring("init").len(), 1);
    assert!(index.search_substring("xyz").is_empty());

    Ok(())
}

#[test]
fn search_substring_short_query() -> Result<()> {
    let (_temp, mut index) = setup()?;

    index.index_file_content("/a.rs".as_ref(), "cat concatenate locate")?;
    index.commit()?;

    assert_eq!(index.search_substring("at").len(), 1);

    Ok(())
}

#[test]
fn search_phrase_respects_token_order_and_gaps() -> Result<()> {
    let (_temp, mut index) = setup()?;

    index.index_file_content("/a.rs".as_ref(), "context.Context")?;
    index.index_file_content("/b.rs".as_ref(), "context other Context")?;
    index.index_file_content("/c.rs".as_ref(), "foo a bar")?;
    index.commit()?;

    let adjacent = index.search_phrase(&[(0, "context".to_string()), (1, "context".to_string())]);
    let gap = index.search_phrase(&[(0, "foo".to_string()), (2, "bar".to_string())]);

    let adjacent_paths = result_paths(&adjacent);
    assert!(adjacent_paths.contains(&PathBuf::from("/a.rs")));
    assert!(!adjacent_paths.contains(&PathBuf::from("/b.rs")));

    let gap_paths = result_paths(&gap);
    assert!(gap_paths.contains(&PathBuf::from("/c.rs")));

    Ok(())
}

#[test]
fn remove_file_clears_all_docs() -> Result<()> {
    let (_temp, mut index) = setup()?;

    index.index_file_content("/a.rs".as_ref(), "alpha beta")?;
    index.commit()?;

    index.remove_by_path("/a.rs".as_ref())?;
    index.commit()?;

    assert!(index.search_exact("alpha").is_empty());
    assert!(index.search_exact("beta").is_empty());

    Ok(())
}

#[test]
fn reindex_file_replaces_old_tokens() -> Result<()> {
    let (_temp, mut index) = setup()?;

    index.index_file_content("/a.rs".as_ref(), "old_func")?;
    index.commit()?;

    index.remove_by_path("/a.rs".as_ref())?;
    index.index_file_content("/a.rs".as_ref(), "new_func")?;
    index.commit()?;

    assert!(index.search_exact("old_func").is_empty());
    assert_eq!(index.search_exact("new_func").len(), 1);

    Ok(())
}

#[test]
fn commit_makes_changes_visible() -> Result<()> {
    let (_temp, mut index) = setup()?;

    index.index_file_content("/a.rs".as_ref(), "before_commit")?;

    // Before commit — not visible
    assert!(index.search_exact("before_commit").is_empty());

    index.commit()?;

    // After commit — visible
    assert_eq!(index.search_exact("before_commit").len(), 1);

    Ok(())
}

#[test]
fn reader_without_writer_succeeds() -> Result<()> {
    let temp = TempDir::new()?;
    let index_dir = temp.path().join("tantivy");

    // First: create index, add data, commit, drop
    {
        let mut index = TantivyIndex::open(&index_dir)?;
        index.index_file_content("/a.rs".as_ref(), "persisted")?;
        index.commit()?;
    }

    // Second: reopen — should be able to search without acquiring writer
    {
        let index = TantivyIndex::open(&index_dir)?;
        let results = index.search_exact("persisted");
        assert_eq!(results.len(), 1);
    }

    Ok(())
}

#[test]
fn case_insensitive_via_lowercased_tokens() -> Result<()> {
    let (_temp, mut index) = setup()?;

    index.index_file_content("/a.rs".as_ref(), "helloworld other")?;
    index.commit()?;

    assert_eq!(index.search_exact("helloworld").len(), 1);

    Ok(())
}

#[test]
fn mixed_case_query_is_normalized_before_exact_search() -> Result<()> {
    let (_temp, mut index) = setup()?;

    index.index_file_content("/a.rs".as_ref(), "SharedinformerFactory other")?;
    index.commit()?;

    assert_eq!(index.search_exact("SharedInformerFactory").len(), 1);

    Ok(())
}

#[test]
fn multi_file_search_returns_correct_paths() -> Result<()> {
    let (_temp, mut index) = setup()?;

    index.index_file_content("/a.rs".as_ref(), "connect other")?;
    index.index_file_content("/b.rs".as_ref(), "connect another")?;
    index.commit()?;

    let results = index.search_exact("connect");
    assert_eq!(results.len(), 2);
    let paths = result_paths(&results);
    assert!(paths.contains(&PathBuf::from("/a.rs")));
    assert!(paths.contains(&PathBuf::from("/b.rs")));

    Ok(())
}

#[test]
fn list_all_files_returns_all_indexed_paths() -> Result<()> {
    let (_temp, mut index) = setup()?;

    index.index_file_content("/a.rs".as_ref(), "alpha")?;
    index.index_file_content("/b.rs".as_ref(), "beta")?;
    index.index_file_content("/c.rs".as_ref(), "gamma")?;
    index.commit()?;

    let results = index.list_all_files();
    let paths = result_paths(&results);
    assert_eq!(paths.len(), 3);
    assert!(paths.contains(&PathBuf::from("/a.rs")));
    assert!(paths.contains(&PathBuf::from("/b.rs")));
    assert!(paths.contains(&PathBuf::from("/c.rs")));
    Ok(())
}

#[test]
fn list_all_files_empty_index() -> Result<()> {
    let (_temp, index) = setup()?;
    assert!(index.list_all_files().is_empty());
    Ok(())
}

#[test]
fn list_all_files_cache_invalidates_after_commit() -> Result<()> {
    let (_temp, mut index) = setup()?;

    index.index_file_content("/a.rs".as_ref(), "alpha")?;
    index.commit()?;

    let first = index.list_all_files();
    assert_eq!(first.len(), 1);

    index.index_file_content("/b.rs".as_ref(), "beta")?;
    index.commit()?;

    let second = index.list_all_files();
    let paths = result_paths(&second);
    assert_eq!(paths.len(), 2);
    assert!(paths.contains(&PathBuf::from("/a.rs")));
    assert!(paths.contains(&PathBuf::from("/b.rs")));

    Ok(())
}

#[test]
fn list_all_files_survives_reopen_without_tantivy_materialization() -> Result<()> {
    let temp = TempDir::new()?;
    let index_dir = temp.path().join("tantivy");

    {
        let mut index = TantivyIndex::open(&index_dir)?;
        index.index_file_content("/a.rs".as_ref(), "alpha")?;
        index.index_file_content("/b.rs".as_ref(), "beta")?;
        index.commit()?;
    }

    let index = TantivyIndex::open(&index_dir)?;
    let results = index.list_all_files();
    let paths = result_paths(&results);
    assert_eq!(
        paths,
        BTreeSet::from([PathBuf::from("/a.rs"), PathBuf::from("/b.rs")])
    );
    assert_eq!(index.file_count(), 2);
    Ok(())
}

#[test]
fn list_all_files_tracks_incremental_add_and_remove_after_reopen() -> Result<()> {
    let temp = TempDir::new()?;
    let index_dir = temp.path().join("tantivy");

    {
        let mut index = TantivyIndex::open(&index_dir)?;
        index.index_file_content("/a.rs".as_ref(), "alpha")?;
        index.index_file_content("/b.rs".as_ref(), "beta")?;
        index.commit()?;
    }

    {
        let mut index = TantivyIndex::open(&index_dir)?;
        index.remove_by_path("/a.rs".as_ref())?;
        index.index_file_content("/c.rs".as_ref(), "gamma")?;
        index.commit()?;
    }

    let index = TantivyIndex::open(&index_dir)?;
    let results = index.list_all_files();
    let paths = result_paths(&results);
    assert_eq!(
        paths,
        BTreeSet::from([PathBuf::from("/b.rs"), PathBuf::from("/c.rs")])
    );
    assert_eq!(index.file_count(), 2);
    Ok(())
}

#[test]
fn indexed_text_survives_reopen_and_removal() -> Result<()> {
    let temp = TempDir::new()?;
    let index_dir = temp.path().join("tantivy");

    {
        let mut index = TantivyIndex::open(&index_dir)?;
        index.index_file_content("/a.rs".as_ref(), "fn stored_text() {}\n")?;
        index.commit()?;
        assert_eq!(
            index.indexed_text("/a.rs".as_ref()).as_deref(),
            Some("fn stored_text() {}\n")
        );
    }

    {
        let index = TantivyIndex::open(&index_dir)?;
        assert_eq!(
            index.indexed_text("/a.rs".as_ref()).as_deref(),
            Some("fn stored_text() {}\n")
        );
    }

    {
        let mut index = TantivyIndex::open(&index_dir)?;
        index.remove_by_path("/a.rs".as_ref())?;
        index.commit()?;
        assert!(index.indexed_text("/a.rs".as_ref()).is_none());
    }

    Ok(())
}
