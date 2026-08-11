/// Integration tests for agentix-daemon's HTTP API.
///
/// These tests require a running daemon. Set AGENTIX_TEST_URL to the base URL
/// (default: http://localhost:11430). Tests that need specific features are
/// skipped automatically when the daemon reports them unavailable.
///
/// Run:
///   AGENTIX_TEST_URL=http://localhost:11430 cargo test -p agentix-daemon -- --test-output immediate
use serde_json::{json, Value};

fn test_url() -> String {
    std::env::var("AGENTIX_TEST_URL").unwrap_or_else(|_| "http://localhost:11430".into())
}

/// Skip the test if the daemon isn't reachable.
macro_rules! require_daemon {
    ($client:expr, $url:expr) => {
        match $client.get(format!("{}/health", $url)).send().await {
            Ok(_) => {}
            Err(_) => {
                eprintln!("SKIP: daemon not reachable at {}", $url);
                return;
            }
        }
    };
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .unwrap()
}

// ── Health ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn health_returns_ok() {
    let url = test_url();
    let c = client();
    require_daemon!(c, url);

    let resp: Value = c
        .get(format!("{url}/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(resp["status"], "ok", "health.status must be ok");
    assert!(
        resp.get("ollama_url").is_some(),
        "health must report ollama_url"
    );
    assert!(
        resp.get("anthropic_auth").is_some(),
        "health must report anthropic_auth"
    );
    assert!(
        resp.get("openrouter").is_some(),
        "health must report openrouter"
    );
}

// ── Models list ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn models_returns_list() {
    let url = test_url();
    let c = client();
    require_daemon!(c, url);

    let resp: Value = c
        .get(format!("{url}/v1/models"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(resp["object"], "list");
    assert!(resp["data"].is_array(), "data must be an array");
}

// ── Local inference (Ollama) ──────────────────────────────────────────────────

/// Returns the first Ollama model ID from /v1/models, or None if Ollama has no models.
async fn first_ollama_model(c: &reqwest::Client, url: &str) -> Option<String> {
    let models: Value = c
        .get(format!("{url}/v1/models"))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    models["data"]
        .as_array()?
        .iter()
        .find(|m| {
            m["owned_by"].as_str() == Some("library")
                || m["owned_by"]
                    .as_str()
                    .map(|o| !["anthropic", "openai", "openrouter"].contains(&o))
                    .unwrap_or(false)
        })
        .and_then(|m| m["id"].as_str())
        .map(|s| s.to_string())
}

#[tokio::test]
async fn local_chat_completion_returns_content() {
    let url = test_url();
    let c = client();
    require_daemon!(c, url);

    let model = match first_ollama_model(&c, &url).await {
        Some(m) => m,
        None => {
            eprintln!("SKIP: no Ollama models available");
            return;
        }
    };

    let resp = c
        .post(format!("{url}/v1/chat/completions"))
        .json(&json!({
            "model": model,
            "messages": [{"role": "user", "content": "Reply with exactly: pong"}],
            "max_tokens": 16,
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200, "expected 200 for local chat");
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["object"], "chat.completion");
    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("");
    assert!(!content.is_empty(), "response content must not be empty");
}

#[tokio::test]
async fn local_chat_streaming_returns_sse() {
    let url = test_url();
    let c = client();
    require_daemon!(c, url);

    let model = match first_ollama_model(&c, &url).await {
        Some(m) => m,
        None => {
            eprintln!("SKIP: no Ollama models available");
            return;
        }
    };

    let resp = c
        .post(format!("{url}/v1/chat/completions"))
        .json(&json!({
            "model": model,
            "messages": [{"role": "user", "content": "Reply with exactly: pong"}],
            "max_tokens": 16,
            "stream": true,
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.contains("text/event-stream"),
        "streaming response must be SSE, got {ct}"
    );

    let text = resp.text().await.unwrap();
    assert!(text.contains("data:"), "SSE body must contain data: lines");
    assert!(text.contains("[DONE]"), "SSE body must end with [DONE]");
}

// ── Anthropic proxy ───────────────────────────────────────────────────────────

#[tokio::test]
async fn anthropic_proxy_chat_rejects_without_auth() {
    let url = test_url();
    let c = client();
    require_daemon!(c, url);

    let health: Value = c
        .get(format!("{url}/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    if health["anthropic_auth"] != "passthrough" {
        eprintln!("SKIP: not in passthrough mode");
        return;
    }

    let resp = c
        .post(format!("{url}/v1/chat/completions"))
        .json(&json!({
            "model": "claude-haiku-4-5-20251001",
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 8,
        }))
        .send()
        .await
        .unwrap();

    assert!(
        resp.status() == 401 || resp.status() == 400,
        "unauthenticated claude request should fail, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn anthropic_proxy_chat_with_api_key() {
    let url = test_url();
    let c = client();
    require_daemon!(c, url);

    let health: Value = c
        .get(format!("{url}/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    if health["anthropic_auth"] != "api_key" {
        eprintln!("SKIP: no API key configured");
        return;
    }

    let resp = c
        .post(format!("{url}/v1/chat/completions"))
        .json(&json!({
            "model": "claude-haiku-4-5-20251001",
            "messages": [{"role": "user", "content": "Reply with exactly: pong"}],
            "max_tokens": 16,
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        200,
        "Anthropic proxy chat should succeed with API key"
    );
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["object"], "chat.completion");
    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("");
    assert!(!content.is_empty());
}

#[tokio::test]
async fn anthropic_native_messages_endpoint() {
    let url = test_url();
    let c = client();
    require_daemon!(c, url);

    let health: Value = c
        .get(format!("{url}/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    if health["anthropic_auth"] != "api_key" {
        eprintln!("SKIP: no API key configured");
        return;
    }

    let resp = c
        .post(format!("{url}/v1/messages"))
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": "claude-haiku-4-5-20251001",
            "messages": [{"role": "user", "content": "Reply with exactly: pong"}],
            "max_tokens": 16,
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        200,
        "/v1/messages native endpoint should work"
    );
    let body: Value = resp.json().await.unwrap();
    assert!(
        body.get("content").is_some() || body.get("choices").is_some(),
        "response should have content or choices"
    );
}

// ── OpenRouter proxy ──────────────────────────────────────────────────────────

#[tokio::test]
async fn openrouter_rejects_without_key() {
    let url = test_url();
    let c = client();
    require_daemon!(c, url);

    let health: Value = c
        .get(format!("{url}/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    if health["openrouter"].as_bool() == Some(true) {
        eprintln!("SKIP: OPENROUTER_API_KEY is configured");
        return;
    }

    // provider/model format routes to OpenRouter
    let resp = c
        .post(format!("{url}/v1/chat/completions"))
        .json(&json!({
            "model": "anthropic/claude-3-haiku",
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 8,
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        401,
        "OpenRouter request without key should return 401"
    );
}

// ── Routing ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn unknown_model_routes_to_default() {
    let url = test_url();
    let c = client();
    require_daemon!(c, url);

    let resp = c
        .post(format!("{url}/v1/chat/completions"))
        .json(&json!({
            "model": "unknown-model-xyz",
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 8,
        }))
        .send()
        .await
        .unwrap();

    assert_ne!(
        resp.status().as_u16(),
        500,
        "unknown model must not cause a 500"
    );
}

#[tokio::test]
async fn bad_request_returns_400() {
    let url = test_url();
    let c = client();
    require_daemon!(c, url);

    let resp = c
        .post(format!("{url}/v1/chat/completions"))
        .header("content-type", "application/json")
        .body(r#"{"not_valid": true}"#)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400, "malformed request should return 400");
}
