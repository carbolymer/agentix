//! Integration tests for ModelStore.
//! Requires AGENTIX_TEST_MODEL_PATH env var pointing to a small GGUF file.
//! These tests are skipped if the env var is not set.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use agentix_infer::store::ModelStore;
use agentix_infer::ModelFormat;

#[test]
fn local_pull_list_remove() {
    let model_path = match std::env::var("AGENTIX_TEST_MODEL_PATH") {
        Ok(p) => p,
        Err(_) => {
            eprintln!("AGENTIX_TEST_MODEL_PATH not set; skipping store integration test");
            return;
        }
    };

    let dir = tempfile::tempdir().expect("temp dir");
    let store = ModelStore::new(dir.path().to_path_buf());

    // Pull from local path
    let info = store.pull(&model_path).expect("pull failed");
    assert!(!info.name.is_empty());
    assert!(!info.capabilities.is_empty());
    assert_eq!(info.format, ModelFormat::Gguf);

    // List should contain the model
    let models = store.list();
    assert!(
        models.iter().any(|m| m.name.contains(&info.name)),
        "model not in list: {:?}",
        models.iter().map(|m| &m.name).collect::<Vec<_>>()
    );

    // info() should find it
    let found = store.info(&info.name).expect("info() returned None");
    assert_eq!(found.name, info.name);

    // Resolve should return a valid blob path
    let blob_path = store.resolve(&info.name).expect("resolve returned None");
    assert!(
        blob_path.exists(),
        "blob file missing: {}",
        blob_path.display()
    );

    // Remove
    store.remove(&info.name).expect("remove failed");
    assert!(
        store.info(&info.name).is_none(),
        "model still found after remove"
    );
    assert!(!blob_path.exists(), "blob file not deleted after remove");
}
