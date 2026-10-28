//! The window: its callbacks bound to commands, and the model drawn for the parts that changed.
//! The only code that writes the window's properties.
//!
//! The two callbacks that answer a key (`input`, `shortcut`) must say accept or reject at once, so
//! they decide from the key and a few facts about the window (`keys.rs`) and hand the rest to a
//! command. Find's bar, the assistant panel and the clipboard are the window's own and stay in
//! it; the model learns of what they ask for through the callbacks.

use slint::private_unstable_api::re_exports::{EventResult, KeyEvent};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use yantrik_app::app::Changed;
use yantrik_app::chrome;
use yantrik_app::cx::Cx;
use yantrik_app::for_mind::HIDDEN;

use crate::command::{Command, Do};
use crate::keys::{self, Pressed};
use crate::model::{
    title_of, Model, PART_DIALOG, PART_EXPLAIN, PART_FIND, PART_FOCUS, PART_FONT, PART_NOTICE, PART_RAISE,
    PART_SCREEN, PART_TABS,
};
use crate::{SessionTab, Terminal, TerminalApp, TerminalRun};

pub fn wire(cx: &Cx<Terminal>) {
    // The shells wake the app through the hub: it needs the core's door, which exists now.
    cx.read(|m| m.hub.clone()).connect(cx.post());
    let ui = cx.ui();
    let acting = cx.clone();
    ui.on_action(move |id| {
        if let Some(d) = Do::of(id.as_str()) {
            acting.run(Command::Do(d));
        }
    });
    ui.on_select_tab(cx.cmd1(Command::SelectTab));
    ui.on_close_tab(cx.cmd1(Command::CloseTab));
    ui.on_resized(cx.cmd2(Command::Resized));
    ui.on_scroll(cx.cmd1(Command::Scroll));
    ui.on_search(cx.cmd1(|q: SharedString| Command::Search(q.to_string())));
    ui.on_paste_text(cx.cmd1(|t: SharedString| Command::Paste(t.to_string())));
    let keyed = cx.clone();
    ui.on_input(move |event| key(&keyed, &event, true));
    let keyed = cx.clone();
    ui.on_shortcut(move |event| key(&keyed, &event, false));
}

/// A key: the Terminal's own shortcut, a scroll, or bytes for the shell. `typed` is the terminal
/// canvas asking; the shortcut callback answers only for shortcuts.
fn key(cx: &Cx<Terminal>, event: &KeyEvent, typed: bool) -> EventResult {
    let finding = cx.ui().get_show_search();
    let window = cx.read(|m| keys::Window {
        asking: m.asking(),
        finding,
        application: m.session().is_some_and(|s| s.application_cursor()),
        rows: m.rows,
        has_shell: m.session().is_some(),
    });
    let pressed = if typed { keys::pressed(event, &window) } else { keys::shortcut(event, &window).unwrap_or(Pressed::Ignored) };
    match pressed {
        Pressed::Ignored => return EventResult::Reject,
        Pressed::Command(stroke) => cx.run(stroke),
        Pressed::Find(open) => cx.ui().set_show_search(open),
        Pressed::CopyScreen => cx.ui().invoke_copy_screen(),
        Pressed::PasteClipboard => cx.ui().invoke_paste_clipboard(),
    }
    EventResult::Accept
}

pub fn render(ui: &TerminalApp, m: &Model, changed: &Changed) {
    if changed.has(PART_TABS) {
        tabs(ui, m);
    }
    if changed.has(PART_SCREEN) {
        screen(ui, m);
    }
    if changed.has(PART_FIND) {
        ui.set_match_count(m.matches.len() as i32);
        ui.set_match_index(if m.matches.is_empty() { 0 } else { m.match_index as i32 + 1 });
        ui.set_match_row(m.match_row);
        // The query is set only where it differs, so the caret stays where it is.
        if ui.get_query().as_str() != m.query {
            ui.set_query(m.query.as_str().into());
        }
    }
    if changed.has(PART_EXPLAIN) {
        ui.set_explaining(m.explain.working);
        ui.set_explanation(m.explain.text.as_str().into());
    }
    if changed.has(PART_NOTICE) && ui.get_notice().as_str() != m.notice {
        ui.set_notice(m.notice.as_str().into());
    }
    if changed.has(PART_DIALOG) {
        let (heading, said, label) = m.dialog();
        ui.set_close_heading(heading.into());
        ui.set_close_said(said.into());
        ui.set_close_label(label.into());
        // The dialog is drawn only while a question is asking behind it.
        ui.set_confirm_close(m.asking());
    }
    if changed.has(PART_FONT) {
        ui.set_font_size(m.font_size);
    }
    // Only once asked: the first render draws every part, and that is no request.
    if changed.has(PART_FOCUS) && m.focus > 0 {
        ui.invoke_focus_terminal();
    }
    if changed.has(PART_RAISE) && m.raises > 0 {
        chrome::bring_back(ui.window());
    }
}

fn tabs(ui: &TerminalApp, m: &Model) {
    let rows: Vec<SessionTab> = m
        .tabs
        .iter()
        .enumerate()
        .map(|(i, t)| SessionTab {
            title: if m.for_mind { HIDDEN.into() } else { title_of(&t.facts.cwd).into() },
            active: i == m.active,
            alive: t.session.alive(),
        })
        .collect();
    ui.set_tabs(ModelRc::new(VecModel::from(rows)));
}

/// The active shell's screen, its cursor and the status line under it.
fn screen(ui: &TerminalApp, m: &Model) {
    let Some(tab) = m.tab() else {
        ui.set_runs(ModelRc::default());
        ui.set_screen_text("".into());
        ui.set_alive(false);
        ui.set_status(if m.spawning > 0 { "Starting shell\u{2026}" } else { "No sessions" }.into());
        ui.set_cursor_visible(false);
        ui.set_directory("".into());
        return;
    };
    if m.for_mind {
        ui.set_runs(ModelRc::default());
        ui.set_screen_text(HIDDEN.into());
        ui.set_cursor_visible(false);
        ui.set_alive(tab.session.alive());
        ui.set_status(HIDDEN.into());
        ui.set_directory(HIDDEN.into());
        return;
    }
    let snap = tab.session.snapshot();
    ui.set_directory(tab.facts.cwd.to_string_lossy().into_owned().into());
    ui.set_screen_text(snap.text.into());
    ui.set_cursor_row(snap.cursor.0.into());
    ui.set_cursor_col(snap.cursor.1.into());
    ui.set_cursor_visible(snap.cursor_visible);
    ui.set_alive(snap.alive);
    ui.set_scrollback(snap.scrollback as i32);
    ui.set_status(
        if snap.alive {
            if snap.scrollback == 0 {
                "Live shell".to_string()
            } else {
                format!("History \u{b7} {} lines back", snap.scrollback)
            }
        } else {
            snap.exit
                .map(|n| format!("Shell exited \u{b7} status {n}"))
                .unwrap_or_else(|| "Shell ended".into())
        }
        .into(),
    );
    let rgb = |(r, g, b)| slint::Color::from_rgb_u8(r, g, b);
    ui.set_runs(ModelRc::new(VecModel::from(
        snap.runs
            .into_iter()
            .map(|r| TerminalRun {
                text: r.text.into(),
                row: r.row.into(),
                col: r.col.into(),
                columns: r.width.into(),
                fg: rgb(r.fg),
                bg: rgb(r.bg),
                bold: r.bold,
                underline: r.underline,
            })
            .collect::<Vec<_>>(),
    )));
}
