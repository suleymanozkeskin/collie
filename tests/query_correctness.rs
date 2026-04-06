use anyhow::Result;
use collie_search::config::CollieConfig;
use collie_search::indexer::IndexBuilder;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn setup() -> Result<(TempDir, PathBuf, IndexBuilder)> {
    let temp = TempDir::new()?;
    let index_path = temp.path().join(".collie");
    let config = CollieConfig::default();
    let builder = IndexBuilder::new(&index_path, &config)?;
    Ok((temp, index_path, builder))
}

fn write_file(root: &Path, relative: &str, content: &str) -> Result<PathBuf> {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, content)?;
    Ok(path)
}

fn result_paths(results: &[collie_search::storage::SearchResult]) -> BTreeSet<PathBuf> {
    results
        .iter()
        .map(|result| result.file_path.clone())
        .collect()
}

#[test]
fn exact_match_single_file() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let file_path = write_file(temp.path(), "a.rs", "fn hello_world() {}")?;

    builder.index_file(&file_path)?;
    builder.save()?;

    let results = builder.search_pattern("hello_world");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].file_path, file_path);
    Ok(())
}

#[test]
fn exact_match_multi_file() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "fn connect() {}")?;
    let b = write_file(temp.path(), "b.rs", "fn connect() {}")?;

    builder.index_file(&a)?;
    builder.index_file(&b)?;
    builder.save()?;

    let results = builder.search_pattern("connect");
    assert_eq!(results.len(), 2);
    assert_eq!(result_paths(&results), BTreeSet::from([a, b]));
    Ok(())
}

#[test]
fn exact_match_no_result() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "fn hello() {}")?;
    builder.index_file(&a)?;
    builder.save()?;

    assert!(builder.search_pattern("nonexistent").is_empty());
    Ok(())
}

#[test]
fn prefix_match() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "initialize initialization final")?;
    builder.index_file(&a)?;
    builder.save()?;

    assert_eq!(builder.search_pattern("init%").len(), 1);
    assert_eq!(builder.search_pattern("fin%").len(), 1);
    Ok(())
}

#[test]
fn prefix_match_no_result() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "hello world")?;
    builder.index_file(&a)?;
    builder.save()?;

    assert!(builder.search_pattern("xyz%").is_empty());
    Ok(())
}

#[test]
fn suffix_match() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "hello jello world")?;
    builder.index_file(&a)?;
    builder.save()?;

    let results = builder.search_pattern("%llo");
    assert_eq!(results.len(), 1);
    Ok(())
}

#[test]
fn suffix_match_no_result() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "hello world")?;
    builder.index_file(&a)?;
    builder.save()?;

    assert!(builder.search_pattern("%xyz").is_empty());
    Ok(())
}

#[test]
fn substring_match() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "initialize initialization final")?;
    builder.index_file(&a)?;
    builder.save()?;

    assert_eq!(builder.search_pattern("%init%").len(), 1);
    Ok(())
}

#[test]
fn substring_match_short_query() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "cat concatenate locate")?;
    builder.index_file(&a)?;
    builder.save()?;

    assert_eq!(builder.search_pattern("%at%").len(), 1);
    Ok(())
}

#[test]
fn substring_match_no_result() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "hello world")?;
    builder.index_file(&a)?;
    builder.save()?;

    assert!(builder.search_pattern("%xyz%").is_empty());
    Ok(())
}

#[test]
fn case_insensitive() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "HelloWorld HELLOWORLD helloworld")?;
    builder.index_file(&a)?;
    builder.save()?;

    let results = builder.search_pattern("helloworld");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].file_path, a);
    Ok(())
}

#[test]
fn persistence_round_trip() -> Result<()> {
    let (temp, index_path, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "fn hello_world() {}")?;
    builder.index_file(&a)?;
    builder.save()?;

    let reloaded = IndexBuilder::new(&index_path, &CollieConfig::default())?;
    let results = reloaded.search_pattern("hello_world");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].file_path, a);
    Ok(())
}

#[test]
fn multi_file_isolation() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "alpha beta")?;
    let b = write_file(temp.path(), "b.rs", "gamma delta")?;
    builder.index_file(&a)?;
    builder.index_file(&b)?;
    builder.save()?;

    let alpha = builder.search_pattern("alpha");
    assert_eq!(alpha.len(), 1);
    assert_eq!(alpha[0].file_path, a);

    let gamma = builder.search_pattern("gamma");
    assert_eq!(gamma.len(), 1);
    assert_eq!(gamma[0].file_path, b);
    Ok(())
}

// ---------------------------------------------------------------------------
// Multi-word queries (Bug #1: spaces in queries returned 0 results)
// ---------------------------------------------------------------------------

#[test]
fn multi_word_exact_match() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.go", "func (h *HollowKubelet) Run() { }")?;
    builder.index_file(&a)?;
    builder.save()?;

    // Multi-word query: tokens AND'd together
    let results = builder.search_pattern("HollowKubelet Run");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].file_path, a);
    Ok(())
}

#[test]
fn multi_word_no_match_when_only_one_term_present() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.go", "func Run() { }")?;
    builder.index_file(&a)?;
    builder.save()?;

    // "HollowKubelet" is not in the file, so AND fails
    let results = builder.search_pattern("HollowKubelet Run");
    assert!(results.is_empty());
    Ok(())
}

#[test]
fn multi_word_across_files() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "fn validate authorization")?;
    let b = write_file(temp.path(), "b.rs", "fn validate only")?;
    builder.index_file(&a)?;
    builder.index_file(&b)?;
    builder.save()?;

    let results = builder.search_pattern("authorization validate");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].file_path, a);
    Ok(())
}

