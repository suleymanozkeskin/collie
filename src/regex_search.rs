use std::io;
use std::path::Path;

use grep_regex::RegexMatcher;
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkMatch};
use regex::Regex;
use regex_syntax::hir::literal::{ExtractKind, Extractor, Seq};

use crate::indexer::tokenizer::{tokenize_query, tokenize_query_with_positions};

/// How to query the index for candidate files.
#[derive(Debug)]
pub enum CandidateQuery {
    /// No useful literals extracted — must scan all indexed files.
    All,
    /// All tokens must appear in a file (AND).
    And(Vec<String>),
    /// Any branch (all tokens in that branch) must appear (OR of ANDs).
    Or(Vec<Vec<String>>),
}

/// A stronger exact candidate extracted from regex literals.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExactCandidate {
    pub terms: Vec<(usize, String)>,
    pub total_bytes: usize,
}

/// A single regex match within a file.
#[derive(Debug)]
pub struct RegexFileMatch {
    /// 1-based line number.
    pub line_number: usize,
    /// Content of the matched line.
    pub line_content: String,
}

/// A single snippet line produced during regex verification.
#[derive(Debug, Clone)]
pub struct RegexSnippetLine {
    /// 1-based line number.
    pub line_number: usize,
    /// Content of the line.
    pub line_content: String,
    /// Whether this line contains a match.
    pub is_match: bool,
}

/// A contiguous snippet window produced during regex verification.
#[derive(Debug, Clone, Default)]
pub struct RegexSnippet {
    pub lines: Vec<RegexSnippetLine>,
}

/// Match lines plus snippet-ready context captured in a single grep pass.
#[derive(Debug, Default)]
pub struct RegexMatchCapture {
    pub matches: Vec<RegexFileMatch>,
    pub snippets: Vec<RegexSnippet>,
}

/// Extract a `CandidateQuery` from a regex pattern string.
///
/// Uses `regex-syntax` to parse the pattern into an HIR, then extracts
/// prefix and suffix literal sequences via `Extractor`. These are tokenized
/// through `tokenize_query` (same pipeline as `collie_body`) to produce
/// index-compatible terms.
pub fn extract_candidate_query(pattern: &str) -> CandidateQuery {
    let hir = match regex_syntax::parse(pattern) {
        Ok(h) => h,
        Err(_) => return CandidateQuery::All,
    };

    let prefix_seq = Extractor::new().kind(ExtractKind::Prefix).extract(&hir);
    let suffix_seq = Extractor::new().kind(ExtractKind::Suffix).extract(&hir);

    merge_candidates(seq_to_candidate(&prefix_seq), seq_to_candidate(&suffix_seq))
}

/// Extract exact literal candidates while preserving token order and positions.
pub fn extract_exact_candidates(pattern: &str) -> Vec<ExactCandidate> {
    let hir = match regex_syntax::parse(pattern) {
        Ok(h) => h,
        Err(_) => return Vec::new(),
    };

    let prefix_seq = Extractor::new().kind(ExtractKind::Prefix).extract(&hir);
    let suffix_seq = Extractor::new().kind(ExtractKind::Suffix).extract(&hir);

    let mut candidates = seq_to_exact_candidates(&prefix_seq);
    candidates.extend(seq_to_exact_candidates(&suffix_seq));
    candidates.sort_by(|a, b| {
        b.terms
            .len()
            .cmp(&a.terms.len())
            .then(b.total_bytes.cmp(&a.total_bytes))
            .then(a.terms.cmp(&b.terms))
    });
    candidates.dedup();
    candidates
}

