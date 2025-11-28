use anyhow::{Context, Result, anyhow};
use fnv::FnvHasher;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::hash::Hasher;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::symbols::Symbol;

use crate::config::CollieConfig;
use crate::indexer::IndexBuilder;
use crate::storage::IndexStats;
use crate::storage::generation::GenerationManager;
use crate::watcher::{self, WatchEvent};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonState {
    pub worktree_root: PathBuf,
    pub index_path: PathBuf,
    pub pid: u32,
    pub status: DaemonStatus,
    pub started_at_unix_ms: u64,
    pub last_event_at_unix_ms: Option<u64>,
    pub last_save_at_unix_ms: Option<u64>,
    pub total_files: usize,
    pub total_terms: usize,
    pub total_postings: usize,
    pub trigram_entries: usize,
    #[serde(default)]
    pub segment_count: usize,
    #[serde(default)]
    pub initial_segment_count: usize,
    #[serde(default)]
    pub generation: Option<String>,
    #[serde(default)]
    pub needs_rebuild: bool,
    #[serde(default)]
    pub compaction_recommended: bool,
    pub last_error: Option<String>,
    #[serde(default)]
    pub skipped_files: usize,
    #[serde(default)]
    pub skipped_samples: Vec<SkippedFile>,
}

/// Maximum number of skipped file samples to keep in state.
const MAX_SKIPPED_SAMPLES: usize = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedFile {
    pub path: String,
    pub reason: String,
    pub kind: SkipKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SkipKind {
    /// Permission denied, file not found, I/O error
    ReadError,
    /// File exceeds max_file_size
    SizeLimit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DaemonStatus {
    Starting,
    Running,
    Stopped,
    Error,
}

#[derive(Debug, Clone)]
struct DaemonPaths {
    root: PathBuf,
    collie_dir: PathBuf,
    index_path: PathBuf,
    pid_path: PathBuf,
    state_path: PathBuf,
    log_path: PathBuf,
}

pub fn start(path: PathBuf, foreground: bool, restart_on_crash: bool) -> Result<()> {
    let root = resolve_worktree_root(path)?;
    let paths = DaemonPaths::for_root(root);
    let first_run = !paths.collie_dir.exists();

    // If .collie/ exists but wasn't created by us, don't overwrite it
    if !first_run && !is_collie_dir(&paths.collie_dir) {
        anyhow::bail!(
            "{} exists but does not appear to be a collie index directory. \
             Remove it manually or choose a different repository path.",
            paths.collie_dir.display()
        );
    }

    fs::create_dir_all(&paths.collie_dir)?;

    if first_run {
        print_gitignore_tip(&paths.root);
    }

    if let Some(pid) = read_pid_if_alive(&paths.pid_path) {
        println!("Collie daemon already running for {}", paths.root.display());
        if !paths.state_path.exists() {
            let state = DaemonState::new_running(&paths, pid, None, None);
            write_state(&paths.state_path, &state)?;
        }
        return Ok(());
    }

    cleanup_stale_files(&paths)?;

    if foreground {
        return run_daemon(paths);
    }

    let mut daemon_child = spawn_daemon_child(&paths)?;

    if restart_on_crash {
        let stop_flag = Arc::new(AtomicBool::new(false));
        signal_hook::flag::register(signal_hook::consts::SIGTERM, stop_flag.clone())?;
        signal_hook::flag::register(signal_hook::consts::SIGINT, stop_flag.clone())?;

        loop {
            thread::sleep(Duration::from_secs(2));
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }
            // try_wait reaps zombies so kill(pid,0) works correctly
            let exited = daemon_child
                .try_wait()
                .map(|status| status.is_some())
                .unwrap_or(false);
            if exited {
                // Check if the daemon exited intentionally (via `collie stop`)
                let was_intentional = paths.state_path.exists()
                    && read_state(&paths.state_path)
                        .map(|s| s.status == DaemonStatus::Stopped)
                        .unwrap_or(false);
                if was_intentional {
                    break;
                }
                println!("Collie daemon crashed, restarting...");
                cleanup_stale_files(&paths)?;
                daemon_child = spawn_daemon_child(&paths)?;
            }
        }

        // Supervisor is shutting down — stop the daemon child so it doesn't orphan.
        if let Some(pid) = read_pid(&paths.pid_path)? {
            if is_pid_alive(pid) {
                let _ = send_sigterm(pid);
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                while std::time::Instant::now() < deadline && is_pid_alive(pid) {
                    thread::sleep(Duration::from_millis(50));
                }
                // Force-kill if it didn't stop gracefully
                if is_pid_alive(pid) {
                    let _ = daemon_child.kill();
                    let _ = daemon_child.wait();
                }
            }
        }
    }

    Ok(())
}

fn spawn_daemon_child(paths: &DaemonPaths) -> Result<std::process::Child> {
    let exe = std::env::current_exe().context("failed to locate current executable")?;
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.log_path)
        .with_context(|| format!("failed to open daemon log at {:?}", paths.log_path))?;
    let log_file_err = log_file.try_clone()?;

    let mut child = Command::new(exe)
        .arg("__daemon")
        .arg(&paths.root)
        .current_dir(&paths.root)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_file_err))
        .spawn()
        .context("failed to start daemon process")?;

    wait_for_ready(paths, Some(&mut child))?;
    Ok(child)
}

