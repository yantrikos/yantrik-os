//! Fallback LLM — wraps a primary backend with an automatic fallback.
//!
//! When the primary backend fails (network error, timeout, CLI crash),
//! the fallback backend is lazily initialized and used instead.
//! This enables offline intelligence via a local GGUF model (e.g. Qwen3.5-0.8B)
//! while keeping the powerful primary backend (Claude CLI, API) for normal use.
//!
//! Supports two fallback modes:
//! - `api` — fallback to a local llama-server (or any OpenAI-compatible endpoint)
//! - `llamacpp` — fallback via embedded llama.cpp (requires `llamacpp` feature)

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use tracing;

use crate::traits::LLMBackend;
use crate::types::{ChatMessage, GenerationConfig, LLMResponse};

/// Configuration for the fallback LLM backend.
#[derive(Debug, Clone)]
pub enum FallbackConfig {
    /// Use an API endpoint (e.g. local llama-server) as fallback.
    Api {
        base_url: String,
        model: String,
    },
    /// Use embedded llama.cpp (requires `llamacpp` feature at compile time).
    LlamaCpp {
        model_path: PathBuf,
        n_gpu_layers: u32,
        context_size: u32,
    },
}

/// Longest system prompt the fallback is handed, in characters, when slimming.
const SLIM_SYSTEM_CHARS: usize = 600;

/// How many preceding plain turns are carried into a slimmed conversation.
/// Only plain chat is walked back over — never into a tool round, because half a
/// tool round is a tool result with nothing that asked for it.
const SLIM_HISTORY_TURNS: usize = 4;

/// Ceiling on the fallback's generation budget.
///
/// It was 512, which is below what a tool round can need: the reply to a tool round is a JSON
/// tool call whose arguments carry whatever the model is passing — a path, a query, the body of
/// a note — and a budget that runs out mid-object reaches the caller as a reply with no tool
/// call in it rather than as an error. 2048 is `GenerationConfig`'s own default, so this now
/// only trims callers that asked for more than the ordinary budget and never cuts below it.
const FALLBACK_MAX_TOKENS: usize = 2048;

/// Assumed context window of the primary, in tokens, when nothing says otherwise.
///
/// Nothing reports this: `LLMBackend` has no context-size method and neither construction site
/// (`yantrik/src/main.rs`, `yantrik-ui/src/bridge.rs`) passes one. So it is a knob with a
/// deliberately large default — the primary is normally the cloud or CLI model — which keeps
/// slimming switched on for an unconfigured deployment, exactly as before.
const DEFAULT_PRIMARY_CONTEXT_TOKENS: usize = 32_768;

/// Assumed context window of the fallback, in tokens, when the config does not carry one.
/// `FallbackConfig::LlamaCpp` does carry a real `context_size` and is believed; the `Api`
/// variant carries nothing, and a local llama-server is started small by default.
const DEFAULT_FALLBACK_CONTEXT_TOKENS: usize = 4_096;

/// An LLM backend that wraps a primary + fallback.
///
/// On every call, tries the primary backend first. If it fails,
/// lazily initializes and uses the fallback.
pub struct FallbackLLM {
    primary: Arc<dyn LLMBackend>,
    fallback: Mutex<Option<Arc<dyn LLMBackend>>>,
    fallback_config: Option<FallbackConfig>,
    /// Track consecutive primary failures to avoid slow retries
    primary_failures: Mutex<u32>,
    /// When the primary's cooldown ends. While `now() < cooldown_until` the primary is skipped;
    /// once it passes, the primary is tried once more, and a further failure starts a new cooldown.
    cooldown_until: Mutex<Option<std::time::Instant>>,
    /// Clock, injectable so a test can advance past the cooldown without sleeping.
    now: Box<dyn Fn() -> std::time::Instant + Send + Sync>,
    /// Believed context windows of the two backends. Slimming exists only to fit a big
    /// conversation into a small model; when the fallback is not the smaller one it is a
    /// mutilation for nothing, so these decide whether it happens at all.
    primary_context_tokens: usize,
    fallback_context_tokens: usize,
}

/// How long a primary that has failed three times is skipped before it is tried once more.
const PRIMARY_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(5 * 60);

