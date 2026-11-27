//! `set_prepare_for_me(on)`: the shell action for Settings → Minds → Initiative → "Prepare for me:
//! research and first drafts" (#1416, slice 5), so a mind that drives this OS can find the switch
//! and ask for it without a mouse.
//!
//! Graded `sensitive` and `always_asks` (on `ALWAYS_ASKED`): turning it on lets the OS search the web and write drafts on its own, so
//! the person is asked on a card every time, in either direction. The handler is the toggle's own
//! path (`wire::initiative::initiative_set`, which calls the store's `initiative_set`), so there is
//! one way to change the setting, and the shell's copy of it is updated from the store's answer.
//! `describe shell` → `agenda.prepared.switch` reads that copy live.

use serde_json::{json, Value};
use slint::ComponentHandle;
use yantrik_app_runtime::control::{answer_later, Action, App as ControlSurface, Param};
use yantrik_ipc_contracts::notifications::opt_in::{prepare_for_me_on, MOMENT_PREPARE_FOR_ME};

use crate::App;

/// The action's name.
pub const ACTION: &str = "set_prepare_for_me";

fn spec() -> Action {
    Action::new(
        ACTION,
        "Turn \"Prepare for me: research and first drafts\" on or off (Settings, Minds, Initiative). On: when an item is due, the OS searches the web using its title and writes drafts (a new folder in Documents/Prepared with a summary, sources and the drafts; no note) on its own; it never sends or changes anything. It is off until the person turns it on, so this is sensitive and asks the person every time. Answers with the switch as the store now holds it",
    )
    .risk("sensitive")
    // The person allows it on a card in every mode but full bypass, and no rule covers it.
    .always_asks()
}

pub fn actions(surface: ControlSurface, ui: &App) -> ControlSurface {
    let weak = ui.as_weak();
    surface.action(
        spec()
        .arg(Param::flag("on").describe("true to turn it on, false to turn it off")),
        move |args| {
            let Some(on) = args.get("on").and_then(Value::as_bool) else {
                return Err("`on` is required: true or false".to_string());
            };
            let weak = weak.clone();
            answer_later(move || {
                let settings = crate::wire::initiative::initiative_set(json!({ "moment": MOMENT_PREPARE_FOR_ME, "on": on }))?;
                let _ = weak.upgrade_in_event_loop(|ui| crate::wire::initiative::refresh(&ui));
                Ok(json!({ "switch": if prepare_for_me_on(&settings) { "on" } else { "off" } }))
            })
            .map(|()| json!({ "answering": "off the UI thread" }))
            .map_err(|_| "the shell has no worker free to answer this; ask again".to_string())
        },
    )
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use yantrik_ipc_transport::always_asked::always_asked;
    use yantrik_ipc_transport::gate::{always_asks, decide, Authority, Mode};

    fn authority(mode: &str) -> Authority {
        Authority { ceiling: "dangerous".into(), mode: Mode::named(mode), granted: false, asks_above: None }
    }

    /// The switch asks in Ask, Auto and Bypass, no session or standing rule answers it, and only the
    /// person's Allow or their full bypass (which they chose, #1353) runs it.
    #[test]
    fn it_asks_in_every_mode_but_full_bypass_and_no_rule_answers_it() {
        let s = spec();
        assert!(always_asked("shell", ACTION) && always_asks(&s.description));
        for mode in ["ask", "auto", "bypass"] {
            let said = decide(&authority(mode), "shell", ACTION, "sensitive", &s.description);
            assert!(said.unwrap_err().starts_with("GRANT:"), "{mode} ran it unasked");
        }
        assert!(decide(&authority("bypass_all"), "shell", ACTION, "sensitive", &s.description).is_ok(), "full bypass asks nothing");
        let mut ruled = authority("ask");
        ruled.mode.session_rules.push(("shell".into(), ACTION.into()));
        ruled.mode.standing_rules.push(("shell".into(), ACTION.into()));
        assert!(decide(&ruled, "shell", ACTION, "sensitive", &s.description).is_err(), "a rule does not cover it");
        let granted = Authority { granted: true, ..authority("bypass") };
        assert!(decide(&granted, "shell", ACTION, "sensitive", &s.description).is_ok(), "Allow runs it");
    }

    #[test]
    fn it_goes_through_the_toggles_own_path() {
        let src = include_str!("control_prepare.rs");
        let code = src.split("#[cfg(all(test").next().unwrap();
        assert!(code.contains("wire::initiative::initiative_set"), "one path to the store");
        assert!(code.contains(".always_asks()"));
    }
}
