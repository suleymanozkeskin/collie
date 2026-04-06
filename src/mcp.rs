use std::path::{Path, PathBuf};

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
};
use serde::Serialize;

use crate::cli::search::{find_index_path, find_worktree_root};
use crate::config::CollieConfig;
use crate::indexer::IndexBuilder;
use crate::storage::tantivy_index::TantivyIndex;
use crate::symbols::query::parse_query;

// ---- Parameter structs ----

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TokenSearchParams {
    /// Token search pattern. Supports % wildcards: handler (exact), handle% (prefix),
    /// %handler (suffix), %handle% (substring), "handle request" (multi-term AND).
    pub pattern: String,

    /// Max number of results to return.
    #[serde(default)]
    pub limit: Option<usize>,

    /// Glob pattern to filter results by file path (e.g. "*.rs", "src/**/*.go").
    #[serde(default)]
    pub glob: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RegexSearchParams {
    /// Regular expression pattern (full regex syntax).
    pub pattern: String,

    /// Max number of results to return.
    #[serde(default)]
    pub limit: Option<usize>,

    /// Glob pattern to filter results by file path.
    #[serde(default)]
    pub glob: Option<String>,

    /// Case-insensitive matching.
    #[serde(default)]
    pub ignore_case: Option<bool>,

    /// Allow . to match newlines.
    #[serde(default)]
    pub multiline: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SymbolSearchParams {
    /// Symbol search query with structured filters.
    /// Examples: "kind:fn handler", "kind:struct Config", "kind:fn lang:go %init%".
    pub query: String,

    /// Max number of results to return.
    #[serde(default)]
    pub limit: Option<usize>,

    /// Glob pattern to filter results by file path.
    #[serde(default)]
    pub glob: Option<String>,

    /// Regex to further filter symbol results by matching against signature or source.
    #[serde(default)]
    pub symbol_regex: Option<String>,

    /// Case-insensitive matching for symbol_regex.
    #[serde(default)]
    pub ignore_case: Option<bool>,

    /// Allow . to match newlines in symbol_regex.
    #[serde(default)]
    pub multiline: Option<bool>,
}

// ---- JSON output types (same schema as --format json) ----

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

// ---- Server ----

#[derive(Clone)]
pub struct CollieServer {
    worktree_root: PathBuf,
    tool_router: ToolRouter<Self>,
}

impl CollieServer {
    pub fn new(path: PathBuf) -> Result<Self, McpError> {
        let canonical = std::fs::canonicalize(&path).map_err(|e| {
            McpError::invalid_params(format!("invalid path: {e}"), None)
        })?;
        let worktree_root = find_worktree_root(&canonical).map_err(|e| {
            McpError::invalid_params(format!("failed to find worktree root: {e}"), None)
        })?;
        Ok(Self {
            worktree_root,
            tool_router: Self::tool_router(),
        })
    }
}

fn invalid_params_error(msg: impl std::fmt::Display) -> McpError {
    McpError::invalid_params(msg.to_string(), None)
}

fn index_not_found(msg: impl std::fmt::Display) -> McpError {
    McpError::resource_not_found(msg.to_string(), None)
}

fn glob_matches(pattern: &glob::Pattern, path: &Path, worktree_root: &Path) -> bool {
    let rel = path.strip_prefix(worktree_root).unwrap_or(path);
    pattern.matches_path(rel)
        || rel
            .file_name()
            .is_some_and(|n| pattern.matches(n.to_string_lossy().as_ref()))
}

#[tool_router]
impl CollieServer {
    #[tool(
        description = "Search indexed code tokens. Fast sub-millisecond search using the Collie index. \
        Supports % wildcards: 'handler' (exact), 'handle%' (prefix), '%handler' (suffix), \
        '%handle%' (substring). Multi-term queries like 'handle request' match files containing all terms."
    )]
    fn collie_search(
        &self,
        Parameters(params): Parameters<TokenSearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let index_path = find_index_path(&self.worktree_root).map_err(|e| index_not_found(e))?;
        let config = CollieConfig::load(&self.worktree_root);
        let limit = params.limit.unwrap_or(config.search.default_limit);

        let builder =
            IndexBuilder::new(&index_path, &config).map_err(|e| index_not_found(e))?;
        let results = builder.search_pattern_ranked(&params.pattern, limit);

        let glob_pattern = params
            .glob
            .as_deref()
            .map(|g| glob::Pattern::new(g).map_err(|e| invalid_params_error(e)))
            .transpose()?;

        let json_results: Vec<JsonResult> = results
            .iter()
            .filter(|r| {
                glob_pattern
                    .as_ref()
                    .map_or(true, |pat| glob_matches(pat, &r.file_path, &self.worktree_root))
            })
            .map(|r| {
                let rel = r
                    .file_path
                    .strip_prefix(&self.worktree_root)
                    .unwrap_or(&r.file_path);
                JsonResult {
                    path: rel.to_string_lossy().to_string(),
                    line: None,
                    content: None,
                    kind: None,
                    name: None,
                    language: None,
                    signature: None,
                }
            })
            .collect();

        let output = JsonOutput {
            pattern: params.pattern,
            search_type: "token".to_string(),
            count: json_results.len(),
            results: json_results,
        };
        let json = serde_json::to_string(&output)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Search code with a regular expression. Full regex syntax with index acceleration — \
        Collie extracts literal fragments from the regex to narrow candidates via the index, then applies \
        the full regex. Returns matching lines with file path, line number, and content."
    )]
    fn collie_search_regex(
        &self,
        Parameters(params): Parameters<RegexSearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let index_path = find_index_path(&self.worktree_root).map_err(|e| index_not_found(e))?;
        let config = CollieConfig::load(&self.worktree_root);
        let limit = params.limit.unwrap_or(config.search.default_limit);
        let ignore_case = params.ignore_case.unwrap_or(false);
        let multiline = params.multiline.unwrap_or(false);

        let builder =
            IndexBuilder::new(&index_path, &config).map_err(|e| index_not_found(e))?;
        let results = builder
            .search_regex(&params.pattern, limit, multiline, ignore_case, true, 0, 0)
            .map_err(|e| invalid_params_error(e))?;

        let glob_pattern = params
            .glob
            .as_deref()
            .map(|g| glob::Pattern::new(g).map_err(|e| invalid_params_error(e)))
            .transpose()?;

        let json_results: Vec<JsonResult> = results
            .iter()
            .filter(|r| {
                glob_pattern
                    .as_ref()
                    .map_or(true, |pat| glob_matches(pat, &r.file_path, &self.worktree_root))
            })
            .flat_map(|r| {
                let rel = r
                    .file_path
                    .strip_prefix(&self.worktree_root)
                    .unwrap_or(&r.file_path)
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

        let output = JsonOutput {
            pattern: params.pattern,
            search_type: "regex".to_string(),
            count: json_results.len(),
            results: json_results,
        };
        let json = serde_json::to_string(&output)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Search for code symbols (functions, structs, methods, classes, etc.) using \
        structured queries. Query syntax: 'kind:fn name', 'kind:struct Config', \
        'kind:method qname:Server::run', 'kind:fn lang:go path:pkg/ init'. \
        Name matching is exact without % wildcards — use '%name%' for substring."
    )]
    fn collie_search_symbols(
        &self,
        Parameters(params): Parameters<SymbolSearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let index_path = find_index_path(&self.worktree_root).map_err(|e| index_not_found(e))?;
        let config = CollieConfig::load(&self.worktree_root);
        let limit = params.limit.unwrap_or(config.search.default_limit);

        let symbol_query = parse_query(&params.query);
        if !symbol_query.has_filters() {
            return Err(McpError::invalid_params(
                "query must contain at least one symbol filter (e.g. kind:fn, lang:go)",
                None,
            ));
        }

        let tantivy =
            TantivyIndex::open(&index_path.join("tantivy")).map_err(|e| index_not_found(e))?;

        let has_regex_refine = params.symbol_regex.is_some();
        let search_limit = if has_regex_refine { 0 } else { limit };
        let results = tantivy
            .search_symbols(&symbol_query, search_limit)
            .map_err(|e| index_not_found(e))?;

        // Apply glob filter
        let glob_pattern = params
            .glob
            .as_deref()
            .map(|g| glob::Pattern::new(g).map_err(|e| invalid_params_error(e)))
            .transpose()?;

        let results: Vec<_> = if let Some(ref pat) = glob_pattern {
            results
                .into_iter()
                .filter(|r| {
                    pat.matches_path(&r.repo_rel_path)
                        || r.repo_rel_path
                            .file_name()
                            .is_some_and(|n| pat.matches(n.to_string_lossy().as_ref()))
                })
                .collect()
        } else {
            results
        };

        // Apply regex refinement
        let results = if let Some(ref refine_pattern) = params.symbol_regex {
            let ignore_case = params.ignore_case.unwrap_or(false);
            let multiline = params.multiline.unwrap_or(false);
            let regex = regex::RegexBuilder::new(refine_pattern)
                .case_insensitive(ignore_case)
                .dot_matches_new_line(multiline)
                .multi_line(true)
                .build()
                .map_err(|e| McpError::invalid_params(
                    format!("invalid symbol_regex: {e}"),
                    None,
                ))?;
            results
                .into_iter()
                .filter(|r| {
                    if let Some(ref sig) = r.signature {
                        if regex.is_match(sig) {
                            return true;
                        }
                    }
                    let abs_path = self.worktree_root.join(&r.repo_rel_path);
                    match std::fs::read_to_string(&abs_path) {
                        Ok(content) => {
                            let lines: Vec<&str> = content.lines().collect();
                            let start = (r.line_start as usize).saturating_sub(1);
                            let end = (r.line_end as usize).min(lines.len());
                            if start >= lines.len() || start >= end {
                                return false;
                            }
                            let snippet = lines[start..end].join("\n");
                            regex.is_match(&snippet)
                        }
                        Err(_) => false,
                    }
                })
                .take(limit)
                .collect()
        } else {
            results
        };

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
            pattern: params.query,
            search_type: "symbol".to_string(),
            count: json_results.len(),
            results: json_results,
        };
        let json = serde_json::to_string(&output)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}

#[tool_handler]
impl ServerHandler for CollieServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
                "Collie is an index-backed code search engine. It provides three search tools: \
                 collie_search (fast token search), collie_search_regex (regex with index acceleration), \
                 and collie_search_symbols (structured symbol search for functions, structs, methods, etc.)."
                    .to_string(),
            )
    }
}
