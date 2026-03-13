//! Task routing — Fast / Balanced / Powerful slot assignment.
//!
//! Different tasks have different quality/latency requirements:
//! - **Fast**: Quick responses, simple queries, autocomplete — use cheapest/fastest provider
//! - **Balanced**: Normal conversation, tool use — use primary provider
//! - **Powerful**: Complex reasoning, multi-step agents — use most capable provider

use serde::{Deserialize, Serialize};

/// Task type that determines which provider slot to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskType {
    /// Quick responses — autocomplete, simple factual queries, status checks.
    /// Routes to the fastest/cheapest provider.
    Fast,
    /// Normal conversation and tool use — the default for most interactions.
    /// Routes to the primary provider.
    Balanced,
    /// Complex reasoning, multi-step agent loops, code generation.
    /// Routes to the most capable provider available.
    Powerful,
}

impl Default for TaskType {
    fn default() -> Self {
        TaskType::Balanced
    }
}

impl TaskType {
    /// Parse from a string (for config/YAML).
    pub fn from_str_lossy(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "fast" | "quick" | "cheap" => TaskType::Fast,
            "powerful" | "strong" | "best" | "reasoning" => TaskType::Powerful,
            _ => TaskType::Balanced,
        }
    }
}

/// Task routing configuration — maps task types to provider IDs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskRoutes {
    /// Provider ID for fast tasks (None = use primary).
    pub fast: Option<String>,
    /// Provider ID for balanced tasks (None = use primary).
    pub balanced: Option<String>,
    /// Provider ID for powerful tasks (None = use primary).
    pub powerful: Option<String>,
}

impl TaskRoutes {
    /// Get the provider ID for a given task type.
    /// Returns None if no specific route is configured (caller should fall back to primary).
    pub fn get(&self, task: TaskType) -> Option<&str> {
        match task {
            TaskType::Fast => self.fast.as_deref(),
            TaskType::Balanced => self.balanced.as_deref(),
            TaskType::Powerful => self.powerful.as_deref(),
        }
    }
}
