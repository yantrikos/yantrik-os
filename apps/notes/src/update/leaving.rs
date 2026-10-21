//! A note whose file is gone never traps the person (#925, decision 1). Its text is on no disk, so
//! it is unsaved; but every way off it asks, through the close prompt's three answers, rather than
//! refusing as a save over it would:
//!
//! - Opening another note, making one, Refresh, Trash, Import: *Save as new* keeps the text as a
//!   new note and then goes on; *Don't save* lets the note go and goes on; *Cancel* stays.
//! - Closing its window, or quitting: the close prompt, worded for a file that is gone, whose
//!   *Save as new* keeps the text as a new note before the window closes (closing.rs).
//! - The deleted bar's own *Let it go* (*Discard*, when the person changed the note): the person
//!   deleted the file, and this honours it once they say so. The document goes from every window,
//!   with its draft, and the library lists it no more; said once, and nothing is offered later.

use yantrik_app::app::WindowId;
use yantrik_app::documents::{Choice, DocumentId, OutsideState};
use yantrik_app::effects::Effects;

use super::closing::{Prompt, CLOSE_DIALOG};
use super::navigate::show_note;
use super::{drafts, outside, prompts, said, show, window, windows};
use crate::command::{Resolution, Step, WindowAction};
use crate::model::{Model, Pending};
use crate::view::render::{CHROME, OUTSIDE, STATUS};
use crate::Notes;

/// Whether `action` leaves a note whose file is gone, and so asks first.
pub fn asks(m: &Model, action: &WindowAction) -> bool {
    action.navigates() && m.current().is_some_and(|c| *c.outside() == OutsideState::Deleted)
}

/// Asks, in the focused window, before `action` leaves its note whose file is gone.
pub fn ask(m: &mut Model, fx: &mut Effects<Notes>, action: WindowAction) {
    // One prompt at a time, of any kind: another window's prompt is answered first (prompts.rs).
    if let Some(sentence) = prompts::waits_on_another(m, m.pane.window, false) {
        return prompts::refuse(m, fx, m.pane.window, sentence);
    }
    m.leaving = Some(Pending { step: Step::Window(action), window: m.pane.window, refuse_if_unsaved: false });
    m.pane.dialog = CLOSE_DIALOG;
    fx.changed(CHROME);
}

/// Whether the leaving prompt asks in `window`: the window it was asked in, and no other. Each
/// prompt is drawn, and answered, only where it asked (closing.rs keeps the same rule).
pub fn asks_in(m: &Model, window: WindowId) -> bool {
    m.leaving.as_ref().is_some_and(|l| l.window == window)
}

/// The person answered the prompt, in the window it asked in (`asks_in`): it acts there. A note
/// that is no longer gone by then (its file came back) asks nothing more: the action goes ahead
/// as any other would, its note saved first when it has edits.
pub fn answer(m: &mut Model, fx: &mut Effects<Notes>, choice: Choice) {
    let Some(leaving) = put_down(m, fx) else { return };
    let window = leaving.window;
    m.within(window, |m| answered(m, fx, leaving, choice));
}

fn answered(m: &mut Model, fx: &mut Effects<Notes>, leaving: Pending, choice: Choice) {
    let still_gone = m.current().is_some_and(|c| *c.outside() == OutsideState::Deleted);
    match choice {
        Choice::Cancel => {}
        Choice::Save | Choice::DontSave if !still_gone => {
            if let Step::Window(action) = leaving.step {
                window::act(m, fx, action);
            }
        }
        // What waited goes ahead once the kept note has landed (`after_store`); a keep that
        // fails, or cannot start, takes it with it and says why.
        Choice::Save if m.busy.is_some() => say(m, fx, said::WAIT_FOR_THE_STORE),
        Choice::Save => {
            m.pending = Some(leaving);
            outside::resolve(m, fx, Resolution::KeepMine);
            if m.busy.is_none() {
                m.pending = None;
            }
        }
        Choice::DontSave => {
            let Some((sentence, windows)) = let_go(m, fx) else { return };
            if let Step::Window(action) = leaving.step {
                window::act(m, fx, action);
            }
            // Said once, unless what went ahead had something of its own to say.
            if m.notice.is_empty() {
                said_in(m, fx, windows, &sentence);
            }
        }
    }
}

/// Escape, a close of its window, or a quit: the prompt is put away, and nothing goes ahead.
pub fn put_away(m: &mut Model, fx: &mut Effects<Notes>) {
    put_down(m, fx);
}

/// `doc` is no longer a note whose file is gone (its file came back, or it was let go in another
/// window): a prompt asking about it asks a question that is no longer true, so it goes.
pub fn no_longer_gone(m: &mut Model, fx: &mut Effects<Notes>, doc: DocumentId) {
    let asked_about = m.leaving.as_ref().and_then(|l| m.pane_of(l.window)).and_then(|p| m.document_in(p));
    if asked_about == Some(doc) {
        put_down(m, fx);
    }
}

fn put_down(m: &mut Model, fx: &mut Effects<Notes>) -> Option<Pending> {
    let leaving = m.leaving.take()?;
    if let Some(pane) = m.pane_of_mut(leaving.window).filter(|p| p.dialog == CLOSE_DIALOG) {
        pane.dialog = 0;
    }
    fx.changed(CHROME);
    Some(leaving)
}

/// The prompt's words while it asks.
pub fn prompt(m: &Model) -> Option<Prompt> {
    let leaving = m.leaving.as_ref()?;
    let c = m.pane_of(leaving.window).and_then(|p| m.current_in(p))?;
    let (heading, sentence) = said::gone_prompt(&c.title(), said::Leaving::Note, c.edited_since_gone());
    let saves = m.vault.read_only().is_none();
    Some(Prompt { heading, sentence, saves, save: if saves { said::SAVE_AS_NEW } else { said::SAVE } })
}

/// The deleted bar's *Let it go*, or *Discard*: the focused window's note whose file is gone.
pub fn let_go_pressed(m: &mut Model, fx: &mut Effects<Notes>) {
    match let_go(m, fx) {
        Some((sentence, windows)) => said_in(m, fx, windows, &sentence),
        None => say(m, fx, said::NOTHING_TO_LET_GO),
    }
}

/// Lets the focused window's note whose file is gone go: its draft removed, its document closed in
/// every window that shows it, and with it its card (`show_note`). Returns what is said of it, and
/// the windows that showed it, where it is said; `None`, and nothing done, when the note's file is
/// not gone.
fn let_go(m: &mut Model, fx: &mut Effects<Notes>) -> Option<(String, Vec<WindowId>)> {
    let c = m.current().filter(|c| *c.outside() == OutsideState::Deleted)?;
    let (doc, sentence): (DocumentId, String) = (c.doc.id, said::let_go(&c.title(), c.edited_since_gone()));
    let windows = m.windows_showing(doc);
    if let Some(d) = m.documents.get_mut(doc) {
        drafts::discard(d, fx);
    }
    // Let go here, it is no question for a window that was asking about it.
    no_longer_gone(m, fx, doc);
    show_note(m, fx, None);
    windows::follow_to(m, fx, doc, None);
    show(m, fx);
    fx.changed(OUTSIDE);
    Some((sentence, windows))
}

fn said_in(m: &mut Model, fx: &mut Effects<Notes>, windows: Vec<WindowId>, sentence: &str) {
    m.say_in(windows, sentence);
    fx.changed(STATUS);
}

fn say(m: &mut Model, fx: &mut Effects<Notes>, sentence: &str) {
    m.notice = sentence.into();
    m.lasting_notice = false;
    fx.changed(STATUS);
}
