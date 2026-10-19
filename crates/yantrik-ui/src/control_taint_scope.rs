//! `taint_scope` and `taint_mark`: which of the person's conversations a bridge's private reads
//! count against, for `yos-mcp` (see `taint_scope`).
//!
//! Both are `safe` and neither takes anything that names a conversation: the caller's
//! conversation is what the desktop can establish about the caller, never what the caller says.
//!
//! **Only a call the desktop can attribute to the Lens conversation follows the Lens scope.**
//! Every other caller answers `follows: false`, and its bridge keeps its own taint for as long as
//! it runs, as the process scope always did. A caller the desktop cannot attribute is never
//! guessed into the Lens's conversation: it keeps its taint (fail closed).
//!
//! - A believed agent token for `<harness>:main` — the person's chat with that mind — follows that
//!   harness's scope. The token is issued for that one conversation and the bridge that carries it
//!   serves that conversation alone, so its calls are the Lens conversation's.
//! - Any other agent (a sub-agent, a role, a recipe) answers `follows: false`: the person's + does
//!   not start that mind's conversation again. A token that is not believed is refused.
//! - **No token** (Hermes and OpenClaw start their bridge with none) answers `follows: false`.
//!   One such bridge is the gateway's, and serves every session the gateway runs: the Lens, and
//!   also its cron jobs, its background (`/bg`) tasks and any other platform it is on. Nothing the
//!   desktop holds says which of those a call came from. The process walk names the mind, not
//!   the session. An open Lens turn says when the Lens is working, not that this call is its
//!   own: a cron job can call in the same second. Following the Lens scope there would let the
//!   person's + clear a background session that had read private state (security review of #817,
//!   design point 1). So a tokenless bridge keeps its taint until it ends, as it did before.
//!
//! A refusal is what the bridge reads as "the same scope": nothing it holds is cleared.
//!
//! `taint_mark` only adds. Nothing on this surface begins a scope or takes a read away: that is
//! the person's + in the Lens (`wire::chat`), and a test in `taint_scope` reads the source to keep
//! it so.

use yantrik_app_runtime::control::{Action, App as ControlSurface, Param};

use crate::agents::AgentId;
use crate::taint_scope::{self, Scope};

pub fn actions(surface: ControlSurface) -> ControlSurface {
    surface
        .action(
            Action::new(
                "taint_scope",
                "Which of the person's conversations the caller is in, for the taint rule: its id, whether the person began it with + in the Lens, and the private reads made in it. Only the person's + begins one",
            )
            .risk("safe"),
            move |_args| {
                Ok(match callers_conversation()? {
                    Caller::Lens(harness) => answer(&harness, &taint_scope::current(&harness)),
                    Caller::Own(why) => not_followed(why),
                })
            },
        )
        .action(
            Action::new(
                "taint_mark",
                "Record that the caller's conversation has read private state. It only ever adds; nothing here clears a conversation's reads",
            )
            .risk("safe")
            .arg(Param::text("read").describe("What was read, by the tool's name: os_describe, shell.read_screen")),
            move |args| {
                Ok(match callers_conversation()? {
                    Caller::Lens(harness) => {
                        let read = args["read"].as_str().unwrap_or_default();
                        answer(&harness, &taint_scope::mark(&harness, read))
                    }
                    Caller::Own(why) => not_followed(why),
                })
            },
        )
}

fn answer(harness: &str, scope: &Scope) -> serde_json::Value {
    serde_json::json!({
        "follows": true,
        "harness": harness,
        "scope": scope.id,
        "begun": scope.begun.as_str(),
        "read": scope.read,
    })
}

fn not_followed(why: &str) -> serde_json::Value {
    serde_json::json!({ "follows": false, "said": why })
}

/// Which conversation a caller's private reads count against.
#[derive(Debug, PartialEq, Eq)]
enum Caller {
    /// The Lens conversation with this harness: the person's + starts it again.
    Lens(String),
    /// Its own, kept for as long as its bridge runs. Why, in the person's words.
    Own(&'static str),
}

/// Another agent's own conversation: the person's + in the Lens is not about it.
const AN_AGENTS_OWN: &str = "This agent's conversation is its own, not one the person starts again with + in the Lens, so its taint lasts as long as its session.";

/// A bridge with no token: one process serving every session of its gateway.
const A_SHARED_BRIDGE: &str = "This bridge carries no agent token, so the desktop cannot tell which of the mind's sessions a call comes from: the Lens conversation, a scheduled job, a background task or another chat. The person's + in the Lens does not clear it, so its taint lasts as long as the bridge runs.";

/// The caller's conversation, from what the desktop itself can establish; an error when a token
/// is presented and not believed.
fn callers_conversation() -> Result<Caller, String> {
    match crate::control_agent_terminal::calling_agent() {
        Some(Ok(agent)) => Ok(attributed(Some(&agent))),
        Some(Err(why)) => Err(format!("the taint scope cannot be read for this caller: {why}")),
        None => Ok(attributed(None)),
    }
}

/// The rule, apart from how the token is read. Only `<harness>:main`, by a believed token, is the
/// Lens conversation. Any other agent, and any caller with no token, keeps its own.
fn attributed(agent: Option<&AgentId>) -> Caller {
    match agent {
        Some(agent) if agent.conversation() == AgentId::MAIN => Caller::Lens(agent.harness().to_string()),
        Some(_) => Caller::Own(AN_AGENTS_OWN),
        None => Caller::Own(A_SHARED_BRIDGE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The person's chat with a mind, by its own token, follows the Lens; an agent the person's +
    /// does not start again keeps its own.
    #[test]
    fn only_a_main_conversation_follows_the_lens() {
        assert_eq!(attributed(Some(&AgentId::new("pi", AgentId::MAIN))), Caller::Lens("pi".to_string()));
        assert_eq!(attributed(Some(&AgentId::new("pi", "c-7f3a91"))), Caller::Own(AN_AGENTS_OWN));
        assert_eq!(attributed(Some(&AgentId::new("deepseek", "c-role1"))), Caller::Own(AN_AGENTS_OWN));
    }

    /// A caller with no token is a gateway's bridge, shared by the Lens and the gateway's cron
    /// jobs, background tasks and other chats. The desktop cannot say which of them is calling,
    /// so it never follows the Lens: the person's + does not clear it (fail closed).
    #[test]
    fn a_caller_with_no_token_keeps_its_own_taint() {
        assert_eq!(attributed(None), Caller::Own(A_SHARED_BRIDGE));
        assert!(A_SHARED_BRIDGE.contains("does not clear it"), "the answer tells the truth about +");
    }

    /// Neither action takes anything that names a conversation, and neither is graded above `safe`.
    /// Attribution reads no process walk and no turn: only the token says a call is the Lens's.
    #[test]
    fn neither_action_takes_a_conversation_from_the_caller() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/control_taint_scope.rs");
        let whole = std::fs::read_to_string(path).unwrap();
        let code = whole.split("#[cfg(test)]").next().unwrap();
        assert_eq!(code.matches("Param::").count(), 1, "taint_mark's `read` is the only argument");
        for name in ["\"harness\")", "\"scope\")", "\"agent\")", "\"conversation\")"] {
            assert!(!code.contains(&format!("Param::text({name}")), "no action takes {name}");
        }
        assert_eq!(code.matches(".risk(\"safe\")").count(), 2);
        assert!(!code.contains("begin_by_person"), "nothing on the surface begins a scope");
        assert_eq!(code.matches("Caller::Lens(").count(), 3, "one rule makes a caller the Lens's; two actions read it");
        assert!(!code.contains("caller_identity::resolve"), "a process walk names the mind, not the session");
    }
}
