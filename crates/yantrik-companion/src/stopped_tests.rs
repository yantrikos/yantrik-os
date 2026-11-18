//! A companion whose provider was removed says so, on every route a turn can take: the turn
//! that was running when it was removed, and the next one. It never says it is "in local mode".

use std::sync::Arc;

use yantrik_ml::{
    ChatMessage, GenerationConfig, LLMBackend, LLMResponse, LlmSwitch, Stopped, NO_PROVIDER,
};
use yantrikdb_core::YantrikDB;

use crate::companion::CompanionService;
use crate::config::CompanionConfig;

const QUESTION: &str = "what do you remember about the garden";

fn companion_on(llm: Arc<dyn LLMBackend>) -> CompanionService {
    let db = YantrikDB::new(":memory:", 64).expect("in-memory database");
    let mut config = CompanionConfig::default();
    config.tools.enabled = false;
    CompanionService::new(db, llm, config)
}

fn companion_with_tools(llm: Arc<dyn LLMBackend>) -> CompanionService {
    let db = YantrikDB::new(":memory:", 64).expect("in-memory database");
    let mut config = CompanionConfig::default();
    config.tools.enabled = true;
    CompanionService::new(db, llm, config)
}

fn answer(text: &str) -> LLMResponse {
    LLMResponse {
        text: text.into(),
        prompt_tokens: 0,
        completion_tokens: 1,
        tool_calls: vec![],
        api_tool_calls: vec![],
        stop_reason: "stop".into(),
    }
}

/// What the shell's stopped backend is: named "none", and every call is `Stopped`.
struct Refusing;

impl LLMBackend for Refusing {
    fn chat(
        &self,
        _: &[ChatMessage],
        _: &GenerationConfig,
        _: Option<&[serde_json::Value]>,
    ) -> anyhow::Result<LLMResponse> {
        Err(Stopped.into())
    }
    fn chat_streaming(
        &self,
        _: &[ChatMessage],
        _: &GenerationConfig,
        _: Option<&[serde_json::Value]>,
        _: &mut dyn FnMut(&str),
    ) -> anyhow::Result<LLMResponse> {
        Err(Stopped.into())
    }
    fn count_tokens(&self, text: &str) -> anyhow::Result<usize> {
        Ok(text.len())
    }
    fn backend_name(&self) -> &str {
        yantrik_ml::STOPPED_BACKEND
    }
}

/// A provider that is removed while it is answering: the press lands inside the call.
struct CutWhileAnswering(Arc<LlmSwitch>);

impl LLMBackend for CutWhileAnswering {
    fn chat(
        &self,
        _: &[ChatMessage],
        _: &GenerationConfig,
        _: Option<&[serde_json::Value]>,
    ) -> anyhow::Result<LLMResponse> {
        self.0.cut();
        Ok(answer("an answer from the removed provider"))
    }
    fn chat_streaming(
        &self,
        m: &[ChatMessage],
        c: &GenerationConfig,
        t: Option<&[serde_json::Value]>,
        on_token: &mut dyn FnMut(&str),
    ) -> anyhow::Result<LLMResponse> {
        let reply = self.chat(m, c, t)?;
        on_token(&reply.text);
        Ok(reply)
    }
    fn count_tokens(&self, text: &str) -> anyhow::Result<usize> {
        Ok(text.len())
    }
    fn backend_name(&self) -> &str {
        "api"
    }
}

fn says_no_provider(mut c: CompanionService) {
    let r = c.handle_message(QUESTION);
    assert_eq!(r.message, NO_PROVIDER);
    assert!(!r.offline_mode, "a removed provider is not an offline one");
    let mut streamed = String::new();
    let r = c.handle_message_streaming(QUESTION, |t| {
        if t == "__REPLACE__" {
            streamed.clear();
        } else {
            streamed.push_str(t);
        }
    });
    assert_eq!(r.message, NO_PROVIDER);
    assert_eq!(streamed, NO_PROVIDER);
    assert!(!r.offline_mode);
    assert!(!r.message.contains("local mode"));
}

#[test]
fn the_next_turn_on_a_stopped_backend_says_no_provider_is_set_up() {
    says_no_provider(companion_on(Arc::new(Refusing)));
}

