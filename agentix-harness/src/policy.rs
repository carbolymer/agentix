/// Controls budget enforcement and stagnation detection thresholds.
#[derive(Debug, Clone)]
pub struct EscalationPolicy {
    /// Hard limit on total tool calls for the entire run.
    pub max_tool_calls: usize,
    /// Number of most recent tool results to examine for stagnation.
    pub stagnation_window: usize,
    /// How many entries in the window must hash identically to trigger intervention.
    pub stagnation_min_matches: usize,
}

impl Default for EscalationPolicy {
    fn default() -> Self {
        Self {
            max_tool_calls: 20,
            stagnation_window: 4,
            stagnation_min_matches: 3,
        }
    }
}
