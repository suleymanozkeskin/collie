mod common;

use anyhow::{Context, Result};
use collie_search::config::CollieConfig;
use collie_search::indexer::IndexBuilder;
use collie_search::watcher::{self, WatchEvent};
use crossbeam_channel::{Receiver, RecvTimeoutError, TryRecvError, bounded};
use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use common::{create_worktree, index_path, write_file};

fn wait_for_event<F>(
    rx: &Receiver<WatchEvent>,
    timeout: Duration,
    predicate: F,
) -> Result<WatchEvent>
where
    F: Fn(&WatchEvent) -> bool,
{
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(remaining.min(Duration::from_millis(250))) {
            Ok(event) if predicate(&event) => return Ok(event),
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                anyhow::bail!("watch event channel disconnected")
            }
        }
    }

    anyhow::bail!("timed out waiting for expected watch event")
}

fn load_builder(index_path: &Path) -> Result<IndexBuilder> {
    let config = CollieConfig::default();
    IndexBuilder::new(index_path, &config)
        .with_context(|| format!("failed to open index {:?}", index_path))
}

#[test]
fn watch_detects_new_file() -> Result<()> {
    let worktree = create_worktree()?;
    let index_path = index_path(worktree.path());
    let (tx, rx) = bounded(100);
    let handle = watcher::start(
        worktree.path().to_path_buf(),
        index_path.clone(),
        CollieConfig::default(),
        Some(Box::new(move |event| {
            let _ = tx.send(event);
        })),
    )?;

    let file = write_file(worktree.path(), "hello.rs", "fn greet() {}")?;
    let event = wait_for_event(
        &rx,
        Duration::from_secs(5),
        |event| matches!(event, WatchEvent::Indexed { path } if path == &file),
    )?;
    assert!(matches!(event, WatchEvent::Indexed { .. }));

    let builder = load_builder(&index_path)?;
    assert_eq!(builder.search_pattern("greet").len(), 1);

    handle.stop();
    handle.join()?;
    Ok(())
}

#[test]
fn watch_detects_modified_file() -> Result<()> {
    let worktree = create_worktree()?;
    let index_path = index_path(worktree.path());
    let (tx, rx) = bounded(100);
    let handle = watcher::start(
        worktree.path().to_path_buf(),
        index_path.clone(),
        CollieConfig::default(),
        Some(Box::new(move |event| {
            let _ = tx.send(event);
        })),
    )?;

    let file = write_file(worktree.path(), "a.rs", "fn old() {}")?;
    wait_for_event(
        &rx,
        Duration::from_secs(5),
        |event| matches!(event, WatchEvent::Indexed { path } if path == &file),
    )?;

    fs::write(&file, "fn updated() {}")?;
    wait_for_event(
        &rx,
        Duration::from_secs(5),
        |event| matches!(event, WatchEvent::Indexed { path } if path == &file),
    )?;

    let builder = load_builder(&index_path)?;
    assert!(builder.search_pattern("old").is_empty());
    assert_eq!(builder.search_pattern("updated").len(), 1);

    handle.stop();
    handle.join()?;
    Ok(())
}

#[test]
fn watch_detects_deleted_file() -> Result<()> {
    let worktree = create_worktree()?;
    let index_path = index_path(worktree.path());
    let (tx, rx) = bounded(100);
    let handle = watcher::start(
        worktree.path().to_path_buf(),
        index_path.clone(),
        CollieConfig::default(),
        Some(Box::new(move |event| {
            let _ = tx.send(event);
        })),
    )?;

    let file = write_file(worktree.path(), "a.rs", "fn doomed() {}")?;
    wait_for_event(
        &rx,
        Duration::from_secs(5),
        |event| matches!(event, WatchEvent::Indexed { path } if path == &file),
    )?;

    fs::remove_file(&file)?;
    wait_for_event(
        &rx,
        Duration::from_secs(5),
        |event| matches!(event, WatchEvent::Removed { path } if path == &file),
    )?;

    let builder = load_builder(&index_path)?;
    assert!(builder.search_pattern("doomed").is_empty());

    handle.stop();
    handle.join()?;
    Ok(())
}

#[test]
fn watch_ignores_hidden_files() -> Result<()> {
    let worktree = create_worktree()?;
    let index_path = index_path(worktree.path());
    let (tx, rx) = bounded(100);
    let handle = watcher::start(
        worktree.path().to_path_buf(),
        index_path.clone(),
        CollieConfig::default(),
        Some(Box::new(move |event| {
            let _ = tx.send(event);
        })),
    )?;

    write_file(worktree.path(), ".hidden.rs", "fn secret() {}")?;
    thread::sleep(Duration::from_secs(2));
    assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));

    let builder = load_builder(&index_path)?;
    assert!(builder.search_pattern("secret").is_empty());

    handle.stop();
    handle.join()?;
    Ok(())
}

