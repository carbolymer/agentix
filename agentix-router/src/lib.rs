/// Which backend to route a request to.
#[derive(Debug, Clone, PartialEq)]
pub enum RouteTarget {
    Local,
    Anthropic,
    OpenAI,
    OpenRouter,
}

/// Backend-selection router. Pure logic — no network surface, no config dependency.
///
/// Routing rules (in priority order):
///   `local/<model>`       → Local
///   `<provider>/<model>`  → OpenRouter  (provider/model syntax)
///   `claude*`             → Anthropic
///   `gpt*`, `o1*`…        → OpenAI
///   known local families  → Local
///   anything else         → Local  (never silently cloud — Principle II)
#[derive(Default)]
pub struct Router;

impl Router {
    pub fn new() -> Self {
        Self
    }

    pub fn route(&self, model: &str) -> RouteTarget {
        let m = model.to_lowercase();

        if m.starts_with("local/") {
            return RouteTarget::Local;
        }

        if m.contains('/') {
            // provider/model syntax always goes via OpenRouter
            return RouteTarget::OpenRouter;
        }

        if m.starts_with("claude") {
            return RouteTarget::Anthropic;
        }

        if m.starts_with("gpt") || m.starts_with("o1") || m.starts_with("o3") || m.starts_with("o4")
        {
            return RouteTarget::OpenAI;
        }

        if m.starts_with("qwen")
            || m.starts_with("mistral")
            || m.starts_with("llama")
            || m.starts_with("laguna")
            || m.starts_with("gemma")
            || m.starts_with("phi")
            || m.starts_with("deepseek")
            || m.starts_with("falcon")
            || m.starts_with("vicuna")
            || m.starts_with("solar")
            || m.starts_with("kimi")
        {
            return RouteTarget::Local;
        }

        // Unrecognized model → always local. Cloud backends are reachable only via
        // explicit prefix or provider/model syntax. This prevents a misconfigured or
        // typo'd model name from becoming an unstructured cloud call (Principle II).
        RouteTarget::Local
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_prefix() {
        assert_eq!(Router::new().route("local/my-model"), RouteTarget::Local);
    }

    #[test]
    fn known_local_families() {
        let r = Router::new();
        assert_eq!(r.route("qwen2.5-coder-32b"), RouteTarget::Local);
        assert_eq!(r.route("mistral-7b"), RouteTarget::Local);
        assert_eq!(r.route("llama-3.1-8b"), RouteTarget::Local);
        assert_eq!(r.route("laguna-xs-2.1"), RouteTarget::Local);
        assert_eq!(r.route("gemma-3-27b"), RouteTarget::Local);
        assert_eq!(r.route("phi-4"), RouteTarget::Local);
        assert_eq!(r.route("deepseek-r1:32b"), RouteTarget::Local);
        assert_eq!(r.route("kimi-k2"), RouteTarget::Local);
    }

    #[test]
    fn claude_to_anthropic() {
        let r = Router::new();
        assert_eq!(r.route("claude-sonnet-4-6"), RouteTarget::Anthropic);
        assert_eq!(r.route("claude-opus-4-7"), RouteTarget::Anthropic);
    }

    #[test]
    fn gpt_to_openai() {
        let r = Router::new();
        assert_eq!(r.route("gpt-4o"), RouteTarget::OpenAI);
        assert_eq!(r.route("o1-preview"), RouteTarget::OpenAI);
        assert_eq!(r.route("o4-mini"), RouteTarget::OpenAI);
    }

    #[test]
    fn provider_slash_model_to_openrouter() {
        let r = Router::new();
        assert_eq!(
            r.route("anthropic/claude-3-5-sonnet"),
            RouteTarget::OpenRouter
        );
        assert_eq!(r.route("openai/gpt-4o"), RouteTarget::OpenRouter);
    }

    #[test]
    fn unknown_model_falls_back_to_local() {
        // Never silently routes unknown models to cloud (Principle II).
        assert_eq!(
            Router::new().route("some-unknown-model"),
            RouteTarget::Local
        );
    }
}
