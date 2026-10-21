//! The only code that changes the model: every key, button, call from the surface, timer and
//! store job lands here as a [`Command`], and asks for its effects.
//!
//! The helpers below are the moments the window has always been brought up to date at: a note
//! shown, a note changed, the list redrawn, a store job started, a store job failed, and what
//! happens once a store job lands (the action that waited for a save, the close that waited).
//!
//! Every command acts in the focused window (pane.rs): one from a window focuses it first, and a
//! job's outcome is applied in the window it is about, then the focus goes back (windows.rs).

use std::time::Duration;

use yantrik_app::app::WindowId;
use yantrik_app::documents::{DocumentId, OutsideState};
use yantrik_app::effects::Effects;
use yantrik_app::timers::TimerId;
use yantrik_app::Refused;

use crate::command::{Command, Step};
use crate::model::{Model, Work};
use crate::note::now;
use crate::pane::Cue;
use crate::store::Vault;
use crate::view::listing;
use crate::view::preview;
use crate::view::render::{CUES, EDIT, LIBRARY, MODEL_ONLY, NOTE, OUTSIDE, PREVIEW, STATUS};
use crate::Notes;

pub mod closing;
mod drafts;
mod editing;
mod follow;
pub mod leaving;
mod library;
mod navigate;
mod outside;
mod prompts;
mod read_only;
pub mod recovery;
pub mod said;
pub mod saving;
mod surface;
mod window;
pub mod windows;

/// The autosave's debounce: this long after the last change, the open note is saved.
pub const AUTOSAVE: TimerId = TimerId("notes.autosave");
pub const AUTOSAVE_AFTER: Duration = Duration::from_millis(750);

/// Said when a store job ended without an answer (it panicked, or the pool refused it).
const WORKER_STOPPED: &str = "Storage worker stopped. Keep this window open to preserve your draft.";

pub fn update(m: &mut Model, command: Command, fx: &mut Effects<Notes>) -> Result<(), Refused> {
    // A cue is for the render of the command that set it, and for no later one.
    m.panes_mut().for_each(|p| p.cues.clear());
    let focused = m.pane.window;
    // A command from a window acts there; one from a window already closed does nothing.
    let Some(command) = windows::arrive(m, command) else { return Ok(()) };
    // A job's outcome, a file seen changed, or a close is applied in the window it is about, and
    // the focus goes back to the person's window after.
    let back = windows::applies_in(m, &command).map(|there| {
        let here = m.pane.window;
        m.focus(there);
        here
    });
    let refused = read_only::refused(m, fx, &command) || recovery::refused(m, fx, &command);
    if !refused {
        dispatch(m, fx, command);
        // Before the drafts are brought up to date: a draft is kept for a dirty document only.
        m.keep_unsaved_dirty();
        drafts::sync(m, fx);
        prompts::settle(m, fx);
    }
    if let Some(here) = back {
        m.focus(here);
    }
    // An offer whose note no window shows is on the focused window's bar: it moves with the person.
    if m.pane.window != focused && !m.recovery.offers.is_empty() {
        fx.changed(OUTSIDE);
    }
    if !refused {
        surface::finish(m, fx);
    }
    Ok(())
}