pub fn stop(path: PathBuf) -> Result<()> {
    let root = resolve_worktree_root(path)?;
    let paths = DaemonPaths::for_root(root);

    let Some(pid) = read_pid(&paths.pid_path)? else {
        println!("Collie daemon is not running for {}", paths.root.display());
        return Ok(());
    };

    if !is_pid_alive(pid) {
        cleanup_stale_files(&paths)?;
        println!("Collie daemon is not running for {}", paths.root.display());
        return Ok(());
    }

    send_sigterm(pid)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        let pid_missing = !paths.pid_path.exists();
        let stopped_state = paths.state_path.exists()
            && read_state(&paths.state_path)
                .map(|state| state.status == DaemonStatus::Stopped)
                .unwrap_or(false);

        if !is_pid_alive(pid) || pid_missing || stopped_state {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    let pid_missing = !paths.pid_path.exists();
    let stopped_state = paths.state_path.exists()
        && read_state(&paths.state_path)
            .map(|state| state.status == DaemonStatus::Stopped)
            .unwrap_or(false);

    if is_pid_alive(pid) && !pid_missing && !stopped_state {
        return Err(anyhow!("timed out waiting for daemon {} to stop", pid));
    }

    if paths.pid_path.exists() {
        let _ = fs::remove_file(&paths.pid_path);
    }

    if paths.state_path.exists() {
        let mut state = read_state(&paths.state_path).unwrap_or_else(|_| {
            DaemonState::new_stopped(&paths, Some(pid), "pid is not alive".to_string())
        });
        state.status = DaemonStatus::Stopped;
        state.last_error = None;
        write_state(&paths.state_path, &state)?;
    }

    println!("Stopped Collie daemon for {}", paths.root.display());
    Ok(())
}

pub fn status(path: PathBuf, json: bool) -> Result<()> {
    let root = resolve_worktree_root(path)?;
    let paths = DaemonPaths::for_root(root);
    let pid = read_pid(&paths.pid_path)?;

    if let Some(pid) = pid {
        if is_pid_alive(pid) {
            let state = read_state(&paths.state_path)
                .unwrap_or_else(|_| DaemonState::new_running(&paths, pid, None, None));
            if json {
                println!("{}", serde_json::to_string_pretty(&state)?);
            } else {
                print_running_status(&paths, &state);
            }
            return Ok(());
        }
    }

    let reason = match pid {
        Some(pid_val) => {
            let state_says_running = paths.state_path.exists()
                && read_state(&paths.state_path)
                    .map(|s| {
                        s.status == DaemonStatus::Running || s.status == DaemonStatus::Starting
                    })
                    .unwrap_or(false);
            if state_says_running {
                format!("daemon crashed (pid {} is no longer alive)", pid_val)
            } else {
                "pid is not alive".to_string()
            }
        }
        None => "pid file missing".to_string(),
    };

    let gen_mgr = GenerationManager::new(&paths.collie_dir);
    let gen_name = gen_mgr
        .active_generation()
        .ok()
        .flatten()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()));
    let rebuild_required = gen_mgr.needs_rebuild();

    if json {
        let stopped = serde_json::json!({
            "status": "stopped",
            "worktree_root": paths.root,
            "index_path": paths.index_path,
            "pid": pid.unwrap_or(0),
            "reason": reason,
            "generation": gen_name,
            "needs_rebuild": rebuild_required,
        });
        println!("{}", serde_json::to_string_pretty(&stopped)?);
    } else {
        let pid_text = pid
            .map(|value| value.to_string())
            .unwrap_or_else(|| "missing".to_string());
        println!("Collie daemon status: stopped");
        println!("Worktree root: {}", paths.root.display());
        println!("Index path: {}", paths.index_path.display());
        println!("PID: {}", pid_text);
        println!("Reason: {}", reason);
        if let Some(ref name) = gen_name {
            println!("Generation: {}", name);
        }
        if rebuild_required {
            println!("Rebuild: required");
        }
    }
    Ok(())
}

