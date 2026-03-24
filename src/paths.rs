use anyhow::{Context, Result};
use fnv::FnvHasher;
use std::fs;
use std::hash::Hasher;
use std::path::{Path, PathBuf};

pub const STATE_DIR_ENV: &str = "COLLIE_STATE_DIR";

const LEGACY_RUNTIME_ENTRIES: &[&str] = &[
    "CURRENT",
    "CURRENT.tmp",
    "collie.pid",
    "daemon-state.json",
    "daemon.log",
    "last_activity",
    "generations",
];

pub fn repo_state_dir(worktree_root: &Path) -> Result<PathBuf> {
    let base = state_base_dir()?;
    Ok(repo_state_dir_with_base(worktree_root, &base))
}

pub fn repo_state_dir_with_base(worktree_root: &Path, base: &Path) -> PathBuf {
    base.join("repos").join(repo_id(worktree_root))
}

pub fn preferred_config_path(worktree_root: &Path) -> PathBuf {
    worktree_root.join(".collie.toml")
}

pub fn legacy_runtime_dir(worktree_root: &Path) -> PathBuf {
    worktree_root.join(".collie")
}

pub fn legacy_config_path(worktree_root: &Path) -> PathBuf {
    legacy_runtime_dir(worktree_root).join("config.toml")
}

pub fn config_path_candidates(worktree_root: &Path) -> [PathBuf; 2] {
    [
        preferred_config_path(worktree_root),
        legacy_config_path(worktree_root),
    ]
}

pub fn migrate_legacy_runtime(worktree_root: &Path, state_dir: &Path) -> Result<()> {
    let legacy_dir = legacy_runtime_dir(worktree_root);
    if !legacy_runtime_exists(worktree_root) {
        return Ok(());
    }

    fs::create_dir_all(state_dir)
        .with_context(|| format!("failed to create collie state dir {:?}", state_dir))?;

    for entry in LEGACY_RUNTIME_ENTRIES {
        let src = legacy_dir.join(entry);
        if !src.exists() {
            continue;
        }

        let dst = state_dir.join(entry);
        if dst.exists() {
            let _ = remove_path(&src);
            continue;
        }

        transfer_path(&src, &dst)?;
    }

    cleanup_legacy_runtime(worktree_root)?;
    Ok(())
}

pub fn cleanup_legacy_runtime(worktree_root: &Path) -> Result<()> {
    let legacy_dir = legacy_runtime_dir(worktree_root);
    if !legacy_dir.exists() {
        return Ok(());
    }

    for entry in LEGACY_RUNTIME_ENTRIES {
        let _ = remove_path(&legacy_dir.join(entry));
    }

    let mut entries = match fs::read_dir(&legacy_dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };
    if entries.next().is_none() {
        let _ = fs::remove_dir(&legacy_dir);
    }

    Ok(())
}

pub fn legacy_runtime_exists(worktree_root: &Path) -> bool {
    let legacy_dir = legacy_runtime_dir(worktree_root);
    LEGACY_RUNTIME_ENTRIES
        .iter()
        .any(|entry| legacy_dir.join(entry).exists())
}

pub fn repo_id(worktree_root: &Path) -> String {
    let canonical = fs::canonicalize(worktree_root).unwrap_or_else(|_| worktree_root.to_path_buf());
    let repo_name = canonical
        .file_name()
        .map(|name| slugify(name.to_string_lossy().as_ref()))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "repo".to_string());

    let mut hasher = FnvHasher::default();
    hasher.write(canonical.to_string_lossy().as_bytes());
    format!("{repo_name}-{:016x}", hasher.finish())
}

fn state_base_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os(STATE_DIR_ENV) {
        return Ok(PathBuf::from(dir).join("collie"));
    }

    #[cfg(target_os = "macos")]
    {
        return home_dir()
            .map(|home| home.join("Library").join("Caches").join("collie"))
            .context("failed to resolve HOME for collie state dir");
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            return Ok(PathBuf::from(local_app_data).join("Collie"));
        }
        return home_dir()
            .map(|home| home.join("AppData").join("Local").join("Collie"))
            .context("failed to resolve LOCALAPPDATA for collie state dir");
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        if let Some(cache_home) = std::env::var_os("XDG_CACHE_HOME") {
            return Ok(PathBuf::from(cache_home).join("collie"));
        }
        return home_dir()
            .map(|home| home.join(".cache").join("collie"))
            .context("failed to resolve HOME for collie state dir");
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn slugify(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

fn transfer_path(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent dir {:?}", parent))?;
    }

    match fs::rename(src, dst) {
        Ok(()) => return Ok(()),
        Err(_) => {}
    }

    if src.is_dir() {
        copy_dir_recursive(src, dst)?;
    } else {
        fs::copy(src, dst).with_context(|| format!("failed to copy {:?} to {:?}", src, dst))?;
    }

    let _ = remove_path(src);
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).with_context(|| format!("failed to create dir {:?}", dst))?;
    for entry in fs::read_dir(src).with_context(|| format!("failed to read dir {:?}", src))? {
        let entry = entry?;
        let child_src = entry.path();
        let child_dst = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&child_src, &child_dst)?;
        } else {
            if let Some(parent) = child_dst.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&child_src, &child_dst)
                .with_context(|| format!("failed to copy {:?} to {:?}", child_src, child_dst))?;
        }
    }
    Ok(())
}

fn remove_path(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        fs::remove_dir_all(path).with_context(|| format!("failed to remove dir {:?}", path))?;
    } else {
        fs::remove_file(path).with_context(|| format!("failed to remove file {:?}", path))?;
    }
    Ok(())
}
