//! Robust agent loop — step tracking, nudge on empty, error recovery.
//!
//! Wraps each tool-calling query with structured tracking so we can:
//! 1. Detect when the LLM stops prematurely and nudge it to continue
//! 2. Suggest alternative tools when one fails
//! 3. Record successful tool chains for learning (via tool_traces)

/// A single step in an agent execution trace.
#[derive(Debug, Clone)]
pub struct AgentStep {
    pub tool_name: String,
    pub args_summary: String,
    pub result_preview: String,
    pub success: bool,
    pub timestamp: f64,
}

/// Tracks the state of a multi-step agent loop.
pub struct AgentLoop {
    pub goal: String,
    pub steps: Vec<AgentStep>,
    pub status: LoopStatus,
    pub nudge_count: usize,
    pub max_nudges: usize,
}

/// Final outcome of the agent loop.
#[derive(Debug, Clone, PartialEq)]
pub enum LoopStatus {
    Running,
    Completed,
    Failed(String),
    MaxSteps,
}

impl AgentLoop {
    /// Start a new agent loop for a given user goal.
    pub fn new(goal: &str, max_nudges: usize) -> Self {
        Self {
            goal: goal.to_string(),
            steps: Vec::new(),
            status: LoopStatus::Running,
            nudge_count: 0,
            max_nudges,
        }
    }

    /// Record a tool execution step.
    pub fn record_step(
        &mut self,
        tool_name: &str,
        args: &serde_json::Value,
        result: &str,
        success: bool,
    ) {
        let args_summary = summarize_args(args);
        let result_preview = if result.len() > 150 {
            format!("{}...", &result[..result.floor_char_boundary(150)])
        } else {
            result.to_string()
        };

        self.steps.push(AgentStep {
            tool_name: tool_name.to_string(),
            args_summary,
            result_preview,
            success,
            timestamp: now_ts(),
        });
    }

    /// Check if the LLM response needs a nudge.
    ///
    /// Returns Some(nudge_message) if the response is empty/unhelpful
    /// and we haven't exceeded the nudge budget.
    pub fn maybe_nudge(&mut self, response_text: &str) -> Option<String> {
        if self.nudge_count >= self.max_nudges {
            return None;
        }

        let text = response_text.trim();
        let needs_nudge = text.is_empty()
            || text.len() < 50
            || text.contains("I can't")
            || text.contains("I don't have")
            || text.contains("I'm not able")
            || text.contains("I cannot")
            || (text.contains("I'm sorry") && text.len() < 100);

        if needs_nudge {
            self.nudge_count += 1;
            Some(format!(
                "You haven't completed the task yet. The user asked: \"{}\". \
                 Call a tool to help, or give a complete and helpful answer.",
                self.goal,
            ))
        } else {
            None
        }
    }

    /// Generate an error recovery hint when a tool fails.
    ///
    /// Returns a message suggesting alternatives based on the tool category.
    pub fn error_recovery_hint(
        tool_name: &str,
        error: &str,
        similar_tools: &[String],
    ) -> String {
        let mut hint = format!(
            "Tool '{}' failed: {}. Try a different approach.",
            tool_name, error,
        );
        if !similar_tools.is_empty() {
            hint.push_str(&format!(
                " Similar tools you could try: {}",
                similar_tools.join(", "),
            ));
        }
        hint
    }

    /// Mark the loop as completed.
    pub fn complete(&mut self) {
        self.status = LoopStatus::Completed;
    }

    /// Mark the loop as failed.
    pub fn fail(&mut self, reason: &str) {
        self.status = LoopStatus::Failed(reason.to_string());
    }

    /// Get the tool chain as a summary (for trace recording).
    pub fn chain_summary(&self) -> Vec<serde_json::Value> {
        self.steps
            .iter()
            .map(|s| {
                serde_json::json!({
                    "tool": s.tool_name,
                    "args": s.args_summary,
                    "success": s.success,
                })
            })
            .collect()
    }

    /// True if any step succeeded.
    pub fn any_success(&self) -> bool {
        self.steps.iter().any(|s| s.success)
    }
}

/// Summarize tool arguments — keep keys, truncate large values.
fn summarize_args(args: &serde_json::Value) -> String {
    match args {
        serde_json::Value::Object(map) => {
            let parts: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    let val = match v {
                        serde_json::Value::String(s) if s.len() > 60 => {
                            format!("\"{}...\"", &s[..s.floor_char_boundary(60)])
                        }
                        _ => v.to_string(),
                    };
                    format!("{k}={val}")
                })
                .collect();
            parts.join(", ")
        }
        _ => args.to_string(),
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
