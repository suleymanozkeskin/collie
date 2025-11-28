use anyhow::Result;
use collie_search::benchmark::{build_benchmark_setup, command_available};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn collie_finds_same_files_as_grep() -> Result<()> {
    if !command_available("grep") {
        return Ok(());
    }

    let temp = TempDir::new()?;
    let setup = build_benchmark_setup(temp.path())?;
    let grep_output = Command::new("grep")
        .args(["-rl", "handle_request"])
        .arg(&setup.corpus_path)
        .output()?;

    let grep_paths: BTreeSet<PathBuf> = String::from_utf8_lossy(&grep_output.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(PathBuf::from)
        .collect();
    let collie_paths: BTreeSet<PathBuf> = setup
        .builder
        .search_pattern("handle_request")
        .into_iter()
        .map(|result| result.file_path)
        .collect();

    assert_eq!(collie_paths, grep_paths);
    Ok(())
}
