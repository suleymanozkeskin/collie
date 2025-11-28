use anyhow::{Result, anyhow};
use crossbeam_channel::{Receiver, RecvTimeoutError, unbounded};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::config::CollieConfig;
use crate::indexer::IndexBuilder;
use crate::storage::IndexStats;

pub struct WatchHandle {
    stop_flag: Arc<AtomicBool>,
    processor_thread: Option<JoinHandle<Result<()>>>,
}

pub enum WatchEvent {
    Indexed {
        path: PathBuf,
    },
    Removed {
        path: PathBuf,
    },
    Skipped {
        path: PathBuf,
        reason: String,
    },
    Error {
        path: PathBuf,
        error: String,
    },
    /// Emitted once after each debounce batch is committed.
    BatchSaved {
        stats: IndexStats,
    },
}

#[derive(Clone, Copy)]
enum ActionKind {
    Index,
    Remove,
}

/// Returns true if the file extension is in the indexable set,
/// accounting for extra/exclude extensions from config.
pub fn has_indexable_extension(path: &Path, config: &CollieConfig) -> bool {
    let Some(ext) = path.extension() else {
        return false;
    };
    let ext_lower = ext.to_string_lossy().to_lowercase();

    // Check exclusions first
    if config
        .index
        .exclude_extensions
        .iter()
        .any(|e| e.to_lowercase() == ext_lower)
    {
        return false;
    }

    // PDF: gated by include_pdfs config
    if ext_lower == "pdf" {
        return config.index.include_pdfs;
    }

    // Check extra extensions
    if config
        .index
        .extra_extensions
        .iter()
        .any(|e| e.to_lowercase() == ext_lower)
    {
        return true;
    }

    // Built-in set
    matches!(
        ext_lower.as_str(),
        "rs" | "py"
            | "js"
            | "ts"
            | "tsx"
            | "jsx"
            | "c"
            | "cpp"
            | "h"
            | "hpp"
            | "go"
            | "java"
            | "kt"
            | "rb"
            | "php"
            | "swift"
            | "md"
            | "txt"
            | "toml"
            | "yaml"
            | "yml"
            | "json"
            | "html"
            | "css"
            | "scss"
            | "sass"
            | "sh"
            | "bash"
            | "zsh"
    )
}

/// Build a gitignore matcher that respects .gitignore files (including nested),
/// .git/info/exclude, and .collieignore in the worktree root.
///
/// Discovers nested .gitignore files via a lightweight walk that skips
/// `.git/` internals to avoid scanning git object storage.
///
/// Note: The matcher is built once at startup. Changes to ignore files while
/// the daemon is running require a restart to take effect.
pub fn build_gitignore(worktree_root: &Path) -> Gitignore {
    let mut builder = GitignoreBuilder::new(worktree_root);
    // .git/info/exclude
    let exclude_path = worktree_root.join(".git").join("info").join("exclude");
    if exclude_path.exists() {
        let _ = builder.add(&exclude_path);
    }
    // .collieignore
    let collieignore_path = worktree_root.join(".collieignore");
    if collieignore_path.exists() {
        let _ = builder.add(&collieignore_path);
    }
    // Discover nested .gitignore files. Use WalkBuilder with hidden(false)
    // so .gitignore files are visible, but apply standard ignore rules so
    // we skip node_modules/, target/, etc. Explicitly filter out .git/ internals.
    for entry in ignore::WalkBuilder::new(worktree_root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .filter_entry(|e| {
            // Skip .git directory internals (but allow the root .git entry for git detection)
            if e.depth() > 0 && e.file_name() == ".git" {
                return false;
            }
            true
        })
        .build()
    {
        if let Ok(entry) = entry {
            if entry.file_type().map_or(false, |ft| ft.is_file())
                && entry.file_name() == ".gitignore"
            {
                let _ = builder.add(entry.path());
            }
        }
    }
    builder
        .build()
        .unwrap_or_else(|_| GitignoreBuilder::new(worktree_root).build().unwrap())
}

/// Check if a path should be skipped based on hidden dirs, gitignore, and extension.
fn should_skip(
    path: &Path,
    worktree_root: &Path,
    gitignore: &Gitignore,
    config: &CollieConfig,
) -> bool {
    // Skip hidden components (relative to worktree root)
    if let Ok(relative) = path.strip_prefix(worktree_root) {
        for component in relative.components() {
            let text = component.as_os_str().to_string_lossy();
            if text.starts_with('.') {
                return true;
            }
        }
    }

    // Skip gitignored paths
    if gitignore.matched(path, path.is_dir()).is_ignore() {
        return true;
    }

    // Skip non-indexable extensions
    if !has_indexable_extension(path, config) {
        return true;
    }

    false
}

pub fn start(
    worktree_root: PathBuf,
    index_path: PathBuf,
    config: CollieConfig,
    on_event: Option<Box<dyn Fn(WatchEvent) + Send>>,
) -> Result<WatchHandle> {
    let (tx, rx) = unbounded();
    let mut watcher = RecommendedWatcher::new(
        move |result| {
            let _ = tx.send(result);
        },
        Config::default(),
    )?;
    watcher.watch(&worktree_root, RecursiveMode::Recursive)?;

    let gitignore = build_gitignore(&worktree_root);

    let stop_flag = Arc::new(AtomicBool::new(false));
    let thread_stop_flag = stop_flag.clone();
    let processor_thread = thread::spawn(move || {
        let _watcher = watcher;
        run_processor_loop(
            worktree_root,
            index_path,
            config,
            gitignore,
            rx,
            on_event,
            thread_stop_flag,
        )
    });

    Ok(WatchHandle {
        stop_flag,
        processor_thread: Some(processor_thread),
    })
}

impl WatchHandle {
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }

    pub fn join(mut self) -> Result<()> {
        if let Some(handle) = self.processor_thread.take() {
            handle
                .join()
                .map_err(|_| anyhow!("processor thread panicked"))?
        } else {
            Ok(())
        }
    }
}

