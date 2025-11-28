use anyhow::Result;
use std::path::PathBuf;

use crate::daemon;

pub fn run(path: PathBuf, json: bool) -> Result<()> {
    daemon::status(path, json)
}
