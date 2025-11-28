use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tantivy::collector::{DocSetCollector, TopDocs};
use tantivy::indexer::NoMergePolicy;
use tantivy::query::{BooleanQuery, Occur, RegexQuery, TermQuery};
use tantivy::schema::{
    FAST, Field, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions, Value,
};
use tantivy::tokenizer::PreTokenizedString;
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term, doc};

use crate::indexer::tokenizer;
use crate::symbols::{Symbol, SymbolKind, SymbolQuery, SymbolResult};

pub struct SearchResult {
    pub file_path: PathBuf,
}

/// Statistics derived entirely from the Tantivy index.
pub struct TantivyStats {
    pub unique_terms: usize,
    pub file_count: usize,
    pub segment_count: usize,
}

struct TantivySchema {
    doc_type: Field,
    file_path: Field,
    // File doc fields
    body: Field,
    body_reversed: Field,
    // Symbol doc fields
    sym_name: Field,
    sym_name_reversed: Field,
    sym_name_original: Field,
    sym_name_parts: Field,
    sym_qualified_name: Field,
    sym_qualified_name_original: Field,
    sym_qualified_name_parts: Field,
    sym_kind: Field,
    sym_language: Field,
    sym_repo_rel_path: Field,
    sym_repo_rel_path_lower: Field,
    sym_container_name: Field,
    sym_visibility: Field,
    sym_signature: Field,
    sym_line_start: Field,
    sym_line_end: Field,
    sym_byte_start: Field,
    sym_byte_end: Field,
    sym_doc_field: Field,
}

pub struct TantivyIndex {
    index: Index,
    reader: IndexReader,
    writer: Option<IndexWriter>,
    schema: TantivySchema,
    no_merge: bool,
    writer_heap_bytes: usize,
}

const COLLIE_BODY: &str = "collie_body";
const COLLIE_BODY_REVERSED: &str = "collie_body_reversed";
const COLLIE_IDENT_PARTS: &str = "collie_ident_parts";
const COLLIE_QNAME_PARTS: &str = "collie_qname_parts";

fn build_schema() -> Schema {
    let mut builder = Schema::builder();

    builder.add_text_field("doc_type", STRING | STORED);
    builder.add_text_field("file_path", STRING | STORED);

    // File doc fields — body uses custom tokenizer, not stored
    let body_indexing = TextFieldIndexing::default()
        .set_tokenizer(COLLIE_BODY)
        .set_index_option(IndexRecordOption::WithFreqsAndPositions);
    let body_opts = TextOptions::default().set_indexing_options(body_indexing);
    builder.add_text_field("body", body_opts);

    let body_rev_indexing = TextFieldIndexing::default()
        .set_tokenizer(COLLIE_BODY_REVERSED)
        .set_index_option(IndexRecordOption::Basic);
    let body_rev_opts = TextOptions::default().set_indexing_options(body_rev_indexing);
    builder.add_text_field("body_reversed", body_rev_opts);

    // Symbol doc fields
    let raw_indexing = TextFieldIndexing::default()
        .set_tokenizer("raw")
        .set_index_option(IndexRecordOption::Basic);
    let raw_opts = TextOptions::default()
        .set_indexing_options(raw_indexing)
        .set_stored();

    builder.add_text_field("sym_name", raw_opts.clone());
    builder.add_text_field("sym_name_reversed", raw_opts.clone());
    builder.add_text_field("sym_name_original", STORED);

    let ident_parts_indexing = TextFieldIndexing::default()
        .set_tokenizer(COLLIE_IDENT_PARTS)
        .set_index_option(IndexRecordOption::Basic);
    let ident_parts_opts = TextOptions::default().set_indexing_options(ident_parts_indexing);
    builder.add_text_field("sym_name_parts", ident_parts_opts);

    builder.add_text_field("sym_qualified_name", raw_opts.clone());
    builder.add_text_field("sym_qualified_name_original", STORED);

    let qname_parts_indexing = TextFieldIndexing::default()
        .set_tokenizer(COLLIE_QNAME_PARTS)
        .set_index_option(IndexRecordOption::Basic);
    let qname_parts_opts = TextOptions::default().set_indexing_options(qname_parts_indexing);
    builder.add_text_field("sym_qualified_name_parts", qname_parts_opts);

    builder.add_text_field("sym_kind", STRING | STORED);
    builder.add_text_field("sym_language", STRING | STORED);
    builder.add_text_field("sym_repo_rel_path", STRING | STORED);
    builder.add_text_field("sym_repo_rel_path_lower", STRING);
    builder.add_text_field("sym_container_name", STORED);
    builder.add_text_field("sym_visibility", STORED);
    builder.add_text_field("sym_signature", STORED);
    builder.add_u64_field("sym_line_start", FAST | STORED);
    builder.add_u64_field("sym_line_end", STORED);
    builder.add_u64_field("sym_byte_start", STORED);
    builder.add_u64_field("sym_byte_end", STORED);
    builder.add_text_field("sym_doc", STORED);

    builder.build()
}