// ---------------------------------------------------------------------------
// Punctuation in queries (Bug #2: query not tokenized like indexed content)
// ---------------------------------------------------------------------------

#[test]
fn punctuation_stripped_from_exact_query() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.go", "func Validate() error { }")?;
    builder.index_file(&a)?;
    builder.save()?;

    // "Validate()" should match — parens stripped by tokenizer
    let results = builder.search_pattern("Validate()");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].file_path, a);
    Ok(())
}

#[test]
fn double_colon_stripped_from_query() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "use std::io::Read;")?;
    builder.index_file(&a)?;
    builder.save()?;

    // "std::io" → tokens ["std", "io"] → multi-term AND
    let results = builder.search_pattern("std::io");
    assert_eq!(results.len(), 1);
    Ok(())
}

#[test]
fn hyphen_stripped_from_query() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.css", "my-variable: red;")?;
    builder.index_file(&a)?;
    builder.save()?;

    // "my-variable" → tokens ["my", "variable"] → multi-term AND
    let results = builder.search_pattern("my-variable");
    assert_eq!(results.len(), 1);
    Ok(())
}

#[test]
fn dot_stripped_from_query() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.py", "obj.method()")?;
    builder.index_file(&a)?;
    builder.save()?;

    // "obj.method" → tokens ["obj", "method"] → multi-term AND
    let results = builder.search_pattern("obj.method");
    assert_eq!(results.len(), 1);
    Ok(())
}

#[test]
fn punctuation_stripped_from_prefix_query() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.go", "func Validate() { }")?;
    builder.index_file(&a)?;
    builder.save()?;

    // "validate(%" → tokenize inner "validate(" → ["validate"] → prefix
    let results = builder.search_pattern("validate(%");
    assert_eq!(results.len(), 1);
    Ok(())
}

#[test]
fn punctuation_stripped_from_suffix_query() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "fn hello_run() { }")?;
    builder.index_file(&a)?;
    builder.save()?;

    // "%::run" → tokenize inner "::run" → ["run"] → suffix for "run"
    let results = builder.search_pattern("%::run");
    assert_eq!(results.len(), 1);
    Ok(())
}

// ---------------------------------------------------------------------------
// Degenerate wildcards (Bug #3: empty/trivial patterns not guarded)
// ---------------------------------------------------------------------------

#[test]
fn degenerate_wildcard_percent_only() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "fn hello() { }")?;
    builder.index_file(&a)?;
    builder.save()?;

    // "%" → inner is empty after trim → tokenize → [] → empty result
    assert!(builder.search_pattern("%").is_empty());
    Ok(())
}

#[test]
fn degenerate_wildcard_double_percent() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "fn hello() { }")?;
    builder.index_file(&a)?;
    builder.save()?;

    // "%%" → inner is empty → tokenize → [] → empty result
    assert!(builder.search_pattern("%%").is_empty());
    Ok(())
}

#[test]
fn degenerate_wildcard_single_char_substring() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "fn hello() { }")?;
    builder.index_file(&a)?;
    builder.save()?;

    // "%a%" → inner "a" → tokenize → [] (dropped by min_length=2) → empty
    assert!(builder.search_pattern("%a%").is_empty());
    Ok(())
}

// ---------------------------------------------------------------------------
// Ranked search
// ---------------------------------------------------------------------------

#[test]
fn ranked_search_returns_results() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "fn hello() { }")?;
    let b = write_file(temp.path(), "b.rs", "fn hello() { hello(); hello(); }")?;
    builder.index_file(&a)?;
    builder.index_file(&b)?;
    builder.save()?;

    let results = builder.search_pattern_ranked("hello", 10);
    assert_eq!(results.len(), 2);
    Ok(())
}

#[test]
fn ranked_search_respects_limit() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.rs", "fn hello() { }")?;
    let b = write_file(temp.path(), "b.rs", "fn hello() { }")?;
    let c = write_file(temp.path(), "c.rs", "fn hello() { }")?;
    builder.index_file(&a)?;
    builder.index_file(&b)?;
    builder.index_file(&c)?;
    builder.save()?;

    let results = builder.search_pattern_ranked("hello", 2);
    assert_eq!(results.len(), 2);
    Ok(())
}

#[test]
fn regex_ranked_search_respects_limit() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    for name in ["a.rs", "b.rs", "c.rs", "d.rs"] {
        let path = write_file(temp.path(), name, "fn hello_world() { hello_world(); }\n")?;
        builder.index_file(&path)?;
    }
    builder.save()?;

    let results = builder.search_regex("hello_.*", 2, false, false, false)?;
    assert_eq!(results.len(), 2);
    Ok(())
}

#[test]
fn regex_ranked_search_with_matches_respects_limit() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    for name in ["a.rs", "b.rs", "c.rs", "d.rs"] {
        let path = write_file(temp.path(), name, "fn hello_world() { hello_world(); }\n")?;
        builder.index_file(&path)?;
    }
    builder.save()?;

    let results = builder.search_regex("hello_.*", 2, false, false, true)?;
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|result| !result.matches.is_empty()));
    Ok(())
}

#[test]
fn ranked_multi_word_search() -> Result<()> {
    let (temp, _, mut builder) = setup()?;
    let a = write_file(temp.path(), "a.go", "func Validate(auth Authorization) { }")?;
    let b = write_file(temp.path(), "b.go", "func Validate(input string) { }")?;
    builder.index_file(&a)?;
    builder.index_file(&b)?;
    builder.save()?;

    let results = builder.search_pattern_ranked("authorization validate", 10);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].file_path, a);
    Ok(())
}
