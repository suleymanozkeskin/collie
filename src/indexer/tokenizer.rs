use std::str::CharIndices;

use tantivy::tokenizer::{
    LowerCaser, PreTokenizedString, TextAnalyzer, Token as TantivyToken, TokenFilter, TokenStream,
    Tokenizer as TantivyTokenizer,
};

// ---------------------------------------------------------------------------
// Collie's own Token type — used for lazy snippet resolution in cli/search.rs
// ---------------------------------------------------------------------------

/// Represents a token extracted from source code.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Token {
    pub text: String,
    /// Byte offset of the first character in the source content.
    pub position: usize,
}

impl Token {
    pub fn new(text: String, position: usize) -> Self {
        Self { text, position }
    }
}

/// Tokenizer for lazy snippet resolution.
///
/// Splits on non-alphanumeric/non-underscore boundaries, lowercases, min_length=2.
/// This MUST match the `collie_body` Tantivy analyzer's behavior exactly.
pub struct Tokenizer;

impl Tokenizer {
    pub fn new() -> Self {
        Self
    }

    /// Tokenize source code into searchable tokens.
    pub fn tokenize(&self, content: &str) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut current_token = String::new();
        let mut token_start = 0;
        let mut in_token = false;

        for (idx, ch) in content.char_indices() {
            if ch.is_alphanumeric() || ch == '_' {
                if !in_token {
                    token_start = idx;
                    in_token = true;
                }
                current_token.push(ch);
            } else if in_token {
                let lowered = current_token.to_lowercase();
                if lowered.len() >= 2 {
                    tokens.push(Token::new(lowered, token_start));
                }
                current_token.clear();
                in_token = false;
            }
        }

        if in_token {
            let lowered = current_token.to_lowercase();
            if lowered.len() >= 2 {
                tokens.push(Token::new(lowered, token_start));
            }
        }

        tokens
    }
}

impl Default for Tokenizer {
    fn default() -> Self {
        Self::new()
    }
}

/// Tokenize a query string using the same rules as `collie_body`.
/// Returns token texts (lowercased, min length 2).
pub fn tokenize_query(input: &str) -> Vec<String> {
    tokenize_query_with_positions(input)
        .into_iter()
        .map(|(_, text)| text)
        .collect()
}

/// Tokenize a query string using the same rules as `collie_body`.
/// Returns `(position, token)` pairs using analyzer token positions.
pub fn tokenize_query_with_positions(input: &str) -> Vec<(usize, String)> {
    let mut tokens = Vec::new();
    let mut position: usize = 0;
    let mut current_start = 0;
    let mut in_token = false;

    for (idx, ch) in input.char_indices() {
        if ch.is_alphanumeric() || ch == '_' {
            if !in_token {
                current_start = idx;
                in_token = true;
            }
        } else if in_token {
            let lowered = input[current_start..idx].to_lowercase();
            if lowered.len() >= 2 {
                tokens.push((position, lowered));
            }
            position += 1;
            in_token = false;
        }
    }

    if in_token {
        let lowered = input[current_start..].to_lowercase();
        if lowered.len() >= 2 {
            tokens.push((position, lowered));
        }
    }

    tokens
}

// ---------------------------------------------------------------------------
// Tantivy tokenizer: CollieTokenizer
// Splits on non-alphanumeric/non-underscore. Like SimpleTokenizer but keeps `_`.
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
pub struct CollieTokenizer {
    token: TantivyToken,
}

pub struct CollieTokenStream<'a> {
    text: &'a str,
    chars: CharIndices<'a>,
    token: &'a mut TantivyToken,
}

impl TantivyTokenizer for CollieTokenizer {
    type TokenStream<'a> = CollieTokenStream<'a>;
    fn token_stream<'a>(&'a mut self, text: &'a str) -> CollieTokenStream<'a> {
        self.token.reset();
        CollieTokenStream {
            text,
            chars: text.char_indices(),
            token: &mut self.token,
        }
    }
}

impl<'a> CollieTokenStream<'a> {
    fn search_token_end(&mut self) -> usize {
        for (offset, c) in &mut self.chars {
            if !c.is_alphanumeric() && c != '_' {
                return offset;
            }
        }
        self.text.len()
    }
}