fn dispatch(m: &mut Model, fx: &mut Effects<Notes>, command: Command) {
    match command {
        // `arrive` has taken the window off every one, and focused it.
        Command::From { .. } => {}
        // The focus moved to the window it came from: `describe` says which.
        Command::Activated => fx.changed(MODEL_ONLY),
        Command::WindowNotOpened { window, why } => windows::not_opened(m, fx, window, &why),
        Command::Load => library::load(m, fx),
        Command::Window(action) => window::act(m, fx, action),
        Command::Edit(text) => editing::edit(m, fx, text),
        Command::Metadata { notebook, tags } => editing::metadata(m, fx, notebook, tags),
        Command::Filter { folder, query } => {
            m.pane.folder = folder;
            m.pane.query = query;
            relist(m, fx);
        }
        Command::ViewSynced { focus_mode, dialog } => {
            let asked_here = m.pane.dialog == closing::CLOSE_DIALOG;
            m.pane.focus_mode = focus_mode;
            m.pane.dialog = dialog;
            if dialog != outside::COMPARE_DIALOG {
                m.pane.theirs = None;
            }
            // Escape on the close prompt is its Cancel, in the window that shows the prompt.
            if asked_here && dialog != closing::CLOSE_DIALOG {
                if leaving::asks_in(m, m.pane.window) {
                    leaving::put_away(m, fx);
                }
                closing::put_away(m, fx);
            }
            fx.changed(MODEL_ONLY);
        }
        Command::Surface(act) => surface::act(m, fx, act),
        Command::Autosave => saving::autosave(m, fx),
        Command::CloseRequested(window) => closing::requested(m, fx, window),
        Command::Quit => closing::quit(m, fx),
        Command::LoggingOut => closing::logging_out(m, fx),
        Command::DraftSaved(name) => drafts::saved(m, fx, &name),
        Command::DraftNotSaved { name, why } => drafts::not_saved(m, fx, &name, &why),
        Command::FileChanged { key, seen } => outside::seen(m, fx, &key, seen),
        Command::Loaded(result) => library::loaded(m, fx, result),
        Command::SaveLanded { doc, seq, result } => saving::landed(m, fx, doc, seq, result),
        Command::Trashed { id, result } => library::trashed(m, fx, &id, result),
        Command::Restored(result) => library::opened(m, fx, result, library::Opened::Restored),
        Command::Imported(result) => library::opened(m, fx, result, library::Opened::Imported),
        Command::Exported(result) => library::exported(m, fx, result),
        Command::Resolved { doc, seq, resolution, result } => outside::resolved(m, fx, doc, seq, resolution, result),
        Command::Compared { doc, result } => outside::compared(m, fx, doc, result),
        Command::Reread { doc, result } => follow::reread(m, fx, doc, result),
        #[cfg(test)]
        Command::UseDisk(disk) => {
            m.vault = Vault::new(m.vault.dir().to_path_buf(), disk);
            fx.changed(MODEL_ONLY);
        }
    }
}

/// The footer's words for the open note's save state.
fn save_state(m: &Model) -> &'static str {
    match m.current() {
        Some(c) if c.trash() => "Recoverable deletion",
        Some(c) if *c.outside() == OutsideState::Deleted => said::FILE_GONE,
        // No autosave is pending while the person's choice is: saving is refused until then.
        Some(c) if c.unsaved() && c.doc.blocked() => said::WAITING_ON_CHOICE,
        Some(c) if c.unsaved() => "Unsaved · autosave pending",
        // The window shows text that is on no disk but in its recovery copy.
        Some(c) if m.recovery.offer_for(c.id()).is_some() => said::WAITING_ON_RECOVERY,
        _ => "All changes saved",
    }
}

/// The footer's line, in the focused window and in every other window showing the same note: one
/// note has one save state, whichever window it is read in.
fn set_status(m: &mut Model, status: &str) {
    for window in m.others_showing() {
        if let Some(pane) = m.pane_of_mut(window) {
            pane.status = status.into();
        }
    }
    m.pane.status = status.into();
}

/// The open note changed: its title, count and save state are said again, in every window that
/// shows it, and the autosave's debounce starts over.
fn refreshed(m: &mut Model, fx: &mut Effects<Notes>) {
    if m.current().is_none() {
        return;
    }
    set_status(m, save_state(m));
    fx.changed(EDIT);
    fx.changed(STATUS);
    windows::follow(m, fx, false);
    fx.after(AUTOSAVE, AUTOSAVE_AFTER, Command::Autosave);
}

/// The pane shows the open note afresh: its undo history starts empty, its fields are the note's,
/// the list is redrawn, and the view scrolls to the top and takes the keys.
fn show(m: &mut Model, fx: &mut Effects<Notes>) {
    m.pane.history.clear();
    let (notebook, tags) = m.shown().map(|c| (c.field("notebook"), c.field("tags"))).unwrap_or_default();
    set_status(m, save_state(m));
    m.pane.notebook = notebook;
    m.pane.tags = tags;
    fx.changed(NOTE);
    fx.changed(STATUS);
    relist(m, fx);
    if m.pane.preview {
        reblock(m, fx);
    }
    m.pane.cues.push(Cue::Shown);
    fx.changed(CUES);
}

/// The library list, worked out again in every window for its section and search: a note saved,
/// made or moved in one window is listed so in the others too.
fn relist(m: &mut Model, fx: &mut Effects<Notes>) {
    let at = now();
    let listings: Vec<(WindowId, listing::Listing)> = m
        .panes()
        .map(|p| (p.window, listing::list(&m.library, m.shown_note_in(p).as_ref(), &p.folder, &p.query, at)))
        .collect();
    for (pane, (_, listing)) in m.panes_mut().zip(listings) {
        pane.listing = listing;
    }
    fx.changed(LIBRARY);
}

