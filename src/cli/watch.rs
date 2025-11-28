use anyhow::Result;
use std::path::PathBuf;

use crate::daemon;

pub fn run(path: PathBuf, foreground: bool, restart_on_crash: bool) -> Result<()> {
    daemon::start(path, foreground, restart_on_crash)
}
