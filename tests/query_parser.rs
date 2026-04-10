use collie_search::symbols::SymbolKind;
use collie_search::symbols::query::parse_query;

fn assert_kinds(input: &str, expected: &[SymbolKind]) {
    let parsed = parse_query(input);
    assert_eq!(parsed.kinds, expected);
}

#[test]
fn plain_query_has_no_filters() {
    let parsed = parse_query("handler");
    assert!(!parsed.has_filters());
    assert_eq!(parsed.name_pattern, "handler");
}

#[test]
fn kind_filter_parsed() {
    let parsed = parse_query("kind:fn handler");
    assert_eq!(parsed.kinds, vec![SymbolKind::Function, SymbolKind::Method]);
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
    assert_eq!(parsed.kinds, vec![SymbolKind::Method]);
    assert_eq!(
        parsed.qualified_name_pattern.as_deref(),
        Some("Server::start%")
    );
    assert_eq!(parsed.name_pattern, "");
}

#[test]
fn multiple_filters_parsed() {
    let parsed = parse_query("kind:fn lang:go path:cmd/ init%");
    assert_eq!(parsed.kinds, vec![SymbolKind::Function, SymbolKind::Method]);
    assert_eq!(parsed.language.as_deref(), Some("go"));
    assert_eq!(parsed.path_prefix.as_deref(), Some("cmd/"));
    assert_eq!(parsed.name_pattern, "init%");
}

#[test]
fn kind_aliases_normalized() {
    assert_kinds("kind:fn", &[SymbolKind::Function, SymbolKind::Method]);
    assert_kinds("kind:function", &[SymbolKind::Function, SymbolKind::Method]);
    assert_kinds("kind:struct", &[SymbolKind::Struct]);
    assert_kinds("kind:var", &[SymbolKind::Variable]);
    assert_kinds("kind:const", &[SymbolKind::Constant]);
    assert_kinds("kind:mod", &[SymbolKind::Module]);
    assert_kinds("kind:prop", &[SymbolKind::Property]);
    assert_kinds("kind:type", &[SymbolKind::TypeAlias]);
    assert_kinds("kind:trait", &[SymbolKind::Trait]);
}

#[test]
fn lang_aliases_normalized() {
    assert_eq!(parse_query("lang:py").language.as_deref(), Some("python"));
    assert_eq!(
        parse_query("lang:ts").language.as_deref(),
        Some("typescript")
    );
    assert_eq!(parse_query("lang:rb").language.as_deref(), Some("ruby"));
    assert_eq!(parse_query("lang:cpp").language.as_deref(), Some("cpp"));
    assert_eq!(parse_query("lang:cs").language.as_deref(), Some("csharp"));
}

#[test]
fn unsupported_lang_filter_is_explicitly_invalid() {
    let parsed = parse_query("kind:fn lang:js handler");
    assert_eq!(parsed.kinds, vec![SymbolKind::Function, SymbolKind::Method]);
    assert_eq!(
        parsed.invalid_filter(),
        Some("unsupported language filter: js")
    );
    assert_eq!(parsed.name_pattern, "");
}

#[test]
fn unsupported_kind_filter_is_explicitly_invalid() {
    let parsed = parse_query("kind:callable handler");
    assert_eq!(parsed.invalid_filter(), Some("unsupported kind filter: callable"));
    assert_eq!(parsed.name_pattern, "");
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
    assert_eq!(parsed.kinds, vec![SymbolKind::Function, SymbolKind::Method]);
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
