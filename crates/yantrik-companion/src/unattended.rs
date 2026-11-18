//! A turn nobody started: an EXECUTE urge or a queued task, run through the model loop (the shell's,
//! and `yantrik serve`'s).
//!
//! It begins and ends a taint turn of its own, so it neither inherits what the person's turn read nor
//! leaves what it read behind; there is no person to ask (`background_research::person_started` is
//! false); and every tool it runs is at most Standard, as a recipe step's is.

use yantrik_companion_core::permission::PermissionLevel;
use yantrik_companion_core::{background_research, taint};

use crate::companion::CompanionService;
use crate::types::AgentResponse;

/// The ceiling an unattended turn runs under, given the one already in force.
pub fn capped(current: Option<PermissionLevel>) -> Option<PermissionLevel> {
    Some(current.map_or(PermissionLevel::Standard, |c| c.min(PermissionLevel::Standard)))
}

/// A taint turn of its own while the guard lives, and the turn that was there before comes back after.
/// Inert inside a person's turn.
pub struct OwnTurn(Option<taint::Carried>);

pub fn own_turn() -> OwnTurn {
    if background_research::person_started() {
        return OwnTurn(None);
    }
    let before = taint::carry();
    taint::begin_turn();
    OwnTurn(Some(before))
}

impl Drop for OwnTurn {
    fn drop(&mut self) {
        if let Some(before) = self.0.take() {
            taint::end_turn();
            taint::adopt(before);
        }
    }
}

impl CompanionService {
    /// `handle_message_streaming` for a turn the person typed or spoke (`yantrik chat`, `yantrik ask`, the voice loop): the
    /// person is there, so the nobody-there rule does not tell them nobody is.
    pub fn handle_persons_message<F: FnMut(&str)>(&mut self, text: &str, on_token: F) -> AgentResponse {
        let _person = background_research::person_turn();
        self.handle_message_streaming(text, on_token)
    }

