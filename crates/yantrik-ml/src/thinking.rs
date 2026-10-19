//! A Qwen3's thinking, on a server the person runs: when to switch it off, and what it means
//! when a reply is all thinking.
//!
//! Qwen3-family chat templates think by default. Behind llama.cpp, vLLM, LM Studio or LocalAI the
//! thinking arrives as `reasoning_content`, separate from the answer, and it comes out of the same
//! `max_tokens`. On VM 542 (13 Oct 2026) a Qwen 3.8 Flash behind llama.cpp spent the companion's
//! whole tool-turn budget thinking about "Make a note called Groceries…", ended by length, and
//! sent no `content` at all: the Lens showed an empty bubble. The template's own switch,
//! `chat_template_kwargs: {"enable_thinking": false}`, stops that; Ollama's `think: false`, which
//! is what was sent, is not read by any of these servers.
//!
//! Which models get the switch is `model_caps::reasoning_for`'s decision, the same table the
//! gateway reads (#791): a Qwen3 family on a server the person runs (custom, llama.cpp, vLLM,
//! LM Studio, LocalAI), never gpt-oss there (it takes `reasoning_effort`), and never a hosted
//! service, which can answer an unknown body field with a 400. One rule for both, so a provider
//! type added there is added here too.

use crate::model_caps::{reasoning_for, Reasoning};

/// Whether a request to this model, on a provider of this type, should carry
/// `chat_template_kwargs: {"enable_thinking": false}`.
pub fn sends_chat_template_thinking(provider_type: &str, model: &str) -> bool {
    reasoning_for(provider_type, model) == Reasoning::ChatTemplateThinking
}

/// The provider type a backend name in `config.yaml` stands for, for the companion's start-up
/// path, which knows a backend name and not a provider entry. `api` is the generic
/// OpenAI-compatible endpoint, which is what a custom provider is.
pub fn provider_type_for_backend(backend: &str) -> &str {
    match backend {
        "api" => "custom",
        other => other,
    }
}

/// What the person is told when the model thought and gave no answer.
pub const THOUGHT_WITHOUT_ANSWER: &str =
    "The model thought but gave no answer; try again or choose another model.";

/// A reply that was all thinking and no answer: not an empty answer, a failure.
///
/// Returned by a backend as its error, so a caller can tell it from a network failure (which is
/// "offline") with `downcast_ref`, and say [`THOUGHT_WITHOUT_ANSWER`] instead of nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThoughtWithoutAnswer {
    /// The server's finish reason: `length` when the thinking used up `max_tokens`.
    pub finish_reason: String,
    /// How much it thought, in characters, for the log.
    pub reasoning_chars: usize,
}

impl std::fmt::Display for ThoughtWithoutAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the model thought ({} characters) but gave no answer (finish reason: {})",
            self.reasoning_chars, self.finish_reason
        )
    }
}

impl std::error::Error for ThoughtWithoutAnswer {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_qwen3_on_a_server_the_person_runs_is_sent_the_switch() {
        for provider in ["custom", "llamacpp", "vllm", "lmstudio", "localai"] {
            assert!(sends_chat_template_thinking(provider, "qwen3.8-flash-next-unsloth-ud-iq4_xs"), "{provider}");
            assert!(sends_chat_template_thinking(provider, "Qwen/Qwen3-30B-A3B"), "{provider}");
        }
    }

    #[test]
    fn a_hosted_service_is_not_sent_the_switch_even_for_a_qwen3() {
        for provider in ["groq", "openrouter", "openai", "together", "ollama"] {
            assert!(!sends_chat_template_thinking(provider, "qwen/qwen3-32b"), "{provider}");
        }
    }

    #[test]
    fn a_model_that_is_not_a_qwen3_is_not_sent_the_switch() {
        for model in ["qwen2.5-7b-instruct", "llama-3.1-8b", "gpt-oss-20b", "gemma-3-12b"] {
            assert!(!sends_chat_template_thinking("custom", model), "{model}");
        }
    }

    #[test]
    fn the_generic_api_backend_is_a_custom_provider() {
        assert_eq!(provider_type_for_backend("api"), "custom");
        assert_eq!(provider_type_for_backend("vllm"), "vllm");
    }
}
