//! The only code that writes the window: the chrome (`crate::chrome`), set on the one bar.
//!
//! Slint compares what it is given with what it holds, so a property set to the words it already
//! has draws nothing: a quiet desk draws no frames.

use slint::SharedString;
use yantrik_app::cx::Cx;

use crate::metrics::{BAR_HEIGHT, HEADER_HEIGHT, STRIP_HEIGHT};
use crate::model::{Command, Model};
use crate::{chrome, MindDesk, MindDeskApp};

fn text(s: &str) -> SharedString {
    SharedString::from(s)
}

/// Bind the window's controls to commands.
pub fn wire(cx: &Cx<MindDesk>, ui: &MindDeskApp) {
    ui.on_stop_pressed(cx.cmd(|| Command::StopAsked));
    ui.on_agents_pressed(cx.cmd(|| Command::AgentsAsked));
}

/// Draw the bar: its heights first, which the compositor reads at map, then its words.
pub fn render(ui: &MindDeskApp, m: &Model) {
    ui.set_bar_height(BAR_HEIGHT as f32);
    ui.set_header_height(HEADER_HEIGHT as f32);
    ui.set_strip_height(STRIP_HEIGHT as f32);
    let t = chrome::top(m);
    ui.set_label(text(&t.label));
    ui.set_tone(t.tone);
    ui.set_headline(text(&t.headline));
    ui.set_can_stop(t.can_stop);
    ui.set_stop_note(text(&t.stop_note));
    ui.set_agents_offered(t.agents_offered);
    ui.set_strip_heading(text(&t.strip_heading));
    ui.set_strip_tool(text(&t.strip_tool));
    ui.set_strip_mark(text(&t.strip_mark));
    ui.set_strip_stats(text(&t.strip_stats));
    let f = chrome::footer(m);
    ui.set_apps(text(&f.apps));
    ui.set_safety(text(&f.safety));
    ui.set_model(text(&f.model));
    ui.set_model_mark(text(&f.model_mark));
}
