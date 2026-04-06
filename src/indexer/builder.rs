use anyhow::{Context, Result};
use grep_regex::RegexMatcher;
use rayon::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::config::CollieConfig;
use crate::indexer::tokenizer::tokenize_query;
use crate::regex_search::{self, CandidateQuery, ExactCandidate, RegexFileMatch};
use crate::storage::tantivy_index::TantivyIndex;
use crate::storage::{IndexStats, SearchResult};
use crate::symbols::adapters::AdapterRegistry;
use crate::symbols::{SymbolQuery, SymbolResult};

/// Result of a regex search for a single file.
pub struct RegexSearchResult {
    pub file_path: PathBuf,
    pub matches: Vec<RegexFileMatch>,
}

enum PatternMode {
    Exact,
    Prefix,
    Suffix,
    Substring,
    MultiTerm,
}

struct ParsedPattern {
    tokens: Vec<String>,
    mode: PatternMode,
}

const REGEX_CANDIDATE_MIN_BUDGET: usize = 100;
const REGEX_CANDIDATE_OVERSAMPLE: usize = 4;
const REGEX_VERIFY_PARALLEL_THRESHOLD: usize = 8;
const REGEX_VERIFY_CHUNK_CAP: usize = 64;

enum ExhaustiveRegexCandidates {
    Shared(Arc<[SearchResult]>),
    Owned(Vec<SearchResult>),
}

impl ExhaustiveRegexCandidates {
    fn as_slice(&self) -> &[SearchResult] {
        match self {
            Self::Shared(files) => files,
            Self::Owned(files) => files,
        }
    }
}

/// Tokenize a query pattern and determine search mode.
///
/// Strips `%` wildcards, tokenizes the inner text through the same pipeline
/// as `collie_body` (split on non-alnum/non-underscore, lowercase, min 2 chars),
/// then decides the mode:
/// - Multiple tokens → MultiTerm (AND)
/// - Single token → Exact / Prefix / Suffix / Substring based on `%` markers
fn parse_file_pattern(pattern: &str) -> Option<ParsedPattern> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return None;
    }

    let starts_with_wildcard = pattern.starts_with('%');
    let ends_with_wildcard = pattern.ends_with('%');

    let inner = match (starts_with_wildcard, ends_with_wildcard) {
        (false, false) => pattern,
        (false, true) => pattern.trim_end_matches('%'),
        (true, false) => pattern.trim_start_matches('%'),
        (true, true) => pattern.trim_matches('%'),
    };

    let tokens = tokenize_query(inner);
    if tokens.is_empty() {
        return None;
    }

    let mode = if tokens.len() > 1 {
        PatternMode::MultiTerm
    } else {
        match (starts_with_wildcard, ends_with_wildcard) {
            (false, false) => PatternMode::Exact,
            (false, true) => PatternMode::Prefix,
            (true, false) => PatternMode::Suffix,
            (true, true) => PatternMode::Substring,
        }
    };

    Some(ParsedPattern { tokens, mode })
}

/// Index builder for creating and maintaining the search index.
///
/// Wraps a TantivyIndex (search + symbol storage).
/// The directory passed to `new` should contain `tantivy/`.
pub struct IndexBuilder {
    tantivy: TantivyIndex,
    symbol_registry: AdapterRegistry,
    max_file_size: u64,
    include_pdfs: bool,
    worktree_root: Option<PathBuf>,
}

impl IndexBuilder {
    /// Create a new index builder.
    ///
    /// `index_path` should be a directory containing `tantivy/`.
    /// If it has a file extension (e.g. legacy `.mmap` path), the parent directory is used.
    pub fn new<P: AsRef<Path>>(index_path: P, config: &CollieConfig) -> Result<Self> {
        let index_path = index_path.as_ref();

        // Backward compat: if path looks like a file, use parent
        let dir = if index_path.extension().is_some() {
            index_path.parent().unwrap_or(index_path).to_path_buf()
        } else {
            index_path.to_path_buf()
        };

        fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create index directory {:?}", dir))?;

        let tantivy_dir = dir.join("tantivy");

        let tantivy = TantivyIndex::open(&tantivy_dir)
            .with_context(|| format!("failed to open tantivy index at {:?}", tantivy_dir))?;

        let worktree_root = infer_worktree_root(&dir);

        Ok(Self {
            tantivy,
            symbol_registry: AdapterRegistry::default(),
            max_file_size: config.index.max_file_size,
            include_pdfs: config.index.include_pdfs,
            worktree_root,
        })
    }

