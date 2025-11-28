use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct GenerationManager {
    collie_dir: PathBuf,
}

impl GenerationManager {
    pub fn new(collie_dir: &Path) -> Self {
        Self {
            collie_dir: collie_dir.to_path_buf(),
        }
    }

    fn current_path(&self) -> PathBuf {
        self.collie_dir.join("CURRENT")
    }

    fn generations_dir(&self) -> PathBuf {
        self.collie_dir.join("generations")
    }

    /// Returns the path to the active generation directory, or None if
    /// CURRENT is missing, corrupt, or points to a nonexistent generation.
    pub fn active_generation(&self) -> Result<Option<PathBuf>> {
        let current_path = self.current_path();
        if !current_path.exists() {
            return Ok(None);
        }

        let gen_name = fs::read_to_string(&current_path)
            .context("failed to read CURRENT")?
            .trim()
            .to_string();

        if gen_name.is_empty() || !gen_name.starts_with("gen-") {
            return Ok(None);
        }

        let gen_dir = self.generations_dir().join(&gen_name);
        if !gen_dir.is_dir() {
            return Ok(None);
        }

        Ok(Some(gen_dir))
    }

    /// Creates a new timestamped generation directory under generations/.
    pub fn create_generation(&self) -> Result<PathBuf> {
        let generations_dir = self.generations_dir();
        fs::create_dir_all(&generations_dir).context("failed to create generations directory")?;

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("time went backwards")?
            .as_nanos();

        let gen_name = format!("gen-{}", timestamp);
        let gen_dir = generations_dir.join(&gen_name);
        fs::create_dir_all(&gen_dir)
            .with_context(|| format!("failed to create generation directory {:?}", gen_dir))?;

        Ok(gen_dir)
    }

    /// Atomically activate a generation by writing CURRENT.
    pub fn activate(&self, gen_dir: &Path) -> Result<()> {
        let gen_name = gen_dir
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("generation dir has no name"))?
            .to_string_lossy()
            .to_string();

        let current_path = self.current_path();
        let tmp_path = self.collie_dir.join("CURRENT.tmp");

        fs::write(&tmp_path, &gen_name).context("failed to write CURRENT.tmp")?;
        fs::rename(&tmp_path, &current_path).context("failed to rename CURRENT.tmp to CURRENT")?;

        Ok(())
    }

    /// Remove generation directories not referenced by CURRENT.
    pub fn cleanup_inactive(&self) -> Result<()> {
        let generations_dir = self.generations_dir();
        if !generations_dir.exists() {
            return Ok(());
        }

        let active_name = self
            .active_generation()?
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()));

        for entry in
            fs::read_dir(&generations_dir).context("failed to read generations directory")?
        {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();

            if Some(&name) != active_name.as_ref() {
                let path = entry.path();
                if path.is_dir() {
                    let _ = fs::remove_dir_all(&path);
                }
            }
        }

        Ok(())
    }

    /// Returns the path to the ACTIVE_DIRTY marker for a generation.
    pub fn dirty_marker(&self, gen_dir: &Path) -> PathBuf {
        gen_dir.join("ACTIVE_DIRTY")
    }

    /// Returns true if a full rebuild is needed:
    /// - CURRENT missing, corrupt, or points to nonexistent generation
    /// - Active generation has ACTIVE_DIRTY marker
    pub fn needs_rebuild(&self) -> bool {
        match self.active_generation() {
            Ok(Some(gen_dir)) => self.dirty_marker(&gen_dir).exists(),
            _ => true,
        }
    }
}
