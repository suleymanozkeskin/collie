use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::config::CollieConfig;
use crate::indexer::IndexBuilder;
use crate::indexer::tokenizer::Tokenizer;
use crate::storage::generation::GenerationManager;
use crate::storage::tantivy_index::TantivyIndex;
use crate::symbols::SymbolResult;
use crate::symbols::query::parse_query;

/// Output format for search results.
#[derive(Clone, Debug, Default, clap::ValueEnum)]
pub enum OutputFormat {
    #[default]
    Default,
    /// Plain line-oriented output: path:line:content (no headers).
    Plain,
    /// JSON output for programmatic consumption by AI agents and tools.
    Json,
}

// --- JSON output types ---

#[derive(Serialize)]
struct JsonOutput {
    pattern: String,
    #[serde(rename = "type")]
    search_type: String,
    count: usize,
    results: Vec<JsonResult>,
}

#[derive(Serialize)]
struct JsonResult {
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    signature: Option<String>,
}

/// Color mode for output.
#[derive(Clone, Debug, Default, clap::ValueEnum)]
pub enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
}

/// All search arguments bundled for cleaner signatures.
#[derive(Default)]
pub struct SearchArgs {
    pub pattern: String,
    pub limit: Option<usize>,
    pub context: Option<usize>,
    pub after_context: Option<usize>,
    pub before_context: Option<usize>,
    pub no_snippets: bool,
    pub is_regex: bool,
    pub ignore_case: bool,
    pub multiline: bool,
    pub files_only: bool,
    pub count: bool,
    pub glob: Option<String>,
    pub color: ColorMode,
    pub format: OutputFormat,
    /// Repository path to search. If None, uses current directory.
    pub path: Option<std::path::PathBuf>,
}

struct Snippet {
    /// (1-based line number, content, is_match_line)
    lines: Vec<(usize, String, bool)>,
}

/// Resolve whether color should be used.
fn use_color(mode: &ColorMode) -> bool {
    match mode {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => atty_stdout(),
    }
}

fn atty_stdout() -> bool {
    unsafe { libc::isatty(libc::STDOUT_FILENO) != 0 }
}

// ANSI color helpers
const GREEN: &str = "\x1b[32m";
const MAGENTA: &str = "\x1b[35m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

/// Run a search. Returns `Ok(true)` if results were found, `Ok(false)` if not.
pub fn run(args: SearchArgs) -> Result<bool> {
    let start_dir = match &args.path {
        Some(p) => std::fs::canonicalize(p)
            .with_context(|| format!("invalid path: {:?}", p))?,
        None => std::env::current_dir()?,
    };
    let worktree_root = find_worktree_root(&start_dir)?;
    let index_path = find_index_path(&worktree_root)?;
    let config = CollieConfig::load(&worktree_root);

    let limit = args.limit.unwrap_or(config.search.default_limit);
    let (before_ctx, after_ctx) = resolve_context(
        args.context,
        args.before_context,
        args.after_context,
        config.search.context_lines,
    );
    let symbol_query = parse_query(&args.pattern);
    let color = use_color(&args.color) && !matches!(args.format, OutputFormat::Json);
    let glob_pattern = args.glob.as_deref().map(|g| glob::Pattern::new(g).ok()).flatten();

    if !crate::daemon::is_daemon_alive(&worktree_root) {
        eprintln!("warning: collie daemon is not running; results may be stale");
    }

    crate::daemon::touch_activity(&worktree_root);

    // Symbol search
    if symbol_query.has_filters() {
        let tantivy = TantivyIndex::open(&index_path.join("tantivy"))
            .with_context(|| format!("Failed to load symbol index from {:?}", index_path))?;
        let results = tantivy.search_symbols(&symbol_query, limit)?;

        // Apply glob filter to symbol results
        let results: Vec<_> = if let Some(ref pat) = glob_pattern {
            results
                .into_iter()
                .filter(|r| {
                    pat.matches_path(&r.repo_rel_path)
                        || r.repo_rel_path
                            .file_name()
                            .map_or(false, |n| pat.matches(n.to_string_lossy().as_ref()))
                })
                .collect()
        } else {
            results
        };

        let found = !results.is_empty();

        if args.count {
            println!("{}", results.len());
            return Ok(found);
        }

        if args.files_only {
            let mut seen = HashSet::new();
            for r in &results {
                let path = r.repo_rel_path.to_string_lossy().to_string();
                if seen.insert(path.clone()) {
                    if color {
                        println!("{MAGENTA}{}{RESET}", path);
                    } else {
                        println!("{}", path);
                    }
                }
            }
            return Ok(found);
        }

        match args.format {
            OutputFormat::Json => print_symbol_results_json(&args.pattern, &results),
            _ => print_symbol_results(&args.pattern, &results, color),
        }
        return Ok(found);
    }

    let builder = IndexBuilder::new(&index_path, &config)
        .with_context(|| format!("Failed to load index from {:?}", index_path))?;

    let opts = SearchOpts {
        pattern: &args.pattern,
        limit,
        before_ctx,
        after_ctx,
        no_snippets: args.no_snippets || args.files_only || args.count,
        files_only: args.files_only,
        count: args.count,
        ignore_case: args.ignore_case,
        multiline: args.multiline,
        glob_pattern: glob_pattern.as_ref(),
        color,
        format: &args.format,
        worktree_root: &worktree_root,
    };

    if args.is_regex {
        run_regex_search(&builder, &opts)
    } else {
        run_token_search(&builder, &opts)
    }
}

