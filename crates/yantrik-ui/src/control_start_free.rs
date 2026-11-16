//! "Start free, no account" on the control surface (#1167).
//!
//! The press is a consent: it lets one free provider be sent what the person's minds know about
//! them. So it is graded `dangerous`, above the ceiling an agent runs under, as "Erase disk and
//! install" is; and its handler refuses whoever reaches it, because the only press is the card's
//! own button, on screen, by the person. `yos describe shell` shows the card, its sentence and
//! whether it was pressed under `start_free`.

use yantrik_app_runtime::control::{Action, App as ControlSurface};

/// Said to any caller of the action: what the press grants, in the card's own words.
pub fn refusal() -> String {
    format!(
        "Start free, no account: {} Only the person's own press on the card does that; an agent cannot.",
        crate::start_free::offers::lead("the provider the card names")
    )
}

pub fn actions(surface: ControlSurface) -> ControlSurface {
    surface.action(
        Action::new(
            "start_free_press",
            "Press \"Start free, no account\": allow one free keyless provider to be sent messages and memories, \
             give the companion Yantrik models, and pick the provider's model. Refused to an agent; the person presses the card.",
        )
        .risk("dangerous")
        .explain(|_| refusal()),
        |_| Err(refusal()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = include_str!("control_start_free.rs");
    const CONTROL: &str = include_str!("control.rs");
    const UI: &str = include_str!("start_free/ui.rs");

    /// The ceiling a call runs under refuses `dangerous` before any handler is read
    /// (yantrik-surface's `the_ceiling_still_refuses_dangerous_whatever_the_grant_or_mode`), as it
    /// does for `installer_install`: so an agent's call is refused, whatever its mode or grant.
    #[test]
    fn an_agents_call_to_the_press_is_refused_twice_over() {
        let action = &SOURCE[SOURCE.find("\"start_free_press\"").expect("the action")..];
        let action = &action[..action.find("#[cfg(test)]").unwrap_or(action.len())];
        assert!(action.contains(".risk(\"dangerous\")"), "graded above an agent's ceiling");
        assert!(action.contains("|_| Err(refusal())"), "and the handler runs nothing for any caller");
        assert!(!action.contains("press::press") && !action.contains("Real::new"), "no way to the press from here");
        assert!(refusal().contains("Only the person's own press") && refusal().contains("what they read on this computer"));
    }

    /// The person's press is the card's button: a callback of the card, which starts the press.
    #[test]
    fn the_person_presses_the_card_and_the_surface_only_describes_it() {
        assert!(UI.contains("g.on_press(") && UI.contains("press::press(&steps, &offer.provider)"));
        assert!(CONTROL.contains("crate::control_start_free::actions(crate::control_installer::actions(surface, ui))"));
        assert!(CONTROL.contains(".with(\"start_free\", crate::start_free::for_describe())"));
    }
}