impl<'a> TokenStream for CollieTokenStream<'a> {
    fn advance(&mut self) -> bool {
        self.token.text.clear();
        self.token.position = self.token.position.wrapping_add(1);
        while let Some((offset_from, c)) = self.chars.next() {
            if c.is_alphanumeric() || c == '_' {
                let offset_to = self.search_token_end();
                self.token.offset_from = offset_from;
                self.token.offset_to = offset_to;
                self.token.text.push_str(&self.text[offset_from..offset_to]);
                return true;
            }
        }
        false
    }

    fn token(&self) -> &TantivyToken {
        self.token
    }

    fn token_mut(&mut self) -> &mut TantivyToken {
        self.token
    }
}

// ---------------------------------------------------------------------------
// RemoveShortFilter — drops tokens shorter than min_length bytes
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct RemoveShortFilter {
    min_length: usize,
}

impl RemoveShortFilter {
    pub fn min_length(min_length: usize) -> Self {
        Self { min_length }
    }
}

impl TokenFilter for RemoveShortFilter {
    type Tokenizer<T: TantivyTokenizer> = RemoveShortFilterWrapper<T>;

    fn transform<T: TantivyTokenizer>(self, tokenizer: T) -> RemoveShortFilterWrapper<T> {
        RemoveShortFilterWrapper {
            min_length: self.min_length,
            inner: tokenizer,
        }
    }
}

#[derive(Clone)]
pub struct RemoveShortFilterWrapper<T: TantivyTokenizer> {
    min_length: usize,
    inner: T,
}

impl<T: TantivyTokenizer> TantivyTokenizer for RemoveShortFilterWrapper<T> {
    type TokenStream<'a> = RemoveShortFilterStream<T::TokenStream<'a>>;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        RemoveShortFilterStream {
            min_length: self.min_length,
            tail: self.inner.token_stream(text),
        }
    }
}

pub struct RemoveShortFilterStream<T> {
    min_length: usize,
    tail: T,
}

impl<T: TokenStream> TokenStream for RemoveShortFilterStream<T> {
    fn advance(&mut self) -> bool {
        while self.tail.advance() {
            if self.tail.token().text.len() >= self.min_length {
                return true;
            }
        }
        false
    }

    fn token(&self) -> &TantivyToken {
        self.tail.token()
    }

    fn token_mut(&mut self) -> &mut TantivyToken {
        self.tail.token_mut()
    }
}

// ---------------------------------------------------------------------------
// ReverseFilter — reverses each token's text
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ReverseFilter;

impl TokenFilter for ReverseFilter {
    type Tokenizer<T: TantivyTokenizer> = ReverseFilterWrapper<T>;

    fn transform<T: TantivyTokenizer>(self, tokenizer: T) -> ReverseFilterWrapper<T> {
        ReverseFilterWrapper { inner: tokenizer }
    }
}

#[derive(Clone)]
pub struct ReverseFilterWrapper<T: TantivyTokenizer> {
    inner: T,
}

impl<T: TantivyTokenizer> TantivyTokenizer for ReverseFilterWrapper<T> {
    type TokenStream<'a> = ReverseFilterStream<T::TokenStream<'a>>;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        ReverseFilterStream {
            tail: self.inner.token_stream(text),
        }
    }
}

pub struct ReverseFilterStream<T> {
    tail: T,
}

impl<T: TokenStream> TokenStream for ReverseFilterStream<T> {
    fn advance(&mut self) -> bool {
        if !self.tail.advance() {
            return false;
        }
        let reversed: String = self.tail.token().text.chars().rev().collect();
        self.tail.token_mut().text = reversed;
        true
    }

    fn token(&self) -> &TantivyToken {
        self.tail.token()
    }

    fn token_mut(&mut self) -> &mut TantivyToken {
        self.tail.token_mut()
    }
}

// ---------------------------------------------------------------------------
// IdentPartTokenizer — splits identifiers on camelCase/snake_case boundaries
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
pub struct IdentPartTokenizer {
    token: TantivyToken,
}

pub struct IdentPartTokenStream<'a> {
    parts: Vec<(usize, usize)>, // (from, to) byte offsets into text
    text: &'a str,
    index: usize,
    token: &'a mut TantivyToken,
}

impl TantivyTokenizer for IdentPartTokenizer {
    type TokenStream<'a> = IdentPartTokenStream<'a>;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> IdentPartTokenStream<'a> {
        self.token.reset();
        let parts = split_identifier_parts(text);
        IdentPartTokenStream {
            parts,
            text,
            index: 0,
            token: &mut self.token,
        }
    }
}

