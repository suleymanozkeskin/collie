use std::path::PathBuf;

use anyhow::{Context, Result};
use rmcp::{ServiceExt, transport::stdio};

use crate::mcp::CollieServer;

pub fn run(path: PathBuf) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to create tokio runtime")?;

    rt.block_on(async {
        let server = CollieServer::new(path).map_err(|e| anyhow::anyhow!("{}", e.message))?;
        let service = server
            .serve(stdio())
            .await
            .context("failed to start MCP server")?;
        service
            .waiting()
            .await
            .context("MCP server terminated with error")?;
        Ok(())
    })
}
