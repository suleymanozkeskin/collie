use anyhow::{Context, Result};
use collie_search::config::CollieConfig;
use collie_search::indexer::IndexBuilder;
use collie_search::storage::generation::GenerationManager;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

static WORKTREE_COUNTER: AtomicUsize = AtomicUsize::new(0);
#[derive(Debug)]
pub struct Worktree {
    root: PathBuf,
}

impl Worktree {
    pub fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for Worktree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub fn create_worktree() -> Result<Worktree> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-worktrees");
    fs::create_dir_all(&base)?;

    let unique = format!(
        "worktree-{}-{}-{}",
        std::process::id(),
        WORKTREE_COUNTER.fetch_add(1, Ordering::Relaxed),
        SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let root = base.join(unique);
    fs::create_dir_all(root.join(".git"))?;
    Ok(Worktree { root })
}

pub fn collie_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_collie-search"))
}

pub fn collie_dir(root: &Path) -> PathBuf {
    canonical_root(root).join(".collie")
}

pub fn index_path(root: &Path) -> PathBuf {
    collie_dir(root)
}

pub fn pid_path(root: &Path) -> PathBuf {
    collie_dir(root).join("collie.pid")
}

pub fn state_path(root: &Path) -> PathBuf {
    collie_dir(root).join("daemon-state.json")
}

pub fn log_path(root: &Path) -> PathBuf {
    collie_dir(root).join("daemon.log")
}

pub fn run_collie(root: &Path, args: &[&str]) -> Result<Output> {
    let output = Command::new(collie_bin())
        .current_dir(root)
        .args(args)
        .output()
        .with_context(|| format!("failed to run collie with args {:?}", args))?;
    Ok(output)
}

pub fn spawn_collie(root: &Path, args: &[&str]) -> Result<Child> {
    let child = Command::new(collie_bin())
        .current_dir(root)
        .args(args)
        .spawn()
        .with_context(|| format!("failed to spawn collie with args {:?}", args))?;
    Ok(child)
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

pub fn write_file(root: &Path, relative: &str, content: &str) -> Result<PathBuf> {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, content)?;
    Ok(path)
}

pub fn build_index(root: &Path, files: &[(&str, &str)]) -> Result<()> {
    let collie = collie_dir(root);
    let mgr = GenerationManager::new(&collie);
    let gen_dir = mgr.create_generation()?;

    let config = CollieConfig::default();
    let mut builder = IndexBuilder::new(&gen_dir, &config)?;
    for (relative, content) in files {
        let path = write_file(root, relative, content)?;
        builder.index_file(path)?;
    }
    builder.save()?;

    mgr.activate(&gen_dir)?;
    Ok(())
}

pub fn canonical_root(root: &Path) -> PathBuf {
    fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf())
}

pub fn read_state(root: &Path) -> Result<Value> {
    let state = fs::read_to_string(state_path(root))?;
    Ok(serde_json::from_str(&state)?)
}

pub fn wait_for_condition<F>(timeout: Duration, mut f: F) -> Result<()>
where
    F: FnMut() -> Result<bool>,
{
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if f()? {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(50));
    }
    anyhow::bail!("condition not met within {:?}", timeout)
}

pub fn wait_for_running(root: &Path) -> Result<()> {
    wait_for_condition(Duration::from_secs(10), || {
        if !state_path(root).exists() {
            return Ok(false);
        }
        let state = read_state(root)?;
        Ok(state["status"] == "Running" || state["status"] == "running")
    })
}

pub fn wait_for_stopped(root: &Path) -> Result<()> {
    wait_for_condition(Duration::from_secs(10), || {
        if !state_path(root).exists() {
            return Ok(true);
        }
        let state = read_state(root)?;
        Ok(state["status"] == "Stopped" || state["status"] == "stopped")
    })
}

pub fn ensure_stopped(root: &Path) {
    let _ = run_collie(root, &["stop", "."]);
}
