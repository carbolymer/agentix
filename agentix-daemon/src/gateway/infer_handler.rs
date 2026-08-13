use super::AppState;
use agentix_infer::{CompletionMessage, CompletionRequest, FinishReason};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use tokio_stream::StreamExt;

/// Handle POST /api/embed (Ollama format) by routing to the in-process InferEngine.
/// Request:  {"model": "...", "input": [...]}
/// Response: {"embeddings": [[...], ...]}
pub async fn ollama_embed(state: &AppState, body: axum::body::Bytes) -> Response {
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("invalid request: {e}")).into_response()
        }
    };

    let model = match req["model"].as_str() {
        Some(m) => m.to_string(),
        None => return (StatusCode::BAD_REQUEST, "missing 'model' field").into_response(),
    };

    let inputs: Vec<String> = match &req["input"] {
        serde_json::Value::String(s) => vec![s.clone()],
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => return (StatusCode::BAD_REQUEST, "input must be string or array").into_response(),
    };

    let input_refs: Vec<&str> = inputs.iter().map(String::as_str).collect();

    match state.infer.embed_batch(&model, &input_refs).await {
        Ok(embeddings) => axum::Json(serde_json::json!({ "embeddings": embeddings })).into_response(),
        Err(agentix_infer::InferError::ModelNotFound(_)) => {
            (StatusCode::NOT_FOUND, "model not in local store").into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("inference error: {e}"),
        )
            .into_response(),
    }
}

/// Handle POST /v1/embeddings by routing to the in-process InferEngine.
pub async fn embeddings(state: &AppState, body: axum::body::Bytes) -> Response {
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("invalid request: {e}")).into_response()
        }
    };

    let model = match req["model"].as_str() {
        Some(m) => m.to_string(),
        None => return (StatusCode::BAD_REQUEST, "missing 'model' field").into_response(),
    };

    // Collect inputs: either a string or an array of strings
    let inputs: Vec<String> = match &req["input"] {
        serde_json::Value::String(s) => vec![s.clone()],
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => return (StatusCode::BAD_REQUEST, "input must be string or array").into_response(),
    };

    let input_refs: Vec<&str> = inputs.iter().map(String::as_str).collect();

    let model_info = state.infer.info(&model);
    tracing::info!(
        model = %model,
        found_in_store = model_info.is_some(),
        capabilities = ?model_info.as_ref().map(|m| &m.capabilities),
        "embed_batch dispatch"
    );

    match state.infer.embed_batch(&model, &input_refs).await {
        Ok(embeddings) => {
            let data: Vec<serde_json::Value> = embeddings
                .into_iter()
                .enumerate()
                .map(|(i, emb)| {
                    serde_json::json!({
                        "object": "embedding",
                        "index": i,
                        "embedding": emb,
                    })
                })
                .collect();

            axum::Json(serde_json::json!({
                "object": "list",
                "model": model,
                "data": data,
                "usage": {
                    "prompt_tokens": 0,
                    "total_tokens": 0,
                }
            }))
            .into_response()
        }
        Err(agentix_infer::InferError::ModelNotFound(_)) => {
            // Model not in the local store — fall through to Ollama proxy
            // Caller checks this sentinel to decide whether to proxy
            (StatusCode::NOT_FOUND, "model not in local store").into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("inference error: {e}"),
        )
            .into_response(),
    }
}