/// Resolve asymmetric context: -A/-B override -C.
fn resolve_context(
    symmetric: Option<usize>,
    before: Option<usize>,
    after: Option<usize>,
    default: usize,
) -> (usize, usize) {
    let base = symmetric.unwrap_or(default);
    (before.unwrap_or(base), after.unwrap_or(base))
}

struct SearchOpts<'a> {
    pattern: &'a str,
    limit: usize,
    before_ctx: usize,
    after_ctx: usize,
    no_snippets: bool,
    files_only: bool,
    count: bool,
    ignore_case: bool,
    multiline: bool,
    glob_pattern: Option<&'a glob::Pattern>,
    color: bool,
    format: &'a OutputFormat,
    worktree_root: &'a Path,
}

fn run_token_search(builder: &IndexBuilder, opts: &SearchOpts) -> Result<bool> {
    let results = builder.search_pattern_ranked(opts.pattern, opts.limit);
    let results = filter_by_glob(&results, opts);
    let found = !results.is_empty();

    if opts.count {
        println!("{}", results.len());
        return Ok(found);
    }

    if matches!(opts.format, OutputFormat::Json) {
        let json_results: Vec<JsonResult> = results
            .iter()
            .map(|r| JsonResult {
                path: relative_path(&r.file_path, opts.worktree_root).to_string_lossy().to_string(),
                line: None,
                content: None,
                kind: None,
                name: None,
                language: None,
                signature: None,
            })
            .collect();
        println!("{}", serde_json::to_string(&JsonOutput {
            pattern: opts.pattern.to_string(),
            search_type: "token".to_string(),
            count: json_results.len(),
            results: json_results,
        })?);
        return Ok(found);
    }

    if results.is_empty() {
        if matches!(opts.format, OutputFormat::Default) {
            println!("No results found for pattern: {}", opts.pattern);
        }
        return Ok(false);
    }

    if opts.files_only {
        for result in &results {
            let rel = relative_path(&result.file_path, opts.worktree_root);
            if opts.color {
                println!("{MAGENTA}{}{RESET}", rel.display());
            } else {
                println!("{}", rel.display());
            }
        }
        return Ok(true);
    }

    match opts.format {
        OutputFormat::Default => {
            println!("Found {} results for pattern: {}", results.len(), opts.pattern);
            if opts.no_snippets {
                println!();
                for (idx, result) in results.iter().enumerate() {
                    if idx > 0 { println!(); }
                    let rel = relative_path(&result.file_path, opts.worktree_root);
                    println!("{}. {}", idx + 1, rel.display());
                }
            } else {
                let ctx = opts.before_ctx.max(opts.after_ctx);
                for result in &results {
                    let rel = relative_path(&result.file_path, opts.worktree_root);
                    println!();
                    match extract_snippets(&result.file_path, opts.pattern, ctx) {
                        Some(snippets) => print_snippets_default(&rel, &snippets, opts.color),
                        None => println!("{} (file not found, index may be stale)", rel.display()),
                    }
                }
            }
        }
        OutputFormat::Plain => {
            if opts.no_snippets {
                for result in &results {
                    let rel = relative_path(&result.file_path, opts.worktree_root);
                    println!("{}", rel.display());
                }
            } else {
                let ctx = opts.before_ctx.max(opts.after_ctx);
                let mut first_group = true;
                for result in &results {
                    let rel = relative_path(&result.file_path, opts.worktree_root);
                    if let Some(snippets) = extract_snippets(&result.file_path, opts.pattern, ctx) {
                        print_snippets_plain(&rel, &snippets, &mut first_group, opts.color);
                    }
                }
            }
        }
        OutputFormat::Json => unreachable!(), // handled above
    }
    Ok(true)
}

