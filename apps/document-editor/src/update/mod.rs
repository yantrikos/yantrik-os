//! The only code that changes the model: every key, button, call from the surface, framework
//! event and job's result lands here as a [`Command`], and asks for its effects.
//!
//! After every command the document is kept in step with what yDoc calls unsaved (so the
//! framework's close flow, outside-change judgement and drafts agree with the window), and the
//! draft is brought up to date: a document with anything unsaved has its text in the lane.

use yantrik_app::effects::Effects;
use yantrik_app::Refused;

use crate::command::Command;
use crate::view::render::CHROME;
use crate::{Model, YDoc};

pub mod closing;
mod document;
pub mod drafts;
pub mod editing;
pub mod find;
mod formatting;
pub mod memory;
mod opening;
mod outside;
pub mod saving;
pub mod surface;
pub mod window;

pub fn update(m: &mut Model, command: Command, fx: &mut Effects<YDoc>) -> Result<(), Refused> {
    // A cue is for the render of the command that set it, and for no later one.
    m.cues.clear();
    if let Command::Surface { act, .. } = &command {
        // A call while a file is read or written is refused: its answer would be about a
        // document that is about to change. Refused before anything, so nothing changes. `show`
        // reads and changes nothing.
        if m.busy.is_some() && !matches!(act, crate::command::Act::Show) {
            return Err(Refused::new(
                "busy",
                format!(
                    "`{}` cannot run while yDoc is reading or writing a file; read `describe` again in a moment.",
                    act.name()
                ),
            ));
        }
    }
    dispatch(m, fx, command);
    // A close that waited for a file to be read or written goes on; then a launch's file waiting
    // for a dialog to be answered opens once it is.
    closing::resume(m, fx);
    opening::next(m, fx);
    keep_in_step(m);
    drafts::sync(m, fx);
    memory::watch(m, fx);
    fx.changed(CHROME);
    Ok(())
}

fn dispatch(m: &mut Model, fx: &mut Effects<YDoc>, command: Command) {
    match command {
        Command::Window(action) => window::act(m, fx, action),
        Command::Edit(text) => editing::typed(m, fx, text),
        Command::FindChanged(query) => {
            m.find.query = query;
            find::search(m, false);
        }
        Command::Format { what, anchor, cursor } => formatting::format(m, fx, what, anchor, cursor),
        Command::Link { url, anchor, cursor } => formatting::link(m, fx, &url, anchor, cursor),
        Command::Goto(offset) => {
            m.cues.push(crate::model::Cue::Select(offset, offset));
            m.cues.push(crate::model::Cue::FocusBody);
        }
        Command::Surface { act, caller } => surface::act(m, fx, act, caller),
        Command::Ask(id) => crate::rail::ask(m, fx, id),
        Command::Rail(command) => crate::rail::command(m, fx, command),
        Command::MemoryDue => memory::due(m, fx),
        Command::Remembered { query, rows } => memory::remembered(m, query, rows),
        Command::Started => drafts::started(m, fx),
        Command::Launch(path) => opening::launch(m, fx, path),
        Command::CloseRequested | Command::Quit => closing::window(m, fx),
        Command::LoggingOut => closing::logging_out(m, fx),
        Command::Presented => closing::presented(m),
        Command::DraftSaved(name) => drafts::saved(m, fx, &name),
        Command::DraftNotSaved { name, why } => drafts::not_saved(m, &name, &why),
        Command::FileChanged { key, seen } => outside::seen(m, fx, &key, seen),
        Command::Opened { then, result } => opening::landed(m, fx, then, result),
        Command::SaveLanded { seq, then, result } => saving::landed(m, fx, seq, then, result),
        Command::Exported { then, html, result } => saving::exported(m, fx, then, html, result),
        Command::Reread { from, result } => outside::reread(m, fx, from, result),
        Command::Located { from, found } => outside::located(m, fx, from, found),
        Command::LegacyArchived(result) => drafts::legacy_archived(m, result),
        #[cfg(test)]
        Command::UseDisk(disk) => m.disk = disk,
    }
}

/// The document is unsaved to the framework exactly when yDoc calls it unsaved: a document typed
/// back to its file's text is saved again, as it always was, and one that is unsaved with nothing
/// typed (restored from a draft, its file found gone) is unsaved to the framework too, so its
/// draft is kept and the close asks about it.
fn keep_in_step(m: &mut Model) {
    let modified = m.modified();
    let d = m.d_mut();
    if modified && !d.dirty() {
        d.edit(|_| ());
    } else if !modified && d.dirty() && !matches!(d.save, yantrik_app::documents::SaveState::Saving { .. }) {
        d.saved_seq = d.seq;
    }
}

/// Says `sentence` in the window's notice line, which is also `describe.notice`.
pub fn say(m: &mut Model, sentence: impl Into<String>) {
    m.notice.set(sentence);
}