/// Handle POST /v1/chat/completions for a model in the InferEngine.
/// Supports both streaming (SSE) and non-streaming JSON responses.
pub async fn complete(
    state: &AppState,
    api_req: &agentix_api::ChatCompletionRequest,
    resolved_model: &str,
) -> Response {
    let mut messages: Vec<CompletionMessage> = Vec::with_capacity(api_req.messages.len());
    for m in &api_req.messages {
        let content = match normalize_content(&m.content) {
            Ok(s) => s,
            Err(e) => {
                return (StatusCode::BAD_REQUEST, e).into_response();
            }
        };
        messages.push(CompletionMessage {
            role: m.role.clone(),
            content,
        });
    }

    let stop: Vec<String> = api_req
        .extra
        .get("stop")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let req = CompletionRequest {
        messages,
        max_tokens: api_req.max_tokens,
        temperature: api_req.temperature,
        top_p: api_req.extra.get("top_p").and_then(|v| v.as_f64()).map(|f| f as f32),
        stop,
    };

    let stream = match state.infer.complete(resolved_model, req).await {
        Ok(s) => s,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("complete error: {e}"))
                .into_response()
        }
    };

    let model_id = api_req.model.clone();
    let completion_id = format!("chatcmpl-{}", uuid_simple());
    let created = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if api_req.stream.unwrap_or(false) {
        // SSE streaming — each chunk becomes a `data: {...}` line
        let sse_stream = stream.map(move |result| {
            let chunk = match result {
                Ok(c) => c,
                Err(e) => {
                    let data = format!("data: {{\"error\":\"{e}\"}}\n\n");
                    return Ok::<_, std::convert::Infallible>(data);
                }
            };

            let finish_reason = chunk.finish_reason.as_ref().map(|r| match r {
                FinishReason::Stop => "stop",
                FinishReason::Length => "length",
                FinishReason::Error => "error",
            });

            let json = serde_json::json!({
                "id": completion_id,
                "object": "chat.completion.chunk",
                "created": created,
                "model": model_id,
                "choices": [{
                    "index": 0,
                    "delta": { "content": chunk.delta },
                    "finish_reason": finish_reason,
                }]
            });
            let data = format!("data: {}\n\n", serde_json::to_string(&json).unwrap_or_default());
            Ok::<_, std::convert::Infallible>(data)
        });

        use axum::body::Body;
        use axum::http::header;

        let done = tokio_stream::iter([Ok::<_, std::convert::Infallible>(
            "data: [DONE]\n\n".to_string(),
        )]);
        let body = Body::from_stream(sse_stream.chain(done));

        axum::response::Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .header(header::CONNECTION, "keep-alive")
            .body(body)
            .unwrap_or_else(|e| {
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
            })
    } else {
        // Non-streaming: collect all chunks then return a single JSON object
        let mut full_content = String::new();
        let mut finish_reason = "stop";
        let mut stream = stream;

        while let Some(result) = stream.next().await {
            match result {
                Ok(chunk) => {
                    full_content.push_str(&chunk.delta);
                    if let Some(reason) = &chunk.finish_reason {
                        finish_reason = match reason {
                            FinishReason::Stop => "stop",
                            FinishReason::Length => "length",
                            FinishReason::Error => "error",
                        };
                    }
                }
                Err(e) => {
                    return (
                        if matches!(e, agentix_infer::InferError::ContextExceeded { .. }) {
                            StatusCode::BAD_REQUEST
                        } else {
                            StatusCode::INTERNAL_SERVER_ERROR
                        },
                        format!("stream error: {e}"),
                    )
                        .into_response()
                }
            }
        }

        axum::Json(serde_json::json!({
            "id": completion_id,
            "object": "chat.completion",
            "created": created,
            "model": model_id,
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": full_content,
                },
                "finish_reason": finish_reason,
            }],
            "usage": {
                "prompt_tokens": 0,
                "completion_tokens": 0,
                "total_tokens": 0,
            }
        }))
        .into_response()
    }
}

fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("{t:08x}")
}

fn normalize_content(content: &serde_json::Value) -> Result<String, String> {
    match content {
        serde_json::Value::String(s) => Ok(s.clone()),
        serde_json::Value::Array(parts) => {
            let mut text = String::new();
            let mut has_images = false;
            for part in parts {
                match part.get("type").and_then(|t| t.as_str()) {
                    Some("text") => {
                        if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                            text.push_str(t);
                        }
                    }
                    Some("image_url") | Some("image") => {
                        has_images = true;
                    }
                    _ => {}
                }
            }
            if has_images && text.is_empty() {
                Err("request contains only image content; vision is not yet supported by InferEngine — use a vision-capable API backend".to_string())
            } else {
                if has_images {
                    tracing::warn!("image content parts in request ignored — vision not yet supported");
                }
                Ok(text)
            }
        }
        other => Ok(other.to_string()),
    }
}
