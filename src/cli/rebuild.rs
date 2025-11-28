use anyhow::Result;
use std::path::PathBuf;
use std::time::Instant;

use crate::daemon;

pub fn run(path: PathBuf) -> Result<()> {
    let start = Instant::now();
    let result = daemon::rebuild(path)?;
    let elapsed = start.elapsed();

    println!("Rebuilt index in {:.1}s", elapsed.as_secs_f64());
    println!("Files indexed:  {}", result.stats.total_files);
    println!("Unique terms:   {}", result.stats.total_terms);
    println!("Postings:       {}", result.stats.total_postings);
    println!("Segments:       {}", result.stats.segment_count);
    println!("Generation:     {}", result.generation);
    if result.skipped_files > 0 {
        println!("Skipped:        {} files", result.skipped_files);
    }

    Ok(())
}
