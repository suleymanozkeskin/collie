use anyhow::Result;
use std::path::PathBuf;

use crate::daemon;

pub fn run(path: PathBuf) -> Result<()> {
    daemon::stop(path)
}
