use collie_search::symbols::SymbolKind;
use collie_search::symbols::query::parse_query;

#[test]
fn plain_query_has_no_filters() {
    let parsed = parse_query("handler");
    assert!(!parsed.has_filters());
    assert_eq!(parsed.name_pattern, "handler");
}

#[test]
fn kind_filter_parsed() {
    let parsed = parse_query("kind:fn handler");
    assert_eq!(parsed.kind, Some(SymbolKind::Function));
    assert_eq!(parsed.name_pattern, "handler");
}

#[test]
fn lang_filter_parsed() {
    let parsed = parse_query("lang:go handler");
    assert_eq!(parsed.language.as_deref(), Some("go"));
    assert_eq!(parsed.name_pattern, "handler");
}

#[test]
fn path_filter_parsed() {
    let parsed = parse_query("path:pkg/api/ handler");
    assert_eq!(parsed.path_prefix.as_deref(), Some("pkg/api/"));
    assert_eq!(parsed.name_pattern, "handler");
}

#[test]
fn qname_filter_parsed() {
    let parsed = parse_query("kind:method qname:Server::start%");
    assert_eq!(parsed.kind, Some(SymbolKind::Method));
    assert_eq!(
        parsed.qualified_name_pattern.as_deref(),
        Some("Server::start%")
    );
    assert_eq!(parsed.name_pattern, "");
}

#[test]
fn multiple_filters_parsed() {
    let parsed = parse_query("kind:fn lang:go path:cmd/ init%");
    assert_eq!(parsed.kind, Some(SymbolKind::Function));
    assert_eq!(parsed.language.as_deref(), Some("go"));
    assert_eq!(parsed.path_prefix.as_deref(), Some("cmd/"));
    assert_eq!(parsed.name_pattern, "init%");
}

#[test]
fn kind_aliases_normalized() {
    assert_eq!(parse_query("kind:fn").kind, Some(SymbolKind::Function));
    assert_eq!(
        parse_query("kind:function").kind,
        Some(SymbolKind::Function)
    );
    assert_eq!(parse_query("kind:struct").kind, Some(SymbolKind::Struct));
    assert_eq!(parse_query("kind:var").kind, Some(SymbolKind::Variable));
    assert_eq!(parse_query("kind:const").kind, Some(SymbolKind::Constant));
    assert_eq!(parse_query("kind:mod").kind, Some(SymbolKind::Module));
    assert_eq!(parse_query("kind:prop").kind, Some(SymbolKind::Property));
    assert_eq!(parse_query("kind:type").kind, Some(SymbolKind::TypeAlias));
    assert_eq!(parse_query("kind:trait").kind, Some(SymbolKind::Trait));
}

#[test]
fn lang_aliases_normalized() {
    assert_eq!(parse_query("lang:py").language.as_deref(), Some("python"));
    assert_eq!(
        parse_query("lang:ts").language.as_deref(),
        Some("typescript")
    );
    assert_eq!(
        parse_query("lang:js").language.as_deref(),
        Some("javascript")
    );
    assert_eq!(parse_query("lang:rb").language.as_deref(), Some("ruby"));
    assert_eq!(parse_query("lang:kt").language.as_deref(), Some("kotlin"));
    assert_eq!(parse_query("lang:cpp").language.as_deref(), Some("cpp"));
}

#[test]
fn name_pattern_preserves_wildcards() {
    assert_eq!(parse_query("kind:fn init%").name_pattern, "init%");
    assert_eq!(parse_query("kind:fn %handler").name_pattern, "%handler");
    assert_eq!(parse_query("kind:fn %request%").name_pattern, "%request%");
}

#[test]
fn empty_name_pattern_means_all() {
    let parsed = parse_query("kind:fn lang:go");
    assert_eq!(parsed.kind, Some(SymbolKind::Function));
    assert_eq!(parsed.language.as_deref(), Some("go"));
    assert_eq!(parsed.name_pattern, "");
}

#[test]
fn has_filters_returns_true_for_any_filter() {
    assert!(parse_query("kind:fn handler").has_filters());
    assert!(!parse_query("handler").has_filters());
    assert!(parse_query("path:src/ foo").has_filters());
}

#[test]
fn unknown_filter_treated_as_name() {
    let parsed = parse_query("foo:bar handler");
    assert!(!parsed.has_filters());
    assert_eq!(parsed.name_pattern, "foo:bar handler");
}
