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
//!   for a card, which can only tighten. It is honoured only for the classes it is defined for
//!   ([`is_sink`]): a flag on a calendar delete is ignored, so a caller cannot raise caller-worded
//!   cards for anything it likes in full bypass.
//!
//! # The shell's own record
//!
//! The flag is the bridge's. The shell also keeps each harness's taint record (`taint_scope`), and
//! [`resolve`] ORs it in for a caller the shell follows (a believed `<harness>:main` token), so for
//! that caller the card does not depend on the caller declaring anything. A caller with no token
//! (Hermes and OpenClaw start their bridge with none) or a sub-agent is one the shell cannot place
//! in a conversation, so its record is not consulted: those get the card only from the bridge's
//! flag, and a caller that skips the bridge entirely and declares nothing gets what the mode gives.

use crate::mind_mode::Decision;
use yantrik_ipc_transport::gate::{fetches_address, leaves_machine, open_ended};

/// Hand-offs to another mind, and commands: mirrored from `HANDOFFS` and `COMMANDS` in
/// `yos-mcp`, which the selftest holds to this list.
pub const SINKS: [(&str, &str); 8] = [
    ("shell", "new_agent"),
    ("shell", "send_to_agent"),
    ("shell", "hand_off"),
    ("shell", "send_message"),
    ("terminal", "run"),
    ("terminal", "send_input"),
    ("shell", "agent_run"),
    ("shell", "agent_input"),
];

/// Is this call one a taint is defined for: a hand-off, a command, the browser's `go`, or an action
/// that declares itself one in its published purpose (runs what it is given, leaves the machine,
/// fetches an address). `app` is the name the app publishes under.
pub fn is_sink(app: &str, action: &str, purposes: &str) -> bool {
    SINKS.contains(&(app, action))
        || (matches!(app, "browser" | "chromium") && action == "go")
        || open_ended(purposes)
        || leaves_machine(purposes)
        || fetches_address(purposes).is_some()
}

/// What `request_approval` decides once the taint is applied: the table's `decision`, made an `Ask`
/// if the call is a sink and either the caller says it is tainted (`flag`) or the shell's own record
/// of the caller's conversation (`shell_record`) says it has read private state.
pub fn resolve(
    decision: Decision,
    flag: Option<&serde_json::Value>,
    shell_record: bool,
    app: &str,
    action: &str,
    purposes: &str,
) -> Decision {
    let tainted = (flagged(flag) || shell_record) && is_sink(app, action, purposes);
    settle(decision, tainted)
}

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

    fn full_bypass_run() -> Decision {
        let now = Instant::now();
        in_mode(Mode::BypassAll, now).decide("sensitive", "shell", "agent_run", Declared::default(), "dangerous", now)
    }

    /// The behaviour of the handler's decision: the flag in full bypass asks, and anything short of
    /// a true flag on a sink does not.
    #[test]
    fn resolve_asks_in_full_bypass_for_a_flagged_sink() {
        assert!(matches!(full_bypass_run(), Decision::Run { .. }), "what bypass_all did");
        let yes = json!(true);
        for (app, action, purposes) in [
            ("shell", "agent_run", ""),
            ("terminal", "run", ""),
            ("shell", "send_to_agent", ""),
            ("browser", "go", ""),
            ("scripts", "run", "Run it. What it runs can do anything you can."),
            ("shell", "report_problem", "Send it. What it is given may leave this machine."),
            ("download-manager", "add", "Download a URL. It fetches the address in `url`."),
        ] {
            assert_eq!(resolve(full_bypass_run(), Some(&yes), false, app, action, purposes), Decision::Ask, "{app}.{action}");
            assert_eq!(resolve(full_bypass_run(), None, true, app, action, purposes), Decision::Ask, "{app}.{action} by the shell's own record");
        }
        // Not tainted, or not a sink: the table's answer, unchanged.
        assert_eq!(resolve(full_bypass_run(), None, false, "shell", "agent_run", ""), full_bypass_run());
        assert_eq!(resolve(full_bypass_run(), Some(&json!(false)), false, "shell", "agent_run", ""), full_bypass_run());
        assert_eq!(resolve(full_bypass_run(), Some(&yes), true, "calendar", "delete_event", "Delete it."), full_bypass_run(), "a flag on a calendar delete is ignored");
        assert_eq!(resolve(full_bypass_run(), Some(&yes), false, "notes", "append", "Add text."), full_bypass_run());
        // And plan mode still refuses a flagged sink.
        let now = Instant::now();
        let plan = in_mode(Mode::Plan, now).decide("sensitive", "shell", "agent_run", Declared::default(), "dangerous", now);
        assert!(matches!(resolve(plan, Some(&yes), true, "shell", "agent_run", ""), Decision::Refuse { .. }));
    }

    /// The handler calls `resolve` before it branches, with the request's own `tainted` argument and
    /// the shell's record of the caller, and declares the argument. Read from the source as the other
    /// wiring tests here are: `resolve` itself is covered by behaviour above, and this holds the one
    /// line that connects it (a different key, `settle(.., false)` or a dropped record changes the line).
    #[test]
    fn request_approval_is_wired_to_resolve() {
        let source = include_str!("control_approvals.rs");
        let wired = "crate::taint_ask::resolve(table, args.get(\"tainted\"), crate::control_taint_scope::caller_has_read_private(), &app, &action, &format!(\"{published_purpose} {purpose}\"))";
        let call = source.find(wired).expect("request_approval must call taint_ask::resolve as written here");
        let branch = source.find("match decision {").expect("the handler branches on the decision");
        assert!(call < branch, "the taint must be applied before the handler branches on the decision");
        assert!(source.contains("Param::flag(\"tainted\")"), "the argument must be declared on request_approval");
        assert!(!source.contains("taint_ask::settle("), "the handler goes through resolve, never settle directly");
    }

    #[test]
    fn the_sink_list_is_what_the_bridge_names() {
        let bridge = include_str!("../../../deploy/yantrik-os/yos-mcp");
        for (app, action) in SINKS {
            assert!(bridge.contains(&format!("(\"{app}\", \"{action}\")")), "{app}.{action} is a sink in yos-mcp too");
        }
    }
}
