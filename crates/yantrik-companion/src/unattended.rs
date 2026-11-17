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
    /// `handle_message` for a turn nobody started.
    pub fn handle_unattended(&mut self, text: &str) -> AgentResponse {
        let _turn = own_turn();
        let before = self.turn_ceiling;
        self.turn_ceiling = capped(before);
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.handle_message(text)));
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
        let mut service = service();
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
}
