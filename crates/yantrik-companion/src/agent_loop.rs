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
    started_at: std::time::Instant,
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
            started_at: std::time::Instant::now(),
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

    /// One line per call this loop has made, in call order: "{tool}: succeeded|failed".
    ///
    /// Handed back to the model on error recovery so it can see what already
    /// worked instead of re-checking it and "correcting" itself for a call it
    /// never made. Deliberately names only the tool and its outcome —
    /// result_preview and args_summary hold tool output and user data, which
    /// must not be echoed back as context the model did not ask for.
    pub fn called_so_far(&self) -> String {
        const MAX_LINES: usize = 12;
        if self.steps.is_empty() {
            return String::new();
        }
        let lines: Vec<String> = self
            .steps
            .iter()
            .map(|s| {
                format!(
                    "{}: {}",
                    s.tool_name,
                    if s.success { "succeeded" } else { "failed" }
                )
            })
            .collect();
        if lines.len() > MAX_LINES {
            // Past the cap the earliest calls are dropped; the marker says why.
            let mut kept = vec!["…".to_string()];
            kept.extend_from_slice(&lines[lines.len() - MAX_LINES..]);
            kept.join("\n")
        } else {
            lines.join("\n")
        }
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

    /// Elapsed time since the loop started, in milliseconds.
    pub fn elapsed_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    /// Check if the same tool has been called too many times (runaway detection).
    /// Returns Some(message) if the tool should be stopped.
    pub fn runaway_check(&self, tool_name: &str, max_per_tool: usize) -> Option<String> {
        let earlier: Vec<&AgentStep> = self
            .steps
            .iter()
            .filter(|s| s.tool_name == tool_name)
            .collect();
        if earlier.len() >= max_per_tool {
            let mut msg = format!(
                "You've already called '{}' {} times. Stop calling it and give a final answer \
                 using the information you already have.",
                tool_name,
                earlier.len(),
            );
            // Quote the first call's result so the model can answer from it
            // instead of re-calling: it already received this same text as
            // that tool's result, so nothing new leaks.
            if let Some(first) = earlier.first() {
                let preview = &first.result_preview;
                let cut = &preview[..preview.floor_char_boundary(150)];
                msg.push_str(&format!(" Your earlier call returned: {}", cut));
            }
            Some(msg)
        } else {
            None
        }
    }
}

/// Per-tool call limits based on tool category.
/// Browser tools need many iterations (snapshot→click→snapshot→type→snapshot...),
/// while memory tools rarely need more than a few calls.
pub fn max_calls_for_tool(tool_name: &str) -> usize {
    match tool_name {
        // Browser tools: web automation requires many round-trips
        "browse" | "browser_snapshot" | "browser_click_element"
        | "browser_type_element" | "browser_scroll" | "browser_search"
        | "browser_read" | "browser_click" | "browser_type"
        | "browser_screenshot" | "browser_tabs" | "launch_browser"
        | "browser_see" | "web_search"
        | "browser_cleanup" | "browser_status" => 25,

        // Life assistant: search orchestration may need several iterations
        "life_search" | "search_sources" | "extract_search_results"
        | "deduplicate_results" | "rank_results" | "recall_preferences" => 15,

        // File tools: may need to read/search many files
        "read_file" | "write_file" | "list_files" | "search_files"
        | "manage_files" | "file_info" => 10,

        // Shell: multi-step debugging/building
        "run_command" => 15,

        // Memory: rarely needs many calls
        "remember" | "recall" | "save_user_fact" => 6,

        // Everything else: reasonable default
        _ => 10,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn json_args() -> serde_json::Value {
        serde_json::json!({})
    }

    #[test]
    fn called_so_far_empty_on_fresh_loop() {
        let loop_ = AgentLoop::new("goal", 3);
        assert_eq!(loop_.called_so_far(), "");
    }

    #[test]
    fn called_so_far_names_each_step_in_order() {
        let mut loop_ = AgentLoop::new("goal", 3);
        loop_.record_step("recall", &json_args(), "found it", true);
        loop_.record_step("calculate", &json_args(), "boom", false);
        let s = loop_.called_so_far();
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines, vec!["recall: succeeded", "calculate: failed"]);
    }

    #[test]
    fn called_so_far_never_leaks_results_or_args() {
        let mut loop_ = AgentLoop::new("goal", 3);
        loop_.record_step(
            "recall",
            &serde_json::json!({"query": "secret fact"}),
            "top secret output",
            true,
        );
        let s = loop_.called_so_far();
        assert!(!s.contains("secret"));
    }

    #[test]
    fn called_so_far_caps_and_marks_the_cut() {
        let mut loop_ = AgentLoop::new("goal", 3);
        for i in 0..15 {
            loop_.record_step(&format!("tool{}", i), &json_args(), "ok", true);
        }
        let s = loop_.called_so_far();
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines[0], "…");
        assert_eq!(lines.len(), 13); // cut marker + 12 most recent
        assert!(s.contains("tool14: succeeded"));
        assert!(s.contains("tool3: succeeded"));
        assert!(!s.contains("tool2: succeeded"));
    }

    #[test]
    fn runaway_check_quotes_the_earlier_result() {
        let mut loop_ = AgentLoop::new("goal", 3);
        loop_.record_step("recall", &json_args(), "the answer is 42", true);
        loop_.record_step("recall", &json_args(), "the answer is 42", true);
        let msg = loop_.runaway_check("recall", 2).expect("guard should fire");
        assert!(msg.contains("recall"));
        assert!(msg.contains("the answer is 42"));
    }

    #[test]
    fn runaway_check_silent_below_threshold() {
        let mut loop_ = AgentLoop::new("goal", 3);
        loop_.record_step("recall", &json_args(), "ok", true);
        assert!(loop_.runaway_check("recall", 2).is_none());
    }

    #[test]
    fn error_hint_plus_called_so_far_names_failure_and_earlier_success() {
        // Mirrors the companion.rs join: hint text + the loop's own record,
        // appended to the failed tool's result the model reads.
        let mut loop_ = AgentLoop::new("goal", 3);
        loop_.record_step("recall", &json_args(), "found it", true);
        let similar = vec!["recall_notes".to_string()];
        let hint = AgentLoop::error_recovery_hint("calculate", "division by zero", &similar);
        let message = format!("{}\n{}", hint, loop_.called_so_far());
        assert!(message.contains("calculate"));
        assert!(message.contains("recall_notes"));
        assert!(message.contains("recall: succeeded"));
    }
}
