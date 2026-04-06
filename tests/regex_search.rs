use collie_search::regex_search::{
    CandidateQuery, LiteralQuery, apply_regex_to_file, apply_regex_to_file_searcher,
    apply_regex_to_file_with_context_with_searcher, build_regex_searcher_with_context,
    extract_candidate_query, extract_exact_candidates, extract_literal_query,
    literal_query_matches,
};
use std::io::Write;

// --- Literal extraction ---

#[test]
fn concat_pattern_extracts_and_tokens() {
    match extract_candidate_query("impl.*for SearchResult") {
        CandidateQuery::And(tokens) => {
            assert!(tokens.contains(&"impl".to_string()));
            assert!(tokens.contains(&"searchresult".to_string()));
        }
        other => panic!("expected And, got {:?}", other),
    }
}

#[test]
fn alternation_produces_or_branches() {
    match extract_candidate_query("TODO|FIXME|HACK") {
        CandidateQuery::Or(branches) => {
            assert_eq!(branches.len(), 3);
            let flat: Vec<String> = branches.into_iter().flatten().collect();
            assert!(flat.contains(&"todo".to_string()));
            assert!(flat.contains(&"fixme".to_string()));
            assert!(flat.contains(&"hack".to_string()));
        }
        other => panic!("expected Or, got {:?}", other),
    }
}

#[test]
fn escaped_special_chars_extract_literal() {
    match extract_candidate_query(r"\.unwrap\(\)") {
        CandidateQuery::And(tokens) => {
            assert!(tokens.contains(&"unwrap".to_string()));
        }
        other => panic!("expected And with 'unwrap', got {:?}", other),
    }
}

#[test]
fn no_literals_returns_all() {
    assert!(matches!(extract_candidate_query(".*"), CandidateQuery::All));
}

#[test]
fn digit_only_returns_all() {
    assert!(matches!(
        extract_candidate_query(r"\d+"),
        CandidateQuery::All
    ));
}

#[test]
fn invalid_regex_returns_all() {
    assert!(matches!(
        extract_candidate_query("[invalid"),
        CandidateQuery::All
    ));
}

#[test]
fn anchored_pattern_extracts_literals() {
    match extract_candidate_query(r"^pub fn ") {
        CandidateQuery::And(tokens) => {
            assert!(tokens.contains(&"pub".to_string()));
        }
        other => panic!("expected And with 'pub', got {:?}", other),
    }
}

#[test]
fn exact_candidates_preserve_phrase_positions() {
    let candidates = extract_exact_candidates(r"context\.Context");
    assert!(candidates.iter().any(|candidate| {
        candidate.terms == vec![(0, "context".to_string()), (1, "context".to_string())]
    }));
}

#[test]
fn exact_candidates_preserve_position_gaps_after_short_tokens() {
    let candidates = extract_exact_candidates("foo a bar");
    assert!(candidates.iter().any(|candidate| {
        candidate.terms == vec![(0, "foo".to_string()), (2, "bar".to_string())]
    }));
}

#[test]
fn literal_query_preserves_raw_punctuation() {
    match extract_literal_query(r"errors?\.New\(") {
        LiteralQuery::Or(branches) => {
            let flat: Vec<String> = branches.into_iter().flatten().collect();
            assert!(flat.iter().any(|literal| literal.contains("New(")));
        }
        other => panic!("expected Or literal branches, got {:?}", other),
    }
}

#[test]
fn literal_query_matching_requires_branch_literals() {
    let query = extract_literal_query(r"TODO|FIXME|HACK");
    assert!(literal_query_matches("contains FIXME here", &query));
    assert!(!literal_query_matches("contains NOTE here", &query));
}

// --- Regex file application ---

#[test]
fn apply_regex_finds_matching_lines() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    write!(
        tmp.as_file(),
        "line one\nfn hello_world() {{}}\nline three\n"
    )
    .unwrap();
    let re = regex::Regex::new("hello_world").unwrap();
    let matches = apply_regex_to_file(tmp.path(), &re, false).unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].line_number, 2);
    assert!(matches[0].line_content.contains("hello_world"));
}