/// Result of a rebuild operation.
pub struct RebuildResult {
    pub stats: IndexStats,
    pub skipped_files: usize,
    pub generation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RepoSnapshot {
    indexable_files: usize,
    signature_hex: String,
}

/// Rebuild the index for a worktree. Stops the running daemon if any,
/// creates a new generation, indexes all files, activates the generation,
/// and cleans up old generations. Does not start a watcher.
pub fn rebuild(path: PathBuf) -> Result<RebuildResult> {
    let root = resolve_worktree_root(path)?;
    let paths = DaemonPaths::for_root(root);

    if paths.collie_dir.exists() && !is_collie_dir(&paths.collie_dir) {
        anyhow::bail!(
            "{} exists but does not appear to be a collie index directory",
            paths.collie_dir.display()
        );
    }

    fs::create_dir_all(&paths.collie_dir)?;

    // Stop running daemon if any
    if let Some(pid) = read_pid_if_alive(&paths.pid_path) {
        send_sigterm(pid)?;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline && is_pid_alive(pid) {
            thread::sleep(Duration::from_millis(50));
        }
        if paths.pid_path.exists() {
            let _ = fs::remove_file(&paths.pid_path);
        }
    }

    let config = CollieConfig::load(&paths.root);
    let gen_mgr = GenerationManager::new(&paths.collie_dir);
    let gen_dir = gen_mgr.create_generation()?;

    let mut builder = IndexBuilder::new(&gen_dir, &config)?;
    let (skips, snapshot) = bulk_rebuild(&paths.root, &mut builder, &config)?;
    builder.save()?;
    let stats = builder.stats();
    drop(builder);
    write_repo_snapshot(&gen_dir, &snapshot)?;

    gen_mgr.activate(&gen_dir)?;
    gen_mgr.cleanup_inactive()?;

    let generation = gen_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    Ok(RebuildResult {
        stats,
        skipped_files: skips.count,
        generation,
    })
}

/// Returns true if a collie daemon is alive for the given worktree root.
pub fn is_daemon_alive(worktree_root: &Path) -> bool {
    let pid_path = worktree_root.join(".collie").join("collie.pid");
    read_pid_if_alive(&pid_path).is_some()
}

/// Touch the activity marker so the daemon knows a client is active.
/// Called by `collie search` on every query.
/// Check if a .collie/ directory was created by collie (has our marker files).
/// An empty directory is considered ours (just created by create_dir_all).
fn is_collie_dir(dir: &Path) -> bool {
    if !dir.exists() {
        return false;
    }
    // Empty dir is fine — we just created it
    if fs::read_dir(dir).map_or(true, |mut d| d.next().is_none()) {
        return true;
    }
    // Must have at least one of our known files
    dir.join("CURRENT").exists()
        || dir.join("daemon-state.json").exists()
        || dir.join("generations").is_dir()
        || dir.join("collie.pid").exists()
        || dir.join("daemon.log").exists()
        || dir.join("config.toml").exists()
}

/// Print a one-time tip about adding .collie to global gitignore.
fn print_gitignore_tip(worktree_root: &Path) {
    // Only suggest if this is a git repo
    if !worktree_root.join(".git").exists() {
        return;
    }
    // Check if .collie is already ignored
    let output = std::process::Command::new("git")
        .args(["check-ignore", "-q", ".collie"])
        .current_dir(worktree_root)
        .output();
    if let Ok(out) = output {
        if out.status.success() {
            return; // already ignored
        }
    }
    eprintln!("hint: add .collie to your global gitignore to avoid committing index files:");
    eprintln!("  echo .collie >> ~/.config/git/ignore");
    eprintln!();
}

/// Remove the .collie directory for a repository.
pub fn clean(path: PathBuf) -> Result<()> {
    let root = resolve_worktree_root(path)?;
    let collie_dir = root.join(".collie");
    if !collie_dir.exists() {
        println!("No collie data found for {}", root.display());
        return Ok(());
    }

    if !is_collie_dir(&collie_dir) {
        anyhow::bail!(
            "{} exists but does not appear to be a collie index directory",
            collie_dir.display()
        );
    }

    // Stop the daemon first if running
    if let Some(pid) = read_pid_if_alive(&collie_dir.join("collie.pid")) {
        let _ = send_sigterm(pid);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline && is_pid_alive(pid) {
            thread::sleep(Duration::from_millis(50));
        }
    }

    let size = dir_size(&collie_dir);
    fs::remove_dir_all(&collie_dir)
        .with_context(|| format!("failed to remove {:?}", collie_dir))?;
    println!(
        "Removed {} ({:.1} MB)",
        collie_dir.display(),
        size as f64 / (1024.0 * 1024.0)
    );
    Ok(())
}

pub fn touch_activity(worktree_root: &Path) {
    let marker = worktree_root.join(".collie").join("last_activity");
    let _ = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&marker)
        .and_then(|f| f.set_len(0));
}

/// Seconds since the activity marker was last touched, or None if missing.
fn secs_since_last_activity(collie_dir: &Path) -> Option<u64> {
    let marker = collie_dir.join("last_activity");
    let meta = fs::metadata(&marker).ok()?;
    let modified = meta.modified().ok()?;
    SystemTime::now().duration_since(modified).ok().map(|d| d.as_secs())
}

pub fn run_internal_daemon(path: PathBuf) -> Result<()> {
    let root = resolve_worktree_root(path)?;
    let paths = DaemonPaths::for_root(root);
    run_daemon(paths)
}

pub fn resolve_worktree_root<P: AsRef<Path>>(path: P) -> Result<PathBuf> {
    let canonical = fs::canonicalize(path.as_ref())
        .with_context(|| format!("failed to canonicalize {:?}", path.as_ref()))?;
    let mut current = if canonical.is_file() {
        canonical
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| anyhow!("file path has no parent"))?
    } else {
        canonical
    };

    loop {
        let git_entry = current.join(".git");
        if git_entry.exists() {
            return Ok(current);
        }
        if !current.pop() {
            break;
        }
    }

    let fallback = fs::canonicalize(path.as_ref())?;
    if fallback.is_file() {
        Ok(fallback
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| anyhow!("file path has no parent"))?)
    } else {
        Ok(fallback)
    }
}