impl FallbackLLM {
    /// Create a new FallbackLLM.
    ///
    /// The fallback is NOT initialized until the primary fails.
    /// If `fallback_config` is None, no fallback is available — primary errors pass through.
    pub fn new(
        primary: Arc<dyn LLMBackend>,
        fallback_config: Option<FallbackConfig>,
    ) -> Self {
        // The llama.cpp variant is told its context size by the caller, so use it rather than
        // the default guess. Nothing tells us the API variant's, or the primary's at all.
        let fallback_context_tokens = match fallback_config {
            Some(FallbackConfig::LlamaCpp { context_size, .. }) => context_size as usize,
            _ => DEFAULT_FALLBACK_CONTEXT_TOKENS,
        };
        Self {
            primary,
            fallback: Mutex::new(None),
            fallback_config,
            primary_failures: Mutex::new(0),
            cooldown_until: Mutex::new(None),
            now: Box::new(std::time::Instant::now),
            primary_context_tokens: DEFAULT_PRIMARY_CONTEXT_TOKENS,
            fallback_context_tokens,
        }
    }

    /// Create with an already-initialized fallback backend (for testing or pre-warming).
    pub fn with_fallback(
        primary: Arc<dyn LLMBackend>,
        fallback: Arc<dyn LLMBackend>,
    ) -> Self {
        Self {
            primary,
            fallback: Mutex::new(Some(fallback)),
            fallback_config: None,
            primary_failures: Mutex::new(0),
            cooldown_until: Mutex::new(None),
            now: Box::new(std::time::Instant::now),
            primary_context_tokens: DEFAULT_PRIMARY_CONTEXT_TOKENS,
            fallback_context_tokens: DEFAULT_FALLBACK_CONTEXT_TOKENS,
        }
    }

    /// Tell the wrapper how big each side's context window really is.
    ///
    /// Without this it guesses, and the guess is "the fallback is the smaller model". A
    /// deployment whose fallback is a peer of the primary should say so here: slimming then
    /// stops entirely and the fallback is handed the conversation and the tools as they came.
    pub fn with_context_tokens(mut self, primary_tokens: usize, fallback_tokens: usize) -> Self {
        self.primary_context_tokens = primary_tokens;
        self.fallback_context_tokens = fallback_tokens;
        self
    }

    /// Get or initialize the fallback backend.
    fn get_fallback(&self) -> Result<Arc<dyn LLMBackend>> {
        let mut guard = self.fallback.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;

        if let Some(ref fb) = *guard {
            return Ok(fb.clone());
        }

        let config = self.fallback_config.as_ref()
            .ok_or_else(|| anyhow::anyhow!("no fallback configured"))?;

        let fb: Arc<dyn LLMBackend> = match config {
            FallbackConfig::Api { base_url, model } => {
                tracing::info!(
                    base_url = %base_url,
                    model = %model,
                    "Initializing fallback LLM (API)"
                );
                #[cfg(feature = "api-llm")]
                {
                    Arc::new(crate::llm::ApiLLM::new(
                        base_url.clone(),
                        None, // no API key for local server
                        model,
                    ))
                }
                #[cfg(not(feature = "api-llm"))]
                {
                    anyhow::bail!("fallback API requires 'api-llm' feature at compile time")
                }
            }
            FallbackConfig::LlamaCpp { model_path, n_gpu_layers, context_size } => {
                if !model_path.exists() {
                    anyhow::bail!(
                        "fallback model not found: {}",
                        model_path.display()
                    );
                }
                tracing::info!(
                    model = %model_path.display(),
                    gpu_layers = n_gpu_layers,
                    ctx = context_size,
                    "Initializing fallback LLM (llama.cpp)"
                );
                #[cfg(feature = "llamacpp")]
                {
                    Arc::new(crate::llm::LlamaCppLLM::from_gguf(
                        model_path,
                        *n_gpu_layers,
                        *context_size,
                    )?)
                }
                #[cfg(not(feature = "llamacpp"))]
                {
                    anyhow::bail!("fallback requires 'llamacpp' feature at compile time")
                }
            }
        };

        *guard = Some(fb.clone());
        Ok(fb)
    }

    /// Record a primary success — reset failure counter and any cooldown.
    fn primary_success(&self) {
        if let Ok(mut count) = self.primary_failures.lock() {
            if *count > 0 {
                tracing::info!(prev_failures = *count, "Primary LLM recovered");
                *count = 0;
            }
        }
        if let Ok(mut until) = self.cooldown_until.lock() {
            *until = None;
        }
    }

    /// Record a primary failure — increment counter and start (or restart) the cooldown.
    fn primary_failure(&self) {
        if let Ok(mut count) = self.primary_failures.lock() {
            *count += 1;
        }
        if let Ok(mut until) = self.cooldown_until.lock() {
            *until = Some((self.now)() + PRIMARY_COOLDOWN);
        }
    }