#[test]
fn watch_ignores_git_directory() -> Result<()> {
    let worktree = create_worktree()?;
    let index_path = index_path(worktree.path());
    let (tx, rx) = bounded(100);
    let handle = watcher::start(
        worktree.path().to_path_buf(),
        index_path,
        CollieConfig::default(),
        Some(Box::new(move |event| {
            let _ = tx.send(event);
        })),
    )?;

    write_file(worktree.path(), ".git/config", "fn gitconfig() {}")?;
    thread::sleep(Duration::from_secs(2));
    assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));

    handle.stop();
    handle.join()?;
    Ok(())
}

#[test]
fn watch_ignores_non_source_files() -> Result<()> {
    let worktree = create_worktree()?;
    let index_path = index_path(worktree.path());
    let (tx, rx) = bounded(100);
    let handle = watcher::start(
        worktree.path().to_path_buf(),
        index_path.clone(),
        CollieConfig::default(),
        Some(Box::new(move |event| {
            let _ = tx.send(event);
        })),
    )?;

    let file = worktree.path().join("image.png");
    fs::write(&file, [0x89, 0x50, 0x4E, 0x47])?;
    thread::sleep(Duration::from_secs(2));
    assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));

    let builder = load_builder(&index_path)?;
    assert!(builder.search_pattern("png").is_empty());

    handle.stop();
    handle.join()?;
    Ok(())
}

#[test]
fn watch_handles_rapid_changes() -> Result<()> {
    let worktree = create_worktree()?;
    let index_path = index_path(worktree.path());
    let (tx, rx) = bounded(100);
    let handle = watcher::start(
        worktree.path().to_path_buf(),
        index_path.clone(),
        CollieConfig::default(),
        Some(Box::new(move |event| {
            let _ = tx.send(event);
        })),
    )?;

    let file = worktree.path().join("a.rs");
    for version in ["v1", "v2", "v3", "v4", "v5"] {
        fs::write(&file, format!("fn {version}() {{}}"))?;
        thread::sleep(Duration::from_millis(50));
    }

    let silence_deadline = Instant::now() + Duration::from_secs(8);
    let mut saw_final_index = false;
    loop {
        match rx.recv_timeout(Duration::from_secs(3)) {
            Ok(WatchEvent::Indexed { path }) if path == file => {
                saw_final_index = true;
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => break,
            Err(RecvTimeoutError::Disconnected) => {
                anyhow::bail!("watch event channel disconnected")
            }
        }

        if Instant::now() > silence_deadline {
            break;
        }
    }
    assert!(
        saw_final_index,
        "expected at least one indexed event for rapid changes"
    );

    let builder = load_builder(&index_path)?;
    assert!(builder.search_pattern("v1").is_empty());
    assert!(builder.search_pattern("v2").is_empty());
    assert!(builder.search_pattern("v3").is_empty());
    assert!(builder.search_pattern("v4").is_empty());
    assert_eq!(builder.search_pattern("v5").len(), 1);

    handle.stop();
    handle.join()?;
    Ok(())
}

#[test]
fn watch_stop_terminates() -> Result<()> {
    let worktree = create_worktree()?;
    let index_path = index_path(worktree.path());
    let handle = watcher::start(
        worktree.path().to_path_buf(),
        index_path,
        CollieConfig::default(),
        None,
    )?;

    let start = Instant::now();
    handle.stop();
    handle.join()?;
    assert!(start.elapsed() < Duration::from_secs(2));
    Ok(())
}

/// Regression: incremental add/remove must propagate within a bounded time,
/// not just "eventually". Covers the fix where NoMergePolicy + BatchSaved
/// eliminated OOM-induced daemon crashes during watcher commits.
#[test]
fn incremental_add_remove_propagates_within_deadline() -> Result<()> {
    let worktree = create_worktree()?;
    let index_path = index_path(worktree.path());
    let (tx, rx) = bounded(100);
    let handle = watcher::start(
        worktree.path().to_path_buf(),
        index_path.clone(),
        CollieConfig::default(),
        Some(Box::new(move |event| {
            let _ = tx.send(event);
        })),
    )?;

    let file = write_file(worktree.path(), "target.rs", "fn original() {}")?;
    wait_for_event(
        &rx,
        Duration::from_secs(5),
        |event| matches!(event, WatchEvent::Indexed { path } if path == &file),
    )?;

    // Add: overwrite file, expect indexed event within 5s
    let add_start = Instant::now();
    fs::write(&file, "fn added_token() {}")?;
    wait_for_event(
        &rx,
        Duration::from_secs(5),
        |event| matches!(event, WatchEvent::Indexed { path } if path == &file),
    )?;
    let add_elapsed = add_start.elapsed();

    let builder = load_builder(&index_path)?;
    assert_eq!(builder.search_pattern("added_token").len(), 1);
    assert!(builder.search_pattern("original").is_empty());

    // Remove: delete file, expect removed event within 5s
    let remove_start = Instant::now();
    fs::remove_file(&file)?;
    wait_for_event(
        &rx,
        Duration::from_secs(5),
        |event| matches!(event, WatchEvent::Removed { path } if path == &file),
    )?;
    let remove_elapsed = remove_start.elapsed();

    let builder = load_builder(&index_path)?;
    assert!(builder.search_pattern("added_token").is_empty());

    // Both operations must complete well within deadline
    assert!(
        add_elapsed < Duration::from_secs(5),
        "add took {:?}, expected < 5s",
        add_elapsed
    );
    assert!(
        remove_elapsed < Duration::from_secs(5),
        "remove took {:?}, expected < 5s",
        remove_elapsed
    );

    handle.stop();
    handle.join()?;
    Ok(())
}

/// Regression: the daemon must survive many repeated watcher updates without
/// crashing. Before the NoMergePolicy fix, background segment merges caused
/// OOM kills after a few updates on large indexes.
#[test]
fn watch_survives_repeated_updates() -> Result<()> {
    let worktree = create_worktree()?;
    let index_path = index_path(worktree.path());
    let (tx, rx) = bounded(500);
    let handle = watcher::start(
        worktree.path().to_path_buf(),
        index_path.clone(),
        CollieConfig::default(),
        Some(Box::new(move |event| {
            let _ = tx.send(event);
        })),
    )?;

    let file = worktree.path().join("churn.rs");

    // 20 rapid update cycles — each writes, waits for index, overwrites
    for cycle in 0..20 {
        let content = format!("fn cycle_{}() {{}}", cycle);
        fs::write(&file, &content)?;
        wait_for_event(
            &rx,
            Duration::from_secs(5),
            |event| matches!(event, WatchEvent::Indexed { path } if path == &file),
        )
        .with_context(|| format!("watcher died or stalled at cycle {}", cycle))?;
    }

    // After all cycles, only the last version should be searchable.
    // Under full-suite load there can be a brief lag between the final
    // callback delivery and a fresh reader observing the committed state.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let builder = load_builder(&index_path)?;
        if builder.search_pattern("cycle_19").len() == 1
            && builder.search_pattern("cycle_0").is_empty()
            && builder.search_pattern("cycle_10").is_empty()
        {
            break;
        }
        if Instant::now() >= deadline {
            anyhow::bail!("final repeated-update state did not converge within 5s");
        }
        thread::sleep(Duration::from_millis(50));
    }

    handle.stop();
    handle.join()?;
    Ok(())
}

