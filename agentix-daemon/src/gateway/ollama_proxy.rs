use super::openai_proxy::relay_response;
use super::AppState;
use anyhow::Context;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use tracing::{info, warn};

/// Proxy a chat completion request to the local Ollama instance.
/// If Ollama reports the model is not found, pull it first, then retry once.
pub async fn proxy_chat(state: &AppState, body: axum::body::Bytes) -> Response {
    let url = format!("{}/v1/chat/completions", state.config.ollama_base_url);

    let resp = match state
        .http
        .post(&url)
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return (StatusCode::BAD_GATEWAY, format!("Ollama proxy error: {e}")).into_response()
        }
    };

    // Ollama returns 404 with "model 'x' not found, try pulling it first" when
    // the model hasn't been pulled yet.  Pull it and retry once.
    if resp.status() == StatusCode::NOT_FOUND {
        let err_bytes = resp.bytes().await.unwrap_or_default();
        let err_str = String::from_utf8_lossy(&err_bytes);

        if err_str.contains("not found") {
            if let Some(model) = extract_model(&body) {
                info!(model, "model not in Ollama — pulling");
                match pull_model(&state.http, &state.config.ollama_base_url, &model).await {
                    Ok(()) => {
                        info!(model, "pull complete — retrying request");
                        return match state
                            .http
                            .post(&url)
                            .header("content-type", "application/json")
                            .body(body)
                            .send()
                            .await
                        {
                            Ok(r) => relay_response(r).await,
                            Err(e) => (
                                StatusCode::BAD_GATEWAY,
                                format!("Ollama error after pull: {e}"),
                            )
                                .into_response(),
                        };
                    }
                    Err(e) => {
                        warn!(model, error = %e, "model pull failed");
                        return (
                            StatusCode::BAD_GATEWAY,
                            format!("Failed to pull model '{model}': {e}"),
                        )
                            .into_response();
                    }
                }
            }
        }

        return (StatusCode::NOT_FOUND, err_bytes).into_response();
    }

    relay_response(resp).await
}

fn extract_model(body: &[u8]) -> Option<String> {
    let json: serde_json::Value = serde_json::from_slice(body).ok()?;
    json["model"].as_str().map(str::to_string)
}

async fn pull_model(
    http: &reqwest::Client,
    ollama_base_url: &str,
    model: &str,
) -> anyhow::Result<()> {
    let url = format!("{}/api/pull", ollama_base_url);

    // stream: false — Ollama buffers progress internally and returns a single
    // {"status":"success"} when the pull completes (or an error object).
    let resp = http
        .post(&url)
        .timeout(std::time::Duration::from_secs(3600))
        .json(&serde_json::json!({ "model": model, "stream": false }))
        .send()
        .await
        .context("sending pull request to Ollama")?;

    if !resp.status().is_success() {
        let body = resp.bytes().await.unwrap_or_default();
        anyhow::bail!("{}", String::from_utf8_lossy(&body));
    }

    let json: serde_json::Value = resp.json().await.context("parsing pull response")?;
    match json["status"].as_str() {
        Some("success") => Ok(()),
        other => anyhow::bail!("pull ended with unexpected status: {:?}", other),
    }
}