    /// Check if we should skip the primary (too many consecutive failures).
    ///
    /// After 3 consecutive failures the primary is skipped for a cooldown of 5 minutes, then tried
    /// once more. A success resets the count; another failure starts a new cooldown. This replaced
    /// the old rule that skipped the primary for the rest of the process once the count reached 3.
    fn should_skip_primary(&self) -> bool {
        let count = match self.primary_failures.lock() {
            Ok(c) => *c,
            Err(_) => return false,
        };
        if count < 3 {
            return false;
        }
        match self.cooldown_until.lock() {
            Ok(until) => until.map_or(false, |u| (self.now)() < u),
            Err(_) => false,
        }
    }

    /// Whether the conversation is worth cutting down before the fallback sees it.
    ///
    /// Only when the fallback is the smaller of the two. A fallback with as much context as the
    /// primary is a peer, and handing it a truncated system prompt and a conversation with its
    /// history removed made it answer worse than it had to, for no reason at all.
    fn should_slim(&self) -> bool {
        self.fallback_context_tokens < self.primary_context_tokens
    }

    /// The conversation and config to hand the fallback, given what each side can hold.
    fn for_fallback(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
    ) -> (Vec<ChatMessage>, GenerationConfig) {
        if self.should_slim() {
            (Self::slim_messages(messages), Self::slim_config(config))
        } else {
            (messages.to_vec(), config.clone())
        }
    }

    /// Slim down messages for the tiny fallback model.
    ///
    /// Keeps the system messages (truncated) and everything from the last user turn onward.
    ///
    /// It used to keep the system prompt plus "the last message, if it is a user message", and
    /// nothing at all otherwise. During a tool round the last message is a tool result or an
    /// assistant tool call, never a user message — so when the primary returned 429 or 500
    /// mid-round, the fallback was handed a system prompt and nothing else. Qwen's chat template
    /// rejects that outright ("no user query"), the llama-server answered 500, the fallback was
    /// counted as failed too, and the companion came up marked offline. The user's question was
    /// sitting three messages back the whole time.
    ///
    /// Anchoring on the last *user* message instead of the last message keeps that question, and
    /// keeps the whole tool round that followed it — so every tool result still has the assistant
    /// tool call that asked for it immediately above it, which is the other thing a chat template
    /// will refuse. Preceding plain turns are carried too, but the walk back stops at the first
    /// message belonging to an earlier tool round rather than slicing one in half.
    ///
    /// A conversation with no user message anywhere is passed through whole. Inventing a user
    /// turn would put words in the user's mouth, and dropping everything is what produced the
    /// fault above.
    fn slim_messages(messages: &[ChatMessage]) -> Vec<ChatMessage> {
        let Some(last_user) = messages.iter().rposition(|m| m.role == "user") else {
            tracing::warn!(
                count = messages.len(),
                "Fallback given a conversation with no user turn — sending it unslimmed"
            );
            return messages.to_vec();
        };

        // Walk back over plain chat only. `tool` results and assistant messages carrying
        // tool_calls belong to a round that started before this point; cutting into one leaves
        // an orphan.
        let mut start = last_user;
        while start > 0 && last_user - start < SLIM_HISTORY_TURNS {
            let prev = &messages[start - 1];
            let plain = (prev.role == "user" || prev.role == "assistant")
                && prev.tool_calls.is_none();
            if !plain {
                break;
            }
            start -= 1;
        }

        // System messages live at the top and are dropped by the slice, so they are collected
        // back. All of them: a second system message is usually the tool or memory preamble,
        // and losing it silently changes what the model thinks it can do.
        let mut slim: Vec<ChatMessage> = messages[..start]
            .iter()
            .filter(|m| m.role == "system")
            .cloned()
            .chain(messages[start..].iter().cloned())
            .collect();

        for msg in slim.iter_mut().filter(|m| m.role == "system") {
            msg.content = truncate_on_char_boundary(&msg.content, SLIM_SYSTEM_CHARS);
        }
        // Once, on the last one — it is the instruction that matters to a small model, and
        // repeating it on every system message spends the context this is trying to save.
        if let Some(last_sys) = slim.iter_mut().filter(|m| m.role == "system").last() {
            last_sys.content.push_str("\nBe concise. Answer directly.");
        }

        slim
    }

    /// Reduced generation config for fallback: bounded tokens, lower temp.
    ///
    /// Built from the caller's config rather than from `Default`, which used to drop the stop
    /// sequences, top_k, repeat penalty and max_context on the floor — and stop sequences are
    /// how a local GGUF backend is told when to stop talking.
    fn slim_config(config: &GenerationConfig) -> GenerationConfig {
        GenerationConfig {
            max_tokens: config.max_tokens.min(FALLBACK_MAX_TOKENS),
            temperature: config.temperature.min(0.5),
            ..config.clone()
        }
    }
}

