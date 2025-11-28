pub mod generation;
pub mod tantivy_index;

pub use tantivy_index::SearchResult;

#[derive(Debug, Clone)]
pub struct IndexStats {
    pub total_files: usize,
    pub total_terms: usize,
    pub total_postings: usize,
    pub trigram_entries: usize, // always 0 — kept for state file compatibility
    pub segment_count: usize,
}
