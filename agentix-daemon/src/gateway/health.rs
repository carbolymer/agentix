use super::AppState;
use axum::{extract::State, response::IntoResponse};

pub async fn handler(State(state): State<AppState>) -> impl IntoResponse {
    let cfg = &state.config;
    let anthropic_auth = if cfg.anthropic_api_key.is_some() {
        "api_key"
    } else {
        "passthrough"
    };

    let local_models = state.infer.list().await;
    let backends = state.infer.backend_names();
    let infer_active = !backends.is_empty();
    let infer_status = serde_json::json!({
        "active": infer_active,
        "backends": backends,
        "model_count": local_models.len(),
        "models": local_models.iter().map(|m| &m.name).collect::<Vec<_>>(),
    });

    axum::Json(serde_json::json!({
        "status": "ok",
        "infer": infer_status,
        "ollama_url": cfg.ollama_base_url,
        "anthropic_auth": anthropic_auth,
        "openai_proxy": cfg.openai_api_key.is_some(),
        "openrouter": cfg.openrouter_api_key.is_some(),
    }))
}
