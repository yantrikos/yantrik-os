//! The window's actions: its header, its keys and its buttons, each as it has always behaved.

use yantrik_app::effects::Effects;

use super::{closing, editing, leaving, library, navigate, outside, reblock, recovery, said, saving, windows};
use crate::command::{Resolution, Step, WindowAction};
use crate::model::{Model, Pending};
use crate::pane::Cue;
use crate::store::expand_home;
use crate::view::render::{CHROME, CUES, STATUS};
use crate::Notes;

/// Which of the window's two dialogs a path was typed into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    Import = 1,
    Export = 2,
}

pub fn act(m: &mut Model, fx: &mut Effects<Notes>, action: WindowAction) {
    // These change only the view, so they never wait: not for the store, not for a save.
    match action {
        WindowAction::Focus => {
            m.pane.focus_mode = !m.pane.focus_mode;
            m.pane.cues.push(Cue::FocusContent);
            fx.changed(CHROME);
            fx.changed(CUES);
        }
        WindowAction::Preview => {
            m.pane.preview = !m.pane.preview;
            if m.pane.preview {
                reblock(m, fx);
                m.pane.cues.push(Cue::FocusShortcuts);
            } else {
                m.pane.cues.push(Cue::FocusEditor);
            }
            fx.changed(CHROME);
            fx.changed(CUES);
        }
        // Said in the notice when the other window's edits overtook this window's history.
        WindowAction::Undo => {
            editing::undo(m, fx, false);
        }
        WindowAction::Redo => {
            editing::undo(m, fx, true);
        }
        WindowAction::NotNow => outside::not_now(m, fx),
        // The close prompt answers while a save it asked for is in flight. The same prompt asks
        // before leaving a note whose file is gone.
        WindowAction::Close(choice) if leaving::asks_in(m, m.pane.window) => leaving::answer(m, fx, choice),
        WindowAction::Close(choice) => closing::answer_from(m, fx, choice),
        // Drops the note and its draft, neither of which is the store's: nothing to wait for.
        WindowAction::LetGo => leaving::let_go_pressed(m, fx),
        // Drops a draft, which is not the store's: nothing to wait for.
        WindowAction::DiscardDraft => recovery::discard(m, fx),
        // Asked of the framework, which answers with `Command::Quit`: its close flow waits for the
        // store itself, and asks before anything unsaved is left.
        WindowAction::Quit => fx.request_quit(),
        other => {
            if may_go_on(m, fx, &other) {
                guarded(m, fx, other);
            }
        }
    }
}

/// What every other action is held to: the store is not busy, the library is open, and the
/// open note is saved before anything leaves it (the action then waits for that save).
fn may_go_on(m: &mut Model, fx: &mut Effects<Notes>, action: &WindowAction) -> bool {
    if m.busy.is_some() {
        m.notice = said::WAIT_FOR_THE_STORE.into();
        fx.changed(STATUS);
        return false;
    }
    if !m.ready {
        m.notice = "The library is unavailable. Resolve the storage error, then Refresh.".into();
        fx.changed(STATUS);
        if *action == WindowAction::Reload {
            library::load(m, fx);
        }
        return false;
    }
    // Never refused off a note whose file is gone: the person is asked what to do with its text.
    if leaving::asks(m, action) {
        leaving::ask(m, fx, action.clone());
        return false;
    }
    if saves_first(m, action) {
        let step = Step::Window(action.clone());
        m.pending = Some(Pending { step, refuse_if_unsaved: false, window: m.pane.window });
        saving::save(m, fx, saving::How::Asked);
        return false;
    }
    m.notice.clear();
    m.lasting_notice = false;
    fx.changed(STATUS);
    true
}

/// Whether `action` waits for the open note to be saved first: it leaves the note, and the note
/// has edits not on disk. A Refresh of a library open read-only does not: the vault would refuse
/// that save, and the Refresh is what can make the library writable again. The reload keeps the
/// unsaved text open across it (`library::loaded`), and saves it once the library is writable.
pub fn saves_first(m: &Model, action: &WindowAction) -> bool {
    let read_only_refresh = *action == WindowAction::Reload && m.vault.read_only().is_some();
    action.navigates() && m.unsaved() && !read_only_refresh
}

