mod common;

use anyhow::{Context, Result};
use common::*;
use serde_json::Value;

/// Read newline-delimited JSON-RPC messages from the reader until we find one
/// with a matching `"id"` field. Notifications (no `"id"`) are skipped.
/// This makes the tests robust against interleaved server notifications.
fn read_response(
    reader: &mut impl std::io::BufRead,
    expected_id: u64,
) -> Result<Value> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if std::time::Instant::now() > deadline {
            anyhow::bail!(
                "timed out waiting for JSON-RPC response with id={}",
                expected_id
            );
        }
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            anyhow::bail!(
                "EOF before receiving JSON-RPC response with id={}",
                expected_id
            );
        }
        let msg: Value = serde_json::from_str(line.trim())
            .with_context(|| format!("invalid JSON-RPC message: {}", line.trim()))?;
        // Notifications have no "id" field — skip them.
        if msg.get("id").is_some_and(|id| id == expected_id) {
            return Ok(msg);
        }
    }
}

/// Helper: run `collie mcp-serve` as a subprocess, send JSON-RPC messages,
/// and return the parsed response for a tools/call request.
///
/// The MCP stdio protocol uses newline-delimited JSON-RPC 2.0.
/// We send initialize → initialized → tools/call → close.
fn mcp_call_tool(root: &std::path::Path, tool_name: &str, arguments: Value) -> Result<Value> {
    use std::io::{BufReader, Write};
    use std::process::{Command, Stdio};

    let mut child = Command::new(collie_bin())
        .current_dir(root)
        .env(collie_search::paths::STATE_DIR_ENV, state_home(root))
        .args(["mcp-serve", "--path", "."])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);

    // 1) Send initialize request
    let init_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "0.1" }
        }
    });
    writeln!(stdin, "{}", serde_json::to_string(&init_req)?)?;
    stdin.flush()?;

    let init_resp = read_response(&mut reader, 1)?;
    assert!(
        init_resp["result"]["capabilities"].is_object(),
        "initialize should return capabilities"
    );

    // 2) Send initialized notification
    let initialized = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    writeln!(stdin, "{}", serde_json::to_string(&initialized)?)?;
    stdin.flush()?;

    // 3) Send tools/call request
    let call_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": arguments
        }
    });
    writeln!(stdin, "{}", serde_json::to_string(&call_req)?)?;
    stdin.flush()?;

    let call_resp = read_response(&mut reader, 2)?;

    // Close stdin to signal EOF; the server should exit
    drop(stdin);
    let _ = child.wait();

    Ok(call_resp)
}

/// Helper: extract the JSON payload from the MCP tool response.
/// MCP tools return Content::text with the JSON string inside.
fn extract_tool_json(resp: &Value) -> Result<Value> {
    let content = &resp["result"]["content"];
    assert!(content.is_array(), "result.content should be an array");
    let text = content[0]["text"]
        .as_str()
        .expect("content[0].text should be a string");
    Ok(serde_json::from_str(text)?)
}

/// Helper: list available tools via MCP.
fn mcp_list_tools(root: &std::path::Path) -> Result<Value> {
    use std::io::{BufReader, Write};
    use std::process::{Command, Stdio};

    let mut child = Command::new(collie_bin())
        .current_dir(root)
        .env(collie_search::paths::STATE_DIR_ENV, state_home(root))
        .args(["mcp-serve", "--path", "."])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);

    // Initialize
    let init_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "0.1" }
        }
    });
    writeln!(stdin, "{}", serde_json::to_string(&init_req)?)?;
    stdin.flush()?;

    let _ = read_response(&mut reader, 1)?;

    let initialized = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    writeln!(stdin, "{}", serde_json::to_string(&initialized)?)?;
    stdin.flush()?;

    // List tools
    let list_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list"
    });
    writeln!(stdin, "{}", serde_json::to_string(&list_req)?)?;
    stdin.flush()?;

    let list_resp = read_response(&mut reader, 2)?;

    drop(stdin);
    let _ = child.wait();

    Ok(list_resp)
}

// ---- Tests ----

