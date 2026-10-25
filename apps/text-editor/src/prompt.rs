//! The close prompt: "<tab> has unsaved changes." with *Cancel*, *Discard* and *Save*.
//!
//! It is drawn from the close flow and from nothing else, so the window can never show the prompt
//! with no question behind it, and there is one flow at a time, so there is never more than one
//! prompt. The editor has one window; every prompt is asked in it.

use yantrik_app::documents::close::Stage;
use yantrik_app::documents::CloseFlow;

use crate::model::{Dialog, Model};
use crate::tab::TabId;

/// A close asking about one tab, or waiting on its save.
#[derive(Debug, Clone)]
pub struct Closing {
    /// The window's close (or a quit): every unsaved tab is asked about in turn, and the window
    /// closes once none is left. Otherwise one tab's close.
    pub quitting: bool,
    /// The tab asked about.
    pub tab: TabId,
    pub flow: CloseFlow,
    /// The prompt's *Save* opened the Save As dialog (the tab has no file): that dialog's save is
    /// the answer, and closes the tab once it lands. A Save As the person opened over the prompt
    /// is not.
    pub via_save_as: bool,
}

/// The dialog numbers `describe.dialog` has always used.
pub const OPEN: i32 = 1;
pub const SAVE_AS: i32 = 2;
pub const PROMPT: i32 = 3;
pub const GOTO: i32 = 4;

/// Whether the close prompt is up: asking, or asking again after its save failed.
pub fn asking(m: &Model) -> bool {
    m.closing.as_ref().is_some_and(|c| matches!(c.flow.stage, Stage::Asking | Stage::Blocked(_)))
}

/// The dialog the window shows: a path or line dialog over everything, else the prompt.
pub fn dialog(m: &Model) -> i32 {
    match m.dialog {
        Dialog::Open => OPEN,
        Dialog::SaveAs => SAVE_AS,
        Dialog::Goto => GOTO,
        Dialog::None if asking(m) => PROMPT,
        Dialog::None => 0,
    }
}

/// The prompt's sentence: `describe.dialog_is`, and empty while nothing asks. For an agent, a
/// tab whose file it is not shown is named `(hidden)`.
pub fn label(m: &Model, agent: bool) -> String {
    let tab = m.closing.as_ref().filter(|_| asking(m)).and_then(|c| m.tab(c.tab));
    match tab {
        Some(t) if agent && t.hidden => format!("{} has unsaved changes.", crate::model::HIDDEN_TAB),
        Some(t) => format!("{} has unsaved changes.", m.title(t)),
        None => String::new(),
    }
}
