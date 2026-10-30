//! The only code that writes the windows: the chrome (`crate::chrome`), set on whichever of the
//! two windows is being drawn.
//!
//! Slint compares what it is given with what it holds, so a property set to the words it already
//! has draws nothing: a quiet desk draws no frames.

use slint::SharedString;
use yantrik_app::app::WindowId;
use yantrik_app::cx::Cx;

use crate::metrics::{BOTTOM_HEIGHT, STRIP_HEIGHT, TOP_HEIGHT};
use crate::model::{Command, Model};
use crate::{chrome, MindDesk, MindDeskApp};

fn text(s: &str) -> SharedString {
    SharedString::from(s)
}

/// Bind a window's controls to commands.
pub fn wire(cx: &Cx<MindDesk>, ui: &MindDeskApp) {
    ui.on_stop_pressed(cx.cmd(|| Command::StopAsked));
    ui.on_stop_confirmed(cx.cmd(|| Command::StopConfirmed));
    ui.on_stop_cancelled(cx.cmd(|| Command::StopCancelled));
    ui.on_agents_pressed(cx.cmd(|| Command::AgentsAsked));
}

/// Which of the two windows `window` is.
pub fn is_footer(m: &Model, window: WindowId) -> bool {
    m.bottom == Some(window)
}

/// Draw `window`: its role and height first, which the compositor reads at map, then its words.
pub fn render(ui: &MindDeskApp, window: WindowId, m: &Model) {
    if is_footer(m, window) {
        ui.set_role(1);
        ui.set_bar_height(BOTTOM_HEIGHT as f32);
        let b = chrome::bottom(m);
        ui.set_apps(text(&b.apps));
        ui.set_safety(text(&b.safety));
        ui.set_model(text(&b.model));
        ui.set_model_mark(text(&b.model_mark));
    } else {
        ui.set_role(0);
        ui.set_bar_height(TOP_HEIGHT as f32);
        ui.set_strip_height(STRIP_HEIGHT as f32);
        let t = chrome::top(m);
        ui.set_label(text(&t.label));
        ui.set_tone(t.tone);
        ui.set_headline(text(&t.headline));
        ui.set_quoted(t.quoted);
        ui.set_reported(text(&t.reported));
        ui.set_can_stop(t.can_stop);
        ui.set_confirming(t.confirming);
        ui.set_confirm_question(text(&t.confirm_question));
        ui.set_stop_note(text(&t.stop_note));
        ui.set_agents_offered(t.agents_offered);
        ui.set_strip_heading(text(&t.strip_heading));
        ui.set_strip_tool(text(&t.strip_tool));
        ui.set_strip_mark(text(&t.strip_mark));
        ui.set_strip_stats(text(&t.strip_stats));
    }
}