fn run_daemon(paths: DaemonPaths) -> Result<()> {
    fs::create_dir_all(&paths.collie_dir)?;
    let pid = std::process::id();
    let config = CollieConfig::load(&paths.root);

    fs::write(&paths.pid_path, pid.to_string())?;
    write_state(&paths.state_path, &DaemonState::new_starting(&paths, pid))?;

    let gen_mgr = GenerationManager::new(&paths.collie_dir);
    let reusable_gen = reusable_active_generation(&gen_mgr, &paths.root, &config)?;
    let (active_gen, skips, stats) = if let Some(active_gen) = reusable_gen {
        let builder = IndexBuilder::new(&active_gen, &config)?;
        let stats = builder.stats();
        (
            active_gen,
            RebuildSkips {
                count: 0,
                samples: Vec::new(),
            },
            stats,
        )
    } else {
        // Build index into a new generation, then atomically activate it.
        let gen_dir = gen_mgr.create_generation()?;

        // Scoped so the builder (and its Tantivy writer lock) is dropped before the watcher starts.
        let (skips, stats) = {
            let mut builder = IndexBuilder::new(&gen_dir, &config)?;
            // Writer heap is set dynamically in bulk_rebuild_parallel based on file count
            let (skips, snapshot) = bulk_rebuild(&paths.root, &mut builder, &config)?;
            builder.save()?;
            let stats = builder.stats();
            write_repo_snapshot(&gen_dir, &snapshot)?;
            (skips, stats)
        };

        gen_mgr.activate(&gen_dir)?;
        gen_mgr.cleanup_inactive()?;
        (gen_dir, skips, stats)
    };

    let mut running_state = DaemonState::new_running(&paths, pid, Some(now_unix_ms()), None);
    running_state.total_files = stats.total_files;
    running_state.total_terms = stats.total_terms;
    running_state.total_postings = stats.total_postings;
    running_state.trigram_entries = stats.trigram_entries;
    running_state.segment_count = stats.segment_count;
    running_state.initial_segment_count = stats.segment_count;
    running_state.generation = active_gen
        .file_name()
        .map(|n| n.to_string_lossy().to_string());
    running_state.needs_rebuild = false;
    running_state.compaction_recommended = false;
    running_state.skipped_files = skips.count;
    running_state.skipped_samples = skips.samples;
    write_state(&paths.state_path, &running_state)?;

    let state_path = paths.state_path.clone();
    let activity_root = paths.root.clone();

    // Keep daemon state in memory; only flush to disk on BatchSaved or Error.
    // This avoids N+1 read/write cycles per debounce batch.
    let in_memory_state = std::sync::Mutex::new(running_state.clone());

    let watch_handle = watcher::start(
        paths.root.clone(),
        active_gen,
        config.clone(),
        Some(Box::new(move |event| {
            touch_activity(&activity_root);
            let mut state = in_memory_state.lock().unwrap();
            state.last_event_at_unix_ms = Some(now_unix_ms());
            let flush = match event {
                WatchEvent::Error { error, .. } => {
                    state.status = DaemonStatus::Error;
                    state.last_error = Some(error);
                    true
                }
                WatchEvent::Skipped { path, reason } => {
                    state.skipped_files += 1;
                    if state.skipped_samples.len() < MAX_SKIPPED_SAMPLES {
                        let kind = if reason.contains("max_file_size") {
                            SkipKind::SizeLimit
                        } else {
                            SkipKind::ReadError
                        };
                        state.skipped_samples.push(SkippedFile {
                            path: path.display().to_string(),
                            reason,
                            kind,
                        });
                    }
                    state.status = DaemonStatus::Running;
                    false
                }
                WatchEvent::BatchSaved { stats } => {
                    state.total_files = stats.total_files;
                    state.total_terms = stats.total_terms;
                    state.total_postings = stats.total_postings;
                    state.trigram_entries = stats.trigram_entries;
                    state.segment_count = stats.segment_count;
                    let baseline = state.initial_segment_count.max(1);
                    state.compaction_recommended = stats.segment_count > baseline * 3;
                    state.last_save_at_unix_ms = Some(now_unix_ms());
                    state.status = DaemonStatus::Running;
                    state.last_error = None;
                    true
                }
                _ => {
                    state.status = DaemonStatus::Running;
                    state.last_error = None;
                    false
                }
            };
            if flush {
                let _ = write_state(&state_path, &state);
            }
        })),
    )?;

    // Touch activity marker so idle clock starts from now
    touch_activity(&paths.root);

    let idle_timeout_secs = config.watcher.idle_timeout_secs;

    let stop_flag = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, stop_flag.clone())?;
    signal_hook::flag::register(signal_hook::consts::SIGINT, stop_flag.clone())?;

    let stop_reason;
    loop {
        if stop_flag.load(Ordering::Relaxed) {
            stop_reason = "signal";
            break;
        }

        // Check idle timeout (0 = disabled)
        if idle_timeout_secs > 0 {
            if let Some(idle_secs) = secs_since_last_activity(&paths.collie_dir) {
                if idle_secs >= idle_timeout_secs {
                    stop_reason = "idle timeout";
                    break;
                }
            }
        }

        thread::sleep(Duration::from_millis(100));
    }

    watch_handle.stop();
    watch_handle.join()?;

    let mut stopped_state = read_state(&paths.state_path)
        .unwrap_or_else(|_| DaemonState::new_stopped(&paths, Some(pid), stop_reason.to_string()));
    stopped_state.status = DaemonStatus::Stopped;
    stopped_state.last_error = Some(stop_reason.to_string());
    write_state(&paths.state_path, &stopped_state)?;
    let _ = fs::remove_file(&paths.pid_path);
    Ok(())
}

struct RebuildSkips {
    count: usize,
    samples: Vec<SkippedFile>,
}

fn reusable_active_generation(
    gen_mgr: &GenerationManager,
    root: &Path,
    config: &CollieConfig,
) -> Result<Option<PathBuf>> {
    if gen_mgr.needs_rebuild() {
        return Ok(None);
    }

    let Some(active_gen) = gen_mgr.active_generation()? else {
        return Ok(None);
    };
    let Some(stored_snapshot) = read_repo_snapshot(&active_gen)? else {
        return Ok(None);
    };
    let current_snapshot = compute_repo_snapshot(root, config)?;
    if current_snapshot == stored_snapshot {
        Ok(Some(active_gen))
    } else {
        Ok(None)
    }
}

