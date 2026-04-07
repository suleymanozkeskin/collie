use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::json;

pub fn run(path: PathBuf, target: &str) -> Result<()> {
    let root = std::fs::canonicalize(&path)
        .with_context(|| format!("invalid path: {:?}", path))?;

    let collie_bin = std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("collie"));

    match target {
        "claude" => write_claude_config(&root, &collie_bin),
        "vscode" => write_vscode_config(&root, &collie_bin),
        other => anyhow::bail!(
            "unknown target {:?}. Supported: claude, vscode",
            other
        ),
    }
}

fn write_claude_config(root: &PathBuf, collie_bin: &PathBuf) -> Result<()> {
    let config_path = root.join(".mcp.json");

    let mut config: serde_json::Value = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("invalid JSON in {}", config_path.display()))?
    } else {
        json!({ "mcpServers": {} })
    };

    config["mcpServers"]["collie"] = json!({
        "type": "stdio",
        "command": collie_bin.to_string_lossy(),
        "args": ["mcp-serve", "--path", root.to_string_lossy()],
        "env": {}
    });

    let output = serde_json::to_string_pretty(&config)?;
    std::fs::write(&config_path, format!("{output}\n"))
        .with_context(|| format!("failed to write {}", config_path.display()))?;

    println!("Wrote Collie MCP config to {}", config_path.display());
    Ok(())
}

fn write_vscode_config(root: &PathBuf, collie_bin: &PathBuf) -> Result<()> {
    let vscode_dir = root.join(".vscode");
    std::fs::create_dir_all(&vscode_dir)?;
    let config_path = vscode_dir.join("mcp.json");

    let mut config: serde_json::Value = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("invalid JSON in {}", config_path.display()))?
    } else {
        json!({ "servers": {} })
    };

    config["servers"]["collie"] = json!({
        "command": collie_bin.to_string_lossy(),
        "args": ["mcp-serve", "--path", "${workspaceFolder}"]
    });

    let output = serde_json::to_string_pretty(&config)?;
    std::fs::write(&config_path, format!("{output}\n"))
        .with_context(|| format!("failed to write {}", config_path.display()))?;

    println!("Wrote Collie MCP config to {}", config_path.display());
    Ok(())
}
