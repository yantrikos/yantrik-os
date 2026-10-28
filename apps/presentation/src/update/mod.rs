//! The only code that changes the model: every key, button, call from the surface, framework
//! event and job's result lands here as a [`Command`], and asks for its effects.
//!
//! After every command the document is kept in step with what yPresent calls unsaved (so the
//! framework's outside-change judgement and drafts agree with the window), and the draft is
//! brought up to date: a deck with anything unsaved has its content in the lane.

use yantrik_app::effects::Effects;
use yantrik_app::for_mind;
use yantrik_app::Refused;

use crate::command::{Act, Command};
use crate::deck::Edits;
use crate::mind_view;
use crate::view::render::{CANVAS, CHROME, CLOCK};
use crate::{Model, YPresent};

pub mod clock;
pub mod closing;
mod drafts;
mod notes;
mod opening;
mod outside;
pub mod saving;
mod search;
mod slides;
pub mod surface;
mod window;

pub fn update(m: &mut Model, command: Command, fx: &mut Effects<YPresent>) -> Result<(), Refused> {
    // A cue is for the render of the command that set it, and for no later one.
    m.cues.clear();
    m.reload = false;
    // A window drawn for a Mind takes nothing that would change the deck, or read or write a file
    // as the person (mind_view.rs).
    for_mind::refuse_if(m.for_mind, mind_view::refuses(&command))?;
    if let Command::Tick = command {
        // The clock alone: nothing else changed, and a deck at rest draws one line a second.
        m.clock.seconds += 1;
        fx.changed(CLOCK);
        return Ok(());
    }
    if let Command::Surface { act, .. } = &command {
        // A call while a file is read or written is refused: its answer would be about a deck
        // that is about to change. Refused before anything, so nothing changes. `show` reads and
        // changes nothing.
        if m.busy.is_some() && !matches!(act, Act::Show) {
            return Err(Refused::new(
                "busy",
                format!(
                    "`{}` cannot run while yPresent is reading or writing a file; read `describe` again in a moment.",
                    act.name()
                ),
            ));
        }
    }
    dispatch(m, fx, command);
    // A close that waited for a file to be read or written goes on; then a launch's file
    // waiting for yPresent to be free opens once it is.
    closing::resume(m, fx);
    opening::next(m, fx);
    keep_in_step(m);
    drafts::sync(m, fx);
    fx.changed(CHROME);
    if m.reload {
        fx.changed(CANVAS);
    }
    Ok(())
}

fn dispatch(m: &mut Model, fx: &mut Effects<YPresent>, command: Command) {
    match command {
        Command::Window { action, canvas } => window::act(m, fx, action, canvas),
        Command::Canvas(canvas) => slides::typed(m, &canvas),
        Command::Surface { act, caller } => surface::act(m, fx, act, caller),
        Command::Started => drafts::started(m, fx),
        Command::Launch(path) => opening::launch(m, fx, path),
        Command::CloseRequested | Command::Quit => closing::window(m, fx),
        Command::LoggingOut => closing::logging_out(m, fx),
        Command::Presented => closing::presented(m),
        Command::Tick => {}
        Command::DraftSaved(name) => drafts::saved(m, fx, &name),
        Command::DraftNotSaved { name, why } => drafts::not_saved(m, &name, &why),
        Command::FileChanged { key, seen } => outside::seen(m, fx, &key, seen),
        Command::Opened { then, result } => opening::landed(m, fx, then, result),
        Command::SaveLanded { seq, then, written, result } => saving::landed(m, fx, seq, then, *written, result),
        Command::Exported { then, slides, remember, result } => saving::exported(m, fx, then, slides, remember, result),
        Command::Reread { from, result } => outside::reread(m, fx, from, result),
        Command::Located { from, found } => outside::located(m, fx, from, found),
        Command::LegacyArchived(result) => drafts::legacy_archived(m, result),
        Command::NotesWritten { ask, result } => notes::written(m, ask, result),
        #[cfg(test)]
        Command::UseDisk(disk) => m.disk = disk,
        #[cfg(test)]
        Command::UseDeckDir(dir) => m.deck_dir = dir,
    }
}

/// The document is unsaved to the framework exactly when yPresent calls it unsaved: a deck whose
/// content changed is a new edit of it (so its draft is owed and a save of the old one does not
/// cover it), a deck typed back to its file's content is saved again, and one that is unsaved with
/// nothing typed (restored from a draft, its file found gone) is unsaved to the framework too, so
/// its draft is kept.
fn keep_in_step(m: &mut Model) {
    let modified = m.modified();
    let revision = m.deck().revision();
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

/// The canvas as the model should take it: what the window held, or, in a window drawn for a
/// Mind, the slide's own text, since what the window holds there is the mask and not the slide.
pub fn canvas_for(m: &Model, held: Edits) -> Edits {
    if m.for_mind {
        Edits::from(m.deck().current_slide())
    } else {
        held
    }
}
