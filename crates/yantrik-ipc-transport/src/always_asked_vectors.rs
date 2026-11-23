//! The always-asked rule's shared vectors (#1353, decision D), written into `surface-vectors.json`
//! under `always_asked` by `gate::tests::surface_vectors_write` and replayed by the Python gate.

use crate::gate::{decide, Authority, Mode, MODES};
use yantrik_ipc_contracts::control_surface::ALWAYS_ASKS;

/// What the always-asked rule decides in each mode (#1353, decision D): the listed actions, the
/// browser's, a declared `ALWAYS_ASKS` purpose, and what is exempt, in every mode, with and
/// without a grant. The Rust gate writes it and the Python gate replays it, so the two cannot
/// differ on the rule again: every mode but full bypass asks, and full bypass asks nothing.
pub(crate) fn vectors() -> Vec<serde_json::Value> {
    let said = "A sentence that says nothing about permanence.";
    let declared = format!("Hand over a key. {ALWAYS_ASKS}");
    let mut subjects: Vec<(String, String, String, &str)> = crate::always_asked::ALWAYS_ASKED
        .iter()
        .map(|(app, action)| (app.to_string(), action.to_string(), said.to_string(), "sensitive"))
        .collect();
    for action in ["type", "click", "a_future_action", "read"] {
        subjects.push(("browser".into(), action.into(), said.into(), "standard"));
    }
    subjects.push(("browser".into(), "type".into(), "Type it. It cannot be undone.".into(), "standard"));
    subjects.push(("anything".into(), "grant".into(), declared.clone(), "sensitive"));
    subjects.push(("anything".into(), "look".into(), declared, "safe"));
    subjects.push(("notes".into(), "report_problem".into(), said.into(), "sensitive"));
    let mut out = Vec::new();
    for (app, action, purpose, graded) in &subjects {
        for (mode, _) in MODES {
            for granted in [false, true] {
                let authority = Authority {
                    ceiling: "dangerous".into(),
                    mode: Mode { name: mode.to_string(), session_rules: Vec::new(), standing_rules: Vec::new() },
                    granted,
                    asks_above: None,
                };
                let (outcome, refusal) = match decide(&authority, app, action, graded, purpose) {
                    Ok(()) => ("allow", serde_json::Value::Null),
                    Err(why) => ("GRANT", serde_json::Value::String(why)),
                };
                out.push(serde_json::json!({
                    "id": format!("always-asked/{app}.{action}/{graded}/{mode}/grant={granted}/{}", tag(purpose)),
                    "app": app, "action": action, "purpose": purpose, "grade": graded,
                    "ceiling": "dangerous", "mode": mode, "session_rule": false, "grant": granted,
                    "outcome": outcome, "refusal": refusal,
                }));
            }
        }
    }
    out
}

/// A short, stable tag for a purpose, so two rows for one action with different sentences have different ids.
fn tag(purpose: &str) -> usize {
    purpose.bytes().fold(0usize, |h, b| (h * 31 + b as usize) % 100_003)
}

