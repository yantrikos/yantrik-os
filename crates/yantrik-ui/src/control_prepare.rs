//! `set_prepare_for_me(on)`: the shell action for Settings → Minds → Initiative → "Prepare for me:
//! research and first drafts" (#1416, slice 5), so a mind that drives this OS can find the switch
//! and ask for it without a mouse.
//!
//! Graded `sensitive`: turning it on lets the OS search the web and write drafts on its own, so
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

pub fn actions(surface: ControlSurface, ui: &App) -> ControlSurface {
    let weak = ui.as_weak();
    surface.action(
        Action::new(
            ACTION,
            "Turn \"Prepare for me: research and first drafts\" on or off (Settings, Minds, Initiative). On: when an item is due, the OS searches the web using its title and writes drafts (a new folder in Documents/Prepared with a summary, sources and the drafts; no note) on its own; it never sends or changes anything. It is off until the person turns it on, so this is sensitive and asks the person every time. Answers with the switch as the store now holds it",
        )
        .risk("sensitive")
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

#[cfg(test)]
mod tests {
    #[test]
    fn it_is_sensitive_and_goes_through_the_toggles_own_path() {
        let src = include_str!("control_prepare.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        assert!(code.contains(".risk(\"sensitive\")"));
        assert!(code.contains("wire::initiative::initiative_set"), "one path to the store");
        assert!(!code.contains("risk(\"standard\")") && !code.contains("risk(\"safe\")"));
    }
}