impl TantivySchema {
    fn from_index(index: &Index) -> Result<Self> {
        let schema = index.schema();
        let get = |name: &str| -> Result<Field> {
            schema.get_field(name).map_err(|_| {
                anyhow::anyhow!(
                    "missing field '{}' — index rebuild required (run 'collie watch .')",
                    name
                )
            })
        };
        Ok(Self {
            doc_type: get("doc_type")?,
            file_path: get("file_path")?,
            body: get("body")?,
            body_reversed: get("body_reversed")?,
            sym_name: get("sym_name")?,
            sym_name_reversed: get("sym_name_reversed")?,
            sym_name_original: get("sym_name_original")?,
            sym_name_parts: get("sym_name_parts")?,
            sym_qualified_name: get("sym_qualified_name")?,
            sym_qualified_name_original: get("sym_qualified_name_original")?,
            sym_qualified_name_parts: get("sym_qualified_name_parts")?,
            sym_kind: get("sym_kind")?,
            sym_language: get("sym_language")?,
            sym_repo_rel_path: get("sym_repo_rel_path")?,
            sym_repo_rel_path_lower: get("sym_repo_rel_path_lower")?,
            sym_container_name: get("sym_container_name")?,
            sym_visibility: get("sym_visibility")?,
            sym_signature: get("sym_signature")?,
            sym_line_start: get("sym_line_start")?,
            sym_line_end: get("sym_line_end")?,
            sym_byte_start: get("sym_byte_start")?,
            sym_byte_end: get("sym_byte_end")?,
            sym_doc_field: get("sym_doc")?,
        })
    }
}

fn register_tokenizers(index: &Index) {
    let tokenizers = index.tokenizers();
    tokenizers.register(COLLIE_BODY, tokenizer::collie_body_analyzer());
    tokenizers.register(
        COLLIE_BODY_REVERSED,
        tokenizer::collie_body_reversed_analyzer(),
    );
    tokenizers.register(COLLIE_IDENT_PARTS, tokenizer::collie_ident_parts_analyzer());
    tokenizers.register(COLLIE_QNAME_PARTS, tokenizer::collie_qname_parts_analyzer());
}

impl TantivyIndex {
    pub fn open(index_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(index_dir)
            .with_context(|| format!("failed to create tantivy dir {:?}", index_dir))?;

        let desired_schema = build_schema();

        let index = if index_dir.join("meta.json").exists() {
            Index::open_in_dir(index_dir)
                .with_context(|| format!("failed to open tantivy index at {:?}", index_dir))?
        } else {
            Index::create_in_dir(index_dir, desired_schema)
                .with_context(|| format!("failed to create tantivy index at {:?}", index_dir))?
        };

        register_tokenizers(&index);

        let fields = TantivySchema::from_index(&index)
            .context("index schema mismatch — rebuild required (run 'collie watch .')")?;

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .context("failed to create tantivy reader")?;

        Ok(Self {
            index,
            reader,
            writer: None,
            schema: fields,
            no_merge: false,
            writer_heap_bytes: 15_000_000,
        })
    }

    /// Disable segment merging. Call before any writes.
    pub fn set_no_merge(&mut self) {
        self.no_merge = true;
    }

    /// Set the writer heap budget in bytes. Call before any writes.
    pub fn set_writer_heap(&mut self, bytes: usize) {
        self.writer_heap_bytes = bytes;
    }