fn seq_to_candidate(seq: &Seq) -> CandidateQuery {
    let lits = match seq.literals() {
        Some(l) if !l.is_empty() => l,
        _ => return CandidateQuery::All,
    };

    if lits.len() == 1 {
        let tokens = tokenize_query(&String::from_utf8_lossy(lits[0].as_bytes()));
        return if tokens.is_empty() {
            CandidateQuery::All
        } else {
            CandidateQuery::And(tokens)
        };
    }

    let branches: Vec<Vec<String>> = lits
        .iter()
        .map(|lit| tokenize_query(&String::from_utf8_lossy(lit.as_bytes())))
        .filter(|t| !t.is_empty())
        .collect();

    match branches.len() {
        0 => CandidateQuery::All,
        1 => CandidateQuery::And(branches.into_iter().next().unwrap()),
        _ => CandidateQuery::Or(branches),
    }
}

fn merge_candidates(a: CandidateQuery, b: CandidateQuery) -> CandidateQuery {
    match (a, b) {
        (CandidateQuery::All, other) | (other, CandidateQuery::All) => other,
        (CandidateQuery::And(mut a_t), CandidateQuery::And(b_t)) => {
            for t in b_t {
                if !a_t.contains(&t) {
                    a_t.push(t);
                }
            }
            CandidateQuery::And(a_t)
        }
        (CandidateQuery::And(t), CandidateQuery::Or(_))
        | (CandidateQuery::Or(_), CandidateQuery::And(t)) => CandidateQuery::And(t),
        (CandidateQuery::Or(a_b), CandidateQuery::Or(_)) => CandidateQuery::Or(a_b),
    }
}

fn seq_to_exact_candidates(seq: &Seq) -> Vec<ExactCandidate> {
    let lits = match seq.literals() {
        Some(l) if !l.is_empty() => l,
        _ => return Vec::new(),
    };

    lits.iter()
        .filter_map(|lit| {
            let text = String::from_utf8_lossy(lit.as_bytes());
            let terms = tokenize_query_with_positions(&text);
            if terms.is_empty() {
                None
            } else {
                Some(ExactCandidate {
                    terms,
                    total_bytes: lit.as_bytes().len(),
                })
            }
        })
        .collect()
}

/// Apply a compiled regex to a file, returning matching line numbers and content.
///
/// In line mode (`multiline=false`): iterates lines, tests each independently.
/// In multiline mode (`multiline=true`): matches against full content, maps byte
/// offsets to line numbers.
///
/// Returns `None` if the file cannot be read.
pub fn apply_regex_to_file(
    file_path: &Path,
    regex: &Regex,
    multiline: bool,
) -> Option<Vec<RegexFileMatch>> {
    if multiline {
        let content = std::fs::read_to_string(file_path).ok()?;
        let mut matches = Vec::new();
        let line_starts: Vec<usize> = std::iter::once(0)
            .chain(
                content
                    .bytes()
                    .enumerate()
                    .filter(|(_, b)| *b == b'\n')
                    .map(|(i, _)| i + 1),
            )
            .collect();
        let lines_vec: Vec<&str> = content
            .split('\n')
            .map(|l| l.trim_end_matches('\r'))
            .collect();
        let mut seen = std::collections::BTreeSet::new();

        for mat in regex.find_iter(&content) {
            let start_ln = byte_to_line(mat.start(), &line_starts);
            let end_ln = byte_to_line(mat.end().saturating_sub(1).max(mat.start()), &line_starts);
            for ln in start_ln..=end_ln {
                if seen.insert(ln) {
                    if let Some(line) = lines_vec.get(ln) {
                        matches.push(RegexFileMatch {
                            line_number: ln + 1,
                            line_content: line.to_string(),
                        });
                    }
                }
            }
        }
        Some(matches)
    } else {
        let file = std::fs::File::open(file_path).ok()?;
        let mut reader = std::io::BufReader::new(file);
        let mut line = String::new();
        let mut line_number = 0usize;
        let mut matches = Vec::new();

        loop {
            line.clear();
            let bytes = std::io::BufRead::read_line(&mut reader, &mut line).ok()?;
            if bytes == 0 {
                break;
            }
            line_number += 1;

            let trimmed = line.trim_end_matches(['\n', '\r']);
            if regex.is_match(trimmed) {
                matches.push(RegexFileMatch {
                    line_number,
                    line_content: trimmed.to_string(),
                });
            }
        }
        Some(matches)
    }
}

