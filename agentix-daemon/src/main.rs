mod config;
mod gateway;

use anyhow::Result;
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "agentix_daemon=info".into()),
        )
        .init();

    let cfg = config::Config::from_env();
    info!(port = cfg.gateway_port, models_dir = %cfg.models_dir.display(), "agentix-daemon starting");

    // Initialise the in-process inference engine
    let infer_cfg = agentix_infer::InferConfig::new(
        cfg.models_dir.clone(),
        cfg.vram_limit_bytes,
        cfg.max_loaded_models,
        cfg.max_ctx,
    );
    let infer = agentix_infer::InferEngine::new(infer_cfg).await?;

    // Register the LlamaCpp backend for GGUF model inference
    use agentix_infer::backend::llamacpp::LlamaCppBackend;
    match LlamaCppBackend::new() {
        Ok(backend) => {
            infer.register_backend(Arc::new(backend));
            info!("registered LlamaCppBackend");
        }
        Err(e) => {
            tracing::warn!("LlamaCppBackend unavailable: {e} — local GGUF inference disabled");
        }
    }

    let model_router = Arc::new(agentix_router::Router::new());
    let router = gateway::router(model_router, infer, cfg.clone());

    let addr = format!("0.0.0.0:{}", cfg.gateway_port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!(addr = %addr, "gateway listening");

    axum::serve(listener, router).await?;
    Ok(())
}