/// Truncate to at most `max` bytes without splitting a UTF-8 character.
fn truncate_on_char_boundary(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

impl LLMBackend for FallbackLLM {
    fn chat(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
    ) -> Result<LLMResponse> {
        // A cancelled call starts nothing, on either backend.
        crate::call_cancel::check()?;
        // If primary has been failing consistently, skip to fallback.
        //
        // Through the same preparation as the failure path below. These two routes to the same
        // backend used to disagree — this one sent the conversation whole, the other cut it to
        // pieces — so which of the two a request took changed what the model was asked.
        if self.should_skip_primary() {
            if let Ok(fb) = self.get_fallback() {
                tracing::debug!("Skipping primary (consecutive failures), using fallback");
                let (msgs, cfg) = self.for_fallback(messages, config);
                let tools = if self.should_slim() { None } else { tools };
                return fb.chat(&msgs, &cfg, tools);
            }
        }

        match self.primary.chat(messages, config, tools) {
            Ok(resp) => {
                self.primary_success();
                Ok(resp)
            }
            // A cancel is not a failure: no count against the primary, and no fallback request.
            Err(e) if crate::call_cancel::is_cancel(&e) => Err(e),
            Err(primary_err) => {
                self.primary_failure();
                tracing::warn!(
                    error = %primary_err,
                    primary = self.primary.backend_name(),
                    "Primary LLM failed, trying fallback"
                );

                // A cancel that arrived after the primary answered is not a reason to start a
                // fallback either: the work this request served is over.
                if crate::call_cancel::cancelled() {
                    return Err(primary_err);
                }

                match self.get_fallback() {
                    Ok(fb) => {
                        // Tools go with the conversation when it is not being slimmed: a
                        // fallback that can hold the round can answer it, and stripping the
                        // definitions leaves it explaining in prose what it was asked to call.
                        let (msgs, cfg) = self.for_fallback(messages, config);
                        let tools = if self.should_slim() { None } else { tools };
                        fb.chat(&msgs, &cfg, tools)
                    }
                    Err(fb_err) => {
                        tracing::error!(
                            fallback_error = %fb_err,
                            "Fallback LLM also failed"
                        );
                        Err(primary_err)
                    }
                }
            }
        }
    }

    fn chat_streaming(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<LLMResponse> {
        // A cancelled call starts nothing, on either backend.
        crate::call_cancel::check()?;
        if self.should_skip_primary() {
            if let Ok(fb) = self.get_fallback() {
                tracing::debug!("Skipping primary (consecutive failures), using fallback streaming");
                let (msgs, cfg) = self.for_fallback(messages, config);
                let tools = if self.should_slim() { None } else { tools };
                return fb.chat_streaming(&msgs, &cfg, tools, on_token);
            }
        }

        match self.primary.chat_streaming(messages, config, tools, on_token) {
            Ok(resp) => {
                self.primary_success();
                Ok(resp)
            }
            // A cancel is not a failure: no count against the primary, and no fallback stream
            // appending a second answer to the tokens already sent.
            Err(e) if crate::call_cancel::is_cancel(&e) => Err(e),
            Err(primary_err) => {
                self.primary_failure();
                tracing::warn!(
                    error = %primary_err,
                    primary = self.primary.backend_name(),
                    "Primary LLM streaming failed, trying fallback"
                );

                // A cancel that arrived after the primary answered is not a reason to start a
                // fallback either: the work this request served is over.
                if crate::call_cancel::cancelled() {
                    return Err(primary_err);
                }

                match self.get_fallback() {
                    Ok(fb) => {
                        let (msgs, cfg) = self.for_fallback(messages, config);
                        let tools = if self.should_slim() { None } else { tools };
                        fb.chat_streaming(&msgs, &cfg, tools, on_token)
                    }
                    Err(_) => Err(primary_err),
                }
            }
        }
    }

    fn count_tokens(&self, text: &str) -> Result<usize> {
        self.primary.count_tokens(text)
            .or_else(|_| {
                self.get_fallback()
                    .and_then(|fb| fb.count_tokens(text))
            })
    }

    fn backend_name(&self) -> &str {
        self.primary.backend_name()
    }

    fn is_degraded(&self) -> bool {
        if let Ok(count) = self.primary_failures.lock() {
            *count > 0
        } else {
            false
        }
    }

    fn model_id(&self) -> &str {
        self.primary.model_id()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ApiToolCall, ApiToolCallFunction};

    /// A backend that is never called — the slimming tests only need a `FallbackLLM` to exist.
    struct NullBackend;

    impl LLMBackend for NullBackend {
        fn chat(
            &self,
            _messages: &[ChatMessage],
            _config: &GenerationConfig,
            _tools: Option<&[serde_json::Value]>,
        ) -> Result<LLMResponse> {
            anyhow::bail!("NullBackend is not meant to be called")
        }

        fn chat_streaming(
            &self,
            _messages: &[ChatMessage],
            _config: &GenerationConfig,
            _tools: Option<&[serde_json::Value]>,
            _on_token: &mut dyn FnMut(&str),
        ) -> Result<LLMResponse> {
            anyhow::bail!("NullBackend is not meant to be called")
        }

        fn count_tokens(&self, text: &str) -> Result<usize> {
            Ok(text.len())
        }

        fn backend_name(&self) -> &str {
            "null"
        }
    }

    fn tool_call(id: &str, name: &str) -> ApiToolCall {
        ApiToolCall {
            id: id.to_string(),
            call_type: "function".to_string(),
            function: ApiToolCallFunction {
                name: name.to_string(),
                arguments: "{}".to_string(),
            },
        }
    }

    fn roles(messages: &[ChatMessage]) -> Vec<&str> {
        messages.iter().map(|m| m.role.as_str()).collect()
    }

    #[test]
    fn slim_keeps_last_user_and_preceding_plain_turns() {
        let messages = vec![
            ChatMessage::system("you are the machine's mind"),
            ChatMessage::user("first question"),
            ChatMessage::assistant("first answer"),
            ChatMessage::user("second question"),
        ];

        let slim = FallbackLLM::slim_messages(&messages);

        assert_eq!(roles(&slim), vec!["system", "user", "assistant", "user"]);
        assert_eq!(slim.last().unwrap().content, "second question");
    }

    #[test]
    fn slim_mid_tool_round_keeps_the_user_turn_and_the_pair() {
        // The shape the primary 429s in the middle of: the last message is a tool result, so
        // the old rule ("keep the last message if it is a user message") kept nothing and the
        // fallback was sent a system prompt with no user query in it.
        let messages = vec![
            ChatMessage::system("you are the machine's mind"),
            ChatMessage::user("what is on my calendar"),
            ChatMessage::assistant_with_tool_calls("", vec![tool_call("call_1", "calendar.list")]),
            ChatMessage::tool("call_1", "calendar.list", "{\"events\":[]}"),
        ];

        let slim = FallbackLLM::slim_messages(&messages);

        assert_eq!(roles(&slim), vec!["system", "user", "assistant", "tool"]);
        assert!(
            slim.iter().any(|m| m.role == "user" && m.content == "what is on my calendar"),
            "the user's question must survive a tool round"
        );

        // Every tool result still has the assistant tool call that asked for it above it.
        let offered: Vec<&str> = slim
            .iter()
            .filter_map(|m| m.tool_calls.as_ref())
            .flatten()
            .map(|c| c.id.as_str())
            .collect();
        for result in slim.iter().filter(|m| m.role == "tool") {
            let id = result.tool_call_id.as_deref().unwrap();
            assert!(offered.contains(&id), "orphan tool result {id}");
        }
    }

    #[test]
    fn slim_does_not_cut_into_an_earlier_tool_round() {
        // Walking back for history must stop at the round boundary, not halfway through it.
        let messages = vec![
            ChatMessage::system("sys"),
            ChatMessage::user("older question"),
            ChatMessage::assistant_with_tool_calls("", vec![tool_call("call_1", "weather.now")]),
            ChatMessage::tool("call_1", "weather.now", "rain"),
            ChatMessage::assistant("it is raining"),
            ChatMessage::user("and tomorrow?"),
        ];

        let slim = FallbackLLM::slim_messages(&messages);

        // The plain assistant reply is carried; the tool result above it is not, because its
        // assistant tool call would have had to come with it.
        assert_eq!(roles(&slim), vec!["system", "assistant", "user"]);
        assert!(slim.iter().all(|m| m.role != "tool"));
        assert_eq!(slim.last().unwrap().content, "and tomorrow?");
    }

    #[test]
    fn slim_with_no_user_message_sends_the_conversation_whole() {
        let messages = vec![
            ChatMessage::system("sys"),
            ChatMessage::assistant_with_tool_calls("", vec![tool_call("call_1", "sys.status")]),
            ChatMessage::tool("call_1", "sys.status", "ok"),
        ];

        let slim = FallbackLLM::slim_messages(&messages);

        // No user turn is invented and nothing is dropped — a fabricated question would be
        // words put in the user's mouth, and an empty list is the fault this replaced.
        assert_eq!(roles(&slim), vec!["system", "assistant", "tool"]);
        assert_eq!(slim[0].content, "sys");
    }

    #[test]
    fn slim_keeps_every_system_message() {
        let messages = vec![
            ChatMessage::system("identity preamble"),
            ChatMessage::system("tool preamble"),
            ChatMessage::user("hello"),
        ];

        let slim = FallbackLLM::slim_messages(&messages);

        assert_eq!(roles(&slim), vec!["system", "system", "user"]);
        assert!(slim[0].content.starts_with("identity preamble"));
        assert!(slim[1].content.starts_with("tool preamble"));
        // The concision instruction is appended once, to the last system message.
        assert!(!slim[0].content.contains("Be concise"));
        assert!(slim[1].content.ends_with("Be concise. Answer directly."));
    }

    #[test]
    fn slim_truncates_a_long_system_prompt_on_a_char_boundary() {
        // Multi-byte, so a blind byte slice would panic rather than truncate.
        let long = "\u{092E}\u{0928}".repeat(2000);
        let messages = vec![ChatMessage::system(long), ChatMessage::user("hi")];

        let slim = FallbackLLM::slim_messages(&messages);

        let sys = &slim[0].content;
        let body = sys.strip_suffix("\nBe concise. Answer directly.").unwrap();
        assert!(body.len() <= SLIM_SYSTEM_CHARS);
        assert!(body.chars().all(|c| c == '\u{092E}' || c == '\u{0928}'));
    }

    #[test]
    fn slim_config_leaves_room_for_a_tool_call_and_keeps_stop_sequences() {
        let config = GenerationConfig {
            max_tokens: 8192,
            temperature: 0.9,
            stop: vec!["<|im_end|>".to_string()],
            ..Default::default()
        };

        let slim = FallbackLLM::slim_config(&config);

        // 512 used to cut a tool call off mid-JSON.
        assert_eq!(slim.max_tokens, FALLBACK_MAX_TOKENS);
        assert!(slim.max_tokens >= 2048);
        assert_eq!(slim.temperature, 0.5);
        assert_eq!(slim.stop, vec!["<|im_end|>".to_string()]);
    }

    #[test]
    fn slim_config_does_not_raise_a_caller_that_asked_for_less() {
        let config = GenerationConfig {
            max_tokens: 64,
            temperature: 0.1,
            ..Default::default()
        };

        let slim = FallbackLLM::slim_config(&config);

        assert_eq!(slim.max_tokens, 64);
        assert_eq!(slim.temperature, 0.1);
    }

    #[test]
    fn a_fallback_with_as_much_context_as_the_primary_is_not_slimmed() {
        let primary: Arc<dyn LLMBackend> = Arc::new(NullBackend);
        let fallback: Arc<dyn LLMBackend> = Arc::new(NullBackend);

        let small = FallbackLLM::with_fallback(primary.clone(), fallback.clone())
            .with_context_tokens(32_768, 4_096);
        assert!(small.should_slim());

        let peer = FallbackLLM::with_fallback(primary, fallback)
            .with_context_tokens(32_768, 32_768);
        assert!(!peer.should_slim());

        let messages = vec![
            ChatMessage::system("sys"),
            ChatMessage::user("q"),
            ChatMessage::assistant_with_tool_calls("", vec![tool_call("call_1", "t")]),
            ChatMessage::tool("call_1", "t", "r"),
        ];
        let config = GenerationConfig { max_tokens: 8192, ..Default::default() };
        let (msgs, cfg) = peer.for_fallback(&messages, &config);

        assert_eq!(msgs.len(), messages.len());
        assert_eq!(msgs[0].content, "sys", "no truncation, no appended instruction");
        assert_eq!(cfg.max_tokens, 8192, "no clamp when the fallback can hold the round");
    }

    #[test]
    fn a_llamacpp_fallback_reports_its_own_context_size() {
        let primary: Arc<dyn LLMBackend> = Arc::new(NullBackend);
        let llm = FallbackLLM::new(
            primary,
            Some(FallbackConfig::LlamaCpp {
                model_path: std::path::PathBuf::from("/nonexistent.gguf"),
                n_gpu_layers: 0,
                context_size: 65_536,
            }),
        );

        // Configured larger than the assumed primary, so nothing is cut.
        assert!(!llm.should_slim());
    }
}

/// A cancelled call is not a primary failure: it starts no fallback request, appends nothing more
/// to the stream, and never pushes the wrapper into skipping a good primary.
#[cfg(test)]
mod cancel_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// Emits one token, then reads the cancel flag the way a stream loop does (setting it first
    /// when `cancel_mid_stream`), or fails outright when `fail`.
    struct Counting {
        calls: AtomicUsize,
        cancel_mid_stream: Option<Arc<AtomicBool>>,
        fail: bool,
    }

    impl Counting {
        fn new(cancel_mid_stream: Option<Arc<AtomicBool>>, fail: bool) -> Arc<Self> {
            Arc::new(Self { calls: AtomicUsize::new(0), cancel_mid_stream, fail })
        }
        fn run(&self, on_token: &mut dyn FnMut(&str)) -> Result<LLMResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            on_token("tok ");
            if let Some(f) = &self.cancel_mid_stream {
                f.store(true, Ordering::SeqCst);
            }
            // A real failure is reported as such even when the flag is set: the cancel check is
            // what a stream loop does between lines, not what a backend that genuinely failed does.
            if self.fail {
                anyhow::bail!("connection reset");
            }
            crate::call_cancel::check()?;
            Ok(LLMResponse { text: "tok".into(), ..Default::default() })
        }
    }

    impl LLMBackend for Counting {
        fn chat(&self, _: &[ChatMessage], _: &GenerationConfig, _: Option<&[serde_json::Value]>) -> Result<LLMResponse> {
            self.run(&mut |_| {})
        }
        fn chat_streaming(&self, _: &[ChatMessage], _: &GenerationConfig, _: Option<&[serde_json::Value]>, on: &mut dyn FnMut(&str)) -> Result<LLMResponse> {
            self.run(on)
        }
        fn count_tokens(&self, t: &str) -> Result<usize> {
            Ok(t.len())
        }
        fn backend_name(&self) -> &str {
            "counting"
        }
    }

    fn msgs() -> Vec<ChatMessage> {
        vec![ChatMessage::system("be brief"), ChatMessage::user("hi")]
    }

    #[test]
    fn a_cancel_mid_stream_starts_no_fallback_and_counts_no_failure() {
        let flag = Arc::new(AtomicBool::new(false));
        let _g = crate::call_cancel::install_cancel(Some(flag.clone()));
        let primary = Counting::new(Some(flag.clone()), false);
        let fallback = Counting::new(None, false);
        let llm = FallbackLLM::with_fallback(primary.clone(), fallback.clone());
        let mut tokens = 0;
        let e = llm.chat_streaming(&msgs(), &GenerationConfig::default(), None, &mut |_| tokens += 1).unwrap_err();
        assert!(crate::call_cancel::is_cancel(&e), "{e:#}");
        assert_eq!(fallback.calls.load(Ordering::SeqCst), 0, "a fallback request was sent after the cancel");
        assert_eq!(tokens, 1, "only the primary's token reached the stream");
        assert!(!llm.is_degraded(), "a cancel was counted as a primary failure");
        // More calls while the flag is set start nothing, on either backend.
        for _ in 0..3 {
            assert!(llm.chat(&msgs(), &GenerationConfig::default(), None).is_err());
        }
        assert_eq!(primary.calls.load(Ordering::SeqCst), 1, "a call started after the flag was set");
        assert_eq!(fallback.calls.load(Ordering::SeqCst), 0);
    }

    /// The Err arm itself, with the guard at the top of the call out of the way: the primary is
    /// cancelled DURING the call, and a later call with a clear flag still goes to the primary.
    #[test]
    fn three_cancels_never_make_it_skip_the_primary() {
        let primary = Counting::new(None, false);
        let fallback = Counting::new(None, false);
        let llm = FallbackLLM::with_fallback(primary.clone(), fallback.clone());
        for _ in 0..3 {
            let flag = Arc::new(AtomicBool::new(false));
            let _g = crate::call_cancel::install_cancel(Some(flag.clone()));
            let mid = Counting::new(Some(flag), false);
            let one = FallbackLLM::with_fallback(mid, fallback.clone());
            assert!(one.chat(&msgs(), &GenerationConfig::default(), None).is_err());
            assert!(!one.is_degraded(), "a cancel was counted as a failure");
        }
        assert_eq!(fallback.calls.load(Ordering::SeqCst), 0, "a cancel fell back");
        llm.chat(&msgs(), &GenerationConfig::default(), None).expect("the primary answers");
        assert_eq!(primary.calls.load(Ordering::SeqCst), 1);
    }

    /// The same wrapper, cancelled mid-stream three times: the count never moves, so the fourth
    /// call (flag clear) still goes to the primary.
    #[test]
    fn three_cancels_on_one_wrapper_never_skip_the_primary() {
        let flag = Arc::new(AtomicBool::new(false));
        let primary = Counting::new(Some(flag.clone()), false);
        let fallback = Counting::new(None, false);
        let llm = FallbackLLM::with_fallback(primary.clone(), fallback.clone());
        for _ in 0..3 {
            flag.store(false, Ordering::SeqCst);
            let _g = crate::call_cancel::install_cancel(Some(flag.clone()));
            let e = llm.chat_streaming(&msgs(), &GenerationConfig::default(), None, &mut |_| {}).unwrap_err();
            assert!(crate::call_cancel::is_cancel(&e), "{e:#}");
            assert!(!llm.is_degraded());
        }
        let _none = crate::call_cancel::install_cancel(None);
        llm.chat(&msgs(), &GenerationConfig::default(), None).expect("the primary answers");
        assert_eq!(primary.calls.load(Ordering::SeqCst), 4, "the primary was skipped after cancels");
        assert_eq!(fallback.calls.load(Ordering::SeqCst), 0);
    }

    /// Real failures still count: after three, the primary is skipped.
    #[test]
    fn three_real_failures_still_skip_the_primary() {
        let primary = Counting::new(None, true);
        let fallback = Counting::new(None, false);
        let llm = FallbackLLM::with_fallback(primary.clone(), fallback.clone());
        for _ in 0..4 {
            llm.chat(&msgs(), &GenerationConfig::default(), None).expect("the fallback answers");
        }
        assert_eq!(primary.calls.load(Ordering::SeqCst), 3, "the fourth call skipped the primary");
        assert_eq!(fallback.calls.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn a_real_primary_failure_still_falls_back() {
        let primary = Counting::new(None, true);
        let fallback = Counting::new(None, false);
        let llm = FallbackLLM::with_fallback(primary.clone(), fallback.clone());
        llm.chat(&msgs(), &GenerationConfig::default(), None).expect("the fallback answers");
        assert_eq!(primary.calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallback.calls.load(Ordering::SeqCst), 1);
    }

    /// After three real failures the primary is skipped for a cooldown, then tried once more.
    #[test]
    fn a_primary_that_failed_three_times_is_tried_again_after_the_cooldown() {
        use std::sync::Mutex;
        let primary = Counting::new(None, true);
        let fallback = Counting::new(None, false);
        let mut llm = FallbackLLM::with_fallback(primary.clone(), fallback.clone());
        let clock = Arc::new(Mutex::new(std::time::Instant::now()));
        llm.now = {
            let c = clock.clone();
            Box::new(move || *c.lock().unwrap())
        };

        // Three real failures: the fourth call skips the primary.
        for _ in 0..4 {
            llm.chat(&msgs(), &GenerationConfig::default(), None).expect("the fallback answers");
        }
        assert_eq!(primary.calls.load(Ordering::SeqCst), 3, "the fourth call skipped the primary");

        // Advance past the cooldown: the primary is tried once more.
        *clock.lock().unwrap() += PRIMARY_COOLDOWN + std::time::Duration::from_secs(1);
        llm.chat(&msgs(), &GenerationConfig::default(), None).expect("the fallback answers");
        assert_eq!(primary.calls.load(Ordering::SeqCst), 4, "the primary was tried again after the cooldown");
    }

    /// A cancel never starts the fallback, whatever error the primary returned.
    #[test]
    fn a_cancel_never_starts_the_fallback() {
        // Non-streamed: the primary fails for a real reason after the flag is set mid-call.
        let flag = Arc::new(AtomicBool::new(false));
        let _g = crate::call_cancel::install_cancel(Some(flag.clone()));
        let primary = Counting::new(Some(flag.clone()), true);
        let fallback = Counting::new(None, false);
        let llm = FallbackLLM::with_fallback(primary.clone(), fallback.clone());
        let e = llm.chat(&msgs(), &GenerationConfig::default(), None).unwrap_err();
        assert!(!crate::call_cancel::is_cancel(&e), "the primary's own error is returned: {e:#}");
        assert_eq!(fallback.calls.load(Ordering::SeqCst), 0, "a fallback started after a cancel");

        // Streamed: same, through the streaming path.
        let flag = Arc::new(AtomicBool::new(false));
        let _g = crate::call_cancel::install_cancel(Some(flag.clone()));
        let primary = Counting::new(Some(flag.clone()), true);
        let fallback = Counting::new(None, false);
        let llm = FallbackLLM::with_fallback(primary.clone(), fallback.clone());
        let e = llm.chat_streaming(&msgs(), &GenerationConfig::default(), None, &mut |_| {}).unwrap_err();
        assert!(!crate::call_cancel::is_cancel(&e), "the primary's own error is returned: {e:#}");
        assert_eq!(fallback.calls.load(Ordering::SeqCst), 0, "a fallback started after a cancel");
    }
}