/// Return whether a compiled regex matches anywhere in a file.
///
/// This is used for `--no-snippets`/`-l`/`--count` style searches where the
/// caller only needs to know whether the file matched, not every matching line.
pub fn file_has_regex_match(file_path: &Path, regex: &Regex, multiline: bool) -> Option<bool> {
    if multiline {
        let content = std::fs::read_to_string(file_path).ok()?;
        Some(regex.is_match(&content))
    } else {
        let file = std::fs::File::open(file_path).ok()?;
        let mut reader = std::io::BufReader::new(file);
        let mut line = String::new();

        loop {
            line.clear();
            let bytes = std::io::BufRead::read_line(&mut reader, &mut line).ok()?;
            if bytes == 0 {
                return Some(false);
            }
            let trimmed = line.trim_end_matches(['\n', '\r']);
            if regex.is_match(trimmed) {
                return Some(true);
            }
        }
    }
}

/// Apply a grep-style regex matcher to a file, returning matching line numbers
/// and content.
///
/// This is used by exhaustive regex search so the heavy verification path uses
/// the same search stack as ripgrep's matcher/searcher crates instead of a
/// handwritten per-file loop.
pub fn apply_regex_to_file_searcher(
    file_path: &Path,
    matcher: &RegexMatcher,
    multiline: bool,
) -> Option<Vec<RegexFileMatch>> {
    let mut searcher = build_regex_searcher(multiline);
    apply_regex_to_file_with_searcher(file_path, matcher, &mut searcher)
}

/// Build a reusable grep-style searcher for exhaustive regex verification.
pub fn build_regex_searcher(multiline: bool) -> Searcher {
    build_regex_searcher_with_context(multiline, 0, 0)
}

/// Build a reusable grep-style searcher with explicit before/after context.
pub fn build_regex_searcher_with_context(
    multiline: bool,
    before_context: usize,
    after_context: usize,
) -> Searcher {
    let mut builder = SearcherBuilder::new();
    builder
        .line_number(true)
        .multi_line(multiline)
        .before_context(before_context)
        .after_context(after_context)
        .binary_detection(BinaryDetection::none());
    builder.build()
}

/// Apply a grep-style regex matcher using a caller-provided reusable searcher.
pub fn apply_regex_to_file_with_searcher(
    file_path: &Path,
    matcher: &RegexMatcher,
    searcher: &mut Searcher,
) -> Option<Vec<RegexFileMatch>> {
    let mut sink = RegexSink::default();
    searcher.search_path(matcher, file_path, &mut sink).ok()?;
    Some(sink.matches)
}

/// Apply a grep-style matcher and capture snippet-ready context lines in the
/// same file pass used for regex verification.
pub fn apply_regex_to_file_with_context_with_searcher(
    file_path: &Path,
    matcher: &RegexMatcher,
    searcher: &mut Searcher,
) -> Option<RegexMatchCapture> {
    let mut sink = RegexContextSink::default();
    searcher.search_path(matcher, file_path, &mut sink).ok()?;
    Some(sink.finish())
}

/// Return whether a grep-style matcher matched a file using a reusable searcher.
pub fn file_has_regex_match_with_searcher(
    file_path: &Path,
    matcher: &RegexMatcher,
    searcher: &mut Searcher,
) -> Option<bool> {
    let mut sink = RegexPresenceSink::default();
    searcher.search_path(matcher, file_path, &mut sink).ok()?;
    Some(sink.found)
}

fn byte_to_line(offset: usize, line_starts: &[usize]) -> usize {
    match line_starts.binary_search(&offset) {
        Ok(idx) => idx,
        Err(idx) => idx.saturating_sub(1),
    }
}

