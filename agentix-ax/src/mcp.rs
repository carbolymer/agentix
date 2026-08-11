use agentix_harness::Tool;
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;

pub struct McpServer {
    _child: Child,
    writer: ChildStdin,
    reader: BufReader<ChildStdout>,
    next_id: u64,
}

pub struct McpToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

impl McpServer {
    async fn send(&mut self, msg: &serde_json::Value) -> Result<()> {
        let line = serde_json::to_string(msg)?;
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(())
    }

    async fn recv(&mut self) -> Result<serde_json::Value> {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).await?;
        if n == 0 {
            anyhow::bail!("MCP server closed stdout unexpectedly");
        }
        serde_json::from_str(line.trim()).context("parsing MCP JSON-RPC response")
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    async fn call_tool(&mut self, name: &str, arguments: serde_json::Value) -> Result<String> {
        let id = self.next_id();
        self.send(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }))
        .await?;

        let resp = self.recv().await?;

        if let Some(error) = resp.get("error") {
            anyhow::bail!(
                "MCP error {}: {}",
                error.get("code").and_then(|c| c.as_i64()).unwrap_or(-1),
                error
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown")
            );
        }

        let content = resp["result"]["content"]
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter(|b| b["type"].as_str() == Some("text"))
                    .filter_map(|b| b["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();

        Ok(content)
    }
}

/// Spawn an MCP server subprocess, run the initialization handshake, and
/// return the server handle (behind an Arc<Mutex>) plus the list of discovered tools.
pub async fn spawn_and_init(
    cmd: &str,
    inherit_stderr: bool,
) -> Result<(Arc<Mutex<McpServer>>, Vec<McpToolDef>)> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    anyhow::ensure!(!parts.is_empty(), "empty MCP command");

    let mut child = tokio::process::Command::new(parts[0])
        .args(&parts[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // MCP server logs go to stderr; inherit in headless mode so they're
        // visible but don't mix with the JSON-RPC stream on stdout.
        .stderr(if inherit_stderr {
            Stdio::inherit()
        } else {
            Stdio::null()
        })
        .spawn()
        .with_context(|| format!("failed to spawn MCP server: {cmd}"))?;

    let writer = child.stdin.take().context("MCP server stdin unavailable")?;
    let reader = BufReader::new(
        child
            .stdout
            .take()
            .context("MCP server stdout unavailable")?,
    );

    let mut server = McpServer {
        _child: child,
        writer,
        reader,
        next_id: 3, // 1=initialize, 2=tools/list; start tool calls at 3
    };

    // 1. Initialize
    server
        .send(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "agentix-ax", "version": "0.1.0" }
            }
        }))
        .await?;
    let init_resp = server.recv().await?;
    if let Some(err) = init_resp.get("error") {
        anyhow::bail!("MCP initialize failed: {err}");
    }

    // 2. Notify initialized (no response expected)
    server
        .send(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }))
        .await?;

    // 3. List tools (first page only)
    server
        .send(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }))
        .await?;
    let list_resp = server.recv().await?;
    if let Some(err) = list_resp.get("error") {
        anyhow::bail!("MCP tools/list failed: {err}");
    }

    let tool_defs: Vec<McpToolDef> = list_resp["result"]["tools"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|t| McpToolDef {
            name: t["name"].as_str().unwrap_or("").to_string(),
            description: t["description"].as_str().unwrap_or("").to_string(),
            // MCP uses "inputSchema", which is already JSON Schema — maps directly
            // to the harness Tool::parameters() shape.
            input_schema: t["inputSchema"].clone(),
        })
        .collect();

    tracing::info!(
        cmd,
        tools = tool_defs
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        "MCP server ready"
    );

    Ok((Arc::new(Mutex::new(server)), tool_defs))
}

/// A tool backed by an MCP server subprocess.
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub server: Arc<Mutex<McpServer>>,
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> serde_json::Value {
        self.input_schema.clone()
    }

    async fn call(&self, args: serde_json::Value) -> Result<String> {
        let mut server = self.server.lock().await;
        server.call_tool(&self.name, args).await
    }
}