    /// Index a single file. Returns `Ok(true)` if indexed, `Ok(false)` if
    /// skipped due to max_file_size, `Err` on read failure.
    pub fn index_file<P: AsRef<Path>>(&mut self, file_path: P) -> Result<bool> {
        let file_path = file_path.as_ref();

        let is_pdf = file_path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"));

        if is_pdf && !self.include_pdfs {
            return Ok(false);
        }

        let metadata = fs::metadata(file_path)
            .with_context(|| format!("Failed to stat file: {:?}", file_path))?;
        if metadata.len() > self.max_file_size {
            self.remove_file(file_path);
            return Ok(false);
        }

        let content = if is_pdf {
            super::pdf::extract_text(file_path)?
        } else {
            let bytes = fs::read(file_path)
                .with_context(|| format!("Failed to read file: {:?}", file_path))?;
            String::from_utf8_lossy(&bytes).into_owned()
        };

        self.index_content(file_path, &content)?;
        Ok(true)
    }

    /// Index content from a string.
    pub fn index_content(&mut self, file_path: &Path, content: &str) -> Result<()> {
        // Remove all existing docs (tokens + symbols) for this file
        self.tantivy.remove_by_path(file_path)?;

        // Add file doc — Tantivy's collie_body analyzer handles tokenization
        self.tantivy.index_file_content(file_path, content)?;

        // Extract and index symbols if a language adapter exists
        if let Some(adapter) = self.symbol_registry.adapter_for_path(file_path) {
            let repo_rel_path = self.repo_relative_path(file_path);
            if let Some(mut parser) = self.symbol_registry.create_parser_for(adapter) {
                let symbols =
                    adapter.extract_symbols_with_parser(&repo_rel_path, content, &mut parser);
                self.tantivy.index_symbols(file_path, &symbols)?;
            }
        }

        Ok(())
    }

    /// Index content with pre-extracted symbols. Used by bulk rebuild where
    /// symbol extraction is timed separately from tantivy writes.
    /// Set `fresh` to true when indexing into a new/empty generation to skip
    /// the redundant remove_by_path call.
    pub fn index_content_with_symbols(
        &mut self,
        file_path: &Path,
        content: &str,
        symbols: &[crate::symbols::Symbol],
        fresh: bool,
    ) -> Result<()> {
        if !fresh {
            self.tantivy.remove_by_path(file_path)?;
        }
        self.tantivy.index_file_content(file_path, content)?;
        if !symbols.is_empty() {
            self.tantivy.index_symbols(file_path, symbols)?;
        }
        Ok(())
    }

    /// Index a file with pre-tokenized body content. Used by bulk rebuild
    /// where tokenization runs in rayon workers. Skips remove_by_path since
    /// bulk rebuild targets a fresh generation.
    pub fn index_pretokenized(
        &mut self,
        file_path: &Path,
        body_tokens: tantivy::tokenizer::PreTokenizedString,
        body_reversed_tokens: tantivy::tokenizer::PreTokenizedString,
        symbols: &[crate::symbols::Symbol],
    ) -> Result<()> {
        self.tantivy.index_file_content_pretokenized(
            file_path,
            body_tokens,
            body_reversed_tokens,
        )?;
        if !symbols.is_empty() {
            self.tantivy.index_symbols(file_path, symbols)?;
        }
        Ok(())
    }

