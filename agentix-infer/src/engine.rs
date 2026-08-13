use crate::{
    backend::{CompletionStream, InferBackend},
    pool::ContextPool,
    store::ModelStore,
    CompletionRequest, InferConfig, InferError, ModelInfo,
};
use std::sync::{Arc, RwLock};

struct EngineInner {
    config: InferConfig,
    pool: ContextPool,
    backends: RwLock<Vec<Arc<dyn InferBackend>>>,
}

/// Top-level inference handle. Clone is cheap (Arc internals).
#[derive(Clone)]
pub struct InferEngine {
    inner: Arc<EngineInner>,
}

impl InferEngine {
    pub async fn new(config: InferConfig) -> Result<Self, InferError> {
        std::fs::create_dir_all(&config.models_dir)?;
        let pool = ContextPool::new(config.vram_limit_bytes, config.max_loaded_models);
        Ok(Self {
            inner: Arc::new(EngineInner {
                pool,
                config,
                backends: RwLock::new(vec![]),
            }),
        })
    }

    pub fn register_backend(&self, backend: Arc<dyn InferBackend>) {
        self.inner
            .backends
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .push(backend);
    }

    // ── Model management ─────────────────────────────────────────────────────

    pub async fn pull(&self, model_ref: &str) -> Result<ModelInfo, InferError> {
        let store = self.store();
        let model_ref = model_ref.to_string();
        tokio::task::spawn_blocking(move || store.pull(&model_ref))
            .await
            .map_err(|e| InferError::Backend(e.to_string()))?
    }

    pub async fn list(&self) -> Vec<ModelInfo> {
        let store = self.store();
        tokio::task::spawn_blocking(move || store.list())
            .await
            .unwrap_or_default()
    }

    pub async fn remove(&self, name: &str) -> Result<(), InferError> {
        let store = self.store();
        let name = name.to_string();
        tokio::task::spawn_blocking(move || store.remove(&name))
            .await
            .map_err(|e| InferError::Backend(e.to_string()))?
    }

    pub fn info(&self, name: &str) -> Option<ModelInfo> {
        self.store().info(name)
    }

    /// Returns the names of all registered backends (e.g. `["llamacpp"]`).
    pub fn backend_names(&self) -> Vec<&'static str> {
        self.inner
            .backends
            .read()
            .map(|b| b.iter().map(|b| b.name()).collect())
            .unwrap_or_default()
    }

    // ── Inference ────────────────────────────────────────────────────────────

    pub async fn embed(&self, model: &str, input: &str) -> Result<Vec<f32>, InferError> {
        let guard = self.acquire(model).await?;
        guard.embed(input).await
    }

    pub async fn embed_batch(
        &self,
        model: &str,
        inputs: &[&str],
    ) -> Result<Vec<Vec<f32>>, InferError> {
        let guard = self.acquire(model).await?;
        guard.embed_batch(inputs).await
    }

    pub async fn complete(
        &self,
        model: &str,
        req: CompletionRequest,
    ) -> Result<CompletionStream, InferError> {
        let guard = self.acquire(model).await?;
        guard.complete(req).await
    }

    // ── Internal ─────────────────────────────────────────────────────────────

    fn store(&self) -> ModelStore {
        ModelStore::new(self.inner.config.models_dir.clone())
    }

    async fn acquire(&self, model: &str) -> Result<crate::pool::ModelGuard, InferError> {
        if let Some(guard) = self.inner.pool.acquire_idle(model) {
            return Ok(guard);
        }

        let store = self.store();
        let info = store
            .info(model)
            .ok_or_else(|| InferError::ModelNotFound(model.to_string()))?;

        let blob_path = store
            .resolve(model)
            .ok_or_else(|| InferError::ModelNotFound(model.to_string()))?;

        let backend = {
            let backends = self
                .inner
                .backends
                .read()
                .map_err(|_| InferError::Backend("backends lock poisoned".to_string()))?;
            backends
                .iter()
                .find(|b| b.supports_format(info.format))
                .cloned()
                .ok_or(InferError::NoBackend(info.format))?
        };

        tracing::info!(model = %model, backend = %backend.name(), "loading model");
        let loaded = backend.load(&blob_path, &info).await?;
        let vram = loaded.vram_bytes();

        self.inner.pool.store_loaded(model, loaded, vram)
    }
}
