//! `approvals_test_state`: whether "approvals off for a test run" is on, read without looking at
//! anything of the person's.
//!
//! `describe shell` publishes the same answer under `approvals_off_for_test`, but a mind's
//! `describe` is a private read for the taint rule, so a test run that only wants this one flag
//! would taint its own session to learn it. This action is the untainted way to ask.
//!
//! It is `safe`, takes no arguments and answers exactly [`crate::never_ask::snapshot`]: `on`,
//! `expires_in_secs` and `means`. That holds no person's data: a flag, a countdown and a sentence.
//!
//! - **Any caller may read it.** Reading is not the switch: [`crate::never_ask::may_switch`] guards
//!   `set_approvals_off_for_test`, and nothing here consults it, a token or a process.
//! - **It cannot change the state.** The handler calls `snapshot()` and nothing else from
//!   `never_ask`; a test reads the source to keep it so.

use yantrik_app_runtime::control::{Action, App as ControlSurface};

pub fn actions(surface: ControlSurface) -> ControlSurface {
    surface.action(
        Action::new(
            "approvals_test_state",
            "Whether a test run has turned the person's approval cards off, and for how long more: on, expires_in_secs, means. Reads one flag and nothing of the person's; it changes nothing",
        )
        .risk("safe"),
        move |_args| Ok(answer()),
    )
}

/// What the action answers: the state, as `never_ask` publishes it.
fn answer() -> serde_json::Value {
    crate::never_ask::snapshot()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one test that engages the mode, so nothing else in this crate's test run sees it on.
    #[test]
    fn it_answers_off_by_default_and_on_while_engaged() {
        let off = answer();
        assert_eq!(off["on"], false, "{off}");
        assert!(off["expires_in_secs"].is_null(), "{off}");
        assert!(off["means"].as_str().is_some_and(|m| m.starts_with("Off:")), "{off}");

        crate::never_ask::switch_on(Some(5));
        let on = answer();
        assert_eq!(on["on"], true, "{on}");
        let left = on["expires_in_secs"].as_u64().expect("a countdown while on");
        assert!((290..=300).contains(&left), "{on}");
        assert!(on["means"].as_str().is_some_and(|m| !m.starts_with("Off:")), "{on}");
        assert_eq!(on.as_object().unwrap().len(), 3, "exactly on, expires_in_secs, means");

        // Reading, however often, leaves it on and does not move the deadline.
        for _ in 0..3 {
            assert_eq!(answer()["on"], true);
        }
        crate::never_ask::switch_off();
        assert_eq!(answer()["on"], false, "{}", answer());
    }

    /// Any caller may read it, and reading cannot change it: no argument, no check of who is
    /// asking, nothing from `never_ask` but `snapshot`, and nothing graded above `safe`.
    #[test]
    fn any_caller_may_read_it_and_it_changes_nothing() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/control_approvals_test_state.rs");
        let whole = std::fs::read_to_string(path).unwrap();
        let code = whole.split("#[cfg(test)]").next().unwrap();
        let code: String = code.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        assert_eq!(code.matches(".risk(\"safe\")").count(), 1);
        assert_eq!(code.matches("Action::new(").count(), 1, "one action");
        for banned in [".arg(", "Param::", "may_switch", "calling_agent", "caller_identity", "token", "switch_on", "switch_off", "engage", "clear"] {
            assert!(!code.contains(banned), "the action must not use `{banned}`");
        }
        assert_eq!(code.matches("never_ask::").count(), 1, "only snapshot() is read");
        assert!(code.contains("never_ask::snapshot()"));
    }
}
