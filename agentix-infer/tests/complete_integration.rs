//! Integration test for the capability guard on the completion path (T013).
//!
//! Loads the all-MiniLM-L6-v2-Q8_0.gguf embedding fixture and calls
//! `engine.complete()`. The model advertises only `Capability::Embedding`,
//! so the call must return `InferError::CapabilityMissing` synchronously.
//!
//! Requires:
//!   AGENTIX_TEST_MODEL_PATH — path to a small GGUF embedding model
//!
//! The test is skipped (not failed) when the env var is unset.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use agentix_infer::{
    backend::llamacpp::LlamaCppBackend, Capability, CompletionMessage, CompletionRequest,
    InferConfig, InferEngine, InferError,
};
use std::sync::Arc;

#[tokio::test]
async fn complete_capability_missing() {
    let model_path = match std::env::var("AGENTIX_TEST_MODEL_PATH") {
        Ok(p) => p,
        Err(_) => {
            eprintln!(
                "AGENTIX_TEST_MODEL_PATH not set — skipping complete_capability_missing test"
            );
            return;
        }
    };

    let dir = tempfile::tempdir().expect("temp dir");
    let config = InferConfig::new(dir.path().to_path_buf(), None, 1, 4096);

    let engine = InferEngine::new(config).await.expect("engine init");
    let backend = LlamaCppBackend::new().expect("LlamaCppBackend::new");
    engine.register_backend(Arc::new(backend));

    let info = engine.pull(&model_path).await.expect("pull failed");
    assert!(
        info.capabilities.contains(&Capability::Embedding),
        "fixture model should report Embedding capability; got: {:?}",
        info.capabilities
    );
    assert!(
        !info.capabilities.contains(&Capability::Completion),
        "fixture model should NOT report Completion capability; got: {:?}",
        info.capabilities
    );

    let req = CompletionRequest::new(
        vec![CompletionMessage::new("user", "hi")],
        Some(10),
        None,
        None,
        vec![],
    );

    let result = engine.complete(&info.name, req).await;

    match result {
        Err(InferError::CapabilityMissing(name, cap)) => {
            assert_eq!(cap, Capability::Completion, "wrong capability in error");
            assert_eq!(name, info.name, "wrong model name in error");
            eprintln!("complete_capability_missing passed: got CapabilityMissing({name}, {cap:?})");
        }
        Err(other) => panic!("expected CapabilityMissing, got: {other:?}"),
        Ok(_) => panic!("expected CapabilityMissing error, got Ok(stream)"),
    }
}