#[test]
fn mcp_server_lists_three_tools() -> Result<()> {
    let worktree = create_worktree()?;
    build_index(worktree.path(), &[("src/lib.rs", "fn hello() {}")])?;

    let resp = mcp_list_tools(worktree.path())?;
    let tools = resp["result"]["tools"].as_array().expect("tools array");

    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"collie_search"), "missing collie_search tool");
    assert!(
        names.contains(&"collie_search_regex"),
        "missing collie_search_regex tool"
    );
    assert!(
        names.contains(&"collie_search_symbols"),
        "missing collie_search_symbols tool"
    );
    assert_eq!(names.len(), 3, "expected exactly 3 tools, got {:?}", names);

    // Each tool should have a description and inputSchema
    for tool in tools {
        assert!(
            tool["description"].is_string(),
            "tool {} missing description",
            tool["name"]
        );
        assert!(
            tool["inputSchema"].is_object(),
            "tool {} missing inputSchema",
            tool["name"]
        );
    }

    Ok(())
}

#[test]
fn mcp_token_search_returns_matching_files() -> Result<()> {
    let worktree = create_worktree()?;
    build_index(
        worktree.path(),
        &[
            ("src/handler.rs", "fn handle_request() {}"),
            ("src/util.rs", "fn format_string() {}"),
        ],
    )?;

    let resp = mcp_call_tool(
        worktree.path(),
        "collie_search",
        serde_json::json!({ "pattern": "handle_request" }),
    )?;

    let json = extract_tool_json(&resp)?;
    assert_eq!(json["type"], "token", "search_type should be 'token'");
    assert_eq!(json["count"], 1);

    let results = json["results"].as_array().unwrap();
    assert_eq!(results[0]["path"], "src/handler.rs");

    Ok(())
}

#[test]
fn mcp_token_search_no_results() -> Result<()> {
    let worktree = create_worktree()?;
    build_index(worktree.path(), &[("src/lib.rs", "fn unrelated() {}")])?;

    let resp = mcp_call_tool(
        worktree.path(),
        "collie_search",
        serde_json::json!({ "pattern": "nonexistent_token" }),
    )?;

    let json = extract_tool_json(&resp)?;
    assert_eq!(json["count"], 0);
    assert!(json["results"].as_array().unwrap().is_empty());

    Ok(())
}

#[test]
fn mcp_regex_search_returns_line_matches() -> Result<()> {
    let worktree = create_worktree()?;
    build_index(
        worktree.path(),
        &[
            ("src/main.rs", "fn main() {\n    // TODO fix this\n}\n"),
            ("src/lib.rs", "fn lib() {}"),
        ],
    )?;

    let resp = mcp_call_tool(
        worktree.path(),
        "collie_search_regex",
        serde_json::json!({ "pattern": "TODO.*fix" }),
    )?;

    let json = extract_tool_json(&resp)?;
    assert_eq!(json["type"], "regex");
    assert!(json["count"].as_u64().unwrap() >= 1);

    let results = json["results"].as_array().unwrap();
    assert_eq!(results[0]["path"], "src/main.rs");
    assert!(results[0]["line"].is_number(), "regex results should have line numbers");
    assert!(results[0]["content"].is_string(), "regex results should have content");

    Ok(())
}

#[test]
fn mcp_regex_search_case_insensitive() -> Result<()> {
    let worktree = create_worktree()?;
    build_index(
        worktree.path(),
        &[("src/lib.rs", "fn MyHandler() {}")],
    )?;

    let resp = mcp_call_tool(
        worktree.path(),
        "collie_search_regex",
        serde_json::json!({ "pattern": "myhandler", "ignore_case": true }),
    )?;

    let json = extract_tool_json(&resp)?;
    assert!(json["count"].as_u64().unwrap() >= 1, "case-insensitive search should match");

    Ok(())
}

#[test]
fn mcp_symbol_search_finds_functions() -> Result<()> {
    let worktree = create_worktree()?;
    build_index(
        worktree.path(),
        &[("src/server.rs", "fn start_server() {\n    println!(\"starting\");\n}\n\nfn stop_server() {\n    println!(\"stopping\");\n}\n")],
    )?;

    let resp = mcp_call_tool(
        worktree.path(),
        "collie_search_symbols",
        serde_json::json!({ "query": "kind:fn %server%" }),
    )?;

    let json = extract_tool_json(&resp)?;
    assert_eq!(json["type"], "symbol");
    assert!(json["count"].as_u64().unwrap() >= 1);

    let results = json["results"].as_array().unwrap();
    for r in results {
        assert!(r["kind"].is_string(), "symbol results should have kind");
        assert!(r["name"].is_string(), "symbol results should have name");
        assert!(r["path"].is_string(), "symbol results should have path");
        assert!(r["line"].is_number(), "symbol results should have line");
        assert!(r["language"].is_string(), "symbol results should have language");
    }

    Ok(())
}

