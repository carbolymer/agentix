use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingRequest {
    pub model: String,
    pub input: serde_json::Value,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionResponse {
    pub text: String,
}

/// The only valid path to a cloud backend (Principle II).
///
/// All three fields are mandatory. A cloud call that bypasses this type and
/// goes directly to a cloud backend MUST NOT be accepted by the router.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationRequest {
    /// What the local model already knows about this problem.
    pub context: String,
    /// A precise, specific question that cannot be answered from local sources.
    pub question: String,
    /// Why local sources are insufficient for this question.
    pub local_insufficiency_reason: String,
}
