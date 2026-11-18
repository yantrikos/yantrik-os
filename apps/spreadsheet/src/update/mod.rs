//! The only code that changes the model: every key, button, call from the surface, framework
//! event and job's result lands here as a [`Command`], and asks for its effects.
//!
//! After every command the document is kept in step with what ySheets calls unsaved (so the
//! framework's outside-change judgement and drafts agree with the window), the sheet is worked
//! out again in a job if a cell was typed into, and the draft is brought up to date.

use yantrik_app::effects::Effects;
use yantrik_app::for_mind;
use yantrik_app::Refused;

use crate::command::{Act, Command};
use crate::mind_view;
use crate::view::render::{CHROME, GRID};
use crate::{Model, YSheets};

mod cells;
pub mod closing;
pub mod drafts;
mod opening;
mod outside;
mod picking;
pub mod recalc;
pub mod saving;
mod solved;
pub mod surface;
mod window;

#[cfg(test)]
pub use outside::conflict as conflict_sentence;

pub fn update(m: &mut Model, command: Command, fx: &mut Effects<YSheets>) -> Result<(), Refused> {
    // A cue is for the render of the command that set it, and for no later one.
    m.cues.clear();
    m.redraw = crate::model::Redraw::Nothing;
    *m.prebuilt.get_mut() = None;
    // A window drawn for a Mind takes nothing that would change the sheet, or read or write a file
    // as the person (mind_view.rs).
    for_mind::refuse_if(m.for_mind, m.for_mind && mind_view::refuses(m, &command))?;
    if let Command::Surface { act, .. } = &command {
        // A call while a file is read or written is refused: its answer would be about a sheet
        // that is about to change. Refused before anything, so nothing changes.
        if m.busy.is_some() {
            return Err(busy(act));
        }
    }
    dispatch(m, fx, command);
    // A close that waited for a file to be read goes on; then a launch's file waiting for ySheets
    // to be free opens once it is.
    closing::resume(m, fx);
    opening::next(m, fx);
    keep_in_step(m);
    recalc::ensure(m, fx);
    drafts::sync(m, fx);
    fx.changed(CHROME);
    if m.redraw != crate::model::Redraw::Nothing {
        fx.changed(GRID);
    }
    Ok(())
}

fn busy(act: &Act) -> Refused {
    Refused::new(
        "busy",
        format!("`{}` cannot run while ySheets is reading or writing a file; read `describe` again in a moment.", act.name()),
    )
}

fn dispatch(m: &mut Model, fx: &mut Effects<YSheets>, command: Command) {
    match command {
        Command::Cell { row, col, typed } => window::clicked(m, fx, row, col, &typed),
        Command::Formula { typed, direction } => window::submitted(m, fx, &typed, &direction),
        Command::Cancelled => m.cues.push(crate::model::Cue::LoadField),
        Command::Header { id, typed } => window::header(m, fx, &id, &typed),
        Command::Confirm { path } => window::confirm(m, fx, path),
        Command::Cancel => window::cancel(m),
        Command::Discard => window::discard(m, fx),
        Command::Masked => window::masked(m),
        Command::Surface { act, caller } => surface::act(m, fx, act, caller),
        Command::Started => drafts::started(m, fx),
        Command::Launch(path) => opening::launch(m, fx, path),
        Command::CloseRequested | Command::Quit => closing::requested(m, fx),
        Command::LoggingOut => closing::logging_out(m, fx),
        Command::Presented => closing::presented(m),
        Command::DraftSaved(name) => drafts::saved(m, &name),
        Command::DraftNotSaved { name, why } => drafts::not_saved(m, &name, &why),
        Command::DraftDue => drafts::due(m, fx),
        Command::DraftEncoded { name, seq, bytes } => drafts::encoded(m, fx, name, seq, bytes),
        Command::FileChanged { key, seen } => outside::seen(m, fx, &key, seen),
        Command::Opened { then, result } => opening::landed(m, fx, then, result),
        Command::Picked { saving, choice, suggested } => picking::picked(m, fx, saving, choice, suggested),
        Command::SaveLanded { write, then, result } => saving::landed(m, fx, *write, then, result),
        Command::Solved { solved, changed, then } => solved::landed(m, fx, *solved, changed, then),
        Command::SolveLost { rev, then } => solved::lost(m, fx, rev, then),
        Command::Reread { from, result } => outside::reread(m, fx, from, result),
        Command::Located { from, found } => outside::located(m, fx, from, found),
        #[cfg(test)]
        Command::UseDisk(disk) => m.disk = disk,
        #[cfg(test)]
        Command::UseAsk(ask) => m.ask = ask,
    }
}

/// The document is unsaved to the framework exactly when ySheets calls it unsaved: a sheet whose
/// content changed is a new edit of it (so its draft is owed and a save of the old one does not
/// cover it), a sheet typed back to its file's content is saved again, and one that is unsaved
/// with nothing typed (restored from a draft, its file found gone) is unsaved to the framework
/// too, so its draft is kept.
pub(super) fn keep_in_step(m: &mut Model) {
    let modified = m.modified();
    let revision = m.book().revision();
    let changed = std::mem::replace(&mut m.doc.seen_revision, revision) != revision;
    let d = m.d_mut();
    if changed || (modified && !d.dirty()) {
        d.edit(|_| ());
    }
    if !modified && d.dirty() && !matches!(d.save, yantrik_app::documents::SaveState::Saving { .. }) {
        d.saved_seq = d.seq;
    }
}

/// Says `sentence` in the window's notice line, which is also `describe.notice`.
pub fn say(m: &mut Model, sentence: impl Into<String>) {
    m.notice.set(sentence);
}
