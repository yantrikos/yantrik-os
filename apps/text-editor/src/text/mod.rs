//! The rules about text that need no disk, no window and no thread: what a document may hold,
//! how much the open tabs may hold between them, finding and replacing, the language a path
//! names and its colours, the part of a document the renderer can be handed, and a tab's undo.

pub mod budget;
pub mod history;
pub mod language;
pub mod limits;
pub mod search;
pub mod viewport;

/// How many lines `text` has, as the editor has always counted them: one more than its newlines.
pub fn lines(text: &str) -> usize {
    text.bytes().filter(|b| *b == b'\n').count() + 1
}
