# Collie — Index-Backed Code Search

Collie indexes source files and provides near-instant search over large codebases.
It runs as a background daemon that watches for file changes and keeps the index current.

## Install

```sh
cargo install collie-search
```

This installs the `collie` command.

## Setup

```sh
collie watch .          # Start daemon, index the repo (run once)
collie status .         # Verify daemon is running
collie stop .           # Stop the daemon when done
collie clean .          # Remove index and free disk space
```

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

```sh
collie search 'kind:fn handler'
collie search 'kind:struct Config'
collie search 'kind:method qname:Server::run'
collie search 'kind:fn lang:go path:pkg/api/ init'
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

## Agent-Recommended Flags

Always use `--format json` for programmatic consumption:

```sh
collie search handler --format json -n 20
collie search 'kind:fn init' --format json
collie search -e 'TODO|FIXME' --format json -g '*.go'
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
```

## Status (JSON)

```sh
collie status . --json
```

Returns daemon status, file count, index size, segment count, and generation info.

## Common Agent Workflows

**Find files containing a term:**
```sh
collie search handler -l --format json
```

**Find function definitions:**
```sh
collie search 'kind:fn handler' --format json
```

**Scope search to a directory:**
```sh
collie search handler -g 'src/api/**' --format json
```

**Count occurrences before diving in:**
```sh
collie search handler -c
```

**Regex grep with index acceleration:**
```sh
collie search -e 'errors?\.New\(' --format json -g '*.go'
```

**Check if daemon is running:**
```sh
collie status . --json 2>/dev/null | jq -r '.status'
```

## Notes

- The daemon must be running (`collie watch .`) for the index to stay current.
- Searches work without the daemon but results may be stale.
- The index lives in `.collie/` — add to global gitignore: `echo .collie >> ~/.config/git/ignore`
- PDF indexing is opt-in: set `include_pdfs = true` in `.collie/config.toml`.
- Stderr warnings (e.g. "daemon not running") do not affect stdout or exit codes.
