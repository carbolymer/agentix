use super::AppState;
use axum::{
    body::{Body, Bytes},
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt as _};

// ── /api/pull ─────────────────────────────────────────────────────────────────

pub async fn pull_handler(State(state): State<AppState>, body: Bytes) -> Response {
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("invalid request: {e}")).into_response()
        }
    };

    let model_ref = match req["model"].as_str() {
        Some(m) => m.to_string(),
        None => return (StatusCode::BAD_REQUEST, "missing 'model' field").into_response(),
    };

    let stream_mode = req["stream"].as_bool().unwrap_or(true);

    if !stream_mode {
        return match state.infer.pull(&model_ref).await {
            Ok(_) => axum::Json(serde_json::json!({"status": "success"})).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response(),
        };
    }

    // Streaming NDJSON
    let (tx, rx) = mpsc::channel::<Bytes>(8);

    tokio::spawn(async move {
        macro_rules! send {
            ($json:expr) => {
                let line = format!("{}\n", $json);
                if tx.send(Bytes::from(line)).await.is_err() {
                    return;
                }
            };
        }

        send!(serde_json::json!({"status": "pulling manifest"}).to_string());

        match state.infer.pull(&model_ref).await {
            Ok(info) => {
                send!(serde_json::json!({
                    "status": format!("pulling {}", info.name),
                    "total": info.size_bytes,
                    "completed": info.size_bytes,
                })
                .to_string());
                send!(serde_json::json!({"status": "verifying sha256 digest"}).to_string());
                send!(serde_json::json!({"status": "writing manifest"}).to_string());
                send!(serde_json::json!({"status": "removing any unused layers"}).to_string());
                send!(serde_json::json!({"status": "success"}).to_string());
            }
            Err(e) => {
                send!(serde_json::json!({"error": e.to_string()}).to_string());
            }
        }
    });

    let stream = ReceiverStream::new(rx).map(Ok::<_, std::io::Error>);
    Body::from_stream(stream).into_response()
}

// ── /api/delete ───────────────────────────────────────────────────────────────

pub async fn delete_handler(State(state): State<AppState>, body: Bytes) -> Response {
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("invalid request: {e}")).into_response()
        }
    };

    let name = match req["model"].as_str().or_else(|| req["name"].as_str()) {
        Some(n) => n.to_string(),
        None => return (StatusCode::BAD_REQUEST, "missing 'model' field").into_response(),
    };

    match state.infer.remove(&name).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(agentix_infer::InferError::ModelNotFound(_)) => {
            (StatusCode::NOT_FOUND, format!("model '{name}' not found")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ── /api/tags ─────────────────────────────────────────────────────────────────

pub async fn tags_handler(State(state): State<AppState>) -> impl IntoResponse {
    let models = state.infer.list().await;

    let tags: Vec<serde_json::Value> = models
        .iter()
        .map(|m| {
            serde_json::json!({
                "name": m.name,
                "modified_at": "1970-01-01T00:00:00Z",
                "size": m.size_bytes,
                "details": {
                    "format": format!("{:?}", m.format).to_lowercase(),
                    "family": m.architecture,
                    "parameter_size": format_param_count(m.parameter_count),
                    "quantization_level": m.quantization.as_deref().unwrap_or("unknown"),
                }
            })
        })
        .collect();

    axum::Json(serde_json::json!({"models": tags}))
}

// ── /api/show ─────────────────────────────────────────────────────────────────

pub async fn show_handler(State(state): State<AppState>, body: Bytes) -> Response {
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("invalid request: {e}")).into_response()
        }
    };

    let name = match req["name"].as_str().or_else(|| req["model"].as_str()) {
        Some(n) => n,
        None => return (StatusCode::BAD_REQUEST, "missing 'name' field").into_response(),
    };

    match state.infer.info(name) {
        Some(info) => axum::Json(serde_json::json!({
            "modelinfo": {
                "general.architecture": info.architecture,
                "general.parameter_count": info.parameter_count,
            },
            "details": {
                "format": format!("{:?}", info.format).to_lowercase(),
                "family": info.architecture,
                "families": [info.architecture],
                "parameter_size": format_param_count(info.parameter_count),
                "quantization_level": info.quantization.as_deref().unwrap_or("unknown"),
            },
            "capabilities": info.capabilities
                .iter()
                .map(|c| format!("{c:?}").to_lowercase())
                .collect::<Vec<_>>(),
        }))
        .into_response(),
        None => (StatusCode::NOT_FOUND, format!("model '{name}' not found")).into_response(),
    }
}

fn format_param_count(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{}B", n / 1_000_000_000)
    } else if n >= 1_000_000 {
        format!("{}M", n / 1_000_000)
    } else {
        format!("{n}")
    }
}