fn run_regex_search(builder: &IndexBuilder, opts: &SearchOpts) -> Result<bool> {
    let results = builder.search_regex(opts.pattern, opts.limit, opts.multiline, opts.ignore_case)?;
    let results: Vec<_> = results
        .into_iter()
        .filter(|r| match_glob(&r.file_path, opts))
        .collect();
    let found = !results.is_empty();

    if opts.count {
        println!("{}", results.len());
        return Ok(found);
    }

    if matches!(opts.format, OutputFormat::Json) {
        let json_results: Vec<JsonResult> = results
            .iter()
            .flat_map(|r| {
                let rel = relative_path(&r.file_path, opts.worktree_root)
                    .to_string_lossy()
                    .to_string();
                if r.matches.is_empty() {
                    vec![JsonResult {
                        path: rel,
                        line: None,
                        content: None,
                        kind: None,
                        name: None,
                        language: None,
                        signature: None,
                    }]
                } else {
                    r.matches
                        .iter()
                        .map(|m| JsonResult {
                            path: rel.clone(),
                            line: Some(m.line_number as u32),
                            content: Some(m.line_content.clone()),
                            kind: None,
                            name: None,
                            language: None,
                            signature: None,
                        })
                        .collect()
                }
            })
            .collect();
        println!("{}", serde_json::to_string(&JsonOutput {
            pattern: opts.pattern.to_string(),
            search_type: "regex".to_string(),
            count: results.len(),
            results: json_results,
        })?);
        return Ok(found);
    }

    if results.is_empty() {
        if matches!(opts.format, OutputFormat::Default) {
            println!("No results found for regex: {}", opts.pattern);
        }
        return Ok(false);
    }

    if opts.files_only {
        for result in &results {
            let rel = relative_path(&result.file_path, opts.worktree_root);
            if opts.color {
                println!("{MAGENTA}{}{RESET}", rel.display());
            } else {
                println!("{}", rel.display());
            }
        }
        return Ok(true);
    }

    let ctx = opts.before_ctx.max(opts.after_ctx);
    match opts.format {
        OutputFormat::Default => {
            println!("Found {} file(s) with matches for regex: {}", results.len(), opts.pattern);
            for result in &results {
                let rel = relative_path(&result.file_path, opts.worktree_root);
                println!();
                if opts.no_snippets {
                    println!("{}", rel.display());
                } else {
                    let match_lines: Vec<usize> = result.matches.iter().map(|m| m.line_number).collect();
                    match build_context_snippets(&result.file_path, &match_lines, ctx) {
                        Some(snippets) => print_snippets_default(&rel, &snippets, opts.color),
                        None => println!("{} (file not found, index may be stale)", rel.display()),
                    }
                }
            }
        }
        OutputFormat::Plain => {
            if opts.no_snippets {
                for result in &results {
                    let rel = relative_path(&result.file_path, opts.worktree_root);
                    println!("{}", rel.display());
                }
            } else {
                let mut first_group = true;
                for result in &results {
                    let rel = relative_path(&result.file_path, opts.worktree_root);
                    let match_lines: Vec<usize> = result.matches.iter().map(|m| m.line_number).collect();
                    if let Some(snippets) = build_context_snippets(&result.file_path, &match_lines, ctx) {
                        print_snippets_plain(&rel, &snippets, &mut first_group, opts.color);
                    }
                }
            }
        }
        OutputFormat::Json => unreachable!(),
    }
    Ok(true)
}

// --- Helpers ---

fn relative_path<'a>(path: &'a Path, root: &Path) -> &'a Path {
    path.strip_prefix(root).unwrap_or(path)
}

