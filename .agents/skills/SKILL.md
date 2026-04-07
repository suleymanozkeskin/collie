---
name: collie-quick-reference
description: How to use collie for fast, index-backed code search in this repository.
---

# Collie — Index-Backed Code Search

Collie indexes source files and provides near-instant search over large codebases.
It runs as a background daemon that watches for file changes and keeps the index current.

Install: `cargo install collie-search` (installs the `collie` command).

## Setup

```sh
collie watch .          # Start daemon, index the repo (run once)
collie status .         # Verify daemon is running
collie stop .           # Stop the daemon when done
collie clean .          # Remove index and free disk space
```

## Repository Scoping

Scope Collie to the actual target repository before trusting results.

- If your current workspace contains multiple repositories, fixtures, or large test corpora, do not default to `.`.
- Prefer running Collie from the real repo root, or pass `--path /absolute/repo/path`.
- Rebuild or watch the same scoped path you plan to search.
- Use `-g` only for narrowing inside an already-correct repo scope, not as a substitute for choosing the right repo root.

Examples:

```sh
collie rebuild /path/to/repo
collie search handler --path /path/to/repo --format json
collie search 'kind:fn init' --path /path/to/repo --format json
```

If a workspace contains fixture trees such as `large-repo-for-test`, broad searches from `.` can return irrelevant noise even when the query itself is correct.

## Search Modes

### Token search (default)
Matches indexed tokens (identifiers, keywords). Case-insensitive. Use `%` for wildcards.

```sh
collie search handler                    # exact token
collie search 'handle%'                  # prefix
collie search '%handler'                 # suffix
collie search '%handle%'                 # substring
collie search 'handle request'           # multi-term AND
```

### Symbol search
Structured queries for functions, types, methods. Use `kind:`, `lang:`, `path:`, `qname:` filters.

**Name matching is EXACT by default.** `kind:fn handler` matches only a symbol named
exactly `handler` — it will NOT match `handleRequest` or `requestHandler`.
Use `%` wildcards for flexible matching (same syntax as token search):

| Pattern | Matches |
|---------|---------|
| `kind:fn handler` | only `handler` (exact) |
| `kind:fn handler%` | `handler`, `handlerFunc`, … (prefix) |
| `kind:fn %handler` | `handler`, `requestHandler`, … (suffix) |
| `kind:fn %handler%` | anything containing `handler` (substring) |

```sh
collie search 'kind:fn %handler%'                        # functions containing "handler"
collie search 'kind:struct %Config%'                      # structs containing "Config"
collie search 'kind:fn handler'                           # exact match only
collie search 'kind:method qname:Server::run'             # qualified name
collie search 'kind:fn lang:go path:pkg/api/ %init%'     # scoped substring
```

**Supported kinds:** function, method, class, struct, enum, interface, trait, variable, field, constant, module, type_alias, import

**Supported languages:** go, rust, python, typescript

### Regex search
Full regex with index acceleration. Use `-e` flag.

```sh
collie search -e 'func\s+\w+Handler'
collie search -e 'TODO|FIXME|HACK'
collie search -e 'impl.*for.*Error' -i    # case-insensitive
collie search -e 'struct\s*\{' -U         # multiline (. matches \n)
```

### Symbol + Regex (recommended for structural patterns)
Combine symbol narrowing with regex refinement via `--symbol-regex`.
Much faster than pure regex for queries about code structure.

```sh
collie search 'kind:fn %Handler' --symbol-regex '\*.*Server'
collie search 'kind:method qname:Server::' --symbol-regex 'Handler'
collie search 'kind:fn %validate%' --symbol-regex 'webhook|Webhook'
```

**When to use `--symbol-regex` instead of `-e`:**
- You know the symbol kind (function, method, struct, etc.)
- You want to match against the signature or body of specific symbols
- Pure regex would be slow because the pattern is structurally complex
- Example: "find methods on Server types ending in Handler" → use `kind:fn %Handler --symbol-regex '\*.*Server'` instead of `-e 'func\s+\(.*\*.*Server\)\s+\w+Handler'`

## Agent-Recommended Flags

Always use `--format json` for programmatic consumption:

```sh
collie search handler --format json -n 20
collie search 'kind:fn %init%' --format json
collie search -e 'TODO|FIXME' --format json -g '*.go'
```

For agent use, prefer explicit repo scoping together with JSON output:

```sh
collie search handler --path /path/to/repo --format json -n 20
collie search 'kind:fn %init%' --path /path/to/repo --format json
collie search -e 'TODO|FIXME' --path /path/to/repo --format json -g '*.go'
```

### JSON output schema

```json
{
  "pattern": "handler",
  "type": "token|symbol|regex",
  "count": 3,
  "results": [
    {
      "path": "src/api/handler.go",
      "line": 42,
      "content": "func handler() {",
      "kind": "function",
      "name": "handler",
      "language": "go",
      "signature": "func handler() { ... }"
    }
  ]
}
```

