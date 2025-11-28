use std::env;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let output_dir = env::args()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("usage: generate-corpus <output-dir>"))?;
    collie_search::benchmark::generate_corpus(&output_dir)
}