impl<'a> TokenStream for IdentPartTokenStream<'a> {
    fn advance(&mut self) -> bool {
        if self.index >= self.parts.len() {
            return false;
        }
        let (from, to) = self.parts[self.index];
        self.token.text.clear();
        self.token.text.push_str(&self.text[from..to]);
        self.token.offset_from = from;
        self.token.offset_to = to;
        self.token.position = self.token.position.wrapping_add(1);
        self.index += 1;
        true
    }

    fn token(&self) -> &TantivyToken {
        self.token
    }

    fn token_mut(&mut self) -> &mut TantivyToken {
        self.token
    }
}

/// Split an identifier into its constituent parts.
///
/// Rules:
/// - `_` is a separator (consumed, not emitted)
/// - camelCase boundary: lowercase followed by uppercase starts a new part
/// - PascalCase/acronym boundary: uppercase followed by uppercase+lowercase
///   (e.g. `HTTPServer` → `HTTP`, `Server`)
/// - digit/letter boundary: transition between digits and letters starts a new part
fn split_identifier_parts(text: &str) -> Vec<(usize, usize)> {
    let mut parts = Vec::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    if chars.is_empty() {
        return parts;
    }

    let mut part_start: Option<usize> = None;

    for i in 0..chars.len() {
        let (byte_idx, ch) = chars[i];

        // Underscores are separators
        if ch == '_' {
            if let Some(start) = part_start {
                parts.push((start, byte_idx));
                part_start = None;
            }
            continue;
        }

        // Non-alphanumeric (shouldn't happen in identifiers, but be safe)
        if !ch.is_alphanumeric() {
            if let Some(start) = part_start {
                parts.push((start, byte_idx));
                part_start = None;
            }
            continue;
        }

        if part_start.is_none() {
            part_start = Some(byte_idx);
            continue;
        }

        // Check if we should split before this character
        let prev_ch = chars[i - 1].1;
        let should_split = if prev_ch.is_lowercase() && ch.is_uppercase() {
            // camelCase: handleRequest → handle | Request
            true
        } else if prev_ch.is_uppercase() && ch.is_uppercase() {
            // Check if next char is lowercase (acronym end): HTTPServer → HTTP | Server
            if i + 1 < chars.len() && chars[i + 1].1.is_lowercase() {
                true
            } else {
                false
            }
        } else if prev_ch.is_ascii_digit() != ch.is_ascii_digit() {
            // digit/letter boundary: Http2Server → Http | 2 | Server
            true
        } else {
            false
        };

        if should_split {
            if let Some(start) = part_start {
                parts.push((start, byte_idx));
            }
            part_start = Some(byte_idx);
        }
    }

    // Flush final part
    if let Some(start) = part_start {
        parts.push((start, text.len()));
    }

    parts
}

// ---------------------------------------------------------------------------
// QNamePartTokenizer — like IdentPartTokenizer but also splits on ::, ., /
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
pub struct QNamePartTokenizer {
    token: TantivyToken,
}

pub struct QNamePartTokenStream<'a> {
    parts: Vec<(usize, usize)>,
    text: &'a str,
    index: usize,
    token: &'a mut TantivyToken,
}

impl TantivyTokenizer for QNamePartTokenizer {
    type TokenStream<'a> = QNamePartTokenStream<'a>;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> QNamePartTokenStream<'a> {
        self.token.reset();
        let parts = split_qualified_name_parts(text);
        QNamePartTokenStream {
            parts,
            text,
            index: 0,
            token: &mut self.token,
        }
    }
}

impl<'a> TokenStream for QNamePartTokenStream<'a> {
    fn advance(&mut self) -> bool {
        if self.index >= self.parts.len() {
            return false;
        }
        let (from, to) = self.parts[self.index];
        self.token.text.clear();
        self.token.text.push_str(&self.text[from..to]);
        self.token.offset_from = from;
        self.token.offset_to = to;
        self.token.position = self.token.position.wrapping_add(1);
        self.index += 1;
        true
    }

    fn token(&self) -> &TantivyToken {
        self.token
    }

    fn token_mut(&mut self) -> &mut TantivyToken {
        self.token
    }
}

