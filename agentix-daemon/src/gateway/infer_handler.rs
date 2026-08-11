use super::AppState;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

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