    fn ensure_writer(&mut self) -> Result<&mut IndexWriter> {
        if self.writer.is_none() {
            let writer = self
                .index
                .writer(self.writer_heap_bytes)
                .context("failed to create tantivy writer")?;
            if self.no_merge {
                writer.set_merge_policy(Box::new(NoMergePolicy));
            }
            self.writer = Some(writer);
        }
        Ok(self.writer.as_mut().unwrap())
    }

    /// Compact segments by dropping the current writer, opening a new one
    /// with the default merge policy, committing to trigger merges, and
    /// waiting for merge threads to finish. Restores NoMerge policy
    /// afterward if it was set. Returns the new segment count.
    pub fn compact(&mut self) -> Result<usize> {
        // Commit and drop the current writer to release the lock
        if let Some(ref mut w) = self.writer {
            w.commit().context("compact: pre-commit failed")?;
        }
        self.writer = None;

        // Open a new writer with default merge policy
        let mut writer: IndexWriter = self
            .index
            .writer(self.writer_heap_bytes)
            .context("compact: failed to create merge writer")?;
        // Commit triggers the default merge policy to schedule merges
        writer.commit().context("compact: merge commit failed")?;
        writer
            .wait_merging_threads()
            .context("compact: merge wait failed")?;

        self.reader.reload().context("compact: reader reload failed")?;
        let segment_count = self.reader.searcher().segment_readers().len();

        // Re-create writer with the appropriate merge policy for continued use
        if self.no_merge {
            let writer = self
                .index
                .writer(self.writer_heap_bytes)
                .context("compact: failed to recreate writer")?;
            writer.set_merge_policy(Box::new(NoMergePolicy));
            self.writer = Some(writer);
        }
        // If not no_merge, leave writer as None — ensure_writer will create on demand

        Ok(segment_count)
    }

    /// Remove all docs (file + symbol) for a given file path.
    pub fn remove_by_path(&mut self, file_path: &Path) -> Result<()> {
        let file_path_field = self.schema.file_path;
        let file_path_str = file_path.to_string_lossy().to_string();
        self.ensure_writer()?;
        let writer = self.writer.as_mut().unwrap();
        let term = Term::from_field_text(file_path_field, &file_path_str);
        writer.delete_term(term);
        Ok(())
    }

    /// Add a single file doc. Tantivy's collie_body analyzer tokenizes the content.
    pub fn index_file_content(&mut self, file_path: &Path, content: &str) -> Result<()> {
        let doc_type_field = self.schema.doc_type;
        let file_path_field = self.schema.file_path;
        let body_field = self.schema.body;
        let body_reversed_field = self.schema.body_reversed;
        let file_path_str = file_path.to_string_lossy().to_string();

        self.ensure_writer()?;
        let writer = self.writer.as_mut().unwrap();

        writer.add_document(doc!(
            doc_type_field => "file",
            file_path_field => file_path_str,
            body_field => content,
            body_reversed_field => content,
        ))?;

        Ok(())
    }

    /// Add a single file doc with pre-tokenized body fields. Bypasses the
    /// collie_body analyzer entirely — tokens must already be lowercased and
    /// filtered. Used by bulk rebuild where tokenization runs in rayon workers.
    pub fn index_file_content_pretokenized(
        &mut self,
        file_path: &Path,
        body_tokens: PreTokenizedString,
        body_reversed_tokens: PreTokenizedString,
    ) -> Result<()> {
        let file_path_str = file_path.to_string_lossy().to_string();

        self.ensure_writer()?;
        let writer = self.writer.as_mut().unwrap();

        let mut doc = TantivyDocument::new();
        doc.add_text(self.schema.doc_type, "file");
        doc.add_text(self.schema.file_path, &file_path_str);
        doc.add_pre_tokenized_text(self.schema.body, body_tokens);
        doc.add_pre_tokenized_text(self.schema.body_reversed, body_reversed_tokens);
        writer.add_document(doc)?;

        Ok(())
    }