/// Split a qualified name into parts.
/// First splits on `::`, `.`, `/`, then applies identifier-part splitting on each segment.
fn split_qualified_name_parts(text: &str) -> Vec<(usize, usize)> {
    let mut all_parts = Vec::new();
    let mut segment_start = 0;
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let is_separator = if bytes[i] == b'.' || bytes[i] == b'/' {
            true
        } else if bytes[i] == b':' && i + 1 < len && bytes[i + 1] == b':' {
            true
        } else {
            false
        };

        if is_separator {
            if i > segment_start {
                let segment = &text[segment_start..i];
                let offset = segment_start;
                for (from, to) in split_identifier_parts(segment) {
                    all_parts.push((offset + from, offset + to));
                }
            }
            // Skip separator
            if bytes[i] == b':' {
                i += 2; // skip ::
            } else {
                i += 1; // skip . or /
            }
            segment_start = i;
        } else {
            i += 1;
        }
    }

    // Flush final segment
    if segment_start < len {
        let segment = &text[segment_start..];
        let offset = segment_start;
        for (from, to) in split_identifier_parts(segment) {
            all_parts.push((offset + from, offset + to));
        }
    }

    all_parts
}

// ---------------------------------------------------------------------------
// Factory functions — build TextAnalyzer instances for registration
// ---------------------------------------------------------------------------

/// Body tokenizer: split on non-alnum/non-underscore, lowercase, min_length=2.
pub fn collie_body_analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(CollieTokenizer::default())
        .filter(LowerCaser)
        .filter(RemoveShortFilter::min_length(2))
        .build()
}

/// Reversed body tokenizer: same as body but reverses each token.
pub fn collie_body_reversed_analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(CollieTokenizer::default())
        .filter(LowerCaser)
        .filter(RemoveShortFilter::min_length(2))
        .filter(ReverseFilter)
        .build()
}

/// Identifier-part tokenizer: split camelCase/snake_case, lowercase, min_length=2.
pub fn collie_ident_parts_analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(IdentPartTokenizer::default())
        .filter(LowerCaser)
        .filter(RemoveShortFilter::min_length(2))
        .build()
}

/// Qualified-name part tokenizer: split on ::, ., / then camelCase/snake_case.
pub fn collie_qname_parts_analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(QNamePartTokenizer::default())
        .filter(LowerCaser)
        .filter(RemoveShortFilter::min_length(2))
        .build()
}

// ---------------------------------------------------------------------------
// Pre-tokenization for bulk rebuild
// ---------------------------------------------------------------------------

/// Pre-tokenize content for the `body` field. Produces tokens identical to
/// what the `collie_body` analyzer would emit, but as a `PreTokenizedString`
/// that bypasses the analyzer at Tantivy index time.
pub fn pretokenize_body(content: &str) -> PreTokenizedString {
    let mut tokens = Vec::new();
    let mut position: usize = 0;
    let mut current_start = 0;
    let mut in_token = false;

    for (idx, ch) in content.char_indices() {
        if ch.is_alphanumeric() || ch == '_' {
            if !in_token {
                current_start = idx;
                in_token = true;
            }
        } else if in_token {
            let raw = &content[current_start..idx];
            let lowered = raw.to_lowercase();
            if lowered.len() >= 2 {
                tokens.push(TantivyToken {
                    offset_from: current_start,
                    offset_to: idx,
                    position,
                    text: lowered,
                    position_length: 1,
                });
            }
            position += 1;
            in_token = false;
        }
    }
    if in_token {
        let raw = &content[current_start..];
        let lowered = raw.to_lowercase();
        if lowered.len() >= 2 {
            tokens.push(TantivyToken {
                offset_from: current_start,
                offset_to: content.len(),
                position,
                text: lowered,
                position_length: 1,
            });
        }
    }

    PreTokenizedString {
        text: String::new(), // body field is not stored, text is unused
        tokens,
    }
}