fn snapshot_path(gen_dir: &Path) -> PathBuf {
    gen_dir.join("repo-snapshot.json")
}

fn write_repo_snapshot(gen_dir: &Path, snapshot: &RepoSnapshot) -> Result<()> {
    let path = snapshot_path(gen_dir);
    let tmp_path = path.with_extension("json.tmp");
    let mut file = File::create(&tmp_path)?;
    file.write_all(serde_json::to_string_pretty(snapshot)?.as_bytes())?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp_path, &path)?;
    Ok(())
}

fn read_repo_snapshot(gen_dir: &Path) -> Result<Option<RepoSnapshot>> {
    let path = snapshot_path(gen_dir);
    if !path.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)?;
    Ok(Some(serde_json::from_str(&raw)?))
}

fn compute_repo_snapshot(root: &Path, config: &CollieConfig) -> Result<RepoSnapshot> {
    let mut walk = ignore::WalkBuilder::new(root);
    walk.hidden(true);
    walk.git_ignore(true);
    walk.git_global(true);
    walk.git_exclude(true);
    let collieignore = root.join(".collieignore");
    if collieignore.exists() {
        walk.add_ignore(&collieignore);
    }

    let mut entries = Vec::new();
    for entry in walk.build() {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && watcher::has_indexable_extension(path, config) {
            let metadata = fs::metadata(path)?;
            let modified_ns = metadata
                .modified()
                .ok()
                .and_then(|ts| ts.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or_default();
            let rel = path.strip_prefix(root).unwrap_or(path);
            entries.push((
                rel.to_string_lossy().to_string(),
                metadata.len(),
                modified_ns,
            ));
        }
    }

    entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = FnvHasher::default();
    hash_config(&mut hasher, config);
    for (path, size, modified_ns) in &entries {
        hasher.write(path.as_bytes());
        hasher.write_u8(0xff);
        hasher.write_u64(*size);
        hasher.write_u8(0xfe);
        hasher.write_u128(*modified_ns);
        hasher.write_u8(0xfd);
    }

    Ok(RepoSnapshot {
        indexable_files: entries.len(),
        signature_hex: format!("{:016x}", hasher.finish()),
    })
}

fn hash_config(hasher: &mut FnvHasher, config: &CollieConfig) {
    hasher.write_u64(config.index.max_file_size);
    for ext in &config.index.extra_extensions {
        hasher.write(ext.as_bytes());
        hasher.write_u8(0xfc);
    }
    hasher.write_u8(0xfb);
    for ext in &config.index.exclude_extensions {
        hasher.write(ext.as_bytes());
        hasher.write_u8(0xfa);
    }
}

/// Payload sent from producer threads to the writer thread.
enum IndexPayload {
    /// File preprocessed successfully: ready for tantivy ingestion.
    Ready {
        path: PathBuf,
        symbols: Vec<Symbol>,
        body_tokens: tantivy::tokenizer::PreTokenizedString,
        body_reversed_tokens: tantivy::tokenizer::PreTokenizedString,
        file_size: u64,
        modified_ns: u128,
    },
    /// File skipped due to size limit.
    SizeLimit(PathBuf),
    /// File could not be read.
    ReadError(PathBuf, String),
}

/// Bounded channel capacity. Limits memory to at most this many file
/// contents buffered between producers and the writer.
const REBUILD_CHANNEL_CAPACITY: usize = 16;

/// Minimum file count to justify the streaming pipeline overhead.
/// Below this threshold, sequential rebuild is used.
const PARALLEL_REBUILD_THRESHOLD: usize = 100;

fn bulk_rebuild(
    root: &Path,
    builder: &mut IndexBuilder,
    config: &CollieConfig,
) -> Result<(RebuildSkips, RepoSnapshot)> {
    let mut skips = RebuildSkips {
        count: 0,
        samples: Vec::new(),
    };

    // --- Phase 1: Walk and collect candidate paths ---
    let mut walk = ignore::WalkBuilder::new(root);
    walk.hidden(true);
    walk.git_ignore(true);
    walk.git_global(true);
    walk.git_exclude(true);
    let collieignore = root.join(".collieignore");
    if collieignore.exists() {
        walk.add_ignore(&collieignore);
    }

    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in walk.build() {
        match entry {
            Ok(e) => {
                let path = e.path();
                if path.is_file() && watcher::has_indexable_extension(path, config) {
                    paths.push(path.to_path_buf());
                }
            }
            Err(err) => {
                let reason = err.to_string();
                eprintln!("warning: skipping entry during rebuild: {}", reason);
                skips.count += 1;
                if skips.samples.len() < MAX_SKIPPED_SAMPLES {
                    skips.samples.push(SkippedFile {
                        path: String::new(),
                        reason,
                        kind: SkipKind::ReadError,
                    });
                }
            }
        }
    }

    // --- Phase 2: Index files and collect snapshot entries ---
    let mut snapshot_entries: Vec<(String, u64, u128)> = Vec::new();

    if paths.len() >= PARALLEL_REBUILD_THRESHOLD {
        bulk_rebuild_parallel(root, builder, config, &paths, &mut skips, &mut snapshot_entries)?;
    } else {
        bulk_rebuild_sequential(root, builder, config, &paths, &mut skips, &mut snapshot_entries)?;
    }

    // --- Phase 3: Compute snapshot from collected entries ---
    snapshot_entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = FnvHasher::default();
    hash_config(&mut hasher, config);
    for (path, size, modified_ns) in &snapshot_entries {
        hasher.write(path.as_bytes());
        hasher.write_u8(0xff);
        hasher.write_u64(*size);
        hasher.write_u8(0xfe);
        hasher.write_u128(*modified_ns);
        hasher.write_u8(0xfd);
    }

    let snapshot = RepoSnapshot {
        indexable_files: snapshot_entries.len(),
        signature_hex: format!("{:016x}", hasher.finish()),
    };

    Ok((skips, snapshot))
}

/// Sequential rebuild for small repos. No threading overhead.
fn bulk_rebuild_sequential(
    root: &Path,
    builder: &mut IndexBuilder,
    _config: &CollieConfig,
    paths: &[PathBuf],
    skips: &mut RebuildSkips,
    snapshot_entries: &mut Vec<(String, u64, u128)>,
) -> Result<()> {
    for path in paths {
        // Collect snapshot data before indexing
        if let Ok(metadata) = fs::metadata(path) {
            let modified_ns = metadata
                .modified()
                .ok()
                .and_then(|ts| ts.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or_default();
            let rel = path.strip_prefix(root).unwrap_or(path);
            snapshot_entries.push((
                rel.to_string_lossy().to_string(),
                metadata.len(),
                modified_ns,
            ));
        }

        match builder.index_file(path) {
            Ok(true) => {}
            Ok(false) => {
                skips.count += 1;
                if skips.samples.len() < MAX_SKIPPED_SAMPLES {
                    skips.samples.push(SkippedFile {
                        path: path.display().to_string(),
                        reason: "exceeds max_file_size".to_string(),
                        kind: SkipKind::SizeLimit,
                    });
                }
            }
            Err(err) => {
                let reason = err.to_string();
                eprintln!("warning: skipping {}: {}", path.display(), reason);
                skips.count += 1;
                if skips.samples.len() < MAX_SKIPPED_SAMPLES {
                    skips.samples.push(SkippedFile {
                        path: path.display().to_string(),
                        reason,
                        kind: SkipKind::ReadError,
                    });
                }
            }
        }
    }
    Ok(())
}

/// Streaming parallel rebuild for larger repos.
/// Rayon workers preprocess files (read + symbol extraction) and send
/// payloads through a bounded channel to a single tantivy writer.
fn bulk_rebuild_parallel(
    root: &Path,
    builder: &mut IndexBuilder,
    config: &CollieConfig,
    paths: &[PathBuf],
    skips: &mut RebuildSkips,
    snapshot_entries: &mut Vec<(String, u64, u128)>,
) -> Result<()> {
    // Scale writer heap with file count: ~8KB per file, clamped to 15MB..400MB.
    // Larger heaps reduce segment flush frequency during bulk ingest.
    let heap_bytes = (paths.len() * 8_192).clamp(15_000_000, 400_000_000);
    builder.set_writer_heap(heap_bytes);
    // Disable merging during bulk ingest — background compaction
    // consolidates segments after the daemon is ready.
    builder.set_no_merge();
    let (tx, rx) = crossbeam_channel::bounded::<IndexPayload>(REBUILD_CHANNEL_CAPACITY);
    let max_file_size = config.index.max_file_size;
    let worktree_root = root.to_path_buf();
    let owned_paths: Vec<PathBuf> = paths.to_vec();

    let producer = std::thread::spawn(move || {
        use rayon::prelude::*;
        let adapter_registry = crate::symbols::adapters::AdapterRegistry::default();

        owned_paths.par_iter().for_each(|path| {
            let payload = preprocess_file(
                path,
                &worktree_root,
                max_file_size,
                &adapter_registry,
            );
            let _ = tx.send(payload);
        });
    });

    for payload in rx {
        match payload {
            IndexPayload::Ready { path, symbols, body_tokens, body_reversed_tokens, file_size, modified_ns } => {
                let rel = path.strip_prefix(root).unwrap_or(&path);
                snapshot_entries.push((
                    rel.to_string_lossy().to_string(),
                    file_size,
                    modified_ns,
                ));

                if let Err(err) = builder.index_pretokenized(&path, body_tokens, body_reversed_tokens, &symbols) {
                    let reason = err.to_string();
                    eprintln!("warning: skipping {}: {}", path.display(), reason);
                    skips.count += 1;
                    if skips.samples.len() < MAX_SKIPPED_SAMPLES {
                        skips.samples.push(SkippedFile {
                            path: path.display().to_string(),
                            reason,
                            kind: SkipKind::ReadError,
                        });
                    }
                }
            }
            IndexPayload::SizeLimit(path) => {
                builder.remove_file(&path);
                skips.count += 1;
                if skips.samples.len() < MAX_SKIPPED_SAMPLES {
                    skips.samples.push(SkippedFile {
                        path: path.display().to_string(),
                        reason: "exceeds max_file_size".to_string(),
                        kind: SkipKind::SizeLimit,
                    });
                }
            }
            IndexPayload::ReadError(path, reason) => {
                eprintln!("warning: skipping {}: {}", path.display(), reason);
                skips.count += 1;
                if skips.samples.len() < MAX_SKIPPED_SAMPLES {
                    skips.samples.push(SkippedFile {
                        path: path.display().to_string(),
                        reason,
                        kind: SkipKind::ReadError,
                    });
                }
            }
        }
    }

    producer.join().expect("producer thread panicked");
    Ok(())
}

/// Preprocess a single file: read, check size, extract symbols.
/// Pure function — no tantivy writes.
fn preprocess_file(
    path: &Path,
    worktree_root: &Path,
    max_file_size: u64,
    adapter_registry: &crate::symbols::adapters::AdapterRegistry,
) -> IndexPayload {
    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(err) => return IndexPayload::ReadError(path.to_path_buf(), err.to_string()),
    };
    if metadata.len() > max_file_size {
        return IndexPayload::SizeLimit(path.to_path_buf());
    }

    let file_size = metadata.len();
    let modified_ns = metadata
        .modified()
        .ok()
        .and_then(|ts| ts.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or_default();

    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(err) => return IndexPayload::ReadError(path.to_path_buf(), err.to_string()),
    };
    let content = String::from_utf8(bytes)
        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());

    let symbols = if let Some(adapter) = adapter_registry.adapter_for_path(path) {
        let repo_rel = path.strip_prefix(worktree_root).unwrap_or(path).to_path_buf();
        // Get or create a thread-local parser for this language
        thread_local! {
            static PARSERS: std::cell::RefCell<std::collections::HashMap<String, tree_sitter::Parser>> =
                std::cell::RefCell::new(std::collections::HashMap::new());
        }
        PARSERS.with(|parsers| {
            let mut parsers = parsers.borrow_mut();
            let lang_id = adapter.language_id().to_string();
            let parser = parsers.entry(lang_id).or_insert_with(|| {
                adapter_registry.create_parser_for(adapter).expect("parser creation")
            });
            adapter.extract_symbols_with_parser(&repo_rel, &content, parser)
        })
    } else {
        Vec::new()
    };

    // Pre-tokenize body fields — moves tokenization from the serial writer
    // thread to the parallel rayon pool.
    let body_tokens = crate::indexer::tokenizer::pretokenize_body(&content);
    let body_reversed_tokens = crate::indexer::tokenizer::pretokenize_body_reversed(&content);

    IndexPayload::Ready {
        path: path.to_path_buf(),
        symbols,
        body_tokens,
        body_reversed_tokens,
        file_size,
        modified_ns,
    }
}

