//! More than one window (design/notes-gate-2026-10-21.md §2, "Views and windows"; §3, gate row
//! "Two document contexts").
//!
//! - *Open in new window* (the note header's button, Ctrl+Shift+N, `open_in_new_window`) shows a
//!   note in a window of its own. A note two windows show is one document: what is typed in one is
//!   drawn in the other, and they share its unsaved state, its conflict and its draft. Each window
//!   keeps its own view of it: caret, scroll, search, dialog and undo history.
//! - A command from a window acts there, and that window is the one the person is in
//!   ([`arrive`]). A store job's outcome, a file seen changed, or a window's close asked for from
//!   the taskbar or the window manager is applied in the window it is about ([`applies_in`]), and
//!   the focus goes back to the person's window after: a window closed in the background never
//!   moves the person's focus.
//! - A window closing takes only its view with it ([`forget`]): a note no other window shows is
//!   closed then, and its file no longer watched.

use yantrik_app::app::WindowId;
use yantrik_app::documents::DocumentId;
use yantrik_app::effects::Effects;

use super::navigate::show_note;
use super::{said, show};
use crate::command::Command;
use crate::model::Model;
use crate::note::Note;
use crate::pane::Pane;
use crate::view::preview;
use crate::view::render::{FIELDS, PREVIEW, STATUS};
use crate::Notes;

/// A command as it arrives: one from a window focuses that window, the one the person is in now.
/// `None` for a command from a window already closed.
pub fn arrive(m: &mut Model, command: Command) -> Option<Command> {
    match command {
        Command::From { window, command } => {
            if m.enter(window) {
                arrive(m, *command)
            } else {
                None
            }
        }
        other => Some(other),
    }
}

/// The window a store job's outcome, a file seen changed, or a close is applied in: for a note's
/// job, the window that started it while it still shows the note, else the first that shows it
/// (the focused one first); for a library job, the window that started it; for a close, the
/// window asked to close, where its prompt shows (the prompt's buttons come from that window, so
/// they act there too). `None` for any other command, which acts in the focused window.
pub fn applies_in(m: &Model, command: &Command) -> Option<WindowId> {
    let (doc, started_in) = match command {
        Command::CloseRequested(window) => return Some(*window),
        Command::SaveLanded { doc, .. }
        | Command::Resolved { doc, .. }
        | Command::Compared { doc, .. }
        | Command::Reread { doc, .. } => (*doc, Some(m.job_window)),
        Command::FileChanged { key, .. } => (m.documents.by_key(key)?, None),
        Command::Loaded(_)
        | Command::Trashed { .. }
        | Command::Restored(_)
        | Command::Imported(_)
        | Command::Exported(_) => return m.pane_of(m.job_window).map(|p| p.window),
        _ => return None,
    };
    let showing = m.windows_showing(doc);
    started_in.filter(|w| showing.contains(w)).or_else(|| showing.first().copied())
}

/// *Open in new window*: `note`, or the focused window's own note when `None`, in a window of its
/// own, which is then the focused one: the window manager gives a new window the keys. A note
/// already open elsewhere is the same document, unsaved edits and all. Returns the new window,
/// or why none was opened.
pub fn open(m: &mut Model, fx: &mut Effects<Notes>, note: Option<Note>) -> Result<WindowId, &'static str> {
    let Some(note) = note.or_else(|| m.current().map(|c| c.as_note())) else {
        return Err(said::NOTHING_FOR_A_WINDOW);
    };
    let window = fx.open_window();
    m.add_pane(Pane::new(window));
    show_note(m, fx, Some(note));
    show(m, fx);
    Ok(window)
}

/// A window `open` asked for could not be made: its pane goes, with the view it held, and the
/// notice says so. The note it was to show stays open where it was.
pub fn not_opened(m: &mut Model, fx: &mut Effects<Notes>, window: WindowId, why: &str) {
    m.within(window, |m| show_note(m, fx, None));
    m.remove_pane(window);
    m.notice = said::window_not_opened(why);
    m.lasting_notice = false;
    fx.changed(STATUS);
}

/// A window closed: its view goes with it, and a note no other window shows is closed (its file
/// no longer watched; its draft kept while it has edits not on disk). The last window's pane is
/// left as it is: the app ends with it.
pub fn forget(m: &mut Model, fx: &mut Effects<Notes>, window: WindowId) {
    if m.windows().len() < 2 {
        return;
    }
    m.within(window, |m| show_note(m, fx, None));
    m.remove_pane(window);
}

/// The other windows showing `doc` follow the focused window to `note`, or to nothing: the note's
/// text moved to another note (*Keep mine as a copy*, *Save as new*, a restore from the Trash), or
/// the note left the library (the Trash). No window goes on showing a document whose text has
/// moved on without it.
pub fn follow_to(m: &mut Model, fx: &mut Effects<Notes>, doc: DocumentId, note: Option<&Note>) {
    let here = m.pane.window;
    for window in m.windows_showing(doc).into_iter().filter(|w| *w != here) {
        m.within(window, |m| {
            show_note(m, fx, note.cloned());
            show(m, fx);
        });
    }
}

/// The other windows showing the focused window's note draw what changed in it: its fields and
/// its preview (its text, title and footer are drawn from the note itself). `afresh`: its text was
/// replaced by the file's (*Take theirs*, a note read again), so every view's undo history starts
/// again, as the design asks; a view's own history never reaches across that.
pub fn follow(m: &mut Model, fx: &mut Effects<Notes>, afresh: bool) {
    let others = m.others_showing();
    let Some(shown) = m.shown().cloned() else { return };
    for window in &others {
        let Some(pane) = m.parked.get_mut(window) else { continue };
        pane.notebook = shown.field("notebook");
        pane.tags = shown.field("tags");
        if pane.preview {
            pane.blocks = preview::blocks(&shown.text);
            fx.changed(PREVIEW);
        }
        if afresh {
            pane.history.clear();
        }
    }
    if !others.is_empty() {
        fx.changed(FIELDS);
    }
}