/// Pre-tokenize content for the `body_reversed` field. Same as `pretokenize_body`
/// but each token's text is reversed for suffix search support.
pub fn pretokenize_body_reversed(content: &str) -> PreTokenizedString {
    let mut tokens = Vec::new();
    let mut position: usize = 0;
    let mut current_start = 0;
    let mut in_token = false;

    for (idx, ch) in content.char_indices() {
        if ch.is_alphanumeric() || ch == '_' {
            if !in_token {
                current_start = idx;
                in_token = true;
            }
        } else if in_token {
            let raw = &content[current_start..idx];
            let lowered = raw.to_lowercase();
            if lowered.len() >= 2 {
                tokens.push(TantivyToken {
                    offset_from: current_start,
                    offset_to: idx,
                    position,
                    text: lowered.chars().rev().collect(),
                    position_length: 1,
                });
            }
            position += 1;
            in_token = false;
        }
    }
    if in_token {
        let raw = &content[current_start..];
        let lowered = raw.to_lowercase();
        if lowered.len() >= 2 {
            tokens.push(TantivyToken {
                offset_from: current_start,
                offset_to: content.len(),
                position,
                text: lowered.chars().rev().collect(),
                position_length: 1,
            });
        }
    }

    PreTokenizedString {
        text: String::new(),
        tokens,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_tokens(analyzer: &mut TextAnalyzer, text: &str) -> Vec<String> {
        let mut stream = analyzer.token_stream(text);
        let mut tokens = Vec::new();
        while let Some(tok) = stream.next() {
            tokens.push(tok.text.clone());
        }
        tokens
    }

    // --- Lazy snippet tokenizer tests (existing behavior) ---

    #[test]
    fn test_simple_tokenization() {
        let tokenizer = Tokenizer::new();
        let tokens = tokenizer.tokenize("hello world");
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].text, "hello");
        assert_eq!(tokens[1].text, "world");
    }

    #[test]
    fn test_identifier_tokenization() {
        let tokenizer = Tokenizer::new();
        let tokens = tokenizer.tokenize("fn calculate_total() -> i32");
        let texts: Vec<_> = tokens.iter().map(|t| t.text.as_str()).collect();
        assert!(texts.contains(&"fn"));
        assert!(texts.contains(&"calculate_total"));
        assert!(texts.contains(&"i32"));
    }

    #[test]
    fn test_underscore_in_identifier() {
        let tokenizer = Tokenizer::new();
        let tokens = tokenizer.tokenize("my_variable_name");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].text, "my_variable_name");
    }

    #[test]
    fn test_min_length_hardcoded() {
        let tokenizer = Tokenizer::new();
        let tokens = tokenizer.tokenize("a ab abc");
        let texts: Vec<_> = tokens.iter().map(|t| t.text.as_str()).collect();
        assert!(!texts.contains(&"a"));
        assert!(texts.contains(&"ab"));
        assert!(texts.contains(&"abc"));
    }

    #[test]
    fn test_case_normalization() {
        let tokenizer = Tokenizer::new();
        let tokens = tokenizer.tokenize("MyFunction CONSTANT");
        assert_eq!(tokens[0].text, "myfunction");
        assert_eq!(tokens[1].text, "constant");
    }

    #[test]
    fn test_position_tracking() {
        let tokenizer = Tokenizer::new();
        let tokens = tokenizer.tokenize("fn main()");
        assert_eq!(tokens[0].text, "fn");
        assert_eq!(tokens[0].position, 0);
        assert_eq!(tokens[1].text, "main");
        assert_eq!(tokens[1].position, 3);
    }

    // --- collie_body analyzer tests ---

    #[test]
    fn collie_body_matches_lazy_tokenizer() {
        let tokenizer = Tokenizer::new();
        let lazy_tokens = tokenizer.tokenize("fn calculate_total(numbers: &[i32]) -> i32 { }");
        let lazy_texts: Vec<_> = lazy_tokens.iter().map(|t| t.text.as_str()).collect();

        let mut analyzer = collie_body_analyzer();
        let tantivy_texts = collect_tokens(
            &mut analyzer,
            "fn calculate_total(numbers: &[i32]) -> i32 { }",
        );

        assert_eq!(lazy_texts, tantivy_texts);
    }

    #[test]
    fn collie_body_matches_lazy_tokenizer_for_unicode_casefold() {
        let tokenizer = Tokenizer::new();
        let lazy_tokens = tokenizer.tokenize("K");
        let lazy_texts: Vec<_> = lazy_tokens.iter().map(|t| t.text.as_str()).collect();

        let mut analyzer = collie_body_analyzer();
        let tantivy_texts = collect_tokens(&mut analyzer, "K");

        assert_eq!(lazy_texts, tantivy_texts);
        assert!(lazy_texts.is_empty());
    }

    #[test]
    fn pretokenize_body_matches_analyzer_positions_after_short_token_filtering() {
        let text = "a bc d ef";
        let pre = pretokenize_body(text);

        let mut analyzer = collie_body_analyzer();
        let mut stream = analyzer.token_stream(text);
        let mut analyzed = Vec::new();
        while let Some(token) = stream.next() {
            analyzed.push((
                token.text.clone(),
                token.position,
                token.offset_from,
                token.offset_to,
            ));
        }

        let pretokenized: Vec<_> = pre
            .tokens
            .into_iter()
            .map(|token| {
                (
                    token.text,
                    token.position,
                    token.offset_from,
                    token.offset_to,
                )
            })
            .collect();

        assert_eq!(pretokenized, analyzed);
    }

    #[test]
    fn collie_body_includes_underscore() {
        let mut analyzer = collie_body_analyzer();
        let tokens = collect_tokens(&mut analyzer, "my_variable other_var");
        assert_eq!(tokens, vec!["my_variable", "other_var"]);
    }

    #[test]
    fn collie_body_removes_short() {
        let mut analyzer = collie_body_analyzer();
        let tokens = collect_tokens(&mut analyzer, "a ab abc");
        assert_eq!(tokens, vec!["ab", "abc"]);
    }

    // --- collie_body_reversed analyzer tests ---

    #[test]
    fn collie_body_reversed_reverses_tokens() {
        let mut analyzer = collie_body_reversed_analyzer();
        let tokens = collect_tokens(&mut analyzer, "hello world");
        assert_eq!(tokens, vec!["olleh", "dlrow"]);
    }

    #[test]
    fn collie_body_reversed_handles_underscore() {
        let mut analyzer = collie_body_reversed_analyzer();
        let tokens = collect_tokens(&mut analyzer, "my_var");
        assert_eq!(tokens, vec!["rav_ym"]);
    }

    // --- IdentPartTokenizer tests ---

    #[test]
    fn ident_parts_camel_case() {
        let mut analyzer = collie_ident_parts_analyzer();
        let tokens = collect_tokens(&mut analyzer, "getPayingUsers");
        assert_eq!(tokens, vec!["get", "paying", "users"]);
    }

    #[test]
    fn ident_parts_snake_case() {
        let mut analyzer = collie_ident_parts_analyzer();
        let tokens = collect_tokens(&mut analyzer, "new_webhook_token");
        assert_eq!(tokens, vec!["new", "webhook", "token"]);
    }

    #[test]
    fn ident_parts_pascal_case() {
        let mut analyzer = collie_ident_parts_analyzer();
        let tokens = collect_tokens(&mut analyzer, "SharedInformerFactory");
        assert_eq!(tokens, vec!["shared", "informer", "factory"]);
    }

    #[test]
    fn ident_parts_screaming_snake() {
        let mut analyzer = collie_ident_parts_analyzer();
        let tokens = collect_tokens(&mut analyzer, "MAX_RETRY_COUNT");
        assert_eq!(tokens, vec!["max", "retry", "count"]);
    }

    #[test]
    fn ident_parts_digit_boundary() {
        let mut analyzer = collie_ident_parts_analyzer();
        let tokens = collect_tokens(&mut analyzer, "Http2Server");
        // Http | 2 | Server → http, server (2 is dropped by min_length=2)
        assert_eq!(tokens, vec!["http", "server"]);
    }

    #[test]
    fn ident_parts_acronym() {
        let mut analyzer = collie_ident_parts_analyzer();
        let tokens = collect_tokens(&mut analyzer, "HTTPServer");
        assert_eq!(tokens, vec!["http", "server"]);
    }

    #[test]
    fn ident_parts_single_word() {
        let mut analyzer = collie_ident_parts_analyzer();
        let tokens = collect_tokens(&mut analyzer, "handler");
        assert_eq!(tokens, vec!["handler"]);
    }

    // --- QNamePartTokenizer tests ---

    #[test]
    fn qname_parts_double_colon() {
        let mut analyzer = collie_qname_parts_analyzer();
        let tokens = collect_tokens(&mut analyzer, "Server::handleRequest");
        assert_eq!(tokens, vec!["server", "handle", "request"]);
    }

    #[test]
    fn qname_parts_dot_separator() {
        let mut analyzer = collie_qname_parts_analyzer();
        let tokens = collect_tokens(&mut analyzer, "pkg.api.Handler");
        assert_eq!(tokens, vec!["pkg", "api", "handler"]);
    }

    #[test]
    fn qname_parts_slash_separator() {
        let mut analyzer = collie_qname_parts_analyzer();
        let tokens = collect_tokens(&mut analyzer, "pkg/api/handler");
        assert_eq!(tokens, vec!["pkg", "api", "handler"]);
    }

    #[test]
    fn qname_parts_mixed() {
        let mut analyzer = collie_qname_parts_analyzer();
        let tokens = collect_tokens(&mut analyzer, "pkg.api.Server::handleRequest");
        assert_eq!(tokens, vec!["pkg", "api", "server", "handle", "request"]);
    }
}