fn wait_for_ready(paths: &DaemonPaths, mut child: Option<&mut std::process::Child>) -> Result<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        if paths.state_path.exists() {
            let state = read_state(&paths.state_path)?;
            match state.status {
                DaemonStatus::Running => return Ok(()),
                DaemonStatus::Error => {
                    return Err(anyhow!(
                        "{}",
                        state
                            .last_error
                            .unwrap_or_else(|| "daemon entered error state".to_string())
                    ));
                }
                DaemonStatus::Starting | DaemonStatus::Stopped => {}
            }
        }

        if let Some(child_ref) = child.as_deref_mut() {
            if let Some(status) = child_ref.try_wait()? {
                return Err(anyhow!("daemon exited before becoming ready: {}", status));
            }
        }

        thread::sleep(Duration::from_millis(50));
    }

    Err(anyhow!("timed out waiting for daemon readiness"))
}

fn read_pid(path: &Path) -> Result<Option<u32>> {
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(path)?;
    let pid = raw.trim().parse::<u32>().context("invalid pid file")?;
    Ok(Some(pid))
}

fn read_pid_if_alive(path: &Path) -> Option<u32> {
    read_pid(path)
        .ok()
        .flatten()
        .filter(|pid| is_pid_alive(*pid))
}

fn cleanup_stale_files(paths: &DaemonPaths) -> Result<()> {
    if let Some(pid) = read_pid(&paths.pid_path)? {
        if is_pid_alive(pid) {
            return Ok(());
        }
    }
    if paths.pid_path.exists() {
        let _ = fs::remove_file(&paths.pid_path);
    }
    if paths.state_path.exists() {
        let _ = fs::remove_file(&paths.state_path);
    }
    Ok(())
}

