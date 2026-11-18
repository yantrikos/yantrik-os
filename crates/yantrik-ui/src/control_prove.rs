//! `prove`: the companion proves itself on request (#1291, first slice).
//!
//! The three proofs the first run makes, by the same functions (`prove`), asked again at any
//! time: `yos act shell prove`. The answer comes a few seconds later, off the UI thread, as the
//! report in words (`yos` prints `text`) with the report beside it as data. The last one is kept
//! and `describe shell` carries it under `proofs`.
//!
//! Graded as `send_message` is, because it makes the companion take a turn with the person's
//! provider: the default grade of `Action::new`, `standard`, which `send_message` takes too.

use yantrik_app_runtime::control::{self, Action, App as ControlSurface};

use crate::prove::{Prover, ShellGate};
use crate::wire::onboarding::CompanionProofs;

/// The worst case is the three proofs' own limits added (90 + 60 + 60 s); it is a second or two
/// when the companion is well.
const EXPECTED_SECS: u32 = 30;

/// The action as the surface publishes it.
fn spec() -> Action {
    Action::new(
        "prove",
        "Prove the companion works now: ask it a question that needs a tool, have it write a note \
         and read it back, have it describe the desktop. The first run's three checks, by the same \
         code. Answers with what passed and what needs attention, and `describe shell` keeps the \
         last result under `proofs`. Not run in Private mode or with no provider; one at a time. \
         Takes a few seconds",
    )
    .risk("standard")
    .expected_seconds(EXPECTED_SECS)
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

pub fn actions(surface: ControlSurface) -> ControlSurface {
    surface.action(spec(), |_| {
        let work = || {
            let reply = Prover::shell().prove(&ShellGate, &CompanionProofs, &crate::wire::onboarding::ask_the_mind, yantrik_version::version(), now());
            Ok(reply.json())
        };
        control::answer_later(work).map(|()| serde_json::json!({ "answering": "off the UI thread" })).or_else(|work| work())
    })
}

/// `describe shell` › `proofs`.
pub fn for_describe() -> serde_json::Value {
    Prover::shell().describe(now())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(rel: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
    }

    /// The text of `src` before its tests.
    fn code(rel: &str) -> String {
        read(rel).split("#[cfg(test)]").next().unwrap_or_default().to_string()
    }

    #[test]
    fn prove_is_graded_as_send_message_is() {
        assert_eq!(spec().permission, Action::new("send_message", "").permission);
        let control = code("src/control.rs");
        let at = control.find("Action::new(\"send_message\"").expect("send_message is declared in control.rs");
        let declared = &control[at..at + control[at..].find("move |args|").unwrap()];
        assert!(!declared.contains(".risk("), "send_message takes the default grade; if it is graded, prove follows:\n{declared}");
        assert_eq!(spec().permission, "standard");
    }

    #[test]
    fn the_action_is_on_the_surface_and_describe_carries_its_proofs() {
        let control = code("src/control.rs");
        assert!(control.contains("crate::control_prove::actions("), "the shell's surface chains the action in");
        assert!(control.contains(".with(\"proofs\", crate::control_prove::for_describe())"), "describe shell carries `proofs`");
    }

    #[test]
    fn the_action_is_answered_off_the_ui_thread() {
        assert!(code("src/control_prove.rs").contains("control::answer_later("));
    }

    #[test]
    fn the_first_run_and_the_action_name_the_same_three_functions() {
        let first = code("src/wire/onboarding.rs");
        let start = first.find("fn start_checks(").expect("the first run starts its checks");
        let checks = &first[start..first[start..].find("\n}\n").unwrap() + start];
        for call in ["CheckKind::Answer => ask_the_mind(&mind)", "CheckKind::Remember => proofs.remember(&mind)", "CheckKind::Desktop => proofs.desktop(&mind)"] {
            assert!(checks.contains(call), "the first run no longer calls `{call}`:\n{checks}");
        }
        assert!(first.contains("Arc::new(CompanionProofs)"), "the first run's proofs are CompanionProofs");

        let run = code("src/prove/mod.rs");
        for call in ["CheckKind::Answer => ask(&id)", "CheckKind::Remember => proofs.remember(&id)", "CheckKind::Desktop => proofs.desktop(&id)"] {
            assert!(run.contains(call), "prove::run does not make `{call}`");
        }
        let here = code("src/control_prove.rs");
        assert!(here.contains("&CompanionProofs") && here.contains("&crate::wire::onboarding::ask_the_mind"), "the action runs the first run's proofs");
    }

    #[test]
    fn the_guide_has_its_row_and_yos_prints_the_text() {
        let guide = read("../../docs/app-control.md");
        let row = guide.lines().find(|l| l.starts_with("| shell | `prove` |")).expect("docs/app-control.md has a row for `prove`");
        assert!(row.contains("| standard |") && row.contains("send_message") && row.contains("proofs"), "{row}");
        let yos = read("../../deploy/yantrik-os/yos");
        assert!(yos.contains("result.get(\"readable\") is True"), "yos prints a readable result's text");
    }
}
