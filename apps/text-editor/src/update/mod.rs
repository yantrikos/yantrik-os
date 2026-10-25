//! The only code that changes the model: every key, button, call from the surface, framework
//! event and job's result lands here as a [`Command`], and asks for its effects.
//!
//! After every command the tabs' documents are kept in step with what the editor calls unsaved
//! (so the framework's close flow, outside-change judgement and drafts agree with the window),
//! and the drafts are brought up to date: a tab with anything unsaved has its text in the lane.

use yantrik_app::effects::Effects;
use yantrik_app::Refused;

use crate::command::Command;
use crate::view::render::{CARET, CHROME, TEXT};
use crate::model::{Finish, Model};
use crate::TextEditor;

pub mod closing;
pub mod drafts;
mod editing;
pub mod find;
mod opening;
mod outside;
pub mod saving;
pub mod surface;
pub mod tabs;
pub mod window;

pub fn update(m: &mut Model, command: Command, fx: &mut Effects<TextEditor>) -> Result<(), Refused> {
    // A cue is for the render of the command that set it, and for no later one.
    m.cues.clear();
    let active = m.active_tab().id;
    if let Command::Surface { act, .. } = &command {
        // A call while a file is read or written is refused, as it always was: its answer would
        // be about a tab that is about to change. Refused before anything, so nothing changes.
        if m.busy.is_some() {
            return Err(Refused::new(
                "busy",
                format!(
                    "`{}` cannot run while the editor is reading or writing a file; read `describe` \
                     again in a moment.",
                    act.name()
                ),
            ));
        }
    }
    // The caret moving changes the footer's line and column and nothing else.
    let caret_only = matches!(command, Command::Cursor(_));
    dispatch(m, fx, command);
    // A close that waited for a file to be read or written goes on; then a launch's file waiting
    // for a dialog to be answered opens once it is.
    closing::resume(m, fx);
    opening::next(m, fx);
    keep_in_step(m);
    drafts::sync(m, fx);
    if m.active_tab().id != active {
        fx.changed(TEXT);
    }
    fx.changed(if caret_only { CARET } else { CHROME });
    finish(m, fx);
    Ok(())
}

/// The window's close, or the editor's end, once every answer owed has settled: closing the last
/// window ends the process, and its shutdown refuses whatever is still owed, so a call whose own
/// work led here (the prompt's `save` or `discard`, a save that ended a linger) is answered first.
fn finish(m: &mut Model, fx: &mut Effects<TextEditor>) {
    if m.answers_pending > 0 {
        return;
    }
    match m.finish.take() {
        // Something was changed while the close waited on its answers (a call arriving then):
        // asked about as any unsaved tab is, rather than closed away.
        Some(Finish::Close) if m.tabs.iter().any(|t| m.modified(t)) => closing::ask_next(m, fx),
        Some(Finish::Close) => fx.close(yantrik_app::app::WindowId::PRIMARY),
        Some(Finish::Exit) => fx.exit(),
        None => {}
    }
}

fn dispatch(m: &mut Model, fx: &mut Effects<TextEditor>, command: Command) {
    match command {
        Command::Window(action) => window::act(m, fx, action),
        Command::Edit(text) => editing::typed(m, fx, text),
        Command::Cursor(offset) => editing::cursor(m, offset),
        Command::Find { query, match_case } => {
            m.find.query = query;
            m.find.match_case = match_case;
            find::search(m, true);
        }
        Command::SelectTab(id) => tabs::select_from_window(m, fx, id),
        Command::CloseTab(id) => closing::close_tab_from_window(m, fx, id),
        Command::Reflow { font_px, limit_px } => {
            m.layout.font_px = font_px;
            m.layout.limit_px = limit_px;
            // The window may have shrunk under a viewport scrolled deep into it.
            m.cues.push(crate::model::Cue::ResetPosition);
            fx.changed(TEXT);
        }
        Command::Surface { act, caller } => surface::act(m, fx, act, caller),
        Command::Started => drafts::started(m, fx),
        Command::Launch(files) => opening::launch(m, fx, files),
        Command::CloseRequested | Command::Quit => closing::window(m, fx),
        Command::LoggingOut => closing::logging_out(m, fx),
        Command::Presented => closing::presented(m),
        Command::DraftSaved(name) => drafts::saved(m, fx, &name),
        Command::DraftNotSaved { name, why } => drafts::not_saved(m, &name, &why),
        Command::FileChanged { key, seen } => outside::seen(m, fx, &key, seen),
        Command::Opened { then, result } => opening::landed(m, fx, then, result),
        Command::SaveLanded { tab, seq, then, result } => saving::landed(m, fx, tab, seq, then, result),
        Command::Reread { tab, seq, result } => outside::reread(m, fx, tab, seq, result),
        Command::Located { tab, from, found } => outside::located(m, fx, tab, from, found),
        Command::Answered { value, failed, agent } => surface::answered(m, fx, value, failed, agent),
        Command::LegacyArchived(result) => drafts::legacy_archived(m, result),
        #[cfg(test)]
        Command::UseDisk(disk) => m.disk = disk,
    }
}

/// Every tab's document is unsaved to the framework exactly when the editor calls it unsaved: a
/// tab typed back to its file's text is saved again, as it always was, and one that is unsaved
/// with nothing typed (restored from a draft, its file found gone) is unsaved to the framework
/// too, so its draft is kept and the close asks about it.
fn keep_in_step(m: &mut Model) {
    let tabs: Vec<(yantrik_app::documents::DocumentId, bool)> =
        m.tabs.iter().map(|t| (t.doc, m.modified(t))).collect();
    for (doc, modified) in tabs {
        let Some(d) = m.documents.get_mut(doc) else { continue };
        if modified && !d.dirty() {
            d.edit(|_| ());
        } else if !modified && d.dirty() && !matches!(d.save, yantrik_app::documents::SaveState::Saving { .. }) {
            d.saved_seq = d.seq;
        }
    }
}

/// Says `sentence` in the window's notice line, which is also `describe.notice`.
pub fn say(m: &mut Model, sentence: impl Into<String>) {
    m.notice.set(sentence);
}
