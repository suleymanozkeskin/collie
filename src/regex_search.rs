use std::path::Path;

use regex::Regex;
use regex_syntax::hir::literal::{ExtractKind, Extractor, Seq};
use regex_syntax::hir::{Hir, HirKind};

use crate::indexer::tokenizer::{tokenize_query, tokenize_query_with_positions};

const MAX_CANDIDATE_BRANCHES: usize = 32;
const MAX_TOKENS_PER_BRANCH: usize = 8;
const MAX_EXACT_CANDIDATES: usize = 12;

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

    branches_to_candidate(simplify_branches(plan_required_branches(&hir)))
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
    collect_exact_candidates(&hir, &mut candidates);
    candidates.retain(|candidate| candidate.terms.len() > 1);
    candidates.sort_by(|a, b| {
        b.terms
            .len()
            .cmp(&a.terms.len())
            .then(b.total_bytes.cmp(&a.total_bytes))
            .then(a.terms.cmp(&b.terms))
    });
    candidates.dedup();
    candidates.truncate(MAX_EXACT_CANDIDATES);
    candidates
}

fn branches_to_candidate(branches: Vec<Vec<String>>) -> CandidateQuery {
    match branches.len() {
        0 => CandidateQuery::All,
        1 => {
            let tokens = &branches[0];
            if tokens.is_empty() {
                CandidateQuery::All
            } else {
                CandidateQuery::And(tokens.clone())
            }
        }
        _ => CandidateQuery::Or(branches),
    }
}

fn plan_required_branches(hir: &Hir) -> Vec<Vec<String>> {
    match hir.kind() {
        HirKind::Empty | HirKind::Class(_) | HirKind::Look(_) => vec![Vec::new()],
        HirKind::Literal(regex_syntax::hir::Literal(bytes)) => {
            let tokens = tokenize_query(&String::from_utf8_lossy(bytes));
            vec![tokens]
        }
        HirKind::Capture(capture) => plan_required_branches(&capture.sub),
        HirKind::Repetition(rep) => {
            if rep.min == 0 {
                vec![Vec::new()]
            } else {
                plan_required_branches(&rep.sub)
            }
        }
        HirKind::Concat(parts) => {
            let mut branches = vec![Vec::new()];
            for part in parts {
                branches = combine_branches(branches, plan_required_branches(part));
                branches = simplify_branches(branches);
            }
            branches
        }
        HirKind::Alternation(parts) => {
            let mut branches = Vec::new();
            for part in parts {
                branches.extend(plan_required_branches(part));
            }
            simplify_branches(branches)
        }
    }
}

fn combine_branches(left: Vec<Vec<String>>, right: Vec<Vec<String>>) -> Vec<Vec<String>> {
    let mut combined = Vec::new();
    for left_branch in &left {
        for right_branch in &right {
            let mut merged = left_branch.clone();
            for token in right_branch {
                if !merged.contains(token) {
                    merged.push(token.clone());
                }
            }
            combined.push(merged);
        }
    }
    combined
}

fn simplify_branches(mut branches: Vec<Vec<String>>) -> Vec<Vec<String>> {
    if branches.is_empty() {
        return vec![Vec::new()];
    }

    for branch in &mut branches {
        dedup_tokens(branch);
        trim_branch(branch);
    }

    if branches.iter().any(Vec::is_empty) {
        return vec![Vec::new()];
    }

    branches.sort_by(|a, b| branch_score(b).cmp(&branch_score(a)).then(a.cmp(b)));
    branches.dedup();

    if branches.len() > MAX_CANDIDATE_BRANCHES {
        let common = common_tokens(&branches);
        return vec![common];
    }
    branches
}

fn dedup_tokens(tokens: &mut Vec<String>) {
    let mut deduped = Vec::with_capacity(tokens.len());
    for token in tokens.drain(..) {
        if !deduped.contains(&token) {
            deduped.push(token);
        }
    }
    *tokens = deduped;
}

fn trim_branch(tokens: &mut Vec<String>) {
    if tokens.len() <= MAX_TOKENS_PER_BRANCH {
        return;
    }

    tokens.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| b.cmp(a)));
    tokens.truncate(MAX_TOKENS_PER_BRANCH);
    tokens.sort();
}

fn common_tokens(branches: &[Vec<String>]) -> Vec<String> {
    let mut common = branches[0].clone();
    common.retain(|token| branches[1..].iter().all(|branch| branch.contains(token)));
    trim_branch(&mut common);
    common
}

fn branch_score(tokens: &[String]) -> (usize, usize) {
    (tokens.iter().map(String::len).sum(), tokens.len())
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

fn collect_exact_candidates(hir: &Hir, out: &mut Vec<ExactCandidate>) {
    match hir.kind() {
        HirKind::Literal(regex_syntax::hir::Literal(bytes)) => {
            let text = String::from_utf8_lossy(bytes);
            let terms = tokenize_query_with_positions(&text);
            if !terms.is_empty() {
                out.push(ExactCandidate {
                    total_bytes: bytes.len(),
                    terms,
                });
            }
        }
        HirKind::Capture(capture) => collect_exact_candidates(&capture.sub, out),
        HirKind::Repetition(rep) => {
            if rep.min > 0 {
                collect_exact_candidates(&rep.sub, out);
            }
        }
        HirKind::Concat(parts) | HirKind::Alternation(parts) => {
            for part in parts {
                collect_exact_candidates(part, out);
            }
        }
        HirKind::Empty | HirKind::Class(_) | HirKind::Look(_) => {}
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

fn byte_to_line(offset: usize, line_starts: &[usize]) -> usize {
    match line_starts.binary_search(&offset) {
        Ok(idx) => idx,
        Err(idx) => idx.saturating_sub(1),
    }
}
