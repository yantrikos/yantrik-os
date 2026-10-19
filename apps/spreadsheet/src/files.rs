//! Open and Save from the window: the desktop's file picker, the typed path when there is no
//! picker to ask, and the unsaved-changes question in front of both.
//!
//! The picker runs on a worker thread (it waits for the person), so its answer comes back to the
//! window thread through `invoke_from_event_loop` and finds the session in [`SESSION`].

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use slint::ComponentHandle;

use crate::picker::{self, Choice};
use crate::render;
use crate::session::{Pending, Session, Shared};
use crate::surface;
use crate::{SheetDialog, SpreadsheetApp};

/// Who is asked for a file: `saving`, and the sheet's current file to suggest.
pub type Ask = fn(bool, Option<PathBuf>) -> Choice;

fn ask_the_desktop(saving: bool, current: Option<PathBuf>) -> Choice {
    if saving { picker::save_file(current.as_deref()) } else { picker::open_file() }
}

thread_local! {
    /// The window's session, for answers that arrive from another thread.
    pub static SESSION: RefCell<Option<Shared>> = const { RefCell::new(None) };
    /// The desktop's picker; the window test puts a stand-in here, so it never opens a real one.
    pub static ASK: std::cell::Cell<Ask> = const { std::cell::Cell::new(ask_the_desktop) };
}

fn with_session(f: impl FnOnce(&mut Session)) {
    SESSION.with(|cell| {
        if let Some(shared) = cell.borrow().as_ref() {
            f(&mut shared.borrow_mut());
        }
    });
}

/// Draw everything, formula bar included: after a file operation nothing is being typed.
pub fn redraw(ui: &SpreadsheetApp, s: &Session) {
    render::render(ui, s);
    render::load_formula_bar(ui, s);
}

/// A sensible folder to suggest: the open file's, else ~/Documents, else home.
fn suggested(s: &Session, name: &str) -> String {
    if let Some(p) = &s.doc.path {
        return p.display().to_string();
    }
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let docs = home.join("Documents");
    let dir = if docs.is_dir() { docs } else { home };
    dir.join(name).display().to_string()
}

/// The header's Open.
pub fn open(ui: &SpreadsheetApp, s: &mut Session) {
    if s.doc.dirty() {
        ask_unsaved(ui, s, Pending::Open);
        return;
    }
    pick(ui, s, false);
}

/// The header's Save: over the sheet's own file, or Save as when it has none.
pub fn save(ui: &SpreadsheetApp, s: &mut Session) {
    match s.doc.path.clone() {
        Some(path) => saved(ui, s, &path, true),
        None => pick(ui, s, true),
    }
}

pub fn save_as(ui: &SpreadsheetApp, s: &mut Session) {
    pick(ui, s, true);
}

fn ask_unsaved(ui: &SpreadsheetApp, s: &mut Session, pending: Pending) {
    s.pending = Some(pending);
    let what = match pending {
        Pending::Open => "opening another file",
        Pending::Close => "closing",
    };
    ui.set_dialog_message(format!("{} has changes that are not saved. Save them before {what}?", s.doc.title()).into());
    ui.set_dialog(SheetDialog::Unsaved);
}

/// Ask the desktop's picker, off the window thread.
fn pick(ui: &SpreadsheetApp, s: &mut Session, saving: bool) {
    if s.picking {
        return;
    }
    s.picking = true;
    s.said = Some((if saving { "Choosing where to save…" } else { "Choosing a file to open…" }.into(), false));
    render::render(ui, s);
    let weak = ui.as_weak();
    let current = s.doc.path.clone();
    let ask = ASK.with(|a| a.get());
    std::thread::spawn(move || {
        let choice = ask(saving, current);
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            with_session(|s| {
                s.picking = false;
                chosen(&ui, s, saving, choice);
            });
        });
    });
}

