use anyhow::Result;
use collie_search::benchmark::{build_benchmark_setup, command_available, generate_corpus};
use std::fs;
use tempfile::TempDir;

#[test]
fn corpus_generator_creates_expected_layout() -> Result<()> {
    let temp = TempDir::new()?;
    let corpus = temp.path().join("corpus");
    generate_corpus(&corpus)?;

    for dir_index in 0..50 {
        let dir = corpus.join(format!("dir_{dir_index:02}"));
        assert!(dir.is_dir());
        for file_index in 0..20 {
            assert!(dir.join(format!("file_{file_index:02}.rs")).is_file());
        }
    }
    Ok(())
}

#[test]
fn corpus_generator_plants_expected_tokens() -> Result<()> {
    let temp = TempDir::new()?;
    let corpus = temp.path().join("corpus");
    generate_corpus(&corpus)?;

    let mut initialize_count = 0;
    let mut connect_count = 0;
    let mut handle_count = 0;

    for entry in ignore::WalkBuilder::new(&corpus).hidden(false).build() {
        let entry = entry?;
        if !entry.path().is_file() {
            continue;
        }
        let content = fs::read_to_string(entry.path())?;
        if content.contains("initialize_connection") {
            initialize_count += 1;
        }
        if content.contains("connect_database") {
            connect_count += 1;
        }
        if content.contains("handle_request") {
            handle_count += 1;
        }
    }

    assert_eq!(initialize_count, 100);
    assert_eq!(connect_count, 50);
    assert_eq!(handle_count, 200);
    Ok(())
}

#[test]
fn benchmark_shared_setup_builds_index_successfully() -> Result<()> {
    let temp = TempDir::new()?;
    let setup = build_benchmark_setup(temp.path())?;

    assert!(setup.index_path.exists());
    assert!(setup.builder.stats().total_files >= 1000);
    Ok(())
}

#[test]
fn benchmark_queries_return_non_empty_expected_results() -> Result<()> {
    let temp = TempDir::new()?;
    let setup = build_benchmark_setup(temp.path())?;

    assert_eq!(setup.builder.search_pattern("handle_request").len(), 200);
    assert_eq!(setup.builder.search_pattern("handle%").len(), 200);
    assert_eq!(setup.builder.search_pattern("%request%").len(), 200);
    Ok(())
}

#[test]
fn grep_and_rg_benchmark_probes_do_not_panic_when_missing() {
    let _ = command_available("grep");
    let _ = command_available("rg");
}