    /// Extract symbols for a file without writing to the index.
    pub fn extract_symbols_for(
        &self,
        file_path: &Path,
        content: &str,
    ) -> Vec<crate::symbols::Symbol> {
        if let Some(adapter) = self.symbol_registry.adapter_for_path(file_path) {
            let repo_rel_path = self.repo_relative_path(file_path);
            if let Some(mut parser) = self.symbol_registry.create_parser_for(adapter) {
                adapter.extract_symbols_with_parser(&repo_rel_path, content, &mut parser)
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        }
    }

    /// Compact segments by merging. Returns the new segment count.
    pub fn compact(&mut self) -> Result<usize> {
        self.tantivy.compact()
    }

    /// Disable segment merging for incremental updates.
    pub fn set_no_merge(&mut self) {
        self.tantivy.set_no_merge();
    }

    /// Set the writer heap budget in bytes. Larger values create fewer
    /// segments during bulk operations. Call before any writes.
    pub fn set_writer_heap(&mut self, bytes: usize) {
        self.tantivy.set_writer_heap(bytes);
    }

    /// Save the index to disk (commits Tantivy segments).
    pub fn save(&mut self) -> Result<()> {
        self.tantivy.commit()
    }

    /// Remove a file from the index.
    pub fn remove_file<P: AsRef<Path>>(&mut self, file_path: P) {
        let _ = self.tantivy.remove_by_path(file_path.as_ref());
    }

    /// Search for exact token match.
    pub fn search_exact(&self, token: &str) -> Vec<SearchResult> {
        self.tantivy.search_exact(token)
    }

    /// Search with wildcard pattern.
    /// `%` at start = suffix, at end = prefix, both = substring, none = exact.
    ///
    /// The query is tokenized through the same pipeline as indexed content,
    /// so punctuation and multi-word queries are handled correctly.
    pub fn search_pattern(&self, pattern: &str) -> Vec<SearchResult> {
        let parsed = match parse_file_pattern(pattern) {
            Some(p) => p,
            None => return Vec::new(),
        };
        match parsed.mode {
            PatternMode::Exact => self.tantivy.search_exact(&parsed.tokens[0]),
            PatternMode::Prefix => self.tantivy.search_prefix(&parsed.tokens[0]),
            PatternMode::Suffix => self.tantivy.search_suffix(&parsed.tokens[0]),
            PatternMode::Substring => self.tantivy.search_substring(&parsed.tokens[0]),
            PatternMode::MultiTerm => self.tantivy.search_multi_term(&parsed.tokens),
        }
    }

    /// Search with wildcard pattern, returning BM25-ranked results.
    pub fn search_pattern_ranked(&self, pattern: &str, limit: usize) -> Vec<SearchResult> {
        let parsed = match parse_file_pattern(pattern) {
            Some(p) => p,
            None => return Vec::new(),
        };
        match parsed.mode {
            PatternMode::Exact => self.tantivy.search_exact_ranked(&parsed.tokens[0], limit),
            PatternMode::Prefix => self.tantivy.search_prefix_ranked(&parsed.tokens[0], limit),
            PatternMode::Suffix => self.tantivy.search_suffix_ranked(&parsed.tokens[0], limit),
            PatternMode::Substring => self
                .tantivy
                .search_substring_ranked(&parsed.tokens[0], limit),
            PatternMode::MultiTerm => self.tantivy.search_multi_term_ranked(&parsed.tokens, limit),
        }
    }

    /// Count files matching a token/wildcard pattern.
    pub fn count_pattern(&self, pattern: &str) -> usize {
        let parsed = match parse_file_pattern(pattern) {
            Some(p) => p,
            None => return 0,
        };
        match parsed.mode {
            PatternMode::Exact => self.tantivy.count_exact(&parsed.tokens[0]),
            PatternMode::Prefix => self.tantivy.count_prefix(&parsed.tokens[0]),
            PatternMode::Suffix => self.tantivy.count_suffix(&parsed.tokens[0]),
            PatternMode::Substring => self.tantivy.count_substring(&parsed.tokens[0]),
            PatternMode::MultiTerm => self.tantivy.count_multi_term(&parsed.tokens),
        }
    }

    /// Search symbols using structured filters.
    pub fn search_symbols(&self, query: &SymbolQuery, limit: usize) -> Result<Vec<SymbolResult>> {
        self.tantivy.search_symbols(query, limit)
    }

    /// Search using a regex pattern (index-accelerated grep).
    ///
    /// Phase 1: extract literals from regex → query index for candidate files.
    /// Phase 2: apply full regex on each candidate file's content.
    pub fn search_regex(
        &self,
        pattern: &str,
        limit: usize,
        multiline: bool,
        ignore_case: bool,
        collect_matches: bool,
    ) -> Result<Vec<RegexSearchResult>> {
        let exhaustive_matcher = grep_regex::RegexMatcherBuilder::new()
            .multi_line(true)
            .dot_matches_new_line(multiline)
            .case_insensitive(ignore_case)
            .build(pattern)
            .with_context(|| format!("invalid regex: {}", pattern))?;

        let exact_candidates = regex_search::extract_exact_candidates(pattern);
        let candidate_query = regex_search::extract_candidate_query(pattern);

        if limit == 0 {
            return Ok(self.search_regex_exhaustive(
                &exhaustive_matcher,
                &exact_candidates,
                &candidate_query,
                multiline,
                collect_matches,
            ));
        }

        let mut results = Vec::new();
        let mut seen_candidates = HashSet::new();

        let mut budget = limit
            .saturating_mul(REGEX_CANDIDATE_OVERSAMPLE)
            .max(REGEX_CANDIDATE_MIN_BUDGET);

        loop {
            let mut new_candidates = 0usize;

            for exact_candidate in &exact_candidates {
                new_candidates += self.process_regex_candidates(
                    self.search_exact_candidate_ranked(exact_candidate, budget),
                    &exhaustive_matcher,
                    multiline,
                    collect_matches,
                    limit,
                    &mut seen_candidates,
                    &mut results,
                );
                if results.len() >= limit {
                    return Ok(results);
                }
            }

            for candidates in self.regex_candidate_passes_ranked(&candidate_query, budget) {
                new_candidates += self.process_regex_candidates(
                    candidates,
                    &exhaustive_matcher,
                    multiline,
                    collect_matches,
                    limit,
                    &mut seen_candidates,
                    &mut results,
                );
                if results.len() >= limit {
                    return Ok(results);
                }
            }

            if new_candidates == 0 {
                break;
            }

            let next_budget = budget.saturating_mul(2);
            if next_budget == budget {
                break;
            }
            budget = next_budget;
        }

        Ok(results)
    }

    /// Count files matching a regex pattern across the full indexed corpus.
    pub fn count_regex(
        &self,
        pattern: &str,
        multiline: bool,
        ignore_case: bool,
    ) -> Result<usize> {
        let exhaustive_matcher = grep_regex::RegexMatcherBuilder::new()
            .multi_line(true)
            .dot_matches_new_line(multiline)
            .case_insensitive(ignore_case)
            .build(pattern)
            .with_context(|| format!("invalid regex: {}", pattern))?;

        let exact_candidates = regex_search::extract_exact_candidates(pattern);
        let candidate_query = regex_search::extract_candidate_query(pattern);

        Ok(self.count_regex_exhaustive(
            &exhaustive_matcher,
            &exact_candidates,
            &candidate_query,
            multiline,
        ))
    }

    fn search_regex_exhaustive(
        &self,
        matcher: &RegexMatcher,
        exact_candidates: &[ExactCandidate],
        candidate_query: &CandidateQuery,
        multiline: bool,
        collect_matches: bool,
    ) -> Vec<RegexSearchResult> {
        let candidates = self.collect_exhaustive_regex_candidates(exact_candidates, candidate_query);
        self.verify_regex_parallel(candidates.as_slice(), matcher, multiline, collect_matches)
    }

    fn count_regex_exhaustive(
        &self,
        matcher: &RegexMatcher,
        exact_candidates: &[ExactCandidate],
        candidate_query: &CandidateQuery,
        multiline: bool,
    ) -> usize {
        let candidates = self.collect_exhaustive_regex_candidates(exact_candidates, candidate_query);
        self.count_regex_parallel(candidates.as_slice(), matcher, multiline)
    }

    fn collect_exhaustive_regex_candidates(
        &self,
        exact_candidates: &[ExactCandidate],
        candidate_query: &CandidateQuery,
    ) -> ExhaustiveRegexCandidates {
        let total_files = self.tantivy.file_count();
        let use_all_files =
            self.should_bypass_exhaustive_narrowing(total_files, exact_candidates, candidate_query);

        if use_all_files {
            return ExhaustiveRegexCandidates::Shared(self.tantivy.list_all_files_shared());
        }

        let mut seen = HashSet::new();
        let mut candidates = Vec::new();

        for exact_candidate in exact_candidates {
            for candidate in self.search_exact_candidate(exact_candidate) {
                if seen.insert(candidate.file_path.clone()) {
                    candidates.push(candidate);
                }
            }
        }

        if !matches!(candidate_query, CandidateQuery::All) {
            for pass in self.regex_candidate_passes(candidate_query) {
                for candidate in pass {
                    if seen.insert(candidate.file_path.clone()) {
                        candidates.push(candidate);
                    }
                }
            }
        }

        if total_files > 0 && candidates.len() * 100 >= total_files * 25 {
            return ExhaustiveRegexCandidates::Shared(self.tantivy.list_all_files_shared());
        }

        ExhaustiveRegexCandidates::Owned(candidates)
    }

    fn verify_regex_parallel(
        &self,
        files: &[SearchResult],
        matcher: &RegexMatcher,
        multiline: bool,
        collect_matches: bool,
    ) -> Vec<RegexSearchResult> {
        let mut results: Vec<RegexSearchResult> = files
            .par_iter()
            .map_init(
                || regex_search::build_regex_searcher(multiline),
                |searcher, candidate| {
                    if collect_matches {
                        regex_search::apply_regex_to_file_with_searcher(
                            &candidate.file_path,
                            matcher,
                            searcher,
                        )
                        .filter(|matches| !matches.is_empty())
                        .map(|matches| RegexSearchResult {
                            file_path: candidate.file_path.clone(),
                            matches,
                        })
                    } else {
                        regex_search::file_has_regex_match_with_searcher(
                            &candidate.file_path,
                            matcher,
                            searcher,
                        )
                        .filter(|matched| *matched)
                        .map(|_| RegexSearchResult {
                            file_path: candidate.file_path.clone(),
                            matches: Vec::new(),
                        })
                    }
                },
            )
            .filter_map(|result| result)
            .collect();
        results.sort_by(|a, b| a.file_path.cmp(&b.file_path));
        results
    }

    fn count_regex_parallel(
        &self,
        files: &[SearchResult],
        matcher: &RegexMatcher,
        multiline: bool,
    ) -> usize {
        files.par_iter()
            .map_init(
                || regex_search::build_regex_searcher(multiline),
                |searcher, candidate| {
                    usize::from(
                        regex_search::file_has_regex_match_with_searcher(
                            &candidate.file_path,
                            matcher,
                            searcher,
                        )
                        .unwrap_or(false),
                    )
                },
            )
            .sum()
    }

    fn should_bypass_exhaustive_narrowing(
        &self,
        total_files: usize,
        exact_candidates: &[ExactCandidate],
        candidate_query: &CandidateQuery,
    ) -> bool {
        if total_files == 0 || matches!(candidate_query, CandidateQuery::All) {
            return true;
        }

        let broad_threshold = total_files.saturating_mul(25) / 100;
        let broad_threshold = broad_threshold.max(1);

        // If any exact phrase candidate is selective, narrowing is worth it.
        for candidate in exact_candidates.iter().take(4) {
            let count = self.count_exact_candidate(candidate);
            if count > 0 && count < broad_threshold {
                return false;
            }
        }

        // For alternations, check the most selective branch — if any single
        // branch is narrow, per-branch narrowing beats a full scan.
        match candidate_query {
            CandidateQuery::All => true,
            CandidateQuery::And(tokens) => {
                let count = self.tantivy.count_multi_term(tokens);
                count == 0 || count >= broad_threshold
            }
            CandidateQuery::Or(branches) => {
                let min_count = self.tantivy.count_min_branch(branches);
                min_count == 0 || min_count >= broad_threshold
            }
        }
    }

    fn regex_candidate_passes(&self, candidate_query: &CandidateQuery) -> Vec<Vec<SearchResult>> {
        match candidate_query {
            CandidateQuery::All => vec![self.tantivy.list_all_files()],
            CandidateQuery::And(tokens) => vec![
                self.tantivy.search_multi_term(tokens),
                self.tantivy.search_multi_substring(tokens),
            ],
            CandidateQuery::Or(branches) => {
                let exact = self.merge_candidate_branches(branches, |branch| {
                    self.tantivy.search_multi_term(branch)
                });
                let substring = self.merge_candidate_branches(branches, |branch| {
                    self.tantivy.search_multi_substring(branch)
                });
                vec![exact, substring]
            }
        }
    }

    fn regex_candidate_passes_ranked(
        &self,
        candidate_query: &CandidateQuery,
        limit: usize,
    ) -> Vec<Vec<SearchResult>> {
        match candidate_query {
            CandidateQuery::All => vec![self.tantivy.list_all_files_ranked(limit)],
            CandidateQuery::And(tokens) => vec![
                self.tantivy.search_multi_term_ranked(tokens, limit),
                self.tantivy.search_multi_substring_ranked(tokens, limit),
            ],
            CandidateQuery::Or(branches) => {
                let exact = self.merge_candidate_branches(branches, |branch| {
                    self.tantivy.search_multi_term_ranked(branch, limit)
                });
                let substring = self.merge_candidate_branches(branches, |branch| {
                    self.tantivy.search_multi_substring_ranked(branch, limit)
                });
                vec![exact, substring]
            }
        }
    }

    fn search_exact_candidate(&self, candidate: &ExactCandidate) -> Vec<SearchResult> {
        match candidate.terms.as_slice() {
            [] => Vec::new(),
            [(_, token)] => self.tantivy.search_exact(token),
            _ => self.tantivy.search_phrase(&candidate.terms),
        }
    }

    fn search_exact_candidate_ranked(
        &self,
        candidate: &ExactCandidate,
        limit: usize,
    ) -> Vec<SearchResult> {
        match candidate.terms.as_slice() {
            [] => Vec::new(),
            [(_, token)] => self.tantivy.search_exact_ranked(token, limit),
            _ => self.tantivy.search_phrase_ranked(&candidate.terms, limit),
        }
    }

    fn count_exact_candidate(&self, candidate: &ExactCandidate) -> usize {
        match candidate.terms.as_slice() {
            [] => 0,
            [(_, token)] => self.tantivy.count_exact(token),
            _ => self.tantivy.count_phrase(&candidate.terms),
        }
    }

    fn merge_candidate_branches<F>(
        &self,
        branches: &[Vec<String>],
        mut search: F,
    ) -> Vec<SearchResult>
    where
        F: FnMut(&[String]) -> Vec<SearchResult>,
    {
        let mut seen = HashSet::new();
        let mut all = Vec::new();
        for branch in branches {
            for result in search(branch) {
                if seen.insert(result.file_path.clone()) {
                    all.push(result);
                }
            }
        }
        all
    }

    fn process_regex_candidates(
        &self,
        candidates: Vec<SearchResult>,
        matcher: &RegexMatcher,
        multiline: bool,
        collect_matches: bool,
        limit: usize,
        seen_candidates: &mut HashSet<PathBuf>,
        results: &mut Vec<RegexSearchResult>,
    ) -> usize {
        let mut new_candidates = 0usize;

        let chunk_size = limit
            .max(1)
            .min(REGEX_VERIFY_CHUNK_CAP);

        for candidate_chunk in candidates.chunks(chunk_size) {
            if limit > 0 && results.len() >= limit {
                break;
            }

            let mut pending = Vec::with_capacity(candidate_chunk.len());
            for candidate in candidate_chunk {
                if seen_candidates.insert(candidate.file_path.clone()) {
                    pending.push(candidate.clone());
                }
            }
            new_candidates += pending.len();

            if pending.is_empty() {
                continue;
            }

            let verified = self.verify_regex_chunk(
                &pending,
                matcher,
                multiline,
                collect_matches,
            );

            for result in verified.into_iter().flatten() {
                results.push(result);
                if limit > 0 && results.len() >= limit {
                    return new_candidates;
                }
            }
        }

        new_candidates
    }

    fn verify_regex_chunk(
        &self,
        candidates: &[SearchResult],
        matcher: &RegexMatcher,
        multiline: bool,
        collect_matches: bool,
    ) -> Vec<Option<RegexSearchResult>> {
        if candidates.len() < REGEX_VERIFY_PARALLEL_THRESHOLD {
            let mut searcher = regex_search::build_regex_searcher(multiline);
            let mut verified = Vec::with_capacity(candidates.len());
            for candidate in candidates {
                let result = if collect_matches {
                    regex_search::apply_regex_to_file_with_searcher(
                        &candidate.file_path,
                        matcher,
                        &mut searcher,
                    )
                    .filter(|matches| !matches.is_empty())
                    .map(|matches| RegexSearchResult {
                        file_path: candidate.file_path.clone(),
                        matches,
                    })
                } else {
                    regex_search::file_has_regex_match_with_searcher(
                        &candidate.file_path,
                        matcher,
                        &mut searcher,
                    )
                    .filter(|matched| *matched)
                    .map(|_| RegexSearchResult {
                        file_path: candidate.file_path.clone(),
                        matches: Vec::new(),
                    })
                };
                verified.push(result);
            }
            return verified;
        }

        candidates
            .par_iter()
            .map_init(
                || regex_search::build_regex_searcher(multiline),
                |searcher, candidate| {
                    if collect_matches {
                        regex_search::apply_regex_to_file_with_searcher(
                            &candidate.file_path,
                            matcher,
                            searcher,
                        )
                        .filter(|matches| !matches.is_empty())
                        .map(|matches| RegexSearchResult {
                            file_path: candidate.file_path.clone(),
                            matches,
                        })
                    } else {
                        regex_search::file_has_regex_match_with_searcher(
                            &candidate.file_path,
                            matcher,
                            searcher,
                        )
                        .filter(|matched| *matched)
                        .map(|_| RegexSearchResult {
                            file_path: candidate.file_path.clone(),
                            matches: Vec::new(),
                        })
                    }
                },
            )
            .collect()
    }

    /// Get index statistics. All values derived from the Tantivy index.
    pub fn stats(&self) -> IndexStats {
        let s = self.tantivy.stats();
        IndexStats {
            total_files: s.file_count,
            total_terms: s.unique_terms,
            total_postings: s.file_count, // one doc per file
            trigram_entries: 0,
            segment_count: s.segment_count,
        }
    }

    pub fn set_worktree_root<P: Into<PathBuf>>(&mut self, root: P) {
        self.worktree_root = Some(root.into());
    }

    fn repo_relative_path(&self, file_path: &Path) -> PathBuf {
        // Try strip_prefix directly first — avoids a syscall per file when
        // the walker already provides absolute paths matching the root.
        if let Some(root) = &self.worktree_root {
            if let Ok(rel) = file_path.strip_prefix(root) {
                return rel.to_path_buf();
            }
            // Fallback: canonicalize if prefix didn't match (e.g. symlinks)
            let canonical = fs::canonicalize(file_path).unwrap_or_else(|_| file_path.to_path_buf());
            canonical
                .strip_prefix(root)
                .map(Path::to_path_buf)
                .unwrap_or(canonical)
        } else {
            file_path.to_path_buf()
        }
    }
}

fn infer_worktree_root(index_dir: &Path) -> Option<PathBuf> {
    let canonical_dir = fs::canonicalize(index_dir).unwrap_or_else(|_| index_dir.to_path_buf());
    let mut current = Some(canonical_dir.as_path());

    while let Some(path) = current {
        if path.file_name().is_some_and(|name| name == ".collie") {
            return path.parent().map(Path::to_path_buf);
        }
        current = path.parent();
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn default_config() -> CollieConfig {
        CollieConfig::default()
    }

    #[test]
    fn test_index_content() {
        let temp = TempDir::new().unwrap();
        let index_dir = temp.path().join(".collie");
        let mut builder = IndexBuilder::new(&index_dir, &default_config()).unwrap();

        builder
            .index_content(Path::new("test.rs"), "fn hello_world() { }")
            .unwrap();
        builder.save().unwrap();

        let results = builder.search_exact("fn");
        assert_eq!(results.len(), 1);

        let results = builder.search_exact("hello_world");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_index_file() {
        let temp = TempDir::new().unwrap();
        let index_dir = temp.path().join(".collie");
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.rs");

        fs::write(
            &file_path,
            "fn calculate_sum(a: i32, b: i32) -> i32 { a + b }",
        )
        .unwrap();

        let mut builder = IndexBuilder::new(&index_dir, &default_config()).unwrap();
        builder.index_file(&file_path).unwrap();
        builder.save().unwrap();

        let results = builder.search_exact("calculate_sum");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].file_path, file_path);
    }

    #[test]
    fn test_pattern_exact() {
        let temp = TempDir::new().unwrap();
        let index_dir = temp.path().join(".collie");
        let mut builder = IndexBuilder::new(&index_dir, &default_config()).unwrap();

        builder
            .index_content(Path::new("test.rs"), "initialize initialization final")
            .unwrap();
        builder.save().unwrap();

        let results = builder.search_pattern("initialize");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_pattern_prefix() {
        let temp = TempDir::new().unwrap();
        let index_dir = temp.path().join(".collie");
        let mut builder = IndexBuilder::new(&index_dir, &default_config()).unwrap();

        builder
            .index_content(Path::new("test.rs"), "initialize initialization final")
            .unwrap();
        builder.save().unwrap();

        // One file doc, matched once (file contains tokens starting with "init")
        let results = builder.search_pattern("init%");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_pattern_suffix() {
        let temp = TempDir::new().unwrap();
        let index_dir = temp.path().join(".collie");
        let mut builder = IndexBuilder::new(&index_dir, &default_config()).unwrap();

        builder
            .index_content(Path::new("test.rs"), "hello jello world")
            .unwrap();
        builder.save().unwrap();

        // One file doc
        let results = builder.search_pattern("%llo");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_pattern_substring() {
        let temp = TempDir::new().unwrap();
        let index_dir = temp.path().join(".collie");
        let mut builder = IndexBuilder::new(&index_dir, &default_config()).unwrap();

        builder
            .index_content(Path::new("test.rs"), "initialize initialization final")
            .unwrap();
        builder.save().unwrap();

        // One file doc
        let results = builder.search_pattern("%init%");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_save_and_reload() {
        let temp = TempDir::new().unwrap();
        let index_dir = temp.path().join(".collie");

        {
            let mut builder = IndexBuilder::new(&index_dir, &default_config()).unwrap();
            builder
                .index_content(Path::new("test.rs"), "fn main() { }")
                .unwrap();
            builder.save().unwrap();
        }

        {
            let builder = IndexBuilder::new(&index_dir, &default_config()).unwrap();
            let results = builder.search_exact("main");
            assert_eq!(results.len(), 1);
        }
    }

    fn pdf_config() -> CollieConfig {
        let mut config = CollieConfig::default();
        config.index.include_pdfs = true;
        config
    }

    /// Minimal valid PDF containing "Hello World".
    fn minimal_pdf_bytes() -> Vec<u8> {
        let mut buf = Vec::new();
        let obj1 = b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n";
        let obj2 = b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n";
        let obj3 = b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>\nendobj\n";
        let stream = b"BT /F1 12 Tf 100 700 Td (Hello World) Tj ET";
        let obj4_hdr = format!("4 0 obj\n<< /Length {} >>\nstream\n", stream.len());
        let obj4_ftr = b"\nendstream\nendobj\n";
        let obj5 = b"5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n";

        buf.extend_from_slice(b"%PDF-1.0\n");
        let off1 = buf.len();
        buf.extend_from_slice(obj1);
        let off2 = buf.len();
        buf.extend_from_slice(obj2);
        let off3 = buf.len();
        buf.extend_from_slice(obj3);
        let off4 = buf.len();
        buf.extend_from_slice(obj4_hdr.as_bytes());
        buf.extend_from_slice(stream);
        buf.extend_from_slice(obj4_ftr);
        let off5 = buf.len();
        buf.extend_from_slice(obj5);

        let xref_off = buf.len();
        buf.extend_from_slice(b"xref\n0 6\n");
        for (off, tag) in [
            (0, "65535 f "),
            (off1, "00000 n "),
            (off2, "00000 n "),
            (off3, "00000 n "),
            (off4, "00000 n "),
            (off5, "00000 n "),
        ] {
            buf.extend_from_slice(format!("{:010} {}\n", off, tag).as_bytes());
        }
        buf.extend_from_slice(b"trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n");
        buf.extend_from_slice(format!("{}\n", xref_off).as_bytes());
        buf.extend_from_slice(b"%%EOF\n");
        buf
    }

    #[test]
    fn test_index_pdf_file() {
        let temp = TempDir::new().unwrap();
        let index_dir = temp.path().join(".collie");
        let dir = TempDir::new().unwrap();
        let pdf_path = dir.path().join("doc.pdf");
        fs::write(&pdf_path, minimal_pdf_bytes()).unwrap();

        let mut builder = IndexBuilder::new(&index_dir, &pdf_config()).unwrap();
        let indexed = builder.index_file(&pdf_path).unwrap();
        assert!(indexed);
        builder.save().unwrap();

        let results = builder.search_exact("hello");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].file_path, pdf_path);
    }

    #[test]
    fn test_pdf_skipped_when_disabled() {
        let temp = TempDir::new().unwrap();
        let index_dir = temp.path().join(".collie");
        let dir = TempDir::new().unwrap();
        let pdf_path = dir.path().join("doc.pdf");
        fs::write(&pdf_path, minimal_pdf_bytes()).unwrap();

        let mut builder = IndexBuilder::new(&index_dir, &default_config()).unwrap();
        let indexed = builder.index_file(&pdf_path).unwrap();
        assert!(!indexed);
    }

    #[test]
    fn test_reindex_file() {
        let temp = TempDir::new().unwrap();
        let index_dir = temp.path().join(".collie");
        let mut builder = IndexBuilder::new(&index_dir, &default_config()).unwrap();

        builder
            .index_content(Path::new("test.rs"), "fn old_function() { }")
            .unwrap();
        builder.save().unwrap();

        builder
            .index_content(Path::new("test.rs"), "fn new_function() { }")
            .unwrap();
        builder.save().unwrap();

        let results = builder.search_exact("old_function");
        assert_eq!(results.len(), 0);

        let results = builder.search_exact("new_function");
        assert_eq!(results.len(), 1);
    }
}