    /// `handle_message` for a turn nobody started.
    pub fn handle_unattended(&mut self, text: &str) -> AgentResponse {
        let _turn = own_turn();
        let before = self.turn_ceiling;
        self.turn_ceiling = capped(before);
        // What it is told and what it answers are not the person's conversation: the instruction would be in the
        // history the model reads on the person's next message, as if they had said it.
        let history = std::mem::take(&mut self.conversation_history);
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.handle_message(text)));
        self.conversation_history = history;
        self.turn_ceiling = before;
        run.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use yantrik_companion_core::tools::{Tool, ToolContext};
    use yantrik_ml::{ChatMessage, GenerationConfig, LLMBackend, LLMResponse, ToolCall};

    static SEEN: Mutex<Vec<PermissionLevel>> = Mutex::new(Vec::new());
    static READ: Mutex<Vec<Option<String>>> = Mutex::new(Vec::new());

    /// Records the ceiling the loop hands every tool it runs.
    struct Probe;
    impl Tool for Probe {
        fn name(&self) -> &'static str {
            "ceiling_probe"
        }
        fn permission(&self) -> PermissionLevel {
            PermissionLevel::Safe
        }
        fn category(&self) -> &'static str {
            "calculator"
        }
        fn definition(&self) -> serde_json::Value {
            serde_json::json!({"type": "function", "function": {"name": "ceiling_probe", "description": "probe", "parameters": {"type": "object", "properties": {}}}})
        }
        fn execute(&self, ctx: &ToolContext, _args: &serde_json::Value) -> String {
            SEEN.lock().unwrap_or_else(|e| e.into_inner()).push(ctx.max_permission);
            READ.lock().unwrap_or_else(|e| e.into_inner()).push(taint::state().0);
            "probed".into()
        }
    }

    /// Asks for the probe once, then answers.
    struct CallsTheProbe(Mutex<bool>);
    impl LLMBackend for CallsTheProbe {
        fn chat(&self, _: &[ChatMessage], _: &GenerationConfig, _: Option<&[serde_json::Value]>) -> anyhow::Result<LLMResponse> {
            let first = !std::mem::replace(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()), true);
            let tool_calls = if first { vec![ToolCall { name: "ceiling_probe".into(), arguments: serde_json::json!({}) }] } else { vec![] };
            Ok(LLMResponse {
                text: if first { String::new() } else { "done".into() },
                prompt_tokens: 0,
                completion_tokens: 1,
                tool_calls,
                api_tool_calls: vec![],
                stop_reason: "stop".into(),
            })
        }
        fn chat_streaming(&self, m: &[ChatMessage], c: &GenerationConfig, t: Option<&[serde_json::Value]>, _: &mut dyn FnMut(&str)) -> anyhow::Result<LLMResponse> {
            self.chat(m, c, t)
        }
        fn count_tokens(&self, text: &str) -> anyhow::Result<usize> {
            Ok(text.len())
        }
        fn backend_name(&self) -> &str {
            "calls-the-probe"
        }
    }

    fn service() -> CompanionService {
        let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
        let mut config = crate::config::CompanionConfig::default();
        config.tools.enabled = true;
        config.tools.max_permission = "dangerous".into();
        let mut service = CompanionService::new(db, std::sync::Arc::new(CallsTheProbe(Mutex::new(false))), config);
        service.registry.register(Box::new(Probe));
        service
    }

    #[test]
    fn the_ceiling_is_standard_at_most_and_what_was_in_force_comes_back() {
        assert_eq!(capped(None), Some(PermissionLevel::Standard));
        assert_eq!(capped(Some(PermissionLevel::Dangerous)), Some(PermissionLevel::Standard));
        assert_eq!(capped(Some(PermissionLevel::Safe)), Some(PermissionLevel::Safe));
    }

    #[test]
    fn an_unattended_model_loop_runs_its_tools_at_standard_at_most() {
        let mut service = service();
        SEEN.lock().unwrap_or_else(|e| e.into_inner()).clear();
        // The person's turn read a page before this ran.
        taint::begin_turn();
        taint::note("browse", "browser");
        READ.lock().unwrap_or_else(|e| e.into_inner()).clear();
        service.handle_unattended("do the thing");
        assert_eq!(taint::state().0.as_deref(), Some("browse"), "the person's turn is as it was");
        assert_eq!(*READ.lock().unwrap_or_else(|e| e.into_inner()), vec![None], "the unattended turn did not inherit what the person's turn read");
        taint::end_turn();
        let seen = SEEN.lock().unwrap_or_else(|e| e.into_inner()).clone();
        assert!(!seen.is_empty(), "the loop never ran the probe: the test proves nothing");
        assert!(seen.iter().all(|l| *l <= PermissionLevel::Standard), "{seen:?}");
        assert_eq!(service.turn_ceiling, None, "what was in force before comes back");
        // The control: the person's own turn, same service, same loop, is not capped.
        let mut service = self::service();
        SEEN.lock().unwrap_or_else(|e| e.into_inner()).clear();
        service.handle_message("do the thing");
        assert!(SEEN.lock().unwrap_or_else(|e| e.into_inner()).contains(&PermissionLevel::Dangerous), "the test cannot tell a cap from none");
    }

    #[test]
    fn a_program_on_the_socket_leaves_the_persons_turn_as_it_found_it() {
        let service = service();
        // A person's turn that read a page: the outside call must neither wipe it nor add its own mark.
        taint::begin_turn();
        taint::note("browse", "browser");
        let out = service.run_tool_for_outside("calculate", &serde_json::json!({"expression": "2+2"}));
        assert!(!out.starts_with("Refused"), "the outside call did not run, so this proves nothing: {out}");
        assert_eq!(taint::state().0.as_deref(), Some("browse"), "the person's turn was changed by a program on the socket");
        taint::end_turn();
        // A clean one stays clean, and a stranger's mark is not left behind.
        taint::begin_turn();
        service.run_tool_for_outside("calculate", &serde_json::json!({"expression": "2+2"}));
        assert_eq!(taint::state(), (None, None), "the socket's mark leaked into the person's turn");
        taint::end_turn();
    }

    #[test]
    fn the_outside_streaming_swap_puts_the_persons_turn_back() {
        // A model that has already "called" its tool answers at once: no probe run, so the statics stay the other tests'.
        let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
        let mut service = CompanionService::new(db, std::sync::Arc::new(CallsTheProbe(Mutex::new(true))), crate::config::CompanionConfig::default());
        taint::begin_turn();
        taint::note("browse", "browser");
        service.set_turn_outside(true);
        service.handle_message_streaming("do the thing", |_| {});
        assert_eq!(taint::state().0.as_deref(), Some("browse"), "the person's turn was not put back after the outside streaming turn");
        taint::end_turn();
    }

    #[test]
    fn the_shell_and_serve_both_enter_through_the_unattended_door() {
        for (file, needle) in [("../yantrik/src/main.rs", "companion.handle_unattended(&instruction)"), ("../yantrik-ui/src/bridge.rs", "companion.handle_unattended(&instruction)"), ("../yantrik-ui/src/bridge.rs", "companion.handle_unattended(&task_prompt)")] {
            let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/").to_string() + file).expect("read the source");
            assert!(src.contains(needle), "{file} no longer runs its urges and tasks unattended");
        }
    }

    #[test]
    fn a_turn_nobody_started_writes_nothing_into_the_persons_conversation() {
        // A model that never calls the probe: this test stays off the probe's shared statics.
        let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
        let mut config = crate::config::CompanionConfig::default();
        config.tools.enabled = true;
        let mut service = CompanionService::new(db, std::sync::Arc::new(CallsTheProbe(Mutex::new(true))), config);
        service.handle_message("hello, it is me");
        let before: Vec<String> = service.conversation_history.iter().map(|m| m.content.clone()).collect();
        assert!(!before.is_empty(), "the person's own turn is in the history: the test proves nothing otherwise");
        service.handle_unattended("EXECUTE: do the thing");
        let after: Vec<String> = service.conversation_history.iter().map(|m| m.content.clone()).collect();
        assert_eq!(before, after, "the unattended instruction or its answer reached the person's history");
        assert!(after.iter().all(|m| !m.contains("EXECUTE")));
    }

    static PERSON: Mutex<Vec<bool>> = Mutex::new(Vec::new());

    /// Records whether a person's turn is running when the loop runs it.
    struct WhoIsThere;
    impl Tool for WhoIsThere {
        fn name(&self) -> &'static str {
            "who_is_there"
        }
        fn permission(&self) -> PermissionLevel {
            PermissionLevel::Safe
        }
        fn category(&self) -> &'static str {
            "calculator"
        }
        fn definition(&self) -> serde_json::Value {
            serde_json::json!({"type": "function", "function": {"name": "who_is_there", "description": "probe", "parameters": {"type": "object", "properties": {}}}})
        }
        fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
            PERSON.lock().unwrap_or_else(|e| e.into_inner()).push(background_research::person_started());
            "here".into()
        }
    }

    /// Asks for `who_is_there` once, then answers.
    struct AsksWho(Mutex<bool>);
    impl LLMBackend for AsksWho {
        fn chat(&self, _: &[ChatMessage], _: &GenerationConfig, _: Option<&[serde_json::Value]>) -> anyhow::Result<LLMResponse> {
            let first = !std::mem::replace(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()), true);
            let tool_calls = if first { vec![ToolCall { name: "who_is_there".into(), arguments: serde_json::json!({}) }] } else { vec![] };
            Ok(LLMResponse { text: if first { String::new() } else { "done".into() }, prompt_tokens: 0, completion_tokens: 1, tool_calls, api_tool_calls: vec![], stop_reason: "stop".into() })
        }
        fn chat_streaming(&self, m: &[ChatMessage], c: &GenerationConfig, t: Option<&[serde_json::Value]>, _: &mut dyn FnMut(&str)) -> anyhow::Result<LLMResponse> {
            self.chat(m, c, t)
        }
        fn count_tokens(&self, text: &str) -> anyhow::Result<usize> {
            Ok(text.len())
        }
        fn backend_name(&self) -> &str {
            "asks-who"
        }
    }

    fn who_service() -> CompanionService {
        let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
        let mut config = crate::config::CompanionConfig::default();
        config.tools.enabled = true;
        config.tools.max_permission = "dangerous".into();
        let mut service = CompanionService::new(db, std::sync::Arc::new(AsksWho(Mutex::new(false))), config);
        service.registry.register(Box::new(WhoIsThere));
        service
    }

    #[test]
    fn a_turn_typed_or_spoken_at_the_terminal_is_the_persons_and_a_bare_call_is_not() {
        // `yantrik chat`, `yantrik ask` and the voice loop call `handle_persons_message`: R1 must not say nobody is here.
        PERSON.lock().unwrap_or_else(|e| e.into_inner()).clear();
        who_service().handle_persons_message("delete my old mail", |_| {});
        assert_eq!(*PERSON.lock().unwrap_or_else(|e| e.into_inner()), vec![true], "the person's own turn ran as if nobody were there");
        assert!(!background_research::person_started(), "the mark ended with the turn");
        // The control: the same loop called without it is unattended, so the test can tell the two apart.
        PERSON.lock().unwrap_or_else(|e| e.into_inner()).clear();
        who_service().handle_message_streaming("delete my old mail", |_| {});
        assert_eq!(*PERSON.lock().unwrap_or_else(|e| e.into_inner()), vec![false]);
    }

    #[test]
    fn the_terminal_commands_call_the_persons_door() {
        let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../yantrik/src/main.rs")).expect("read main.rs");
        assert_eq!(src.matches("companion.handle_persons_message(").count(), 3, "chat, ask and the voice loop");
        assert!(!src.contains("companion.handle_message_streaming("), "a terminal turn without the person's mark");
    }

    #[test]
    fn the_shells_system_sent_turns_get_the_standard_ceiling_and_a_phone_stays_safe() {
        let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../yantrik-ui/src/bridge.rs")).expect("read bridge.rs");
        let arm = src.lines().find(|l| l.contains("companion.set_turn_ceiling(")).expect("the SendMessage arm sets the ceiling");
        assert!(arm.contains("remote.then_some(yantrik_companion::tools::PermissionLevel::Safe)"), "a phone's turn stays Safe: {arm}");
        assert!(arm.contains("(!outside && !person).then_some(yantrik_companion::tools::PermissionLevel::Standard)"), "a turn that is neither the person's nor from outside is Standard: {arm}");
    }
}
