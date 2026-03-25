use std::path::Path;

use regex::Regex;
use regex_syntax::hir::literal::{ExtractKind, Extractor, Seq};

use crate::indexer::tokenizer::tokenize_query;

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

/// A single regex match within a file.
#[derive(Debug)]
pub struct RegexFileMatch {
    /// 1-based line number.
    pub line_number: usize,
    /// Content of the matched line.
    pub line_content: String,
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
    let content = std::fs::read_to_string(file_path).ok()?;
    let mut matches = Vec::new();

    if multiline {
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
    } else {
        for (idx, line) in content.lines().enumerate() {
            if regex.is_match(line) {
                matches.push(RegexFileMatch {
                    line_number: idx + 1,
                    line_content: line.to_string(),
                });
            }
        }
    }

    Some(matches)
}

fn byte_to_line(offset: usize, line_starts: &[usize]) -> usize {
    match line_starts.binary_search(&offset) {
        Ok(idx) => idx,
        Err(idx) => idx.saturating_sub(1),
    }
}
