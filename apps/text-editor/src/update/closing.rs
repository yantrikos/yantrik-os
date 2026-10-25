//! Closing a tab, closing the window, quitting, and the session ending.
//!
//! A tab with unsaved changes is not closed without asking: "<tab> has unsaved changes." with
//! *Cancel*, *Discard* and *Save* (prompt.rs). Closing the window asks about each such tab in
//! turn, as it always did, and closes once none is left; *Cancel* keeps everything. *Save* closes
//! the tab only once its save has landed; a save that fails keeps the prompt up with the reason.
//!
//! The window closed while a save is in flight, with nothing else to ask about, closes at once and
//! the editor runs on with no window until the save lands (`fx.linger`), then ends; a second
//! launch meanwhile brings the window back and the editor runs on. A save that fails then is not
//! lost: the tab's text is in its draft, offered at the next start, and the person is told.
//!
//! The window closed while a file is read or written, with another tab unsaved, waits for that
//! file, then goes on with the close as asked: the usual prompt over each tab still unsaved, the
//! window staying until it is answered. A save that fails then keeps everything open, and says
//! why.
//!
//! The session ending (a logout, SIGTERM, the display gone) asks nothing: every unsaved tab's text
//! goes to its draft in the command that answers, and the editor ends.

use std::time::Duration;

use yantrik_app::app::WindowId;
use yantrik_app::documents::close::{Next, Stage};
use yantrik_app::documents::{Choice, CloseFlow};
use yantrik_app::effects::Effects;
use yantrik_app::Notification;

use super::{drafts, say, saving, tabs};
use crate::command::AfterSave;
use crate::model::{Dialog, Finish, Model, Work};
use crate::prompt::{self, Closing};
use crate::tab::TabId;
use crate::TextEditor;

/// How long the editor runs on without a window for a save in flight to land.
pub const LINGER: Duration = Duration::from_secs(10);

/// Said when the window is closed while a file is read, or a save is in flight and other tabs are
/// unsaved: the close is taken, and goes on once that file is done.
pub const CLOSING_WHEN_DONE: &str = "The window closes once the file being read or written is done.";

/// Said when the save the close waited on failed: nothing closes.
pub const NOT_CLOSED: &str = "The window was not closed, so nothing is lost.";

/// The person closed the window, or asked the editor to quit.
pub fn window(m: &mut Model, fx: &mut Effects<TextEditor>) {
    // The same close again needs no words: its prompt, or its save, is already under way.
    if m.closing.is_some() {
        return;
    }
    match m.busy {
        Some(Work::Save(saving)) if !m.tabs.iter().any(|t| t.id != saving && m.modified(t)) => {
            m.dialog = Dialog::None;
            fx.close(WindowId::PRIMARY);
            fx.linger(LINGER);
            m.lingering = true;
        }
        Some(_) => {
            m.close_when_done = true;
            say(m, CLOSING_WHEN_DONE);
        }
        None => {
            // An Open, Save As or Go to line dialog is put away: the window's close is the answer.
            m.dialog = Dialog::None;
            ask_next(m, fx);
        }
    }
}

/// The next unsaved tab asked about, while the window closes; the window closed once none is left.
pub fn ask_next(m: &mut Model, fx: &mut Effects<TextEditor>) {
    match m.tabs.iter().position(|t| m.modified(t)) {
        Some(index) => ask(m, fx, index, true),
        // Once every answer owed is settled: a call whose own work closed the window is answered.
        None => m.finish = Some(Finish::Close),
    }
}

/// Asks about the tab at `index`, brought forward so the person sees what they are asked about.
fn ask(m: &mut Model, fx: &mut Effects<TextEditor>, index: usize, quitting: bool) {
    tabs::select(m, fx, index);
    let tab = &m.tabs[index];
    let flow = CloseFlow::new(Some(WindowId::PRIMARY), vec![tab.doc]);
    m.closing = Some(Closing { quitting, tab: tab.id, flow, via_save_as: false });
    m.dialog_error.clear();
}

/// The strip's ×, or Ctrl+W (`id` the tab in front): not while a file is read or written, or a
/// dialog is up.
pub fn close_tab_from_window(m: &mut Model, fx: &mut Effects<TextEditor>, id: TabId) {
    if m.busy.is_some() || prompt::dialog(m) != 0 {
        return;
    }
    close_tab(m, fx, id);
}

/// Closes tab `id`, asking first when it has unsaved changes. Whether it asked.
pub fn close_tab(m: &mut Model, fx: &mut Effects<TextEditor>, id: TabId) -> bool {
    let Some(index) = m.index_of(id) else { return false };
    if m.modified(&m.tabs[index]) {
        ask(m, fx, index, false);
        return true;
    }
    tabs::remove(m, fx, id);
    false
}