    /// Add symbol docs for a file. Caller must call `remove_by_path` first.
    pub fn index_symbols(&mut self, file_path: &Path, symbols: &[Symbol]) -> Result<()> {
        let doc_type_f = self.schema.doc_type;
        let file_path_f = self.schema.file_path;
        let sym_name_f = self.schema.sym_name;
        let sym_name_reversed_f = self.schema.sym_name_reversed;
        let sym_name_original_f = self.schema.sym_name_original;
        let sym_name_parts_f = self.schema.sym_name_parts;
        let sym_qname_f = self.schema.sym_qualified_name;
        let sym_qname_original_f = self.schema.sym_qualified_name_original;
        let sym_qname_parts_f = self.schema.sym_qualified_name_parts;
        let sym_kind_f = self.schema.sym_kind;
        let sym_language_f = self.schema.sym_language;
        let sym_rrp_f = self.schema.sym_repo_rel_path;
        let sym_rrp_lower_f = self.schema.sym_repo_rel_path_lower;
        let sym_container_f = self.schema.sym_container_name;
        let sym_visibility_f = self.schema.sym_visibility;
        let sym_signature_f = self.schema.sym_signature;
        let sym_line_start_f = self.schema.sym_line_start;
        let sym_line_end_f = self.schema.sym_line_end;
        let sym_byte_start_f = self.schema.sym_byte_start;
        let sym_byte_end_f = self.schema.sym_byte_end;
        let sym_doc_f = self.schema.sym_doc_field;
        let file_path_str = file_path.to_string_lossy().to_string();

        self.ensure_writer()?;
        let writer = self.writer.as_mut().unwrap();

        for symbol in symbols {
            let name_lower = symbol.name.to_lowercase();
            let name_reversed: String = name_lower.chars().rev().collect();
            let qualified_name_lower = symbol
                .qualified_name
                .as_deref()
                .map(|s| s.to_lowercase())
                .unwrap_or_default();
            let qualified_name_original =
                symbol.qualified_name.as_deref().unwrap_or("").to_string();
            let repo_rel_path = symbol.repo_rel_path.to_string_lossy().to_string();
            let repo_rel_path_lower = repo_rel_path.to_lowercase();

            writer.add_document(doc!(
                doc_type_f => "symbol",
                file_path_f => file_path_str.clone(),
                sym_name_f => name_lower,
                sym_name_reversed_f => name_reversed,
                sym_name_original_f => symbol.name.clone(),
                sym_name_parts_f => symbol.name.clone(),
                sym_qname_f => qualified_name_lower,
                sym_qname_original_f => qualified_name_original.clone(),
                sym_qname_parts_f => qualified_name_original,
                sym_kind_f => symbol.kind.as_str(),
                sym_language_f => symbol.language.to_lowercase(),
                sym_rrp_f => repo_rel_path,
                sym_rrp_lower_f => repo_rel_path_lower,
                sym_container_f => symbol.container_name.as_deref().unwrap_or(""),
                sym_visibility_f => symbol.visibility.as_deref().unwrap_or(""),
                sym_signature_f => symbol.signature.as_deref().unwrap_or(""),
                sym_line_start_f => symbol.line_start as u64,
                sym_line_end_f => symbol.line_end as u64,
                sym_byte_start_f => symbol.byte_start as u64,
                sym_byte_end_f => symbol.byte_end as u64,
                sym_doc_f => symbol.doc.as_deref().unwrap_or(""),
            ))?;
        }

        Ok(())
    }

