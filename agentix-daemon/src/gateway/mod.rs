mod anthropic;
mod health;
mod infer_handler;
mod ollama_manage;
mod openai_proxy;

use crate::config::Config;
use agentix_infer::InferEngine;
use agentix_router::{RouteTarget, Router as ModelRouter};
use anyhow::Context as _;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Router,
};
use std::sync::Arc;
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub model_router: Arc<ModelRouter>,
    pub infer: InferEngine,
    pub config: Config,
    pub http: reqwest::Client,
}

pub fn router(
    model_router: Arc<ModelRouter>,
    infer: InferEngine,
    config: Config,
) -> anyhow::Result<Router> {
    let http = reqwest::Client::builder()
        .user_agent("agentix-daemon/0.1")
        .build()
        .context("failed to build HTTP client")?;

    let state = AppState {
        model_router,
        infer,
        config,
        http,
    };

    Ok(Router::new()
        .route("/health", get(health::handler))
        .route("/v1/models", get(models_handler))
        .route("/v1/chat/completions", post(chat_completions_handler))
        .route("/v1/embeddings", post(embeddings_handler))
        // Anthropic-native endpoint (for clients using the Anthropic SDK directly)
        .route("/v1/messages", post(messages_handler))
        // Ollama-compatible embedding endpoint (used by ingest/mcp-server)
        .route("/api/embed", post(ollama_embed_handler))
        // Ollama-compatible model management endpoints
        .route("/api/pull", post(ollama_manage::pull_handler))
        .route("/api/delete", delete(ollama_manage::delete_handler))
        .route("/api/tags", get(ollama_manage::tags_handler))
        .route("/api/show", post(ollama_manage::show_handler))
        .layer(TraceLayer::new_for_http())
        .with_state(state))
}

async fn chat_completions_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let req: agentix_api::ChatCompletionRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("invalid request: {e}")).into_response()
        }
    };

    // Check local store before consulting the router: HuggingFace model names contain '/'
    // (e.g. "org/repo:tag") which the router's provider/model heuristic would wrongly send
    // to OpenRouter. A local hit always wins.
    let local_resolved = resolve_local_model(&state, &req.model).await;

    let target = if local_resolved.is_some() {
        RouteTarget::Local
    } else {
        state.model_router.route(&req.model)
    };
    tracing::debug!(model = %req.model, target = ?target, "routing chat completion");

    match target {
        RouteTarget::Anthropic => anthropic::proxy_chat(&state, headers, body).await,
        RouteTarget::OpenAI => openai_proxy::proxy_chat(&state, headers, body).await,
        RouteTarget::OpenRouter => openai_proxy::proxy_openrouter(&state, headers, body).await,
        RouteTarget::Local => match local_resolved {
            Some(resolved) => infer_handler::complete(&state, &req, &resolved).await,
            None => (
                StatusCode::NOT_FOUND,
                format!(
                    "model '{}' not found in InferEngine — pull it first with POST /api/pull",
                    req.model
                ),
            )
                .into_response(),
        },
    }
}

async fn embeddings_handler(
    State(state): State<AppState>,
    _headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let model = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v["model"].as_str().map(str::to_string))
        .unwrap_or_default();

    tracing::info!(model = %model, "embeddings request");

    // Try the in-process InferEngine first
    let resp = infer_handler::embeddings(&state, body.clone()).await;

    tracing::info!(model = %model, status = %resp.status(), "infer engine response");

    // If the model isn't in the local store, fall back to Ollama
    if resp.status() == StatusCode::NOT_FOUND {
        let url = format!("{}/v1/embeddings", state.config.ollama_base_url);
        tracing::info!(model = %model, ollama_url = %url, "falling back to Ollama proxy");
        return match state
            .http
            .post(&url)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
        {
            Ok(r) => openai_proxy::relay_response(r).await,
            Err(e) => (
                StatusCode::BAD_GATEWAY,
                format!("embeddings proxy error: {e}"),
            )
                .into_response(),
        };
    }

    resp
}

async fn ollama_embed_handler(State(state): State<AppState>, body: axum::body::Bytes) -> Response {
    let resp = infer_handler::ollama_embed(&state, body.clone()).await;
    if resp.status() == StatusCode::NOT_FOUND {
        // Fall back to Ollama's /api/embed
        let url = format!("{}/api/embed", state.config.ollama_base_url);
        return match state
            .http
            .post(&url)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
        {
            Ok(r) => openai_proxy::relay_response(r).await,
            Err(e) => (StatusCode::BAD_GATEWAY, format!("embed proxy error: {e}")).into_response(),
        };
    }
    resp
}

async fn messages_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    anthropic::proxy_messages(&state, headers, body).await
}

async fn models_handler(State(state): State<AppState>) -> impl IntoResponse {
    let mut models = vec![];

    // Local models from InferEngine
    for m in state.infer.list().await {
        models.push(serde_json::json!({
            "id": m.name,
            "object": "model",
            "owned_by": "local",
        }));
    }

    if state.config.anthropic_api_key.is_some() {
        models.push(
            serde_json::json!({"id":"claude-opus-4-7","object":"model","owned_by":"anthropic"}),
        );
        models.push(
            serde_json::json!({"id":"claude-sonnet-4-6","object":"model","owned_by":"anthropic"}),
        );
        models.push(serde_json::json!({"id":"claude-haiku-4-5-20251001","object":"model","owned_by":"anthropic"}));
    }
    if state.config.openai_api_key.is_some() {
        models.push(serde_json::json!({"id":"gpt-4o","object":"model","owned_by":"openai"}));
    }
    if state.config.openrouter_api_key.is_some() {
        models.push(
            serde_json::json!({"id":"openrouter/*","object":"model","owned_by":"openrouter"}),
        );
    }

    // Also check Ollama for any additional local models
    let ollama_url = format!("{}/v1/models", state.config.ollama_base_url);
    if let Ok(resp) = state.http.get(&ollama_url).send().await {
        if let Ok(body) = resp.json::<serde_json::Value>().await {
            if let Some(data) = body["data"].as_array() {
                for m in data {
                    models.push(m.clone());
                }
            }
        }
    }

    axum::Json(serde_json::json!({ "object": "list", "data": models }))
}

/// Find a model in the InferEngine by the name a client might request.
/// Tries exact match first, then fuzzy suffix/alias matching so that e.g.
/// "deepseek-r1:7b" resolves to "registry.ollama.ai/library/deepseek-r1/7b".
/// Returns the canonical store name, or None if the model isn't local.
async fn resolve_local_model(state: &AppState, requested: &str) -> Option<String> {
    // 1. Exact match
    if state.infer.info(requested).is_some() {
        return Some(requested.to_string());
    }

    // 2. Scan all loaded models for a suffix/alias match
    let all = state.infer.list().await;

    // Normalize the requested name: "deepseek-r1:7b" → "deepseek-r1/7b"
    let normalized = requested.replace(':', "/");

    for info in &all {
        let stored = &info.name;
        // Suffix match: stored ends with the normalized requested name
        if stored.ends_with(&normalized) || stored.ends_with(requested) {
            return Some(stored.clone());
        }
        // Also match the short name after the last slash
        if let Some(short) = stored.rsplit('/').next() {
            if short == requested || short == normalized {
                return Some(stored.clone());
            }
        }
    }

    None
}
