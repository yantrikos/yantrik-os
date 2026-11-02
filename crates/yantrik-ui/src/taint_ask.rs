//! A tainted outward action raises the card in every mode.
//!
//! The MCP bridge (`deploy/yantrik-os/yos-mcp`) knows when a conversation has read the person's
//! private state, and then a hand-off to another mind, a command, or a web address it has not
//! been to can carry that state out. It says so on `request_approval` with `tainted: true`.
//!
//! # The one named exception to full bypass asking nothing
//!
//! Full bypass (`bypass_all`) answers `Run` to everything under the machine's ceiling, so a
//! tainted call there used to be refused by the bridge with no card: the person's most permissive
//! mode was the strictest one, and a mind could not run its first command. Here a tainted call
//! that the table would run is asked instead, in every mode, whatever session rule or standing
//! rule covers it. It is never run unasked.
//!
//! # What this cannot do
//!
//! - **Loosen anything.** It turns `Run` into `Ask` and touches nothing else. A refusal (plan
//!   mode, the machine's ceiling, a grade this OS does not define) stands; no card is raised for
//!   what no answer could allow.
//! - **Run anything.** An `Ask` meets `approvals::request`, which refuses at once when approvals
//!   are off for a test run (`never_ask`) or the card cannot be shown whole (`approval_bounds`).
//!   Those are refusals with their reasons, never a run.
//! - **Widen a grant.** The grant a card mints is the store's, bound to the exact app, action and
//!   arguments and spent once. `tainted` is not an argument of the action and is not in the grant.
//! - **Be claimed by a mind to gain anything.** A caller that sets it on its own call has asked
//!   for a card, which can only tighten.

use crate::mind_mode::Decision;

/// Whether the request says its call is tainted. Only a boolean `true` (or the text `true`, as a
/// command line spells it) counts; anything else is an untainted call, decided exactly as before.
pub fn flagged(value: Option<&serde_json::Value>) -> bool {
    match value {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => s.trim().eq_ignore_ascii_case("true"),
        _ => false,
    }
}

/// The table's decision, with the taint applied: a tainted `Run` is an `Ask`.
pub fn settle(decision: Decision, tainted: bool) -> Decision {
    match decision {
        Decision::Run { .. } if tainted => Decision::Ask,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mind_mode::{Bypass, Declared, Mode, Modes};
    use serde_json::json;
    use std::time::Instant;

    const GRADES: [&str; 4] = ["safe", "standard", "sensitive", "dangerous"];

    fn in_mode(mode: Mode, now: Instant) -> Modes {
        let mut modes = Modes::new(Mode::Ask);
        modes.person_set_mode(mode, Bypass::Hour, now, 0);
        // A session rule that covers the action, to show a rule does not answer a tainted call.
        modes.person_add_rule_at("shell", "agent_run", "standard", "Run a command.", now).unwrap();
        modes
    }

    #[test]
    fn a_tainted_run_is_asked_in_every_mode_and_nothing_else_changes() {
        let now = Instant::now();
        for mode in Mode::ALL {
            let modes = in_mode(mode, now);
            for grade in GRADES {
                for declared in [Declared::default(), Declared { cannot_be_undone: false, open_ended: true }] {
                    let table = modes.decide(grade, "shell", "agent_run", declared, "dangerous", now);
                    // Not tainted: exactly the table.
                    assert_eq!(settle(table.clone(), false), table, "{mode:?}/{grade}");
                    let settled = settle(table.clone(), true);
                    match table {
                        Decision::Refuse { .. } | Decision::Ask => {
                            assert_eq!(settled, table, "{mode:?}/{grade}: a refusal and an ask stand")
                        }
                        Decision::Run { .. } => {
                            assert_eq!(settled, Decision::Ask, "{mode:?}/{grade}: never run unasked")
                        }
                    }
                    assert!(!matches!(settled, Decision::Run { .. }), "{mode:?}/{grade}: a tainted call is never a Run");
                }
            }
        }
    }

    #[test]
    fn full_bypass_asks_a_tainted_call_and_plan_still_refuses_it() {
        let now = Instant::now();
        let full = in_mode(Mode::BypassAll, now);
        let table = full.decide("standard", "shell", "agent_run", Declared::default(), "dangerous", now);
        assert!(matches!(table, Decision::Run { .. }), "what bypass_all did: {table:?}");
        assert_eq!(settle(table, true), Decision::Ask);
        let plan = in_mode(Mode::Plan, now);
        let refused = settle(plan.decide("standard", "shell", "agent_run", Declared::default(), "dangerous", now), true);
        assert!(matches!(refused, Decision::Refuse { ref why } if why.contains("plan mode")), "{refused:?}");
        // The machine's ceiling stands too: nothing a card could not satisfy is shown.
        let over = settle(full.decide("dangerous", "shell", "agent_run", Declared::default(), "standard", now), true);
        assert!(matches!(over, Decision::Refuse { ref why } if why.contains("tool_permission")), "{over:?}");
    }

    #[test]
    fn only_a_true_flag_is_a_taint() {
        assert!(flagged(Some(&json!(true))));
        assert!(flagged(Some(&json!("true"))));
        assert!(!flagged(Some(&json!(false))));
        assert!(!flagged(Some(&json!("false"))));
        assert!(!flagged(Some(&json!(1))));
        assert!(!flagged(Some(&json!(null))));
        assert!(!flagged(None));
    }

    /// The handler goes through `settle` before it branches on the decision, and declares the
    /// argument. Read from the source, as the other wiring tests here are: a handler that stopped
    /// calling it would pass every test above and run a tainted call in full bypass.
    #[test]
    fn request_approval_settles_the_decision_with_the_taint() {
        let source = include_str!("control_approvals.rs");
        let call = source.find("taint_ask::settle(").expect("request_approval must call taint_ask::settle");
        let branch = source.find("match decision {").expect("the handler branches on the decision");
        assert!(call < branch, "the taint must be applied before the handler branches on the decision");
        assert!(source.contains("Param::flag(\"tainted\")"), "the argument must be declared on request_approval");
        assert!(source.contains("taint_ask::flagged("), "the handler must read the argument");
    }
}