fn is_pid_alive(pid: u32) -> bool {
    let result = unsafe { libc::kill(pid as i32, 0) };
    if result == 0 {
        return true;
    }
    // EPERM means the process exists but we lack permission to signal it
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn send_sigterm(pid: u32) -> Result<()> {
    let result = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    if result == 0 {
        Ok(())
    } else {
        Err(anyhow!("failed to send SIGTERM to pid {}", pid))
    }
}

fn read_state(path: &Path) -> Result<DaemonState> {
    let raw = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

fn write_state(path: &Path, state: &DaemonState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension("json.tmp");
    let mut file = File::create(&tmp_path)?;
    file.write_all(serde_json::to_string_pretty(state)?.as_bytes())?;
    file.sync_all()?;
    drop(file);
    if let Err(e) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e.into());
    }
    Ok(())
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis() as u64
}

fn format_duration(millis: u64) -> String {
    let total_secs = millis / 1000;
    let days = total_secs / 86400;
    let hours = (total_secs % 86400) / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;

    if days > 0 {
        format!("{}d {}h", days, hours)
    } else if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else if mins > 0 {
        format!("{}m {}s", mins, secs)
    } else {
        format!("{}s", secs)
    }
}

fn dir_size(path: &Path) -> u64 {
    if path.is_file() {
        return fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    }
    ignore::WalkBuilder::new(path)
        .hidden(false)
        .git_ignore(false)
        .build()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .map(|e| fs::metadata(e.path()).map(|m| m.len()).unwrap_or(0))
        .sum()
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

fn format_ago(now_ms: u64, then_ms: Option<u64>) -> String {
    match then_ms {
        Some(ts) if now_ms >= ts => format!("{} ago", format_duration(now_ms - ts)),
        _ => "none".to_string(),
    }
}

fn print_running_status(paths: &DaemonPaths, state: &DaemonState) {
    let now_ms = now_unix_ms();
    let uptime_ms = now_ms.saturating_sub(state.started_at_unix_ms);
    let gen_mgr = GenerationManager::new(&paths.collie_dir);
    let index_dir = gen_mgr
        .active_generation()
        .ok()
        .flatten()
        .unwrap_or_else(|| paths.collie_dir.clone());
    let index_size = dir_size(&index_dir);

    println!("Collie daemon status: running");
    println!("Worktree root:  {}", paths.root.display());
    println!("PID:            {}", state.pid);
    println!("Uptime:         {}", format_duration(uptime_ms));
    println!("Index path:     {}", paths.index_path.display());
    println!("Index size:     {}", format_bytes(index_size));
    println!("Files indexed:  {}", state.total_files);
    println!("Unique terms:   {}", state.total_terms);
    println!("Postings:       {}", state.total_postings);
    println!("Trigram entries: {}", state.trigram_entries);
    println!(
        "Segments:       {} (baseline: {})",
        state.segment_count, state.initial_segment_count
    );
    if let Some(ref generation_name) = state.generation {
        println!("Generation:     {}", generation_name);
    }
    if state.compaction_recommended {
        println!("Compaction:     recommended (run 'collie rebuild .')");
    }
    if state.needs_rebuild {
        println!("Rebuild:        required");
    }
    if state.skipped_files > 0 {
        println!("Skipped:        {} files", state.skipped_files);
        for sample in &state.skipped_samples {
            let kind_label = match sample.kind {
                SkipKind::ReadError => "read error",
                SkipKind::SizeLimit => "size limit",
            };
            println!(
                "                - {} ({}): {}",
                sample.path, kind_label, sample.reason
            );
        }
        if state.skipped_files > state.skipped_samples.len() {
            println!(
                "                ... and {} more",
                state.skipped_files - state.skipped_samples.len()
            );
        }
    }
    println!(
        "Last save:      {}",
        format_ago(now_ms, state.last_save_at_unix_ms)
    );
    println!(
        "Last event:     {}",
        format_ago(now_ms, state.last_event_at_unix_ms)
    );
}

impl DaemonPaths {
    fn for_root(root: PathBuf) -> Self {
        let collie_dir = root.join(".collie");
        Self {
            index_path: collie_dir.clone(),
            pid_path: collie_dir.join("collie.pid"),
            state_path: collie_dir.join("daemon-state.json"),
            log_path: collie_dir.join("daemon.log"),
            root,
            collie_dir,
        }
    }
}

impl DaemonState {
    fn new_starting(paths: &DaemonPaths, pid: u32) -> Self {
        Self {
            worktree_root: paths.root.clone(),
            index_path: paths.index_path.clone(),
            pid,
            status: DaemonStatus::Starting,
            started_at_unix_ms: now_unix_ms(),
            last_event_at_unix_ms: None,
            last_save_at_unix_ms: None,
            total_files: 0,
            total_terms: 0,
            total_postings: 0,
            trigram_entries: 0,
            segment_count: 0,
            initial_segment_count: 0,
            generation: None,
            needs_rebuild: false,
            compaction_recommended: false,
            last_error: None,
            skipped_files: 0,
            skipped_samples: Vec::new(),
        }
    }

    fn new_running(
        paths: &DaemonPaths,
        pid: u32,
        last_save_at_unix_ms: Option<u64>,
        last_event_at_unix_ms: Option<u64>,
    ) -> Self {
        Self {
            worktree_root: paths.root.clone(),
            index_path: paths.index_path.clone(),
            pid,
            status: DaemonStatus::Running,
            started_at_unix_ms: now_unix_ms(),
            last_event_at_unix_ms,
            last_save_at_unix_ms,
            total_files: 0,
            total_terms: 0,
            total_postings: 0,
            trigram_entries: 0,
            segment_count: 0,
            initial_segment_count: 0,
            generation: None,
            needs_rebuild: false,
            compaction_recommended: false,
            last_error: None,
            skipped_files: 0,
            skipped_samples: Vec::new(),
        }
    }

    fn new_stopped(paths: &DaemonPaths, pid: Option<u32>, reason: String) -> Self {
        Self {
            worktree_root: paths.root.clone(),
            index_path: paths.index_path.clone(),
            pid: pid.unwrap_or_default(),
            status: DaemonStatus::Stopped,
            started_at_unix_ms: now_unix_ms(),
            last_event_at_unix_ms: None,
            last_save_at_unix_ms: None,
            total_files: 0,
            total_terms: 0,
            total_postings: 0,
            trigram_entries: 0,
            segment_count: 0,
            initial_segment_count: 0,
            generation: None,
            needs_rebuild: false,
            compaction_recommended: false,
            last_error: Some(reason),
            skipped_files: 0,
            skipped_samples: Vec::new(),
        }
    }
}
