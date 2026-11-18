//! The unsaved-changes prompt: "<document> has unsaved changes." with *Cancel*, *Discard* and
//! *Save*, standing in front of whatever the person asked for.
//!
//! It is drawn from the close flow and from nothing else, so the window can never show the prompt
//! with no question behind it, and there is one flow at a time, so there is never more than one
//! prompt. yDoc has one window; every prompt is asked in it.

use std::path::PathBuf;

use yantrik_app::documents::close::Stage;
use yantrik_app::documents::CloseFlow;

use crate::model::{Dialog, Model};

/// What the unsaved-changes prompt is standing in front of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// The window closing, or yDoc being quit.
    Close,
    /// A new, empty document.
    New,
    /// Open this file, once the draft on screen has been saved or given up on purpose. The path
    /// travels with the intent so that answering the prompt does not mean retyping it.
    Open(PathBuf),
}

/// A prompt asking about the document, or waiting on its save.
#[derive(Debug, Clone)]
pub struct Closing {
    pub intent: Intent,
    pub flow: CloseFlow,
    /// The prompt's *Save* opened the Save As dialog (the document has no file): that dialog's
    /// save is the answer. A Save As the person opened over the prompt is not.
    pub via_save_as: bool,
}

/// The dialog numbers the screen and `describe.prompt_open` have always used.
pub const NONE: i32 = 0;
pub const OPEN: i32 = 1;
pub const SAVE_AS: i32 = 2;
pub const EXPORT_MD: i32 = 3;
pub const EXPORT_HTML: i32 = 4;
pub const UNSAVED: i32 = 5;

/// Whether the prompt is up: asking, or asking again after its save failed.
pub fn asking(m: &Model) -> bool {
    m.closing.as_ref().is_some_and(|c| matches!(c.flow.stage, Stage::Asking | Stage::Blocked(_)))
}

/// The dialog the window shows: a file dialog over everything, else the prompt.
pub fn dialog(m: &Model) -> i32 {
    match m.dialog {
        Dialog::Open => OPEN,
        Dialog::SaveAs => SAVE_AS,
        Dialog::ExportMarkdown => EXPORT_MD,
        Dialog::ExportHtml => EXPORT_HTML,
        Dialog::None if asking(m) => UNSAVED,
        Dialog::None => NONE,
    }
}
