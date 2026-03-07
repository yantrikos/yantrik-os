//! Shared types for all ML backends.
//!
//! These types are backend-agnostic — they appear in the `LLMBackend` and
//! `STTBackend` trait signatures and are used throughout the companion.

use serde::{Deserialize, Serialize};

// ── Chat messages ──────────────────────────────────────────────────────

/// A single chat message.
///
/// Supports the OpenAI chat completions format including native tool calling:
/// - `tool_calls`: Present on assistant messages that invoke tools
/// - `tool_call_id`: Present on `role: "tool"` messages (tool results)
/// - `name`: Tool name on `role: "tool"` messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    /// Tool calls made by the assistant (only on role="assistant" messages).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ApiToolCall>>,
    /// ID of the tool call this message is responding to (only on role="tool" messages).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Tool name (only on role="tool" messages).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    /// Assistant message with native tool calls.
    pub fn assistant_with_tool_calls(content: impl Into<String>, calls: Vec<ApiToolCall>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
            tool_calls: if calls.is_empty() { None } else { Some(calls) },
            tool_call_id: None,
            name: None,
        }
    }

    /// Tool result message (role="tool").
    pub fn tool(call_id: impl Into<String>, tool_name: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: Some(call_id.into()),
            name: Some(tool_name.into()),
        }
    }
}

/// A tool call in the OpenAI API format (returned by the model).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: ApiToolCallFunction,
}

/// Function details within an API tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiToolCallFunction {
    pub name: String,
    /// Arguments as a JSON string (OpenAI format).
    pub arguments: String,
}

/// A parsed tool call from model output (internal format).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

impl ToolCall {
    /// Convert from API format to internal format.
    pub fn from_api(api: &ApiToolCall) -> Option<Self> {
        let args: serde_json::Value = serde_json::from_str(&api.function.arguments).ok()?;
        Some(Self {
            name: api.function.name.clone(),
            arguments: args,
        })
    }
}

// ── Generation config ──────────────────────────────────────────────────

/// Configuration for text generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationConfig {
    /// Maximum number of tokens to generate.
    pub max_tokens: usize,
    /// Sampling temperature (0.0 = greedy/argmax).
    pub temperature: f64,
    /// Top-p (nucleus) sampling threshold.
    pub top_p: Option<f64>,
    /// Top-k sampling (number of highest-probability tokens to keep).
    pub top_k: Option<usize>,
    /// Repetition penalty (1.0 = no penalty).
    pub repeat_penalty: f32,
    /// Window size for repetition penalty.
    pub repeat_last_n: usize,
    /// Random seed for sampling.
    pub seed: u64,
    /// Stop sequences — generation halts when any of these is produced.
    pub stop: Vec<String>,
}

impl Default for GenerationConfig {
    fn default() -> Self {
        Self {
            max_tokens: 512,
            temperature: 0.7,
            top_p: Some(0.9),
            top_k: None,
            repeat_penalty: 1.1,
            repeat_last_n: 64,
            seed: 42,
            stop: vec![],
        }
    }
}

impl GenerationConfig {
    /// Greedy decoding (argmax, no randomness).
    pub fn greedy() -> Self {
        Self {
            temperature: 0.0,
            top_p: None,
            top_k: None,
            ..Default::default()
        }
    }
}

// ── LLM response ───────────────────────────────────────────────────────

/// Response from a generation call.
#[derive(Debug, Clone)]
pub struct LLMResponse {
    /// The generated text (full output).
    pub text: String,
    /// Number of prompt tokens processed.
    pub prompt_tokens: usize,
    /// Number of tokens generated.
    pub completion_tokens: usize,
    /// Any tool calls parsed from the output (internal format).
    pub tool_calls: Vec<ToolCall>,
    /// Native API tool calls (OpenAI format) — present when the API returns structured tool_calls.
    pub api_tool_calls: Vec<ApiToolCall>,
    /// Stop reason: "stop", "length", "eos", or "tool_calls".
    pub stop_reason: String,
}

// ── STT result ─────────────────────────────────────────────────────────

/// Transcription result.
#[derive(Debug, Clone)]
pub struct TranscribeResult {
    pub text: String,
    pub tokens: usize,
}

// ── Voice params ───────────────────────────────────────────────────────

/// Bond-adaptive voice parameters.
#[derive(Debug, Clone)]
pub struct VoiceParams {
    /// Speech rate multiplier: 1.0 = normal, 0.5 = half speed, 2.0 = double speed.
    pub rate: f32,
    /// Pitch: 1.0 = normal, 0.5 = lower, 2.0 = higher.
    pub pitch: f32,
    /// Volume: 1.0 = full, 0.5 = half.
    pub volume: f32,
}

impl Default for VoiceParams {
    fn default() -> Self {
        Self {
            rate: 1.0,
            pitch: 1.0,
            volume: 1.0,
        }
    }
}
