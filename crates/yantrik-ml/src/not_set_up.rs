//! The companion's model when none is chosen: the shipped config names none (the public ISO has no
//! bundled model, and a name nothing serves is a fake), so the companion still starts, and its
//! chat answers with one plain sentence that names the fix rather than an error.
//!
//! A streamed answer (the person's chat) is [`NOT_SET_UP`]; any other call (the companion's own
//! background thinking) fails quietly, so nothing it does unasked says the sentence.

use anyhow::Result;

use crate::traits::LLMBackend;
use crate::types::{ChatMessage, GenerationConfig, LLMResponse};

/// What the chat says while no mind can think.
pub const NOT_SET_UP: &str = "No mind can think yet. Choose who does the thinking in Settings → AI.";

/// [`LLMBackend::backend_name`] of [`NotSetUpLLM`].
pub const BACKEND: &str = "not-set-up";

/// Whether `model` names a model: absent, empty or blank is none.
pub fn chosen(model: Option<&str>) -> Option<&str> {
    model.map(str::trim).filter(|m| !m.is_empty())
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NotSetUpLLM;

impl LLMBackend for NotSetUpLLM {
    fn chat(&self, _: &[ChatMessage], _: &GenerationConfig, _: Option<&[serde_json::Value]>) -> Result<LLMResponse> {
        anyhow::bail!("{NOT_SET_UP}")
    }

    fn chat_streaming(
        &self,
        _: &[ChatMessage],
        _: &GenerationConfig,
        _: Option<&[serde_json::Value]>,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<LLMResponse> {
        on_token(NOT_SET_UP);
        Ok(LLMResponse {
            text: NOT_SET_UP.to_string(),
            prompt_tokens: 0,
            completion_tokens: 0,
            tool_calls: Vec::new(),
            api_tool_calls: Vec::new(),
            stop_reason: "stop".into(),
        })
    }

    fn count_tokens(&self, text: &str) -> Result<usize> {
        Ok(text.len() / 4)
    }

    fn backend_name(&self) -> &str {
        BACKEND
    }

    fn model_id(&self) -> &str {
        BACKEND
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chat_says_how_to_choose_a_model_and_background_calls_fail_quietly() {
        let mut said = String::new();
        let r = NotSetUpLLM.chat_streaming(&[], &GenerationConfig::default(), None, &mut |t| said.push_str(t)).unwrap();
        assert_eq!(said, NOT_SET_UP);
        assert_eq!(r.text, NOT_SET_UP);
        assert!(NotSetUpLLM.chat(&[], &GenerationConfig::default(), None).is_err());
        assert_eq!(NotSetUpLLM.backend_name(), BACKEND);
    }

    #[test]
    fn a_blank_model_is_no_model() {
        assert_eq!(chosen(None), None);
        assert_eq!(chosen(Some("")), None);
        assert_eq!(chosen(Some("  ")), None);
        assert_eq!(chosen(Some(" qwen3:8b ")), Some("qwen3:8b"));
    }
}