fn run_processor_loop(
    worktree_root: PathBuf,
    index_path: PathBuf,
    config: CollieConfig,
    gitignore: Gitignore,
    rx: Receiver<notify::Result<Event>>,
    on_event: Option<Box<dyn Fn(WatchEvent) + Send>>,
    stop_flag: Arc<AtomicBool>,
) -> Result<()> {
    let mut builder = IndexBuilder::new(&index_path, &config)?;
    // Disable segment merging for incremental updates.
    builder.set_no_merge();
    let mut pending: HashMap<PathBuf, ActionKind> = HashMap::new();
    let mut compacted = false;

    loop {
        // Wait for at least one event, then drain everything available.
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(first) => {
                // Process the first event
                match first {
                    Ok(event) => collect_event(&event, &mut pending),
                    Err(err) => {
                        if let Some(on_event) = on_event.as_ref() {
                            on_event(WatchEvent::Error {
                                path: index_path.clone(),
                                error: err.to_string(),
                            });
                        }
                    }
                }
                // Drain all remaining queued events without waiting
                while let Ok(result) = rx.try_recv() {
                    match result {
                        Ok(event) => collect_event(&event, &mut pending),
                        Err(err) => {
                            if let Some(on_event) = on_event.as_ref() {
                                on_event(WatchEvent::Error {
                                    path: index_path.clone(),
                                    error: err.to_string(),
                                });
                            }
                        }
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        // Process all pending events immediately — no debounce delay.
        // Deduplication within the 50ms recv window is still handled by
        // the HashMap (same path → latest action wins).
        let actionable: Vec<(PathBuf, ActionKind)> = pending
            .drain()
            .filter_map(|(path, action)| {
                if should_skip(&path, &worktree_root, &gitignore, &config) {
                    return None;
                }
                match action {
                    ActionKind::Index if !path.is_file() => {
                        // File was touched then deleted within the window.
                        // Convert to Remove so the index stays consistent.
                        Some((path, ActionKind::Remove))
                    }
                    _ => Some((path, action)),
                }
            })
            .collect();

        if !actionable.is_empty() {
            // Mark dirty BEFORE any mutation. If the process dies anywhere
            // between here and the clear below, the next startup will detect
            // ACTIVE_DIRTY and do a full rebuild.
            let dirty_marker = index_path.join("ACTIVE_DIRTY");
            let _ = std::fs::write(&dirty_marker, "");

            let mut processed_any = false;
            let mut post_save_events = Vec::new();
            for (path, action) in actionable {
                match action {
                    ActionKind::Index => match builder.index_file(&path) {
                        Ok(true) => {
                            processed_any = true;
                            post_save_events.push(WatchEvent::Indexed { path: path.clone() });
                        }
                        Ok(false) => {
                            processed_any = true;
                            post_save_events.push(WatchEvent::Skipped {
                                path: path.clone(),
                                reason: "exceeds max_file_size".to_string(),
                            });
                        }
                        Err(err) => {
                            post_save_events.push(WatchEvent::Skipped {
                                path: path.clone(),
                                reason: err.to_string(),
                            });
                        }
                    },
                    ActionKind::Remove => {
                        builder.remove_file(&path);
                        processed_any = true;
                        post_save_events.push(WatchEvent::Removed { path: path.clone() });
                    }
                }
            }

            if processed_any {
                builder.save()?;
            }

            // Mutations are consistent — clear the dirty marker.
            let _ = std::fs::remove_file(&dirty_marker);

            if let Some(on_event) = on_event.as_ref() {
                for event in post_save_events {
                    on_event(event);
                }
                // Emit stats from the watcher's own builder — no new
                // IndexBuilder is opened, so no extra memory pressure.
                if processed_any {
                    on_event(WatchEvent::BatchSaved {
                        stats: builder.stats(),
                    });
                }
            }
        }

        // Background compaction: merge segments once after the first idle
        // period following startup. Improves subsequent search latency
        // without blocking rebuild time-to-ready.
        if !compacted && pending.is_empty() {
            if let Ok(_new_segments) = builder.compact() {
                compacted = true;
            }
        }

        if stop_flag.load(Ordering::Relaxed) {
            break;
        }
    }

    Ok(())
}

fn collect_event(event: &Event, pending: &mut HashMap<PathBuf, ActionKind>) {
    for path in &event.paths {
        if let Some(action) = map_action(&event.kind) {
            // Latest action wins. A Remove followed by an Index (file
            // recreated within the debounce window) should resolve to Index.
            pending.insert(path.clone(), action);
        }
    }
}

fn map_action(kind: &EventKind) -> Option<ActionKind> {
    match kind {
        EventKind::Create(CreateKind::File)
        | EventKind::Create(CreateKind::Any)
        | EventKind::Create(_) => Some(ActionKind::Index),
        EventKind::Modify(ModifyKind::Data(_))
        | EventKind::Modify(ModifyKind::Any)
        | EventKind::Modify(ModifyKind::Name(RenameMode::To)) => Some(ActionKind::Index),
        EventKind::Modify(ModifyKind::Name(RenameMode::From))
        | EventKind::Modify(ModifyKind::Name(_))
        | EventKind::Remove(RemoveKind::File)
        | EventKind::Remove(RemoveKind::Any)
        | EventKind::Remove(_) => Some(ActionKind::Remove),
        _ => None,
    }
}
