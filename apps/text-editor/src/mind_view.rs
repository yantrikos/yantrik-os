//! The Text Editor as drawn for a Mind (`Boot::for_mind`): Mind View, where `read_mind_view` turns
//! every line on the window into text for an agent.
//!
//! The field shows the framework's one sentence (`yantrik_app::for_mind::HIDDEN`) in place of the
//! document, with no gutter, colours or line counts, and takes no edit. File names stay, as
//! `describe` gives them to the agent anyway. A search finds nothing, since nothing it could match
//! is shown (the match counter reads 0).
//!
//! Visual only: `describe` and the intents are unchanged by `for_mind`.

use crate::command::{Command, WindowAction};

/// A command a Mind's window cannot run, by the framework's read-only rule. Each is refused before
/// it touches a document or the model:
/// - typing, replace, replace all, undo and redo would change the document;
/// - open, save, save as, the save on the close prompt, and the dialog's confirm would read or
///   write a file as the person;
/// - Go to line would tell the length of the document from where it lands;
/// - Discard on the close prompt drops unsaved text, a change as the person.
pub fn refuses(command: &Command) -> bool {
    match command {
        Command::Edit(_) => true,
        Command::Window(action) => matches!(
            action,
            WindowAction::Open
                | WindowAction::Save
                | WindowAction::SaveAs
                | WindowAction::SaveClose
                | WindowAction::Goto
                | WindowAction::Confirm { .. }
                | WindowAction::Replace { .. }
                | WindowAction::ReplaceAll { .. }
                | WindowAction::Undo
                | WindowAction::Redo
                | WindowAction::Discard
        ),
        _ => false,
    }
}
