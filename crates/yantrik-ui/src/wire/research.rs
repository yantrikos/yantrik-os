//! Settings → Minds → Initiative → Background research (#1171, Fable's decision, item 1).
//!
//! Shows whether the companion may search the web on its own, and the one control each way: Turn
//! off (it lets less out, so it asks nothing) and Allow, which is the person asking to be asked: it
//! forgets an earlier No and raises the same card the companion's own wish would, naming what is
//! sent and to whom. Nothing here turns research on; the card's Allow does (`control_research`).

use slint::ComponentHandle;

use crate::{App, InitiativeState};

pub fn wire(ui: &App) {
    let state = ui.global::<InitiativeState>();
    let weak = ui.as_weak();
    state.on_research_off(move || {
        if let Err(why) = crate::control_research::turn_off() {
            tracing::warn!("background research could not be turned off: {why}");
        }
        if let Some(ui) = weak.upgrade() {
            refresh(&ui);
        }
    });
    state.on_research_ask(|| {
        match crate::control_research::ask_again() {
            Ok(()) => yantrik_companion::research_offer::offer("Settings"),
            Err(why) => tracing::warn!("{why}"),
        }
    });
    refresh(ui);
}

/// Draw where the switch stands.
pub fn refresh(ui: &App) {
    let state = ui.global::<InitiativeState>();
    state.set_research_on(yantrik_companion::background_research::is_on());
    state.set_research_declined(yantrik_companion::background_research::declined());
}
