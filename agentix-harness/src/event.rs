/// Events emitted by the agent loop as it runs.
/// Consumers receive these via a `tokio::sync::mpsc::UnboundedSender<AgentEvent>`.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// A tool call is about to be executed.
    ToolCallStarted { name: String, args_preview: String },
    /// A tool call returned a result.
    ToolCallCompleted {
        name: String,
        result_preview: String,
    },
    /// Stagnation detected; an intervention message was injected.
    StagnationDetected,
    /// The tool call budget was exhausted; forcing a final answer.
    BudgetExhausted,
    /// `ask_cloud` was called. Emitted instead of `ToolCallStarted` so
    /// consumers can distinguish cloud escalations from local tool calls.
    CloudEscalation { question_preview: String },
}