/// The person's answer to the prompt.
pub fn answer(m: &mut Model, fx: &mut Effects<TextEditor>, choice: Choice) {
    let Some(id) = m.closing.as_ref().map(|c| c.tab) else { return };
    // A tab with no file needs a place first: the Save As dialog, whose save closes it.
    let untitled = m.tab(id).is_some_and(|t| m.path(t).is_none());
    if choice == Choice::Save && untitled {
        if let Some(closing) = m.closing.as_mut() {
            closing.via_save_as = true;
        }
        m.dialog = Dialog::SaveAs;
        m.dialog_path = "~/untitled.txt".into();
        m.dialog_error.clear();
        m.cues.push(crate::model::Cue::Dialog);
        return;
    }
    let Some(closing) = m.closing.as_mut() else { return };
    let next = closing.flow.answer(choice);
    apply(m, fx, next, AfterSave::Window);
}

/// The Save As dialog's save, when the prompt's *Save* opened it: its landing closes the tab.
pub fn saving_as(m: &mut Model) {
    if let Some(closing) = m.closing.as_mut().filter(|c| c.via_save_as) {
        let _ = closing.flow.answer(Choice::Save);
    }
}

/// Cancel: the prompt goes, and every tab stays as it was.
pub fn cancel(m: &mut Model) {
    m.closing = None;
}

/// What the prompt's answer or a save's landing leads to. `then` is what a save started here
/// answers when it lands.
pub fn apply(m: &mut Model, fx: &mut Effects<TextEditor>, next: Next, then: AfterSave) {
    let Some(closing) = m.closing.clone() else { return };
    match next {
        Next::Stay => m.closing = None,
        Next::Wait => {}
        Next::Save(_) => {
            // A call's save is held to the agent's write rule, as its own `save` is.
            let agent = match &then {
                AfterSave::Save { caller, .. } | AfterSave::SaveAs { caller, .. } => caller.agent,
                AfterSave::Window => false,
            };
            saving::start(m, fx, closing.tab, None, false, then, agent);
        }
        Next::Close { .. } => {
            m.closing = None;
            tabs::remove(m, fx, closing.tab);
            if closing.quitting {
                ask_next(m, fx);
            }
        }
    }
}

/// Tab `id` was saved: a close waiting on it goes ahead, and a linger waiting on it ends.
pub fn saved(m: &mut Model, fx: &mut Effects<TextEditor>, id: TabId) {
    let asked_here = m.closing.as_ref().is_some_and(|c| c.tab == id);
    let still_unsaved = m.tab(id).is_some_and(|t| m.modified(t));
    if let Some(closing) = m.closing.as_mut().filter(|_| asked_here) {
        if closing.flow.stage == Stage::Asking && !still_unsaved {
            // Saved by a save that was not the prompt's answer (`save_as`): the question is no
            // longer true, so the prompt goes and the tab stays.
            m.closing = None;
        } else {
            let next = closing.flow.progress(&m.documents, None);
            apply(m, fx, next, AfterSave::Window);
        }
    }
    if m.lingering {
        end_after_linger(m, fx, None);
    }
}

/// Tab `id`'s save failed: a close waiting on it stays, with the reason; a linger ends, with the
/// text kept as a draft.
pub fn not_saved(m: &mut Model, fx: &mut Effects<TextEditor>, id: TabId, why: &str) {
    if let Some(closing) = m.closing.as_mut().filter(|c| c.tab == id) {
        // Blocked: the prompt stays up, asking again, with the reason under it.
        let _ = closing.flow.progress(&m.documents, Some(why.to_string()));
    }
    if std::mem::take(&mut m.close_when_done) {
        // Added to the failure already said, in both its forms (notice.rs).
        let (window, agents) = (m.notice.window().to_string(), m.notice.read(true).to_string());
        m.notice.set_about(window != agents, format!("{window} {NOT_CLOSED}"), || format!("{agents} {NOT_CLOSED}"));
    }
    if m.lingering {
        end_after_linger(m, fx, Some(why));
    }
}

/// The save the window closed on has landed: the editor ends. Whatever is unsaved now (that save
/// failed, or a call changed a tab meanwhile) goes to its draft first and the person is told,
/// since there is no window left to tell them in.
fn end_after_linger(m: &mut Model, fx: &mut Effects<TextEditor>, why: Option<&str>) {
    let kept = drafts::checkpoint_all(m, fx);
    if kept > 0 {
        let body = match why {
            Some(why) => format!("{why} The text is kept and comes back the next time the editor opens."),
            None => "Unsaved text is kept and comes back the next time the editor opens.".to_string(),
        };
        fx.notify(Notification::titled("Text Editor could not save everything").body(body));
    }
    m.lingering = false;
    m.finish = Some(Finish::Exit);
}

/// After every command: a close that waited for a file to be read or written goes on once it is.
pub fn resume(m: &mut Model, fx: &mut Effects<TextEditor>) {
    if m.close_when_done && m.busy.is_none() {
        m.close_when_done = false;
        m.notice.clear();
        window(m, fx);
    }
}

/// A second launch brought the window back during a linger: the editor runs on, and the save in
/// flight lands as any other.
pub fn presented(m: &mut Model) {
    m.lingering = false;
}

/// The session or the system is ending the editor, with no one to ask: every unsaved tab's text
/// is handed to its draft now, rather than after its debounce, and the editor ends in this one
/// command.
pub fn logging_out(m: &mut Model, fx: &mut Effects<TextEditor>) {
    drafts::checkpoint_all(m, fx);
    fx.exit();
}
