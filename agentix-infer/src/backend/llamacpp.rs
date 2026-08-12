use crate::{
    backend::{CompletionStream, InferBackend, LoadedModel},
    Capability, CompletionRequest, InferError, ModelFormat, ModelInfo,
};
use std::{num::NonZeroU32, path::Path, sync::Arc};

use llama_cpp_2::{
    context::params::{LlamaContextParams, LlamaPoolingType},
    llama_backend::LlamaBackend,
    llama_batch::LlamaBatch,
    model::{params::LlamaModelParams, AddBos, LlamaModel},
};

// True encoder-only architectures. These support llama_encode() and use
// CLS/last-token pooling. Everything else (qwen2, llama, mistral, …) is a
// decoder model and must use llama_decode() with all-token output for
// mean-pooling based embeddings.
const ENCODER_ARCHS: &[&str] = &["bert", "nomic_bert", "roberta", "xlm_roberta"];

// Message type for the inference thread
enum InferMessage {
    EmbedBatch {
        inputs: Vec<String>,
        reply: tokio::sync::oneshot::Sender<Result<Vec<Vec<f32>>, InferError>>,
    },
}

pub struct LlamaCppLoadedModel {
    tx: std::sync::mpsc::SyncSender<InferMessage>,
    vram_est: u64,
}

pub struct LlamaCppBackend {
    backend: Arc<LlamaBackend>,
    /// Number of model layers to offload to GPU. `u32::MAX` = all layers.
    /// Reads `AGENTIX_GPU_LAYERS` env var; defaults to `u32::MAX` when the
    /// `cuda` feature is enabled, 0 (CPU-only) otherwise.
    n_gpu_layers: u32,
}

impl LlamaCppBackend {
    pub fn new() -> Result<Self, InferError> {
        let mut backend = LlamaBackend::init()
            .map_err(|e| InferError::Backend(format!("llama.cpp init failed: {:?}", e)))?;

        // Redirect noisy llama.cpp C-library logs through Rust's tracing
        // so they honour the configured log level instead of flooding stderr.
        backend.void_logs();

        let n_gpu_layers = std::env::var("AGENTIX_GPU_LAYERS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(if cfg!(feature = "cuda") { u32::MAX } else { 0 });

        tracing::info!(n_gpu_layers, cuda = cfg!(feature = "cuda"), "LlamaCppBackend initialised");
        Ok(Self {
            backend: Arc::new(backend),
            n_gpu_layers,
        })
    }
}

#[async_trait::async_trait]
impl InferBackend for LlamaCppBackend {
    fn name(&self) -> &'static str {
        "llamacpp"
    }

    fn supports_format(&self, format: ModelFormat) -> bool {
        format == ModelFormat::Gguf
    }

    async fn load(
        &self,
        blob_path: &Path,
        info: &ModelInfo,
    ) -> Result<Arc<dyn LoadedModel>, InferError> {
        let path = blob_path.to_path_buf();
        let backend = Arc::clone(&self.backend);

        // When the manifest doesn't explicitly include Embedding, re-read GGUF
        // to confirm — this catches stale manifests written before the name
        // heuristic was added (e.g. jina models with no pooling_type key).
        let (is_embedding, gguf_meta) = if !info.capabilities.contains(&Capability::Embedding) {
            match crate::meta::gguf::read_gguf_metadata(&path) {
                Ok(m) => {
                    let emb = m.capabilities.contains(&Capability::Embedding);
                    (emb, Some(m))
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), err = %e, "GGUF metadata read failed; assuming completion-only");
                    (false, None)
                }
            }
        } else {
            (true, None)
        };

        // Prefer GGUF-derived architecture (fresh read) over manifest (may be stale).
        let architecture = gguf_meta
            .as_ref()
            .map(|m| m.architecture.clone())
            .unwrap_or_else(|| info.architecture.clone());

        // Encoder-only models use llama_encode() + last-token output.
        // Decoder models used as embedding models use llama_decode() + all-token output.
        let use_encoder_path = ENCODER_ARCHS.contains(&architecture.as_str());

        tracing::info!(
            model = %info.name,
            architecture = %architecture,
            is_embedding,
            use_encoder_path,
            n_gpu_layers = self.n_gpu_layers,
            "loading model",
        );

        let n_ctx_val = info.context_length.clamp(64, 4096).max(256);
        let size_bytes = info.size_bytes;

        let n_gpu_layers = self.n_gpu_layers;

        // Phase 1: load the model weights (blocking; model is Send)
        let model = tokio::task::spawn_blocking({
            let backend = Arc::clone(&backend);
            move || {
                let params = LlamaModelParams::default().with_n_gpu_layers(n_gpu_layers);
                LlamaModel::load_from_file(&backend, &path, &params)
                    .map_err(|e| InferError::Backend(format!("model load failed: {e:?}")))
            }
        })
        .await
        .map_err(|e| InferError::Backend(e.to_string()))??;

        // Phase 2: spawn a dedicated thread that owns model + context
        let (tx, rx) = std::sync::mpsc::sync_channel::<InferMessage>(16);

        std::thread::Builder::new()
            .name(format!(
                "llama-{}",
                blob_path
                    .file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default()
            ))
            .spawn(move || {
                // n_ctx_val >= 256 due to clamping; NonZeroU32::MIN (1) is an unreachable fallback
                let n_ctx = NonZeroU32::new(n_ctx_val).unwrap_or(NonZeroU32::MIN);
                // Decoder-based embedding models need explicit mean pooling — the GGUF
                // has no pooling_type key so llama.cpp defaults to Unspecified (no
                // per-sequence pool), which makes embeddings_seq_ith return null.
                let pooling_type = if is_embedding && !use_encoder_path {
                    LlamaPoolingType::Mean
                } else {
                    LlamaPoolingType::Unspecified
                };
                let ctx_params = LlamaContextParams::default()
                    .with_n_ctx(Some(n_ctx))
                    .with_embeddings(is_embedding)
                    .with_pooling_type(pooling_type);

                let mut ctx = match model.new_context(&backend, ctx_params) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!("context creation failed: {:?}", e);
                        return;
                    }
                };

                for msg in rx {
                    match msg {
                        InferMessage::EmbedBatch { inputs, reply } => {
                            let result =
                                embed_batch_sync(&model, &mut ctx, &inputs, use_encoder_path);
                            let _ = reply.send(result);
                        }
                    }
                }
                tracing::debug!("inference thread exiting");
            })
            .map_err(|e| InferError::Backend(format!("failed to spawn inference thread: {e}")))?;

        Ok(Arc::new(LlamaCppLoadedModel {
            tx,
            vram_est: size_bytes,
        }))
    }
}

