use collie_search::regex_search::{apply_regex_to_file, extract_candidate_query, CandidateQuery};
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

// --- Regex file application ---

#[test]
fn apply_regex_finds_matching_lines() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    write!(tmp.as_file(), "line one\nfn hello_world() {{}}\nline three\n").unwrap();
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
    write!(
        tmp.as_file(),
        "start\nmatch across\nlines end\nafter\n"
    )
    .unwrap();
    let re = regex::RegexBuilder::new("match across\nlines")
        .multi_line(true)
        .dot_matches_new_line(true)
        .build()
        .unwrap();
    let matches = apply_regex_to_file(tmp.path(), &re, true).unwrap();
    assert!(matches.iter().any(|m| m.line_number == 2));
    assert!(matches.iter().any(|m| m.line_number == 3));
}