Fields present depend on search type:
- **Token search:** `path` only
- **Symbol search:** `path`, `line`, `kind`, `name`, `language`, `signature`
- **Regex search:** `path`, `line`, `content`

### Exit codes

| Code | Meaning |
|------|---------|
| 0 | Results found |
| 1 | No results |
| 2 | Error (invalid pattern, no index, etc.) |

## Filtering and Output

```sh
collie search handler -g '*.go'           # glob filter by file path
collie search handler -l                   # file paths only (one per line)
collie search handler -c                   # count of matching files
collie search handler -n 50                # limit to 50 results
collie search handler -C 3                 # 3 lines of context
collie search handler -A 2 -B 0            # 2 lines after, 0 before
collie search handler --format plain       # path:line:content output
collie search handler --color never        # no ANSI escape codes
collie search handler --path /other/repo   # search a different repo
```

## Management

```sh
collie status . --json                    # daemon status as JSON
collie clean .                            # stop daemon + remove index
collie rebuild .                          # rebuild index from scratch
collie config --init .                    # create example config
collie skill                              # print this reference
```

## Common Agent Workflows

**Find files containing a term:**
```sh
collie search handler --path /path/to/repo -l --format json
```

**Find function definitions (substring — use `%` wildcards):**
```sh
collie search 'kind:fn %handler%' --path /path/to/repo --format json
```

**Scope search to a directory:**
```sh
collie search handler --path /path/to/repo -g 'src/api/**' --format json
```

**Scope symbol search to file type:**
```sh
collie search 'kind:fn %init%' --path /path/to/repo -g '*.go' --format json
```

**Count occurrences before diving in:**
```sh
collie search handler -c
```

**Regex grep with index acceleration:**
```sh
collie search -e 'errors?\.New\(' --path /path/to/repo --format json -g '*.go'
```

**Find methods on a specific type with a regex constraint:**
```sh
collie search 'kind:method qname:Server::' --path /path/to/repo --symbol-regex 'Handler' --format json
```

**Find functions whose signature matches a complex pattern:**
```sh
collie search 'kind:fn %Handler' --path /path/to/repo --symbol-regex '\*.*Server' --format json
```

**Search a different repo without cd:**
```sh
collie search handler --path /path/to/repo --format json
```

**Check if daemon is running:**
```sh
collie status . --json 2>/dev/null | jq -r '.status'
```

## MCP Server

Collie can run as an MCP (Model Context Protocol) server over stdio, exposing its
search capabilities as tools for AI agents, editors, and IDE extensions.

```sh
collie mcp-serve --path /path/to/repo
```

This starts a JSON-RPC server on stdin/stdout with three tools:

| Tool | Description |
|------|-------------|
| `collie_search` | Token search (same as `collie search`) |
| `collie_search_regex` | Regex search (same as `collie search -e`) |
| `collie_search_symbols` | Symbol search (same as `collie search 'kind:...'`) |

All tools return JSON with the same schema as `--format json` output. Parameters
match CLI flags: `pattern`/`query`, `limit`, `glob`, `ignore_case`, `multiline`,
`symbol_regex`.

### Quick setup

```sh
collie mcp-setup                     # writes .mcp.json for Claude Code (default)
collie mcp-setup --target vscode     # writes .vscode/mcp.json for VS Code
```

This auto-detects the `collie` binary path, resolves the repo root, and writes
the config file. Merges into existing config if present.

### Manual configuration

Claude Code (via CLI):
```sh
claude mcp add --transport stdio collie -- collie mcp-serve --path /path/to/repo
```

Or edit `.mcp.json` directly:
```json
{
  "mcpServers": {
    "collie": {
      "type": "stdio",
      "command": "collie",
      "args": ["mcp-serve", "--path", "/path/to/repo"]
    }
  }
}
```

VS Code (`.vscode/mcp.json`):
```json
{
  "servers": {
    "collie": {
      "command": "collie",
      "args": ["mcp-serve", "--path", "${workspaceFolder}"]
    }
  }
}
```

### Error codes

| Code | Meaning |
|------|---------|
| -32602 | Invalid params (bad regex, bad glob, missing symbol filters) |
| -32002 | Resource not found (no index — run `collie watch .` first) |
| -32603 | Internal error (server fault) |

## Notes

- The daemon must be running (`collie watch .`) for the index to stay current.
- Searches work without the daemon but results may be stale.
- The index lives in `.collie/` — add to global gitignore: `echo .collie >> ~/.config/git/ignore`
- PDF indexing is opt-in: set `include_pdfs = true` in `.collie/config.toml`.
- Stderr warnings (e.g. "daemon not running") do not affect stdout or exit codes.