fn chosen(ui: &SpreadsheetApp, s: &mut Session, saving: bool, choice: Choice) {
    match choice {
        // The picker has already asked the person about replacing a file that is there.
        Choice::Chosen(path) if saving => saved(ui, s, &path, true),
        Choice::Chosen(path) => opened(ui, s, &path),
        Choice::Cancelled => {
            s.pending = None;
            s.said = Some(("Nothing was chosen, so nothing changed.".into(), false));
            render::render(ui, s);
        }
        Choice::Unavailable(why) => {
            tracing::warn!(%why, "No file picker; asking for a typed path");
            let (kind, name) = if saving {
                (SheetDialog::SavePath, "sheet.csv")
            } else {
                (SheetDialog::OpenPath, "")
            };
            ui.set_dialog_message(
                format!("The desktop's file picker could not be asked: {why}. Type the path instead (/ or ~/).").into(),
            );
            ui.set_dialog_path(suggested(s, name).into());
            ui.set_dialog(kind);
            render::render(ui, s);
        }
    }
}

fn opened(ui: &SpreadsheetApp, s: &mut Session, path: &Path) {
    // Failure is already on the status line and in the notice.
    let _ = surface::open_path(s, path);
    redraw(ui, s);
}

fn saved(ui: &SpreadsheetApp, s: &mut Session, path: &Path, replace: bool) {
    let ok = surface::save_path(s, path, replace).is_ok();
    redraw(ui, s);
    if ok {
        continue_pending(ui, s);
    } else {
        s.pending = None;
    }
}

/// After a save the unsaved-changes question asked for, do what was waiting on it.
fn continue_pending(ui: &SpreadsheetApp, s: &mut Session) {
    match s.pending.take() {
        Some(Pending::Open) => pick(ui, s, false),
        Some(Pending::Close) => close_now(ui, s),
        None => {}
    }
}

fn close_now(ui: &SpreadsheetApp, s: &mut Session) {
    s.closing = true;
    // Next turn of the loop: the close request runs `may_close`, which needs the session this
    // call is still holding.
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
        if let Some(ui) = weak.upgrade() {
            yantrik_app_runtime::chrome::close(ui.window());
        }
    });
}

/// The dialog's main button: Open or Save for a typed path, Save for unsaved changes.
pub fn confirm(ui: &SpreadsheetApp, s: &mut Session) {
    let kind = ui.get_dialog();
    match kind {
        SheetDialog::Unsaved => {
            ui.set_dialog(SheetDialog::None);
            match s.doc.path.clone() {
                Some(path) => saved(ui, s, &path, true),
                None => pick(ui, s, true),
            }
        }
        SheetDialog::OpenPath | SheetDialog::SavePath => {
            let base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
            let path = crate::document::resolve(&ui.get_dialog_path(), &base);
            if kind == SheetDialog::OpenPath {
                ui.set_dialog(SheetDialog::None);
                opened(ui, s, &path);
                return;
            }
            // A typed path is not the picker: nobody has asked about replacing what is there.
            if path.exists() && s.confirm_replace.as_deref() != Some(path.as_path()) {
                s.confirm_replace = Some(path.clone());
                ui.set_dialog_message(format!("{} is already there. Press Save again to replace it.", path.display()).into());
                return;
            }
            ui.set_dialog(SheetDialog::None);
            saved(ui, s, &path, true);
        }
        SheetDialog::None => {}
    }
}

pub fn cancel(ui: &SpreadsheetApp, s: &mut Session) {
    ui.set_dialog(SheetDialog::None);
    s.pending = None;
    s.confirm_replace = None;
    s.said = Some(("Nothing changed.".into(), false));
    render::render(ui, s);
}

/// Unsaved changes, thrown away on purpose.
pub fn discard(ui: &SpreadsheetApp, s: &mut Session) {
    ui.set_dialog(SheetDialog::None);
    match s.pending.take() {
        Some(Pending::Open) => pick(ui, s, false),
        Some(Pending::Close) => close_now(ui, s),
        None => {}
    }
}

/// The window's close: asked about first when there is unsaved work.
pub fn may_close(ui: &SpreadsheetApp, s: &mut Session) -> bool {
    if s.closing || !s.doc.dirty() {
        return true;
    }
    ask_unsaved(ui, s, Pending::Close);
    false
}