fn filter_by_glob<'a>(
    results: &'a [crate::storage::SearchResult],
    opts: &SearchOpts,
) -> Vec<&'a crate::storage::SearchResult> {
    results
        .iter()
        .filter(|r| match_glob(&r.file_path, opts))
        .collect()
}

fn match_glob(path: &Path, opts: &SearchOpts) -> bool {
    match opts.glob_pattern {
        Some(pat) => {
            let rel = path.strip_prefix(opts.worktree_root).unwrap_or(path);
            pat.matches_path(rel)
                || rel.file_name().map_or(false, |n| pat.matches(n.to_string_lossy().as_ref()))
        }
        None => true,
    }
}

// --- Output formatters ---

fn print_snippets_default(relative: &Path, snippets: &[Snippet], color: bool) {
    if color {
        println!("{MAGENTA}{BOLD}{}{RESET}", relative.display());
    } else {
        println!("{}", relative.display());
    }
    for (si, snippet) in snippets.iter().enumerate() {
        if si > 0 {
            println!("  ...");
        }
        let max_line_num = snippet.lines.last().map(|(n, _, _)| *n).unwrap_or(1);
        let width = max_line_num.to_string().len();
        for (line_num, content, is_match) in &snippet.lines {
            if color && *is_match {
                println!("  {GREEN}{:>width$}{RESET} | {}", line_num, content, width = width);
            } else {
                println!("  {:>width$} | {}", line_num, content, width = width);
            }
        }
    }
}

fn print_snippets_plain(relative: &Path, snippets: &[Snippet], first_group: &mut bool, color: bool) {
    let rel_str = relative.display().to_string();
    for snippet in snippets {
        if !*first_group {
            println!("--");
        }
        *first_group = false;
        for (line_num, content, is_match) in &snippet.lines {
            if *is_match {
                if color {
                    println!("{MAGENTA}{}{RESET}:{GREEN}{}{RESET}:{}", rel_str, line_num, content);
                } else {
                    println!("{}:{}:{}", rel_str, line_num, content);
                }
            } else {
                println!("{}-{}-{}", rel_str, line_num, content);
            }
        }
    }
}

fn print_symbol_results_json(pattern: &str, results: &[SymbolResult]) {
    let json_results: Vec<JsonResult> = results
        .iter()
        .map(|r| JsonResult {
            path: r.repo_rel_path.to_string_lossy().to_string(),
            line: Some(r.line_start),
            content: None,
            kind: Some(r.kind.as_str().to_string()),
            name: Some(r.name.clone()),
            language: Some(r.language.clone()),
            signature: r.signature.clone(),
        })
        .collect();
    let output = JsonOutput {
        pattern: pattern.to_string(),
        search_type: "symbol".to_string(),
        count: json_results.len(),
        results: json_results,
    };
    println!("{}", serde_json::to_string(&output).unwrap_or_default());
}

fn print_symbol_results(pattern: &str, results: &[SymbolResult], color: bool) {
    if results.is_empty() {
        println!("No symbols found for: {}", pattern);
        return;
    }

    println!("Found {} symbols for: {}", results.len(), pattern);
    println!();

    for (idx, result) in results.iter().enumerate() {
        if idx > 0 {
            println!();
        }
        if color {
            println!("{}. {BOLD}{}{RESET} ({})", idx + 1, result.name, result.kind.as_str());
            println!(
                "   {MAGENTA}{}:{}{RESET}  lang:{}",
                result.repo_rel_path.display(),
                result.line_start,
                result.language
            );
        } else {
            println!("{}. {} ({})", idx + 1, result.name, result.kind.as_str());
            println!(
                "   {}:{}  lang:{}",
                result.repo_rel_path.display(),
                result.line_start,
                result.language
            );
        }
        if let Some(signature) = &result.signature {
            println!("   {}", signature.replace('\n', " ").trim());
        }
    }
}

// --- Snippet building ---