fn embed_batch_sync(
    model: &LlamaModel,
    ctx: &mut llama_cpp_2::context::LlamaContext<'_>,
    inputs: &[String],
    use_encoder_path: bool,
) -> Result<Vec<Vec<f32>>, InferError> {
    let mut results = Vec::with_capacity(inputs.len());

    for input in inputs {
        let tokens = model
            .str_to_token(input, AddBos::Never)
            .map_err(|e| InferError::Backend(format!("tokenize error: {e:?}")))?;

        if tokens.is_empty() {
            results.push(vec![]);
            continue;
        }

        // Clear KV cache before each sequence so prior decode passes don't
        // bleed through. seq_id is always 0 — one sequence per decode call.
        ctx.clear_kv_cache();

        let n = tokens.len();
        let mut batch = LlamaBatch::new(n, 1);

        for (pos, &token) in tokens.iter().enumerate() {
            // Encoder models: only mark last token as output (encoder pooling handles it).
            // Decoder models: mark ALL tokens as output so llama.cpp can mean-pool them.
            let logit_output = !use_encoder_path || pos == n - 1;
            batch
                .add(token, pos as i32, &[0], logit_output)
                .map_err(|e| InferError::Backend(format!("batch add error: {e:?}")))?;
        }

        if use_encoder_path {
            ctx.encode(&mut batch)
                .map_err(|e| InferError::Backend(format!("encode error: {e:?}")))?;
        } else {
            ctx.decode(&mut batch)
                .map_err(|e| InferError::Backend(format!("decode error: {e:?}")))?;
        }

        let emb = ctx
            .embeddings_seq_ith(0)
            .map_err(|e| InferError::Backend(format!("embeddings error: {e:?}")))?;

        results.push(emb.to_vec());
    }

    Ok(results)
}

#[async_trait::async_trait]
impl LoadedModel for LlamaCppLoadedModel {
    async fn embed(&self, input: &str) -> Result<Vec<f32>, InferError> {
        let mut results = self.embed_batch(&[input]).await?;
        results
            .pop()
            .ok_or_else(|| InferError::Backend("empty embedding result".to_string()))
    }

    async fn embed_batch(&self, inputs: &[&str]) -> Result<Vec<Vec<f32>>, InferError> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(InferMessage::EmbedBatch {
                inputs: inputs.iter().map(|s| s.to_string()).collect(),
                reply: reply_tx,
            })
            .map_err(|_| InferError::Backend("inference thread closed".to_string()))?;
        reply_rx
            .await
            .map_err(|_| InferError::Backend("reply channel dropped".to_string()))?
    }

    async fn complete(&self, _req: CompletionRequest) -> Result<CompletionStream, InferError> {
        // Phase 5 (T036): streaming completion via sampling loop. Not yet implemented.
        Err(InferError::Backend(
            "completion not yet implemented in LlamaCppBackend (Phase 5)".to_string(),
        ))
    }

    async fn tokenize(&self, _text: &str) -> Result<Vec<i32>, InferError> {
        Err(InferError::Backend(
            "tokenize not yet implemented".to_string(),
        ))
    }

    fn vram_bytes(&self) -> u64 {
        self.vram_est
    }
}
