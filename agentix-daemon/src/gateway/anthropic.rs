use super::AppState;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use reqwest::header::{HeaderMap as ReqwestHeaders, HeaderValue};

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Proxy the Anthropic-native /v1/messages endpoint directly (no translation needed).
pub async fn proxy_messages(
    state: &AppState,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let base = state
        .config
        .anthropic_base_url
        .as_deref()
        .unwrap_or(ANTHROPIC_API_URL);
    let url = format!("{base}/v1/messages");
    forward_to_anthropic(state, headers, body, &url).await
}

/// Proxy an OpenAI-format /v1/chat/completions request to Anthropic.
/// Translates the request format and maps the response back to OpenAI shape.
pub async fn proxy_chat(
    state: &AppState,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    // Parse the OpenAI request
    let oai_req: agentix_api::ChatCompletionRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("invalid request: {e}")).into_response()
        }
    };

    let is_streaming = oai_req.stream.unwrap_or(false);

    // Map OpenAI messages to Anthropic format, extracting any system message
    let mut system: Option<String> = None;
    let mut messages: Vec<serde_json::Value> = vec![];

    for msg in &oai_req.messages {
        match msg.role.as_str() {
            "system" => {
                let text = extract_text_content(&msg.content);
                system = Some(text);
            }
            _ => {
                // Convert content to Anthropic format
                let content = normalize_content(&msg.content);
                messages.push(serde_json::json!({
                    "role": msg.role,
                    "content": content,
                }));
            }
        }
    }

    let mut anthropic_req = serde_json::json!({
        "model": oai_req.model,
        "messages": messages,
        "max_tokens": oai_req.max_tokens.unwrap_or(4096),
    });

    if let Some(sys) = system {
        anthropic_req["system"] = serde_json::Value::String(sys);
    }
    if let Some(t) = oai_req.temperature {
        anthropic_req["temperature"] = serde_json::Value::from(t);
    }
    if is_streaming {
        anthropic_req["stream"] = serde_json::Value::Bool(true);
    }

    // Forward any tool definitions if present
    if let Some(tools) = oai_req.extra.get("tools") {
        anthropic_req["tools"] = tools.clone();
    }

    let body_bytes = match serde_json::to_vec(&anthropic_req) {
        Ok(b) => b,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let base = state
        .config
        .anthropic_base_url
        .as_deref()
        .unwrap_or(ANTHROPIC_API_URL);
    let url = format!("{base}/v1/messages");

    let req_headers = build_anthropic_headers(&state.config.anthropic_api_key, &headers);

    let resp = match state
        .http
        .post(&url)
        .headers(req_headers)
        .body(body_bytes)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                format!("Anthropic proxy error: {e}"),
            )
                .into_response()
        }
    };

    if is_streaming {
        // Return the SSE stream as-is — Anthropic streaming format is close enough
        // that most clients handle it directly. Full SSE translation is future work.
        super::openai_proxy::relay_response(resp).await
    } else {
        // Translate Anthropic response → OpenAI shape
        let status = resp.status();
        let resp_bytes = match resp.bytes().await {
            Ok(b) => b,
            Err(e) => {
                return (StatusCode::BAD_GATEWAY, format!("relay error: {e}")).into_response()
            }
        };

        if !status.is_success() {
            return (
                StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
                axum::body::Body::from(resp_bytes),
            )
                .into_response();
        }

        let anthropic_resp: serde_json::Value = match serde_json::from_slice(&resp_bytes) {
            Ok(v) => v,
            Err(_) => {
                return (StatusCode::BAD_GATEWAY, axum::body::Body::from(resp_bytes))
                    .into_response()
            }
        };

        let oai_resp = anthropic_to_openai(&anthropic_resp);
        axum::Json(oai_resp).into_response()
    }
}

async fn forward_to_anthropic(
    state: &AppState,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
    url: &str,
) -> Response {
    let req_headers = build_anthropic_headers(&state.config.anthropic_api_key, &headers);
    match state
        .http
        .post(url)
        .headers(req_headers)
        .body(body)
        .send()
        .await
    {
        Ok(resp) => super::openai_proxy::relay_response(resp).await,
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            format!("Anthropic proxy error: {e}"),
        )
            .into_response(),
    }
}

/// Build outgoing headers for an Anthropic request.
///
/// Two auth modes:
/// - API key set (`ANTHROPIC_API_KEY`): inject `x-api-key`, strip client's `authorization`.
/// - No API key (enterprise/URL login): pass through the client's `authorization` header
///   (Claude Code sends `Authorization: Bearer <session_token>` from its OAuth login).
fn build_anthropic_headers(
    api_key: &Option<String>,
    incoming: &axum::http::HeaderMap,
) -> ReqwestHeaders {
    let mut h = ReqwestHeaders::new();
    h.insert("content-type", HeaderValue::from_static("application/json"));
    h.insert(
        "anthropic-version",
        HeaderValue::from_static(ANTHROPIC_VERSION),
    );

    match api_key {
        Some(key) => {
            if let Ok(v) = HeaderValue::from_str(key) {
                h.insert("x-api-key", v);
            }
        }
        None => {
            // Pass through the client's Authorization header for enterprise session tokens.
            if let Some(auth) = incoming.get("authorization") {
                if let Ok(v) = reqwest::header::HeaderValue::from_bytes(auth.as_bytes()) {
                    h.insert("authorization", v);
                }
            }
        }
    }

    // Pass through anthropic-* headers (beta features, etc.) except version (already set).
    for (name, value) in incoming {
        let n = name.as_str().to_lowercase();
        if n.starts_with("anthropic-") && n != "anthropic-version" {
            if let Ok(v) = reqwest::header::HeaderValue::from_bytes(value.as_bytes()) {
                if let Ok(k) = reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()) {
                    h.insert(k, v);
                }
            }
        }
    }

    h
}

fn extract_text_content(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(parts) => parts
            .iter()
            .filter_map(|p| {
                if p.get("type").and_then(|t| t.as_str()) == Some("text") {
                    p.get("text")
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn normalize_content(content: &serde_json::Value) -> serde_json::Value {
    match content {
        serde_json::Value::String(s) => serde_json::json!([{"type": "text", "text": s}]),
        serde_json::Value::Array(_) => content.clone(),
        _ => serde_json::json!([{"type": "text", "text": content.to_string()}]),
    }
}

fn anthropic_to_openai(resp: &serde_json::Value) -> serde_json::Value {
    let id = resp
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("msg_unknown");
    let model = resp
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("claude");

    let text = resp
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| {
            arr.iter().find_map(|block| {
                if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                    block
                        .get("text")
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            })
        })
        .unwrap_or_default();

    let input_tokens = resp
        .get("usage")
        .and_then(|u| u.get("input_tokens"))
        .and_then(|t| t.as_u64())
        .unwrap_or(0);
    let output_tokens = resp
        .get("usage")
        .and_then(|u| u.get("output_tokens"))
        .and_then(|t| t.as_u64())
        .unwrap_or(0);

    let stop_reason = match resp
        .get("stop_reason")
        .and_then(|r| r.as_str())
        .unwrap_or("end_turn")
    {
        "end_turn" => "stop",
        "max_tokens" => "length",
        "tool_use" => "tool_calls",
        other => other,
    };

    serde_json::json!({
        "id": id,
        "object": "chat.completion",
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": text,
            },
            "finish_reason": stop_reason,
        }],
        "usage": {
            "prompt_tokens": input_tokens,
            "completion_tokens": output_tokens,
            "total_tokens": input_tokens + output_tokens,
        }
    })
}
