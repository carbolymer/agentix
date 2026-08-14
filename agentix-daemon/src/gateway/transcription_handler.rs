//! Placeholder for POST /v1/audio/transcriptions.
//!
//! The real implementation lives in agentix-whisper (a separate process).
//! This endpoint will proxy to agentix-whisper's Unix socket once
//! spec 010-socket-backends is implemented.

use axum::{http::StatusCode, response::{IntoResponse, Response}};

pub async fn handler() -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        "audio transcription is served by agentix-whisper — socket proxy not yet wired (see spec 010-socket-backends)",
    )
        .into_response()
}