/// The preview's blocks, from the open note as it is now. Nothing open leaves them as they were.
fn reblock(m: &mut Model, fx: &mut Effects<Notes>) {
    if let Some(shown) = m.shown() {
        m.pane.blocks = preview::blocks(&shown.text);
        fx.changed(PREVIEW);
    }
}

/// Starts the store's one job. `lost` is applied if the job ends without an answer.
fn begin(
    m: &mut Model,
    fx: &mut Effects<Notes>,
    work: Work,
    lost: Command,
    job: impl FnOnce(&Vault) -> Command + Send + 'static,
) {
    m.busy = Some(work);
    m.job_window = m.pane.window;
    set_status(m, "Working…");
    fx.changed(STATUS);
    let vault = m.vault.clone();
    fx.spawn_or(lost, move |_| job(&vault));
}

/// A store job failed: said in the notice, the draft kept, and nothing waits on it any more. A
/// call from the surface that waited for this save to go on is refused. `doc` is the note whose
/// save failed, `None` for any other job: a note the person's window does not show is named, and
/// a close waiting on other notes' saves goes on with them.
fn failed(m: &mut Model, fx: &mut Effects<Notes>, doc: Option<DocumentId>, sentence: String) {
    if let Some(pending) = m.pending.take() {
        if pending.refuse_if_unsaved {
            if let Some(asked) = m.asked.as_mut() {
                asked.refused = Some(if sentence.is_empty() {
                    "The open note has unsaved changes that could not be written; nothing else was done."
                        .to_string()
                } else {
                    format!("The open note could not be saved, so nothing else was done: {sentence}")
                });
            }
        }
    }
    let sentence = closing::failed(m, doc, &sentence).unwrap_or_else(|| named(m, doc, sentence));
    m.notice = sentence;
    m.lasting_notice = false;
    // Held back by an outside change, the note waits on the person's choice, which the footer
    // names in two words (the bar says it in full); any other failure needs the disk seen to.
    let held = doc.and_then(|d| m.documents.get(d)).is_some_and(|d| d.blocked());
    let status = if held && doc == m.current_id() { save_state(m) } else { "Needs attention · draft kept" };
    set_status(m, status);
    fx.changed(STATUS);
    if m.busy.is_none() {
        closing::progress(m, fx);
    }
}

/// `sentence`, with the title of the note it is about when the person's window does not show it:
/// a failure in another window is never read as one of the note in front of them.
fn named(m: &Model, doc: Option<DocumentId>, sentence: String) -> String {
    match doc.filter(|d| !m.in_view(*d)).and_then(|d| m.note_of(d)) {
        Some(note) => said::note_failed(&note.title(), &sentence),
        None => sentence,
    }
}

/// A store job landed: once the open note is saved, what waited for it goes ahead (the action,
/// then the close); while it is not, it is saved again.
fn after_store(m: &mut Model, fx: &mut Effects<Notes>) {
    // A follow-up save is in flight; its landing decides.
    if m.busy.is_some() {
        return;
    }
    // The action runs in the window it was asked in, once that window's note is saved.
    if let Some(window) = m.pending.as_ref().map(|p| p.window) {
        let ran = m.within(window, |m| {
            if !m.unsaved() {
                if let Some(pending) = m.pending.take() {
                    run(m, fx, pending.step);
                }
            } else {
                saving::save(m, fx, saving::How::Asked);
            }
        });
        // Its window closed meanwhile: there is nothing left for it to act on.
        if ran.is_none() {
            m.pending = None;
        }
    }
    closing::progress(m, fx);
    // A change seen while the store was busy is followed now that it is not, in its window.
    for window in m.windows() {
        let owed = m.pane_of(window).is_some_and(|p| p.reread_owed);
        if owed && m.busy.is_none() {
            m.within(window, |m| {
                m.pane.reread_owed = false;
                follow::follow(m, fx);
            });
        }
    }
}

/// What waited for the open note to be saved.
fn run(m: &mut Model, fx: &mut Effects<Notes>, step: Step) {
    match step {
        Step::Window(action) => window::act(m, fx, action),
        // Only the surface asks for a note with a body; a note that cannot be made refuses the
        // call with the reason (the notice says it too), never answers with another note.
        Step::NewNote { title, body } => {
            if let Err(sentence) = navigate::new_note(m, fx, &title, &body) {
                if let Some(asked) = m.asked.as_mut() {
                    asked.refused = Some(sentence);
                }
            }
        }
        Step::Import { path } => window::confirm(m, fx, window::Dialog::Import, &path),
        Step::Export { path } => window::confirm(m, fx, window::Dialog::Export, &path),
    }
}
