//! Close as a document flow (design §2, "Close is a document flow"; §3 "Close cancelled"),
//! through the framework's [`CloseFlow`]. Closing a window, or quitting, asks about the notes
//! with edits not on disk, once: *Save*, *Don't save* or *Cancel*. Nothing unsaved closes at once.
//! A window asks only about a note no other window shows: closing one of two windows on a note
//! asks nothing, and the note stays open, edits and all, in the other. Quitting asks once over
//! every note with edits not on disk, in every window.
//!
//! - *Cancel* keeps everything as it was and saves nothing.
//! - *Save* saves the latest edits, retrying a save that failed before, and closes only once they
//!   are on disk: every note asked about, one after another, as the store does one thing at a
//!   time. While it waits, editing goes on and the save that closes is of the latest edit. A save
//!   that fails, or cannot start because an outside change waits on the person's choice, keeps
//!   everything open and says why, and which note. Another note's save failing meanwhile (its
//!   autosave, in another window) is said, and the close goes on with the notes it asked about.
//! - *Don't save* drops the edits and their draft, and closes.
//!
//! A second close while one waits asks nothing and saves nothing more; another window's close
//! that would ask says, in that window only, to answer the first or to wait for its save. A
//! library open read-only cannot save, so the prompt offers only *Don't save* and *Cancel*, and
//! says Export keeps a copy.
//!
//! The autosave goes on under the prompt. The prompt names only the notes still not on disk, and
//! once none is left, it answers itself and the close goes ahead: the person asked to close, and
//! the prompt was only there because of text that is now saved.

use yantrik_app::app::WindowId;
use yantrik_app::documents::close::Stage;
use yantrik_app::documents::{Choice, CloseFlow, Closing, DocumentId, OutsideState};
use yantrik_app::effects::Effects;

use super::navigate::show_note;
use super::saving::{self, How};
use super::{drafts, leaving, outside, prompts, show, windows};
use crate::command::Resolution;
use crate::model::Model;
use crate::update::said;
use crate::view::render::{CHROME, STATUS};
use crate::Notes;

/// The dialog number of the close prompt.
pub const CLOSE_DIALOG: i32 = 4;

/// The close prompt's words: its heading, its sentence, whether it offers its save button (there
/// is something to save, in a library that may be written), and that button's label.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prompt {
    pub heading: String,
    pub sentence: String,
    pub saves: bool,
    pub save: &'static str,
}

/// The person asked a window to close.
pub fn requested(m: &mut Model, fx: &mut Effects<Notes>, window: WindowId) {
    if leaving::asks_in(m, window) {
        leaving::put_away(m, fx);
    }
    m.keep_unsaved_dirty();
    let closing = m.documents.closing_window(window);
    ask(m, fx, Some(window), closing);
}

/// The app was asked to end: every note with edits not on disk, asked about once.
pub fn quit(m: &mut Model, fx: &mut Effects<Notes>) {
    if leaving::asks_in(m, m.pane.window) {
        leaving::put_away(m, fx);
    }
    m.keep_unsaved_dirty();
    let closing = m.documents.quitting();
    ask(m, fx, None, closing);
}

/// The session or the system is ending Notes (SIGTERM, SIGHUP): there is no one to ask. Every open
/// note's draft is written now rather than after its debounce, so the next start offers back
/// whatever was not saved, and Notes ends in this one command, without waiting on the store.
pub fn logging_out(m: &mut Model, fx: &mut Effects<Notes>) {
    m.keep_unsaved_dirty();
    let open: Vec<DocumentId> = m.documents.iter().map(|d| d.id).collect();
    for id in open {
        if let (Some(d), Some(file)) = (m.documents.get_mut(id), m.files.get(&id)) {
            if let Some((name, bytes)) = drafts::owed(d, file) {
                fx.checkpoint_draft(&name, bytes);
                d.draft_asked();
            }
        }
    }
    fx.exit();
}

fn ask(m: &mut Model, fx: &mut Effects<Notes>, window: Option<WindowId>, closing: Closing) {
    let asked: Vec<DocumentId> = match closing {
        Closing::Now => Vec::new(),
        Closing::Ask(documents) => documents.into_iter().filter(|d| m.unsaved_doc(*d)).collect(),
    };
    let nothing_to_ask = asked.is_empty() && unstored_at_close(m, window) == 0;
    // The same close again needs no words: its prompt, or its save, is already under way. A save
    // that failed ends the flow (`failed`), so the person who asks again is asked again.
    if m.closing.as_ref().is_some_and(|f| f.window == window) {
        return;
    }
    // One prompt at a time, of any kind (prompts.rs): while another window's asks, this one is
    // refused, said in the window that asked. A window with nothing to ask still closes.
    let raised_in = window.unwrap_or(m.pane.window);
    if let Some(sentence) = prompts::waits_on_another(m, raised_in, window.is_none()) {
        let closes_anyway = window.is_some() && nothing_to_ask && m.closing.as_ref().is_none_or(|f| f.window.is_some());
        if !closes_anyway {
            return prompts::refuse(m, fx, raised_in, sentence);
        }
    }
    if nothing_to_ask {
        // A store job in flight would land on a closed window: the window waits for it, in words.
        // Quitting does not; the framework's shutdown waits for the job within its grace.
        if let Some(window) = window.filter(|_| m.busy.is_some()) {
            return prompts::refuse(m, fx, window, said::WAIT_BEFORE_CLOSING);
        }
        return close(m, fx, window);
    }
    m.closing = Some(CloseFlow::new(window, asked));
    m.close_asked_in = m.pane.window;
    m.pane.dialog = CLOSE_DIALOG;
    fx.changed(CHROME);
}

