use super::AppState;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use reqwest::header::HeaderMap as ReqwestHeaders;

pub async fn proxy_chat(
    state: &AppState,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let base = state
        .config
        .openai_base_url
        .as_deref()
        .unwrap_or("https://api.openai.com");

    let key = match &state.config.openai_api_key {
        Some(k) => k.clone(),
        None => return (StatusCode::UNAUTHORIZED, "OPENAI_API_KEY not configured").into_response(),
    };

    proxy_to(state, headers, body, base, &key, "OpenAI").await
}

pub async fn proxy_openrouter(
    state: &AppState,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let key = match &state.config.openrouter_api_key {
        Some(k) => k.clone(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                "OPENROUTER_API_KEY not configured",
            )
                .into_response()
        }
    };

    proxy_to(
        state,
        headers,
        body,
        "https://openrouter.ai/api/v1",
        &key,
        "OpenRouter",
    )
    .await
}

async fn proxy_to(
    state: &AppState,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
    base: &str,
    key: &str,
    label: &str,
) -> Response {
    let url = format!("{base}/v1/chat/completions");

    let mut req_headers = ReqwestHeaders::new();
    req_headers.insert("content-type", "application/json".parse().unwrap());
    req_headers.insert("authorization", format!("Bearer {key}").parse().unwrap());

    for (name, value) in &headers {
        let n = name.as_str().to_lowercase();
        if n == "authorization" || n == "content-type" || n == "host" || n == "content-length" {
            continue;
        }
        if let Ok(v) = reqwest::header::HeaderValue::from_bytes(value.as_bytes()) {
            if let Ok(k) = reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()) {
                req_headers.insert(k, v);
            }
        }
    }

    match state
        .http
        .post(&url)
        .headers(req_headers)
        .body(body)
        .send()
        .await
    {
        Ok(resp) => relay_response(resp).await,
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{label} proxy error: {e}")).into_response(),
    }
}

/// Stream or buffer a reqwest response back as an axum Response.
pub async fn relay_response(resp: reqwest::Response) -> Response {
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = axum::response::Response::builder().status(status);

    for (k, v) in resp.headers() {
        if k == "transfer-encoding" {
            continue;
        }
        builder = builder.header(k.as_str(), v.as_bytes());
    }

    let body = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_GATEWAY, format!("relay error: {e}")).into_response(),
    };

    builder
        .body(axum::body::Body::from(body))
        .unwrap_or_else(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response())
}