fn guarded(m: &mut Model, fx: &mut Effects<Notes>, action: WindowAction) {
    match action {
        WindowAction::New(title) => {
            // The window's own New has no use for the file name; a refusal is already the notice.
            let _ = navigate::new_note(m, fx, title.as_deref().unwrap_or("Untitled"), "");
        }
        WindowAction::Open { id, trash } => navigate::open(m, fx, &id, trash),
        WindowAction::Save => saving::save(m, fx, saving::How::Asked),
        WindowAction::Reload => library::load(m, fx),
        WindowAction::Pin => editing::pin(m, fx),
        WindowAction::Trash => library::trash(m, fx),
        WindowAction::Restore => library::restore(m, fx),
        WindowAction::Copy => navigate::copy(m, fx),
        WindowAction::KeepMine => outside::resolve(m, fx, Resolution::KeepMine),
        WindowAction::TakeTheirs => outside::resolve(m, fx, Resolution::TakeTheirs),
        WindowAction::Compare => outside::compare(m, fx),
        WindowAction::RestoreDraft => recovery::restore(m, fx),
        WindowAction::ShowDraft => recovery::show_offer(m, fx),
        WindowAction::NewWindow => {
            if let Err(sentence) = windows::open(m, fx, None) {
                m.notice = sentence.into();
                fx.changed(STATUS);
            }
        }
        WindowAction::Import => open_dialog(m, fx, Dialog::Import),
        WindowAction::Export => {
            if m.current().is_some() {
                open_dialog(m, fx, Dialog::Export);
            }
        }
        WindowAction::Confirm { path } => {
            m.pane.path = path.clone();
            match m.pane.dialog {
                1 => confirmed(m, fx, Dialog::Import, &path, false),
                2 => confirmed(m, fx, Dialog::Export, &path, false),
                _ => {}
            }
        }
        // The view's own actions were handled before the guards.
        WindowAction::Focus
        | WindowAction::Preview
        | WindowAction::Undo
        | WindowAction::Redo
        | WindowAction::NotNow
        | WindowAction::Close(_)
        | WindowAction::DiscardDraft
        | WindowAction::LetGo
        | WindowAction::Quit => {}
    }
}

fn open_dialog(m: &mut Model, fx: &mut Effects<Notes>, dialog: Dialog) {
    m.pane.path.clear();
    m.pane.dialog = dialog as i32;
    fx.changed(CHROME);
}

/// An import or export the surface asked for, once the open note is saved: held to the same
/// guards as the dialog's Continue, without opening the dialog. `agent`: an agent asked.
pub fn confirm(m: &mut Model, fx: &mut Effects<Notes>, dialog: Dialog, path: &str, agent: bool) {
    let action = WindowAction::Confirm { path: path.into() };
    if may_go_on(m, fx, &action) {
        confirmed(m, fx, dialog, path, agent);
    }
}

fn confirmed(m: &mut Model, fx: &mut Effects<Notes>, dialog: Dialog, path: &str, agent: bool) {
    let path = expand_home(path.trim());
    if !path.is_absolute() {
        m.notice = "Enter an absolute file path.".into();
        fx.changed(STATUS);
        return;
    }
    match dialog {
        Dialog::Import => library::import(m, fx, path, agent),
        Dialog::Export => library::export(m, fx, path, agent),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_sends_these_ids_and_nothing_else_is_an_action() {
        let path = || "/tmp/x.md".to_string();
        assert_eq!(WindowAction::parse("new", path), Some(WindowAction::New(None)));
        assert_eq!(WindowAction::parse("new:Field notes", path), Some(WindowAction::New(Some("Field notes".into()))));
        assert_eq!(
            WindowAction::parse("open:trash:a.md", path),
            Some(WindowAction::Open { id: "a.md".into(), trash: true })
        );
        assert_eq!(WindowAction::parse("confirm", path), Some(WindowAction::Confirm { path: "/tmp/x.md".into() }));
        assert_eq!(WindowAction::parse("take-theirs", path), Some(WindowAction::TakeTheirs));
        assert_eq!(WindowAction::parse("nonsense", path), None);
        assert!(WindowAction::Trash.navigates() && !WindowAction::Copy.navigates());
    }
}