/// The person answered the prompt (its buttons, or Escape for *Cancel*).
pub fn answer(m: &mut Model, fx: &mut Effects<Notes>, choice: Choice) {
    let Some(flow) = m.closing.as_mut() else { return };
    let asking = flow.stage == Stage::Asking;
    let next = flow.answer(choice);
    if asking {
        // The prompt goes from the window that showed it, and from no other.
        if let Some(pane) = m.pane_of_mut(m.close_asked_in).filter(|p| p.dialog == CLOSE_DIALOG) {
            pane.dialog = 0;
        }
        fx.changed(CHROME);
    }
    apply(m, fx, next);
}

/// The person's answer, from the focused window: a prompt that asks is answered only from the
/// window it asks in, never from another window's dialog.
pub fn answer_from(m: &mut Model, fx: &mut Effects<Notes>, choice: Choice) {
    let asking = m.closing.as_ref().is_some_and(|f| f.stage == Stage::Asking);
    if !asking || asks_in(m, m.pane.window) {
        answer(m, fx, choice);
    }
}

/// Whether the close prompt asks in `window`: each prompt is drawn, and answered, only in the
/// window it asked in (leaving.rs keeps the same rule), so two prompts in two windows never
/// cross over.
pub fn asks_in(m: &Model, window: WindowId) -> bool {
    m.closing.as_ref().is_some_and(|f| f.stage == Stage::Asking) && m.close_asked_in == window
}

/// A store job landed, and the store is idle. A close waiting on its saves closes if everything
/// asked about is on disk, and saves the next one if not (or the same one again, if it was edited
/// meanwhile). A prompt still asking is said again for the notes still unsaved, and, when the
/// autosave has saved them all under it, answers itself: there is nothing left to ask, so the
/// close the person asked for goes ahead.
pub fn progress(m: &mut Model, fx: &mut Effects<Notes>) {
    let Some(stage) = m.closing.as_ref().map(|f| f.stage.clone()) else { return };
    match stage {
        Stage::SavingThenClose => {
            let Some(flow) = m.closing.as_mut() else { return };
            let next = flow.progress(&m.documents, None);
            apply(m, fx, next);
        }
        Stage::Asking => {
            let Some(flow) = m.closing.as_ref() else { return };
            let saved = flow.asked.iter().all(|d| !m.unsaved_doc(*d));
            let nothing_left = saved && unstored_at_close(m, flow.window) == 0;
            fx.changed(CHROME);
            if nothing_left && m.busy.is_none() {
                answer(m, fx, Choice::Save);
            }
        }
        Stage::Blocked(_) => {}
    }
}

/// A save of `doc` failed, or could not start; `None` for another store job that failed. When a
/// close waits on the saves of the notes it asked about and `doc` is one of them, the close is
/// over and everything stays open: returns what the notice says, `sentence` with the note's title
/// when the close asked about more than one or the person's window does not show it. Any other
/// failure leaves the close waiting (`None`), and it goes on once the store is idle (`progress`).
pub fn failed(m: &mut Model, doc: Option<DocumentId>, sentence: &str) -> Option<String> {
    let flow = m.closing.as_mut().filter(|f| f.stage == Stage::SavingThenClose)?;
    let doc = doc.filter(|d| flow.asked.contains(d))?;
    let several = flow.asked.len() > 1;
    flow.progress(&m.documents, Some(sentence.to_string()));
    m.closing = None;
    let title = m.note_of(doc).map(|c| c.title()).filter(|_| several || !m.in_view(doc));
    Some(match title {
        Some(title) => said::close_failed(&title, sentence),
        None => sentence.to_string(),
    })
}

fn apply(m: &mut Model, fx: &mut Effects<Notes>, next: yantrik_app::documents::close::Next) {
    use yantrik_app::documents::close::Next;
    match next {
        Next::Stay => m.closing = None,
        Next::Wait => {}
        Next::Save(documents) => save_then_close(m, fx, &documents),
        Next::Close { discard } => {
            if !discard.is_empty() {
                drop_edits(m, fx, &discard);
            }
            let window = m.closing.take().and_then(|f| f.window);
            close(m, fx, window);
        }
    }
}