/// Regression: search correctness must hold after many incremental batches
/// with NoMergePolicy. Segments accumulate (one per batch) but all query
/// types must still return correct results across all segments.
#[test]
fn search_correct_after_many_incremental_batches() -> Result<()> {
    let worktree = create_worktree()?;
    let index_path = index_path(worktree.path());
    let (tx, rx) = bounded(500);
    let handle = watcher::start(
        worktree.path().to_path_buf(),
        index_path.clone(),
        CollieConfig::default(),
        Some(Box::new(move |event| {
            let _ = tx.send(event);
        })),
    )?;

    // Create 10 files in separate batches (each triggers a separate commit/segment)
    let mut expected_files = Vec::new();
    for i in 0..10 {
        let name = format!("module_{}.rs", i);
        let content = format!("fn initialize_handler_{i}() {{ let connection_{i} = open(); }}");
        let path = write_file(worktree.path(), &name, &content)?;
        wait_for_event(
            &rx,
            Duration::from_secs(5),
            |event| matches!(event, WatchEvent::Indexed { path: p } if p == &path),
        )
        .with_context(|| format!("stalled indexing file {}", i))?;
        expected_files.push(path);
        // Small delay between files to ensure separate debounce batches
        thread::sleep(Duration::from_millis(400));
    }

    // Verify all query types work across the accumulated segments
    let builder = load_builder(&index_path)?;

    // Exact match: each file's unique token
    for i in 0..10 {
        let pattern = format!("initialize_handler_{}", i);
        assert_eq!(
            builder.search_pattern(&pattern).len(),
            1,
            "exact match failed for {}",
            pattern
        );
    }

    // Prefix match across all segments
    let prefix_results = builder.search_pattern("initialize_%");
    assert_eq!(
        prefix_results.len(),
        10,
        "prefix match should find all 10 handlers"
    );

    // Suffix match across all segments
    let suffix_results = builder.search_pattern("%open");
    assert_eq!(
        suffix_results.len(),
        10,
        "suffix match should find 'open' in all 10 files"
    );

    // Substring match across all segments
    let substring_results = builder.search_pattern("%handler%");
    assert_eq!(
        substring_results.len(),
        10,
        "substring match should find 'handler' in all 10 files"
    );

    handle.stop();
    handle.join()?;
    Ok(())
}