#[test]
fn the_next_turn_on_a_cut_backend_says_no_provider_is_set_up() {
    let switch = Arc::new(LlmSwitch::new());
    let armed = switch.arm(Arc::new(Live));
    switch.cut();
    says_no_provider(companion_on(armed));
}

#[test]
fn a_turn_cut_while_it_was_answering_says_no_provider_is_set_up() {
    let switch = Arc::new(LlmSwitch::new());
    let armed = switch.arm(Arc::new(CutWhileAnswering(switch.clone())));
    let mut c = companion_on(armed);
    let mut shown = String::new();
    // The backend is live at the start, so the guard does not answer; the Err arm must.
    let r = c.handle_message_streaming(QUESTION, |t| {
        if t == "__REPLACE__" {
            shown.clear()
        } else {
            shown.push_str(t)
        }
    });
    assert_eq!(r.message, NO_PROVIDER);
    assert_eq!(shown, NO_PROVIDER);
    assert!(!r.offline_mode);
}

/// A live stand-in for the removed provider, to be armed and then cut.
struct Live;

impl LLMBackend for Live {
    fn chat(
        &self,
        _: &[ChatMessage],
        _: &GenerationConfig,
        _: Option<&[serde_json::Value]>,
    ) -> anyhow::Result<LLMResponse> {
        Ok(answer("hello"))
    }
    fn chat_streaming(
        &self,
        _: &[ChatMessage],
        _: &GenerationConfig,
        _: Option<&[serde_json::Value]>,
        on_token: &mut dyn FnMut(&str),
    ) -> anyhow::Result<LLMResponse> {
        on_token("hello");
        Ok(answer("hello"))
    }
    fn count_tokens(&self, text: &str) -> anyhow::Result<usize> {
        Ok(text.len())
    }
    fn backend_name(&self) -> &str {
        "api"
    }
}

/// The gate's turn: the model answers with a tool call, the tool runs, and the provider is removed
/// while the round after it is on the wire. The first reply is a call to `calculate`; the second
/// call is the one the press lands in.
struct ToolThenCut(Arc<LlmSwitch>);

impl LLMBackend for ToolThenCut {
    fn chat(
        &self,
        _: &[ChatMessage],
        _: &GenerationConfig,
        _: Option<&[serde_json::Value]>,
    ) -> anyhow::Result<LLMResponse> {
        // The round after the tool: the press lands here, and the call is thrown away.
        self.0.cut();
        Ok(answer("an answer from the removed provider"))
    }
    fn chat_streaming(
        &self,
        _: &[ChatMessage],
        _: &GenerationConfig,
        _: Option<&[serde_json::Value]>,
        _: &mut dyn FnMut(&str),
    ) -> anyhow::Result<LLMResponse> {
        let arguments = r#"{"expression":"1+1"}"#;
        let mut reply = answer("");
        reply.tool_calls = vec![yantrik_ml::ToolCall {
            name: "calculate".into(),
            arguments: serde_json::from_str(arguments).expect("fixed JSON"),
        }];
        reply.api_tool_calls = vec![yantrik_ml::ApiToolCall {
            id: "call_1".into(),
            call_type: "function".into(),
            function: yantrik_ml::ApiToolCallFunction { name: "calculate".into(), arguments: arguments.into() },
        }];
        Ok(reply)
    }
    fn count_tokens(&self, text: &str) -> anyhow::Result<usize> {
        Ok(text.len())
    }
    fn backend_name(&self) -> &str {
        "api"
    }
}

#[test]
fn a_turn_cut_in_its_tool_round_says_no_provider_is_set_up() {
    let switch = Arc::new(LlmSwitch::new());
    let armed = switch.arm(Arc::new(ToolThenCut(switch.clone())));
    let mut c = companion_with_tools(armed);
    let mut shown = String::new();
    let r = c.handle_message_streaming("what is one plus one", |t| {
        if t == "__REPLACE__" {
            shown.clear()
        } else {
            shown.push_str(t)
        }
    });
    assert_eq!(r.message, NO_PROVIDER);
    assert_eq!(shown, NO_PROVIDER);
    assert!(!r.offline_mode, "a removed provider is not an offline one");
}
