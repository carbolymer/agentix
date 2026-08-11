use crate::{client::GatewayClient, tool::Tool};
use anyhow::Result;
use async_trait::async_trait;

/// Built-in tool that escalates a specific question to a cloud model via the gateway.
///
/// The gateway routes any `provider/model` name (e.g. `moonshotai/kimi-k2`,
/// `anthropic/claude-sonnet-4-6`) to OpenRouter automatically, so the local
/// agent never needs an API key directly.
///
/// Three required fields force the model to articulate the knowledge gap before
/// escalating, preventing lazy escalation ("I don't know" → cloud).
pub struct AskCloud {
    client: GatewayClient,
    cloud_model: String,
}

impl AskCloud {
    pub fn new(gateway_url: &str, cloud_model: impl Into<String>) -> Result<Self> {
        Ok(Self {
            client: GatewayClient::new(gateway_url)?,
            cloud_model: cloud_model.into(),
        })
    }
}

#[async_trait]
impl Tool for AskCloud {
    fn name(&self) -> &str {
        "ask_cloud"
    }

    fn description(&self) -> &str {
        "Escalate a specific technical question to a capable cloud model when local reasoning \
         or available sources are insufficient. You must supply: what you already know, a precise \
         question, and why you cannot answer it from local tools."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "what_i_already_know": {
                    "type": "string",
                    "description": "Summarise the evidence and context gathered so far."
                },
                "specific_question": {
                    "type": "string",
                    "description": "The exact technical question that needs answering."
                },
                "why_i_cant_answer_locally": {
                    "type": "string",
                    "description": "Why local tools or the available sources cannot answer this."
                }
            },
            "required": ["what_i_already_know", "specific_question", "why_i_cant_answer_locally"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> Result<String> {
        let context = args["what_i_already_know"].as_str().unwrap_or("");
        let question = args["specific_question"].as_str().unwrap_or("");
        let why = args["why_i_cant_answer_locally"].as_str().unwrap_or("");

        let prompt = format!(
            "Context from local agent:\n{context}\n\n\
             Why local sources are insufficient:\n{why}\n\n\
             Question:\n{question}"
        );

        let messages = vec![serde_json::json!({"role": "user", "content": prompt})];

        let resp = self.client.chat(&self.cloud_model, &messages, &[]).await?;

        Ok(resp
            .content
            .unwrap_or_else(|| "(cloud model returned no content)".into()))
    }
}
