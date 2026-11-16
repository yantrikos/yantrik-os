//! The sandbox the code runners (`code_execute`, `script_run`) run a script in, and the two rules
//! about it that keep "nothing reaches the web on its own" true (#1171):
//!
//! - **The network is a different reach.** `strict` has none; `network` does, and
//!   `yantrik_companion_core::reach` counts a call that asks for one as a browser (a turn the person
//!   started only). That table lists the names; [`SandboxMode::parse`] must read the same ones,
//!   and a test below pins it. `none` and `unsafe` stay in the table, though [`SandboxMode::parse`]
//!   refuses them (#1247, review R11-H2): a script with no sandbox reads every protected file as the
//!   person, and nothing asks them.
//! - **A sandbox that was asked for and is missing is a refusal.** `strict` without bubblewrap used
//!   to run the script with nothing around it. The image ships bubblewrap (`packages.list`), so the
//!   shipped path is unchanged.
//!
//! Installing a script's dependencies (pip, npm) runs outside the sandbox and fetches from the
//! public registries, so it asks the same gate as any web fetch ([`allow_install`]).

use crate::spawn::SandboxMode;

/// `Ok` when the script may run, which is only inside bubblewrap; or why it must not run.
pub(crate) fn plan(bwrap_present: bool) -> Result<(), String> {
    match bwrap_present {
        true => Ok(()),
        false => Err("Refused: this script was to run in a sandbox, and bubblewrap (bwrap) is not installed, so it was not run. Nothing was executed outside a sandbox."
            .into()),
    }
}

/// The gate for installing dependencies: a fetch from the open web, so `Web` in the reach table.
pub(crate) fn allow_install() -> Result<(), String> {
    yantrik_companion_core::reach::check_call("install_deps", "coder", &serde_json::json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_companion_core::reach::{reach, Reach};

    #[test]
    fn a_missing_sandbox_is_a_refusal_not_a_bare_run() {
        assert!(plan(false).unwrap_err().starts_with("Refused:"));
        assert_eq!(plan(true), Ok(()));
    }

    /// There is no mode with no sandbox: `none` and `unsafe` are refused in the rule's words, not taken
    /// as `strict` (review R11-H2).
    #[test]
    fn a_request_for_no_sandbox_is_refused() {
        for mode in ["none", "unsafe"] {
            assert!(SandboxMode::parse(mode).unwrap_err().starts_with("Access denied"), "{mode}");
        }
    }

    #[test]
    fn the_modes_the_reach_table_calls_the_network_are_the_modes_that_have_one() {
        for mode in ["network", "web", "none", "unsafe"] {
            // The two that can run have a network; the other two never run, and the table still counts them.
            if let Ok(parsed) = SandboxMode::parse(mode) {
                assert_ne!(parsed, SandboxMode::Strict, "{mode}");
            }
            for tool in ["code_execute", "script_run"] {
                assert_eq!(reach(tool, "coder", &serde_json::json!({ "sandbox": mode })), Reach::Browser, "{tool} {mode}");
            }
        }
        for tool in ["code_execute", "script_run"] {
            assert_eq!(reach(tool, "coder", &serde_json::json!({ "sandbox": "strict" })), Reach::Local);
            assert_eq!(reach(tool, "coder", &serde_json::json!({})), Reach::Local, "strict is the default");
        }
    }

    #[test]
    fn installing_dependencies_is_refused_unattended_and_allowed_in_a_persons_turn() {
        // The test process never turns research on.
        let said = allow_install().unwrap_err();
        assert!(said.starts_with("Refused:") && said.contains("install_deps"), "{said}");
        let _mine = yantrik_companion_core::background_research::person_turn();
        assert_eq!(allow_install(), Ok(()));
    }
}