fn trim_line_ending(line: &[u8]) -> &[u8] {
    let line = if line.last() == Some(&b'\n') {
        &line[..line.len().saturating_sub(1)]
    } else {
        line
    };
    if line.last() == Some(&b'\r') {
        &line[..line.len().saturating_sub(1)]
    } else {
        line
    }
}

#[derive(Default)]
struct RegexSink {
    matches: Vec<RegexFileMatch>,
    seen_lines: std::collections::BTreeSet<usize>,
}

#[derive(Default)]
struct RegexPresenceSink {
    found: bool,
}

#[derive(Default)]
struct RegexContextSink {
    matches: Vec<RegexFileMatch>,
    snippets: Vec<RegexSnippet>,
    current_snippet: Option<RegexSnippet>,
    seen_match_lines: std::collections::BTreeSet<usize>,
}

impl Sink for RegexSink {
    type Error = io::Error;

    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, Self::Error> {
        let start_line = mat.line_number().unwrap_or(1) as usize;
        for (offset, line) in mat.lines().enumerate() {
            let line_number = start_line + offset;
            if self.seen_lines.insert(line_number) {
                self.matches.push(RegexFileMatch {
                    line_number,
                    line_content: String::from_utf8_lossy(trim_line_ending(line)).into_owned(),
                });
            }
        }
        Ok(true)
    }
}

impl Sink for RegexPresenceSink {
    type Error = io::Error;

    fn matched(&mut self, _searcher: &Searcher, _mat: &SinkMatch<'_>) -> Result<bool, Self::Error> {
        self.found = true;
        Ok(false)
    }
}

impl RegexContextSink {
    fn finish(mut self) -> RegexMatchCapture {
        self.flush_snippet();
        RegexMatchCapture {
            matches: self.matches,
            snippets: self.snippets,
        }
    }

    fn flush_snippet(&mut self) {
        if let Some(snippet) = self.current_snippet.take() {
            if !snippet.lines.is_empty() {
                self.snippets.push(snippet);
            }
        }
    }

    fn push_line(&mut self, line_number: usize, line_content: String, is_match: bool) {
        if let Some(current) = self.current_snippet.as_ref() {
            if let Some(last) = current.lines.last() {
                if line_number > last.line_number + 1 {
                    self.flush_snippet();
                }
            }
        }

        let snippet = self
            .current_snippet
            .get_or_insert_with(RegexSnippet::default);
        if let Some(existing) = snippet
            .lines
            .iter_mut()
            .find(|line| line.line_number == line_number)
        {
            existing.is_match |= is_match;
            return;
        }

        snippet.lines.push(RegexSnippetLine {
            line_number,
            line_content: line_content.clone(),
            is_match,
        });

        if is_match && self.seen_match_lines.insert(line_number) {
            self.matches.push(RegexFileMatch {
                line_number,
                line_content,
            });
        }
    }
}

impl Sink for RegexContextSink {
    type Error = io::Error;

    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, Self::Error> {
        let start_line = mat.line_number().unwrap_or(1) as usize;
        for (offset, line) in mat.lines().enumerate() {
            let line_number = start_line + offset;
            self.push_line(
                line_number,
                String::from_utf8_lossy(trim_line_ending(line)).into_owned(),
                true,
            );
        }
        Ok(true)
    }

    fn context(
        &mut self,
        _searcher: &Searcher,
        context: &SinkContext<'_>,
    ) -> Result<bool, Self::Error> {
        let line_number = context.line_number().unwrap_or(1) as usize;
        self.push_line(
            line_number,
            String::from_utf8_lossy(trim_line_ending(context.bytes())).into_owned(),
            false,
        );
        Ok(true)
    }

    fn context_break(&mut self, _searcher: &Searcher) -> Result<bool, Self::Error> {
        self.flush_snippet();
        Ok(true)
    }
}
