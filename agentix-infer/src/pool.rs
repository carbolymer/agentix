use crate::{backend::LoadedModel, InferError};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::sync::Notify;

struct PoolSlot {
    loaded: Arc<dyn LoadedModel>,
    last_used: Instant,
}

type ReleaseFn = Box<dyn FnOnce(Arc<dyn LoadedModel>) + Send>;

/// Returned to callers; releases the model back to the pool on drop.
pub struct ModelGuard {
    loaded: Arc<dyn LoadedModel>,
    on_release: Option<ReleaseFn>,
}

impl std::ops::Deref for ModelGuard {
    type Target = dyn LoadedModel;
    fn deref(&self) -> &Self::Target {
        self.loaded.as_ref()
    }
}

impl Drop for ModelGuard {
    fn drop(&mut self) {
        if let Some(f) = self.on_release.take() {
            f(Arc::clone(&self.loaded));
        }
    }
}

struct PoolState {
    idle: HashMap<String, Vec<PoolSlot>>,
}

impl PoolState {
    fn total_vram(&self) -> u64 {
        self.idle
            .values()
            .flat_map(|v| v.iter())
            .map(|s| s.loaded.vram_bytes())
            .sum()
    }

    /// Evict the idle slot with the oldest last_used timestamp.
    fn evict_lru(&mut self) {
        let mut oldest: Option<(String, usize)> = None;

        for (name, slots) in &self.idle {
            for (i, slot) in slots.iter().enumerate() {
                let is_older = oldest
                    .as_ref()
                    .and_then(|(n, idx)| self.idle.get(n).and_then(|v| v.get(*idx)))
                    .map(|s| slot.last_used < s.last_used)
                    .unwrap_or(true);
                if is_older {
                    oldest = Some((name.clone(), i));
                }
            }
        }

        if let Some((name, idx)) = oldest {
            if let Some(slots) = self.idle.get_mut(&name) {
                let vram_freed = slots.get(idx).map(|s| s.loaded.vram_bytes()).unwrap_or(0);
                slots.remove(idx);
                if slots.is_empty() {
                    self.idle.remove(&name);
                }
                tracing::info!(
                    model = %name,
                    vram_freed_bytes = vram_freed,
                    "evicted LRU model from pool",
                );
            }
        }
    }

    fn total_idle_count(&self) -> usize {
        self.idle.values().map(|v| v.len()).sum()
    }
}

pub struct ContextPool {
    state: Arc<Mutex<PoolState>>,
    /// Guards concurrent loads for the same model key.
    /// When a thread begins loading a model it inserts a Notify here; other
    /// threads that want the same model wait on that Notify instead of
    /// starting a duplicate load (T027).
    loading: Arc<Mutex<HashMap<String, Arc<Notify>>>>,
    vram_limit: Option<u64>,
    max_loaded_models: usize,
}

impl ContextPool {
    pub fn new(vram_limit: Option<u64>, max_loaded_models: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(PoolState {
                idle: HashMap::new(),
            })),
            loading: Arc::new(Mutex::new(HashMap::new())),
            vram_limit,
            max_loaded_models,
        }
    }

    /// If another caller is already loading `name`, wait for it to finish and
    /// return `true` (the caller should then retry `acquire_idle`).
    /// If no load is in progress, register `name` as loading and return `false`.
    pub async fn wait_if_loading(&self, name: &str) -> bool {
        let notify = {
            let map = self
                .loading
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            map.get(name).cloned()
        };
        if let Some(notify) = notify {
            notify.notified().await;
            true
        } else {
            false
        }
    }

    /// Mark `name` as currently loading. Call `finish_loading` when done.
    pub fn begin_loading(&self, name: &str) -> Arc<Notify> {
        let notify = Arc::new(Notify::new());
        self.loading
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(name.to_string(), Arc::clone(&notify));
        notify
    }

    /// Remove the loading sentinel and wake all waiters.
    pub fn finish_loading(&self, name: &str, notify: Arc<Notify>) {
        self.loading
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(name);
        notify.notify_waiters();
    }

    /// Try to get an idle instance for `name` without loading.
    pub fn acquire_idle(&self, name: &str) -> Option<ModelGuard> {
        let mut state = self.state.lock().ok()?;
        let slots = state.idle.get_mut(name)?;
        if slots.is_empty() {
            return None;
        }
        let slot = slots.pop()?;
        drop(state);

        let state_arc = Arc::clone(&self.state);
        let name_owned = name.to_string();
        Some(ModelGuard {
            loaded: slot.loaded,
            on_release: Some(Box::new(move |model| {
                if let Ok(mut s) = state_arc.lock() {
                    s.idle.entry(name_owned).or_default().push(PoolSlot {
                        loaded: model,
                        last_used: Instant::now(),
                    });
                }
            })),
        })
    }

    /// Register a freshly loaded model and return a guard for immediate use.
    /// Evicts idle models if VRAM budget or slot count is exceeded.
    pub fn store_loaded(
        &self,
        name: &str,
        loaded: Arc<dyn LoadedModel>,
        needed_vram: u64,
    ) -> Result<ModelGuard, InferError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| InferError::Backend("pool lock poisoned".to_string()))?;

        // Enforce VRAM limit
        if let Some(limit) = self.vram_limit {
            while state.total_vram() + needed_vram > limit {
                if state.total_idle_count() == 0 {
                    return Err(InferError::VramExhausted {
                        model: name.to_string(),
                        required_bytes: needed_vram,
                    });
                }
                state.evict_lru();
            }
        }

        // Enforce slot count
        while state.total_idle_count() >= self.max_loaded_models {
            if state.total_idle_count() == 0 {
                break;
            }
            state.evict_lru();
        }

        drop(state);

        let state_arc = Arc::clone(&self.state);
        let name_owned = name.to_string();
        Ok(ModelGuard {
            loaded,
            on_release: Some(Box::new(move |model| {
                if let Ok(mut s) = state_arc.lock() {
                    s.idle.entry(name_owned).or_default().push(PoolSlot {
                        loaded: model,
                        last_used: Instant::now(),
                    });
                }
            })),
        })
    }
}