#[test]
fn mcp_token_search_respects_limit() -> Result<()> {
    let worktree = create_worktree()?;
    let mut files = Vec::new();
    for i in 0..10 {
        files.push((
            format!("src/file_{i:02}.rs"),
            "fn shared_token() {}".to_string(),
        ));
    }
    let tuples: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, c)| (p.as_str(), c.as_str()))
        .collect();
    build_index(worktree.path(), &tuples)?;

    let resp = mcp_call_tool(
        worktree.path(),
        "collie_search",
        serde_json::json!({ "pattern": "shared_token", "limit": 3 }),
    )?;

    let json = extract_tool_json(&resp)?;
    assert!(
        json["count"].as_u64().unwrap() <= 3,
        "limit should cap results at 3"
    );

    Ok(())
}

#[test]
fn mcp_token_search_respects_glob() -> Result<()> {
    let worktree = create_worktree()?;
    build_index(
        worktree.path(),
        &[
            ("src/handler.rs", "fn target() {}"),
            ("tests/handler_test.rs", "fn target() {}"),
        ],
    )?;

    let resp = mcp_call_tool(
        worktree.path(),
        "collie_search",
        serde_json::json!({ "pattern": "target", "glob": "src/*.rs" }),
    )?;

    let json = extract_tool_json(&resp)?;
    assert_eq!(json["count"], 1, "glob should filter to only src/");

    let results = json["results"].as_array().unwrap();
    assert_eq!(results[0]["path"], "src/handler.rs");

    Ok(())
}

// ---- Error path tests ----

#[test]
fn mcp_error_invalid_regex_returns_invalid_params() -> Result<()> {
    let worktree = create_worktree()?;
    build_index(worktree.path(), &[("src/lib.rs", "fn hello() {}")])?;

    let resp = mcp_call_tool(
        worktree.path(),
        "collie_search_regex",
        serde_json::json!({ "pattern": "(unclosed" }),
    )?;

    let error = &resp["error"];
    assert!(error.is_object(), "should return a JSON-RPC error");
    assert_eq!(error["code"], -32602, "invalid regex should be invalid_params (-32602)");
    let msg = error["message"].as_str().unwrap_or("");
    assert!(msg.contains("regex"), "error message should mention regex: {msg}");

    Ok(())
}

#[test]
fn mcp_error_invalid_glob_returns_invalid_params() -> Result<()> {
    let worktree = create_worktree()?;
    build_index(worktree.path(), &[("src/lib.rs", "fn hello() {}")])?;

    let resp = mcp_call_tool(
        worktree.path(),
        "collie_search",
        serde_json::json!({ "pattern": "hello", "glob": "[invalid" }),
    )?;

    let error = &resp["error"];
    assert!(error.is_object(), "should return a JSON-RPC error");
    assert_eq!(error["code"], -32602, "invalid glob should be invalid_params (-32602)");

    Ok(())
}

#[test]
fn mcp_error_missing_index_returns_resource_not_found() -> Result<()> {
    let worktree = create_worktree()?;
    // No build_index — index does not exist

    let resp = mcp_call_tool(
        worktree.path(),
        "collie_search",
        serde_json::json!({ "pattern": "anything" }),
    )?;

    let error = &resp["error"];
    assert!(error.is_object(), "should return a JSON-RPC error");
    assert_eq!(
        error["code"], -32002,
        "missing index should be resource_not_found (-32002)"
    );

    Ok(())
}

#[test]
fn mcp_error_symbol_query_without_filters_returns_invalid_params() -> Result<()> {
    let worktree = create_worktree()?;
    build_index(worktree.path(), &[("src/lib.rs", "fn hello() {}")])?;

    let resp = mcp_call_tool(
        worktree.path(),
        "collie_search_symbols",
        serde_json::json!({ "query": "just_a_name_no_filters" }),
    )?;

    let error = &resp["error"];
    assert!(error.is_object(), "should return a JSON-RPC error");
    assert_eq!(
        error["code"], -32602,
        "symbol query without kind:/lang: filters should be invalid_params (-32602)"
    );

    Ok(())
}