    /// Search symbols using structured filters.
    pub fn search_symbols(&self, query: &SymbolQuery, limit: usize) -> Result<Vec<SymbolResult>> {
        let mut subqueries: Vec<(Occur, Box<dyn tantivy::query::Query>)> = Vec::new();

        subqueries.push((
            Occur::Must,
            Box::new(TermQuery::new(
                Term::from_field_text(self.schema.doc_type, "symbol"),
                IndexRecordOption::Basic,
            )),
        ));

        if let Some(kind) = query.kind {
            subqueries.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(self.schema.sym_kind, kind.as_str()),
                    IndexRecordOption::Basic,
                )),
            ));
        }

        if let Some(language) = &query.language {
            subqueries.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(self.schema.sym_language, &language.to_lowercase()),
                    IndexRecordOption::Basic,
                )),
            ));
        }

        if let Some(path_prefix) = &query.path_prefix {
            let pattern = format!("{}.*", regex::escape(&path_prefix.to_lowercase()));
            let regex_query =
                RegexQuery::from_pattern(&pattern, self.schema.sym_repo_rel_path_lower)
                    .context("invalid path prefix regex")?;
            subqueries.push((Occur::Must, Box::new(regex_query)));
        }

        if !query.name_pattern.is_empty() {
            let name_query = self.build_name_query(&query.name_pattern)?;
            subqueries.push((Occur::Must, name_query));
        }

        if let Some(qname_pattern) = &query.qualified_name_pattern {
            let qname_query = self.build_qualified_name_query(qname_pattern)?;
            subqueries.push((Occur::Must, qname_query));
        }

        let bool_query = BooleanQuery::new(subqueries);

        let searcher = self.reader.searcher();
        let doc_addresses = searcher
            .search(&bool_query, &DocSetCollector)
            .context("symbol search failed")?;

        let mut results = Vec::with_capacity(doc_addresses.len().min(limit.max(1)));
        for addr in doc_addresses {
            let doc: tantivy::TantivyDocument = searcher.doc(addr)?;
            results.push(self.doc_to_symbol_result(&doc));
        }

        results.sort_by(|a, b| {
            a.repo_rel_path
                .to_string_lossy()
                .to_lowercase()
                .cmp(&b.repo_rel_path.to_string_lossy().to_lowercase())
                .then(a.line_start.cmp(&b.line_start))
                .then(a.kind.as_str().cmp(b.kind.as_str()))
                .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        if limit > 0 {
            results.truncate(limit);
        }

        Ok(results)
    }

    /// Commit pending changes and reload the reader.
    pub fn commit(&mut self) -> Result<()> {
        if let Some(ref mut writer) = self.writer {
            writer.commit().context("tantivy commit failed")?;
        }
        self.reader
            .reload()
            .context("tantivy reader reload failed")?;
        Ok(())
    }

    /// Search for an exact token match.
    pub fn search_exact(&self, token: &str) -> Vec<SearchResult> {
        let normalized = token.to_lowercase();
        let term = Term::from_field_text(self.schema.body, &normalized);
        let query = TermQuery::new(term, IndexRecordOption::Basic);
        self.execute_file_query(&query)
    }

    /// Search for tokens starting with the given prefix.
    pub fn search_prefix(&self, prefix: &str) -> Vec<SearchResult> {
        let normalized = prefix.to_lowercase();
        let pattern = format!("{}.*", regex::escape(&normalized));
        match RegexQuery::from_pattern(&pattern, self.schema.body) {
            Ok(query) => self.execute_file_query(&query),
            Err(_) => Vec::new(),
        }
    }

    /// Search for tokens ending with the given suffix via the reversed field.
    pub fn search_suffix(&self, suffix: &str) -> Vec<SearchResult> {
        let normalized = suffix.to_lowercase();
        let reversed: String = normalized.chars().rev().collect();
        let pattern = format!("{}.*", regex::escape(&reversed));
        match RegexQuery::from_pattern(&pattern, self.schema.body_reversed) {
            Ok(query) => self.execute_file_query(&query),
            Err(_) => Vec::new(),
        }
    }

    /// Search for tokens containing the given substring.
    pub fn search_substring(&self, substring: &str) -> Vec<SearchResult> {
        let normalized = substring.to_lowercase();
        let pattern = format!(".*{}.*", regex::escape(&normalized));
        match RegexQuery::from_pattern(&pattern, self.schema.body) {
            Ok(query) => self.execute_file_query(&query),
            Err(_) => Vec::new(),
        }
    }

    /// Search for files containing ALL given tokens (AND).
    pub fn search_multi_term(&self, tokens: &[String]) -> Vec<SearchResult> {
        let subqueries: Vec<(Occur, Box<dyn tantivy::query::Query>)> = tokens
            .iter()
            .map(|token| {
                (
                    Occur::Must,
                    Box::new(TermQuery::new(
                        Term::from_field_text(self.schema.body, &token.to_lowercase()),
                        IndexRecordOption::Basic,
                    )) as Box<dyn tantivy::query::Query>,
                )
            })
            .collect();
        self.execute_file_query(&BooleanQuery::new(subqueries))
    }

    /// Search for files containing ALL given substrings (AND of substring matches).
    ///
    /// Unlike `search_multi_term` which requires exact token matches, this finds
    /// files where each term appears as a substring of any indexed token.
    /// Used by regex search where extracted literals may be fragments of larger tokens.
    pub fn search_multi_substring(&self, tokens: &[String]) -> Vec<SearchResult> {
        let subqueries: Vec<(Occur, Box<dyn tantivy::query::Query>)> = tokens
            .iter()
            .filter_map(|token| {
                let normalized = token.to_lowercase();
                let pattern = format!(".*{}.*", regex::escape(&normalized));
                RegexQuery::from_pattern(&pattern, self.schema.body)
                    .ok()
                    .map(|q| (Occur::Must, Box::new(q) as Box<dyn tantivy::query::Query>))
            })
            .collect();
        if subqueries.is_empty() {
            return Vec::new();
        }
        self.execute_file_query(&BooleanQuery::new(subqueries))
    }

    /// Return all indexed file paths.
    pub fn list_all_files(&self) -> Vec<SearchResult> {
        let query = TermQuery::new(
            Term::from_field_text(self.schema.doc_type, "file"),
            IndexRecordOption::Basic,
        );
        self.execute_file_query(&query)
    }

    /// Search for an exact token match, ranked by BM25.
    pub fn search_exact_ranked(&self, token: &str, limit: usize) -> Vec<SearchResult> {
        let normalized = token.to_lowercase();
        let term = Term::from_field_text(self.schema.body, &normalized);
        let query = TermQuery::new(term, IndexRecordOption::WithFreqsAndPositions);
        self.execute_file_query_ranked(&query, limit)
    }

    /// Search for tokens starting with prefix, ranked by BM25.
    pub fn search_prefix_ranked(&self, prefix: &str, limit: usize) -> Vec<SearchResult> {
        let normalized = prefix.to_lowercase();
        let pattern = format!("{}.*", regex::escape(&normalized));
        match RegexQuery::from_pattern(&pattern, self.schema.body) {
            Ok(query) => self.execute_file_query_ranked(&query, limit),
            Err(_) => Vec::new(),
        }
    }

    /// Search for tokens ending with suffix, ranked by BM25.
    pub fn search_suffix_ranked(&self, suffix: &str, limit: usize) -> Vec<SearchResult> {
        let normalized = suffix.to_lowercase();
        let reversed: String = normalized.chars().rev().collect();
        let pattern = format!("{}.*", regex::escape(&reversed));
        match RegexQuery::from_pattern(&pattern, self.schema.body_reversed) {
            Ok(query) => self.execute_file_query_ranked(&query, limit),
            Err(_) => Vec::new(),
        }
    }

    /// Search for tokens containing substring, ranked by BM25.
    pub fn search_substring_ranked(
        &self,
        substring: &str,
        limit: usize,
    ) -> Vec<SearchResult> {
        let normalized = substring.to_lowercase();
        let pattern = format!(".*{}.*", regex::escape(&normalized));
        match RegexQuery::from_pattern(&pattern, self.schema.body) {
            Ok(query) => self.execute_file_query_ranked(&query, limit),
            Err(_) => Vec::new(),
        }
    }

    /// Search for files containing ALL given tokens, ranked by BM25.
    pub fn search_multi_term_ranked(
        &self,
        tokens: &[String],
        limit: usize,
    ) -> Vec<SearchResult> {
        let subqueries: Vec<(Occur, Box<dyn tantivy::query::Query>)> = tokens
            .iter()
            .map(|token| {
                (
                    Occur::Must,
                    Box::new(TermQuery::new(
                        Term::from_field_text(self.schema.body, &token.to_lowercase()),
                        IndexRecordOption::WithFreqsAndPositions,
                    )) as Box<dyn tantivy::query::Query>,
                )
            })
            .collect();
        self.execute_file_query_ranked(&BooleanQuery::new(subqueries), limit)
    }

    /// Index statistics derived entirely from the Tantivy index.
    pub fn stats(&self) -> TantivyStats {
        let searcher = self.reader.searcher();
        let mut unique_terms = 0u64;
        let segment_count = searcher.segment_readers().len();

        for segment_reader in searcher.segment_readers() {
            if let Ok(inverted_index) = segment_reader.inverted_index(self.schema.body) {
                let term_dict = inverted_index.terms();
                unique_terms += term_dict.num_terms() as u64;
            }
        }

        // Count file docs via doc_type == "file"
        let file_term = Term::from_field_text(self.schema.doc_type, "file");
        let file_query = TermQuery::new(file_term, IndexRecordOption::Basic);
        let file_count = searcher
            .search(&file_query, &tantivy::collector::Count)
            .unwrap_or(0);

        TantivyStats {
            unique_terms: unique_terms as usize,
            file_count,
            segment_count,
        }
    }

    fn execute_file_query(&self, query: &dyn tantivy::query::Query) -> Vec<SearchResult> {
        let searcher = self.reader.searcher();
        let doc_addresses = match searcher.search(query, &DocSetCollector) {
            Ok(addrs) => addrs,
            Err(_) => return Vec::new(),
        };

        let mut results = Vec::new();

        for addr in doc_addresses {
            let doc: tantivy::TantivyDocument = match searcher.doc(addr) {
                Ok(d) => d,
                Err(_) => continue,
            };

            // Skip symbol docs — only return file docs
            let doc_type = doc
                .get_first(self.schema.doc_type)
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            if doc_type != "file" {
                continue;
            }

            let file_path = match doc
                .get_first(self.schema.file_path)
                .and_then(|v| v.as_str())
            {
                Some(p) => PathBuf::from(p),
                None => continue,
            };

            results.push(SearchResult { file_path });
        }

        results
    }

    fn execute_file_query_ranked(
        &self,
        query: &dyn tantivy::query::Query,
        limit: usize,
    ) -> Vec<SearchResult> {
        let searcher = self.reader.searcher();
        let top_docs = match searcher.search(query, &TopDocs::with_limit(limit)) {
            Ok(docs) => docs,
            Err(_) => return Vec::new(),
        };

        let mut results = Vec::new();
        for (_score, addr) in top_docs {
            let doc: tantivy::TantivyDocument = match searcher.doc(addr) {
                Ok(d) => d,
                Err(_) => continue,
            };

            let doc_type = doc
                .get_first(self.schema.doc_type)
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            if doc_type != "file" {
                continue;
            }

            let file_path = match doc
                .get_first(self.schema.file_path)
                .and_then(|v| v.as_str())
            {
                Some(p) => PathBuf::from(p),
                None => continue,
            };

            results.push(SearchResult { file_path });
        }

        results
    }

    fn build_name_query(&self, pattern: &str) -> Result<Box<dyn tantivy::query::Query>> {
        let normalized = pattern.to_lowercase();

        // Multi-word query without wildcards → identifier-part search
        if !normalized.contains('%') && normalized.contains(' ') {
            let words: Vec<&str> = normalized.split_whitespace().collect();
            let mut sub: Vec<(Occur, Box<dyn tantivy::query::Query>)> = Vec::new();
            for word in words {
                sub.push((
                    Occur::Must,
                    Box::new(TermQuery::new(
                        Term::from_field_text(self.schema.sym_name_parts, word),
                        IndexRecordOption::Basic,
                    )),
                ));
            }
            return Ok(Box::new(BooleanQuery::new(sub)));
        }

        let starts = normalized.starts_with('%');
        let ends = normalized.ends_with('%');

        match (starts, ends) {
            (false, false) => Ok(Box::new(TermQuery::new(
                Term::from_field_text(self.schema.sym_name, &normalized),
                IndexRecordOption::Basic,
            ))),
            (false, true) => {
                let prefix = normalized.trim_end_matches('%');
                let pat = format!("{}.*", regex::escape(prefix));
                Ok(Box::new(
                    RegexQuery::from_pattern(&pat, self.schema.sym_name)
                        .context("invalid name prefix regex")?,
                ))
            }
            (true, false) => {
                let suffix = normalized.trim_start_matches('%');
                let reversed: String = suffix.chars().rev().collect();
                let pat = format!("{}.*", regex::escape(&reversed));
                Ok(Box::new(
                    RegexQuery::from_pattern(&pat, self.schema.sym_name_reversed)
                        .context("invalid name suffix regex")?,
                ))
            }
            (true, true) => {
                let inner = normalized.trim_matches('%');
                if inner.chars().count() < 3 {
                    anyhow::bail!("symbol substring search requires at least 3 chars");
                }
                let pat = format!(".*{}.*", regex::escape(inner));
                Ok(Box::new(
                    RegexQuery::from_pattern(&pat, self.schema.sym_name)
                        .context("invalid name substring regex")?,
                ))
            }
        }
    }

    fn build_qualified_name_query(&self, pattern: &str) -> Result<Box<dyn tantivy::query::Query>> {
        let normalized = pattern.to_lowercase();

        // Multi-word query without wildcards → qualified-name part search
        if !normalized.contains('%') && normalized.contains(' ') {
            let words: Vec<&str> = normalized.split_whitespace().collect();
            let mut sub: Vec<(Occur, Box<dyn tantivy::query::Query>)> = Vec::new();
            for word in words {
                sub.push((
                    Occur::Must,
                    Box::new(TermQuery::new(
                        Term::from_field_text(self.schema.sym_qualified_name_parts, word),
                        IndexRecordOption::Basic,
                    )),
                ));
            }
            return Ok(Box::new(BooleanQuery::new(sub)));
        }

        let starts = normalized.starts_with('%');
        let ends = normalized.ends_with('%');

        match (starts, ends) {
            (false, false) => Ok(Box::new(TermQuery::new(
                Term::from_field_text(self.schema.sym_qualified_name, &normalized),
                IndexRecordOption::Basic,
            ))),
            (false, true) => {
                let prefix = normalized.trim_end_matches('%');
                let pat = format!("{}.*", regex::escape(prefix));
                Ok(Box::new(
                    RegexQuery::from_pattern(&pat, self.schema.sym_qualified_name)
                        .context("invalid qname prefix regex")?,
                ))
            }
            (true, false) => {
                let suffix = normalized.trim_start_matches('%');
                let pat = format!(".*{}", regex::escape(suffix));
                Ok(Box::new(
                    RegexQuery::from_pattern(&pat, self.schema.sym_qualified_name)
                        .context("invalid qname suffix regex")?,
                ))
            }
            (true, true) => {
                let inner = normalized.trim_matches('%');
                if inner.chars().count() < 3 {
                    anyhow::bail!("symbol substring search requires at least 3 chars");
                }
                let pat = format!(".*{}.*", regex::escape(inner));
                Ok(Box::new(
                    RegexQuery::from_pattern(&pat, self.schema.sym_qualified_name)
                        .context("invalid qname substring regex")?,
                ))
            }
        }
    }

    fn doc_to_symbol_result(&self, doc: &tantivy::TantivyDocument) -> SymbolResult {
        let name = doc
            .get_first(self.schema.sym_name_original)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let kind_str = doc
            .get_first(self.schema.sym_kind)
            .and_then(|v| v.as_str())
            .unwrap_or("function");
        let kind = symbol_kind_from_str(kind_str).unwrap_or(SymbolKind::Function);

        let qualified_name = doc
            .get_first(self.schema.sym_qualified_name_original)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);

        let language = doc
            .get_first(self.schema.sym_language)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let repo_rel_path = doc
            .get_first(self.schema.sym_repo_rel_path)
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_default();

        let container_name = doc
            .get_first(self.schema.sym_container_name)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);

        let visibility = doc
            .get_first(self.schema.sym_visibility)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);

        let signature = doc
            .get_first(self.schema.sym_signature)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);

        let line_start = doc
            .get_first(self.schema.sym_line_start)
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        let line_end = doc
            .get_first(self.schema.sym_line_end)
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        let byte_start = doc
            .get_first(self.schema.sym_byte_start)
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        let byte_end = doc
            .get_first(self.schema.sym_byte_end)
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        let sym_doc = doc
            .get_first(self.schema.sym_doc_field)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);

        SymbolResult {
            kind,
            name,
            qualified_name,
            language,
            repo_rel_path,
            container_name,
            visibility,
            signature,
            line_start,
            line_end,
            byte_start,
            byte_end,
            doc: sym_doc,
        }
    }
}

fn symbol_kind_from_str(value: &str) -> Option<SymbolKind> {
    match value {
        "function" => Some(SymbolKind::Function),
        "method" => Some(SymbolKind::Method),
        "class" => Some(SymbolKind::Class),
        "struct" => Some(SymbolKind::Struct),
        "enum" => Some(SymbolKind::Enum),
        "interface" => Some(SymbolKind::Interface),
        "trait" => Some(SymbolKind::Trait),
        "variable" => Some(SymbolKind::Variable),
        "field" => Some(SymbolKind::Field),
        "property" => Some(SymbolKind::Property),
        "constant" => Some(SymbolKind::Constant),
        "module" => Some(SymbolKind::Module),
        "type_alias" => Some(SymbolKind::TypeAlias),
        "import" => Some(SymbolKind::Import),
        _ => None,
    }
}