/// *Save*: the asked notes saved, one at a time, each in a window that shows it; the close waits
/// on their landing, and `progress` starts the next.
fn save_then_close(m: &mut Model, fx: &mut Effects<Notes>, documents: &[DocumentId]) {
    let Some(next) = documents.iter().copied().find(|d| m.unsaved_doc(*d)) else {
        let window = m.closing.take().and_then(|f| f.window);
        return close(m, fx, window);
    };
    let gone = m.documents.get(next).is_some_and(|d| d.outside == OutsideState::Deleted);
    if gone {
        // Saved as new, in a window that shows it: the window moves to the kept note, which is on
        // disk, and the close goes on once that lands. While the store is busy, it waits.
        if m.busy.is_some() {
            return;
        }
        if let Some(window) = m.windows_showing(next).first().copied() {
            m.within(window, |m| outside::resolve(m, fx, Resolution::KeepMine));
        }
    } else {
        // Retried even after a failure: a close must not wait on an autosave that will not come.
        saving::save_document(m, fx, next, How::Asked);
    }
    let waiting = m.closing.as_ref().is_some_and(|f| f.stage == Stage::SavingThenClose);
    if waiting && m.busy.is_none() {
        // Nothing started and nothing refused it (the library is not open): the window stays.
        if let Some(sentence) = failed(m, Some(next), said::NOT_SAVED_NOW) {
            m.notice = sentence;
            fx.changed(STATUS);
        }
    }
}

/// *Don't save*: the edits and their draft dropped; the note is shown as its file holds it. A note
/// whose file is gone has nothing left on disk, so it leaves the library as it closes (show_note).
fn drop_edits(m: &mut Model, fx: &mut Effects<Notes>, documents: &[DocumentId]) {
    for id in documents {
        if let Some(d) = m.documents.get_mut(*id) {
            drafts::discard(d, fx);
        }
    }
    // A note whose file is gone is not opened again: there is no file to show it as.
    let again = m
        .current()
        .filter(|c| *c.outside() != OutsideState::Deleted)
        .and_then(|c| m.library.iter().find(|n| n.id == c.id() && n.trash == c.trash()).cloned());
    show_note(m, fx, None);
    show_note(m, fx, again);
    show(m, fx);
}

/// A window closes, its view with it; or, for `None`, the app ends. A close that asked has been
/// taken off `Model::closing` by now; another window's that is still asking stays.
fn close(m: &mut Model, fx: &mut Effects<Notes>, window: Option<WindowId>) {
    match window {
        Some(window) => {
            windows::forget(m, fx, window);
            fx.close(window);
        }
        None => fx.exit(),
    }
}

/// Notes listed in the library that are on no disk and in no open document: another program's
/// version that could not be written. Each is kept as a draft, and the prompt says so.
fn unstored(m: &Model) -> usize {
    let open: Vec<&str> = m.files.values().map(|f| f.id.as_str()).collect();
    m.library.iter().filter(|n| !n.stored && !n.trash && !open.contains(&n.id.as_str())).count()
}

/// The same, counted only when this close ends the app (quitting, or the last window). Closing
/// one of two windows leaves them listed in the window that stays.
fn unstored_at_close(m: &Model, window: Option<WindowId>) -> usize {
    if window.is_none() || m.windows().len() < 2 {
        unstored(m)
    } else {
        0
    }
}

/// The prompt's words for the notes asked about. One note whose file is gone is asked about as
/// what it is: *Save as new* keeps it, *Don't save* lets it go.
pub fn prompt(m: &Model) -> Option<Prompt> {
    let flow = m.closing.as_ref().filter(|f| f.stage == Stage::Asking)?;
    // Only the notes still not on disk: one the autosave saved under the prompt is not asked about.
    let notes: Vec<_> = flow.asked.iter().filter(|id| m.unsaved_doc(**id)).filter_map(|id| m.note_of(*id)).collect();
    let titles: Vec<String> = notes.iter().map(|c| c.title()).collect();
    let gone: Vec<String> =
        notes.iter().filter(|c| *c.outside() == OutsideState::Deleted).map(|c| c.title()).collect();
    let read_only = m.vault.read_only().is_some();
    let unstored = unstored_at_close(m, flow.window);
    let saves = !titles.is_empty() && !read_only;
    let leaving = if flow.window.is_none() { said::Leaving::Notes } else { said::Leaving::Window };
    let (heading, sentence, save) = match notes.as_slice() {
        [one] if !read_only && unstored == 0 && *one.outside() == OutsideState::Deleted => {
            let (heading, sentence) = said::gone_prompt(&one.title(), leaving, one.edited_since_gone());
            (heading, sentence, said::SAVE_AS_NEW)
        }
        _ => {
            let (heading, mut sentence) = said::close_prompt(&titles, flow.window.is_none(), read_only, unstored);
            if !gone.is_empty() && saves {
                sentence = format!("{sentence} {}", said::gone_among(&gone));
            }
            (heading, sentence, said::SAVE)
        }
    };
    Some(Prompt { heading, sentence, saves, save })
}

/// Escape, or the dialog put away by the window it asks in: the person cancelled the prompt.
pub fn put_away(m: &mut Model, fx: &mut Effects<Notes>) {
    if asks_in(m, m.pane.window) {
        answer(m, fx, Choice::Cancel);
    }
}
