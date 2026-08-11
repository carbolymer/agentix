use crate::tool::{Tool, ToolCall};
use anyhow::{Context, Result};

/// Parsed response from a chat completion call.
pub struct ChatResponse {
    /// Text content of the assistant message, if any (absent when finish_reason is "tool_calls").
    pub content: Option<String>,
    /// Tool calls requested by the model; empty on plain-text responses.
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: String,
}

/// Thin async wrapper around the agentix-daemon OpenAI-compatible HTTP API.
pub struct GatewayClient {
    http: reqwest::Client,
    base_url: String,
}

impl GatewayClient {
    pub fn new(base_url: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent("agentix-harness/0.1")
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    /// Send a chat completion. Pass `tools = &[]` for tool-free calls (e.g. final answer).
    pub async fn chat(
        &self,
        model: &str,
        messages: &[serde_json::Value],
        tools: &[&dyn Tool],
    ) -> Result<ChatResponse> {
        let mut body = serde_json::json!({
            "model": model,
            "messages": messages,
        });

        if !tools.is_empty() {
            let defs: Vec<serde_json::Value> = tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": t.name(),
                            "description": t.description(),
                            "parameters": t.parameters(),
                        }
                    })
                })
                .collect();
            body["tools"] = serde_json::Value::Array(defs);
        }

        let url = format!("{}/v1/chat/completions", self.base_url);
        let resp = self
            .http
            .post(&url)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .context("HTTP request to gateway failed")?;

        let status = resp.status();
        let bytes = resp.bytes().await.context("reading response body")?;

        if !status.is_success() {
            anyhow::bail!(
                "gateway returned {}: {}",
                status,
                String::from_utf8_lossy(&bytes)
            );
        }

        let json: serde_json::Value =
            serde_json::from_slice(&bytes).context("deserializing gateway response")?;

        parse_response(&json)
    }
}

fn parse_response(json: &serde_json::Value) -> Result<ChatResponse> {
    let choice = json
        .get("choices")
        .and_then(|c| c.get(0))
        .context("no choices in response")?;

    let finish_reason = choice
        .get("finish_reason")
        .and_then(|r| r.as_str())
        .unwrap_or("stop")
        .to_string();

    let message = choice.get("message").context("no message in choice")?;

    let content = message
        .get("content")
        .and_then(|c| c.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let mut tool_calls = vec![];
    if let Some(calls) = message.get("tool_calls").and_then(|tc| tc.as_array()) {
        for call in calls {
            let id = call
                .get("id")
                .and_then(|i| i.as_str())
                .unwrap_or("")
                .to_string();

            let function = call.get("function").context("tool_call missing function")?;

            let name = function
                .get("name")
                .and_then(|n| n.as_str())
                .context("tool_call function missing name")?
                .to_string();

            // arguments is a JSON-encoded string on the wire, not a JSON object.
            let arguments_str = function
                .get("arguments")
                .and_then(|a| a.as_str())
                .unwrap_or("{}");
            let arguments: serde_json::Value =
                serde_json::from_str(arguments_str).unwrap_or(serde_json::json!({}));

            tool_calls.push(ToolCall {
                id,
                name,
                arguments,
            });
        }
    }

    Ok(ChatResponse {
        content,
        tool_calls,
        finish_reason,
    })
}