/// Build context-window snippets from a list of 1-based match line numbers.
fn build_context_snippets(
    file_path: &Path,
    match_line_numbers: &[usize],
    context: usize,
) -> Option<Vec<Snippet>> {
    let content = std::fs::read_to_string(file_path).ok()?;
    let lines_vec: Vec<&str> = content
        .split('\n')
        .map(|l| l.trim_end_matches('\r'))
        .collect();
    let total_lines = lines_vec.len();

    let mut sorted = match_line_numbers.to_vec();
    sorted.sort();
    sorted.dedup();

    if sorted.is_empty() {
        return None;
    }

    let match_set: HashSet<usize> = sorted.iter().copied().collect();

    // Convert to 0-based for window calculations
    let match_lines_0: Vec<usize> = sorted.iter().map(|&n| n.saturating_sub(1)).collect();

    let mut snippets = Vec::new();
    let mut i = 0;
    while i < match_lines_0.len() {
        let match_line = match_lines_0[i];
        let window_start = match_line.saturating_sub(context);
        let mut window_end = (match_line + context + 1).min(total_lines);

        while i + 1 < match_lines_0.len() {
            let next = match_lines_0[i + 1];
            let next_start = next.saturating_sub(context);
            if next_start <= window_end {
                window_end = (next + context + 1).min(total_lines);
                i += 1;
            } else {
                break;
            }
        }

        let lines: Vec<(usize, String, bool)> = (window_start..window_end)
            .filter_map(|ln| {
                lines_vec.get(ln).map(|line| {
                    let line_num = ln + 1;
                    (line_num, line.to_string(), match_set.contains(&line_num))
                })
            })
            .collect();

        if !lines.is_empty() {
            snippets.push(Snippet { lines });
        }
        i += 1;
    }

    Some(snippets)
}

/// Find byte-offset positions of tokens matching the query pattern in content.
fn find_match_positions(content: &str, pattern: &str) -> Option<Vec<u32>> {
    let tokenizer = Tokenizer::new();
    let tokens = tokenizer.tokenize(content);

    let pattern = pattern.trim();
    let normalized = pattern.to_lowercase();
    let starts = normalized.starts_with('%');
    let ends = normalized.ends_with('%');

    let positions: Vec<u32> = tokens
        .iter()
        .filter(|t| match (starts, ends) {
            (false, false) => t.text == normalized,
            (false, true) => t.text.starts_with(normalized.trim_end_matches('%')),
            (true, false) => t.text.ends_with(normalized.trim_start_matches('%')),
            (true, true) => t.text.contains(normalized.trim_matches('%')),
        })
        .map(|t| t.position as u32)
        .collect();

    if positions.is_empty() {
        None
    } else {
        Some(positions)
    }
}

/// Extract code snippets for token search by finding match positions and
/// building context windows.
fn extract_snippets(file_path: &Path, pattern: &str, context: usize) -> Option<Vec<Snippet>> {
    let content = std::fs::read_to_string(file_path).ok()?;
    let positions = find_match_positions(&content, pattern)?;

    // Build line-start offset table
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(
            content
                .bytes()
                .enumerate()
                .filter(|(_, b)| *b == b'\n')
                .map(|(i, _)| i + 1),
        )
        .collect();

    // Convert byte offsets to 1-based line numbers
    let match_lines: Vec<usize> = positions
        .iter()
        .map(|&pos| {
            let zero_based = match line_starts.binary_search(&(pos as usize)) {
                Ok(idx) => idx,
                Err(idx) => idx.saturating_sub(1),
            };
            zero_based + 1
        })
        .collect();

    build_context_snippets(file_path, &match_lines, context)
}

// --- Path resolution ---

fn find_index_path(worktree_root: &Path) -> Result<PathBuf> {
    let mut current = worktree_root.to_path_buf();

    loop {
        let candidate = current.join(".collie");

        if candidate.join("CURRENT").is_file() {
            let mgr = GenerationManager::new(&candidate);
            if let Ok(Some(gen_dir)) = mgr.active_generation() {
                if mgr.needs_rebuild() && !crate::daemon::is_daemon_alive(worktree_root) {
                    anyhow::bail!(
                        "Index requires rebuild (dirty or corrupt). \
                         Run 'collie watch .' to rebuild."
                    );
                }
                return Ok(gen_dir);
            }
        }

        if candidate.join("tantivy").is_dir() {
            return Ok(candidate);
        }

        if current == *worktree_root {
            break;
        }

        if !current.pop() {
            break;
        }
    }

    anyhow::bail!("No index found. Run 'collie watch .' from the worktree root first.");
}

fn find_worktree_root(start: &PathBuf) -> Result<PathBuf> {
    let mut current = start.clone();

    loop {
        if current.join(".git").exists() {
            return Ok(current);
        }

        if !current.pop() {
            break;
        }
    }

    Ok(start.clone())
}
