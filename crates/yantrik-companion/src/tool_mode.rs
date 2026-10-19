//! How the companion offers its tools to the model it is talking to.
//!
//! Start-up and a provider hot swap both derive this from the backend and its capability
//! profile, here, so the two can never disagree. VM 542, 16 Oct 2026: a fresh install started
//! on the not-set-up backend, which got tools described in the prompt; the person then chose
//! Claude in Settings and `swap_llm` re-detected the profile (NativeFunctionCall) but kept the
//! start-up choice. Claude was sent no `tools`, was told to write `<tool_call>` XML, and the
//! person saw raw `<invoke name="recall">` markup as the reply.

use yantrik_ml::{template_for_family, LLMBackend, ModelCapabilityProfile};

use crate::tools::{PermissionLevel, ToolRegistry};

/// The tool set-up for one backend.
pub(crate) struct ToolMode {
    /// Tools go in the request's `tools` field and come back as `tool_calls`.
    pub native: bool,
    /// The tools described in the prompt, for a backend that is not native; empty otherwise.
    pub prefix: String,
    /// The always-on tool schemas for the request's `tools` field.
    pub native_core: Vec<serde_json::Value>,
}

/// Native calling needs both an API backend that sends `tools` and a model that can use them.
/// The not-set-up and in-process backends describe the tools in the prompt instead.
pub(crate) fn wants_native_tools(llm: &dyn LLMBackend, profile: &ModelCapabilityProfile) -> bool {
    llm.backend_name() == "api" && profile.uses_native_tools()
}

/// The tool set-up for `llm` with `profile`: native tools only, or a prompt prefix only.
pub(crate) fn for_backend(
    llm: &dyn LLMBackend,
    profile: &ModelCapabilityProfile,
    registry: &ToolRegistry,
    always: &[&str],
    tools_enabled: bool,
    max_perm: PermissionLevel,
) -> ToolMode {
    let native = wants_native_tools(llm, profile);
    let native_core = if tools_enabled { registry.definitions_for(always, max_perm) } else { Vec::new() };
    let prefix = if tools_enabled && !native {
        tracing::info!(
            always = native_core.len(),
            total = registry.definitions(max_perm).len(),
            "Tools prefix: text-injected for {} backend (dynamic selection active)",
            llm.backend_name(),
        );
        template_for_family(profile.family).format_tools_compact(&native_core, profile.tier)
    } else {
        if tools_enabled {
            tracing::info!(
                always = native_core.len(),
                total = registry.definitions(max_perm).len(),
                "Native tool calling: {} always-on + dynamic per-query selection",
                native_core.len(),
            );
        }
        String::new()
    };
    ToolMode { native, prefix, native_core }
}
