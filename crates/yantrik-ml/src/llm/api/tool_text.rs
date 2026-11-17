//! How an [`ApiLLM`] offers tools on the OpenAI-compatible path: in the request's `tools` field,
//! or described in the system prompt for a model that cannot take them.

use crate::capability::ModelFamily;
use crate::types::ChatMessage;

use super::ApiLLM;

impl ApiLLM {
    /// Maximum tools Ollama's nemotron template can handle natively.
    /// Ollama's Go XML renderer still fails with >25 tools (XML syntax errors).
    /// When exceeded, falls back to text-injection in the system prompt.
    pub(super) const MAX_OLLAMA_NEMOTRON_NATIVE_TOOLS: usize = 25;

    /// Whether this backend should use text-injection for tools (vs native API tools).
    ///
    /// Text-injection injects tool definitions into the system prompt and parses
    /// tool calls from the model's text output using family-specific templates.
    ///
    /// Used when:
    /// - Nemotron models (always): Ollama's native tool template for nemotron produces
    ///   XML parsing errors with many tools. Our text-injection parser is more robust.
    /// - Non-Ollama local models (llama-server, etc.): typically lack proper jinja
    ///   templates for native tool calling.
    /// - NOT used for cloud APIs (OpenAI, Anthropic) or for the gateway, which handle tools
    ///   natively.
    pub(super) fn use_text_injection_tools(&self) -> bool {
        // The gateway is told what it is: its model's name is a placeholder.
        if self.gateway {
            return false;
        }
        // Cloud APIs handle tools natively
        if matches!(self.family, ModelFamily::OpenAI | ModelFamily::Anthropic) {
            return false;
        }
        // Other local models on non-Ollama servers need text-injection
        !self.is_ollama
    }

    /// Whether to fall back to text-injection for this specific call due to
    /// Ollama template limitations with many tools.
    pub(super) fn needs_text_injection_for_tool_count(&self, tool_count: usize) -> bool {
        self.is_ollama
            && matches!(self.family, ModelFamily::Nemotron)
            && tool_count > Self::MAX_OLLAMA_NEMOTRON_NATIVE_TOOLS
    }

    /// Inject tool definitions into the system message using the family-specific template.
    pub(super) fn inject_tools_into_messages(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> Vec<ChatMessage> {
        let template = self.template();
        let tool_block = template.format_tools_compact(tools, self.tier);
        if tool_block.is_empty() {
            return messages.to_vec();
        }

        let mut patched = messages.to_vec();
        if !patched.is_empty() && patched[0].role == "system" {
            patched[0] = ChatMessage::system(
                format!("{}{}", patched[0].content, tool_block),
            );
        } else {
            patched.insert(0, ChatMessage::system(tool_block));
        }
        patched
    }
}