#[test]
fn apply_regex_multiple_matches() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    write!(tmp.as_file(), "TODO: fix\nclean\nTODO: also\n").unwrap();
    let re = regex::Regex::new("TODO").unwrap();
    let matches = apply_regex_to_file(tmp.path(), &re, false).unwrap();
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].line_number, 1);
    assert_eq!(matches[1].line_number, 3);
}

#[test]
fn apply_regex_trims_line_endings_in_streaming_mode() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    write!(tmp.as_file(), "hit\r\nmiss\r\n").unwrap();
    let re = regex::Regex::new("hit").unwrap();
    let matches = apply_regex_to_file(tmp.path(), &re, false).unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].line_content, "hit");
}

#[test]
fn apply_regex_searcher_matches_streaming_line_mode() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    write!(tmp.as_file(), "line one\r\nfn hello_world() {{}}\r\n").unwrap();
    let re = regex::Regex::new("hello_world").unwrap();
    let matcher = grep_regex::RegexMatcherBuilder::new()
        .multi_line(true)
        .build("hello_world")
        .unwrap();

    let string_matches = apply_regex_to_file(tmp.path(), &re, false).unwrap();
    let searcher_matches = apply_regex_to_file_searcher(tmp.path(), &matcher, false).unwrap();

    assert_eq!(string_matches.len(), searcher_matches.len());
    assert_eq!(
        string_matches[0].line_number,
        searcher_matches[0].line_number
    );
    assert_eq!(
        string_matches[0].line_content,
        searcher_matches[0].line_content
    );
}

#[test]
fn apply_regex_no_match_returns_empty() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    write!(tmp.as_file(), "nothing here\n").unwrap();
    let re = regex::Regex::new("nonexistent").unwrap();
    let matches = apply_regex_to_file(tmp.path(), &re, false).unwrap();
    assert!(matches.is_empty());
}

#[test]
fn apply_regex_multiline_spans_lines() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    write!(tmp.as_file(), "start\nmatch across\nlines end\nafter\n").unwrap();
    let re = regex::RegexBuilder::new("match across\nlines")
        .multi_line(true)
        .dot_matches_new_line(true)
        .build()
        .unwrap();
    let matches = apply_regex_to_file(tmp.path(), &re, true).unwrap();
    assert!(matches.iter().any(|m| m.line_number == 2));
    assert!(matches.iter().any(|m| m.line_number == 3));
}

#[test]
fn apply_regex_searcher_multiline_spans_lines() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    write!(tmp.as_file(), "start\nmatch across\nlines end\nafter\n").unwrap();
    let matcher = grep_regex::RegexMatcherBuilder::new()
        .multi_line(true)
        .dot_matches_new_line(true)
        .build("match across\nlines")
        .unwrap();
    let matches = apply_regex_to_file_searcher(tmp.path(), &matcher, true).unwrap();
    assert!(matches.iter().any(|m| m.line_number == 2));
    assert!(matches.iter().any(|m| m.line_number == 3));
}

#[test]
fn apply_regex_searcher_with_context_captures_surrounding_lines() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    write!(
        tmp.as_file(),
        "line one\nfn hello_world() {{}}\nline three\n"
    )
    .unwrap();
    let matcher = grep_regex::RegexMatcherBuilder::new()
        .multi_line(true)
        .build("hello_world")
        .unwrap();
    let mut searcher = build_regex_searcher_with_context(false, 1, 1);

    let capture =
        apply_regex_to_file_with_context_with_searcher(tmp.path(), &matcher, &mut searcher)
            .unwrap();

    assert_eq!(capture.matches.len(), 1);
    assert_eq!(capture.matches[0].line_number, 2);
    assert_eq!(capture.snippets.len(), 1);
    let snippet_lines: Vec<_> = capture.snippets[0]
        .lines
        .iter()
        .map(|line| (line.line_number, line.line_content.as_str(), line.is_match))
        .collect();
    assert_eq!(
        snippet_lines,
        vec![
            (1, "line one", false),
            (2, "fn hello_world() {}", true),
            (3, "line three", false),
        ]
    );
}
