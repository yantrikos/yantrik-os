//! yPresent as drawn for a Mind (`Boot::for_mind`): Mind View, where `read_mind_view` turns every
//! line on the window into text for an agent.
//!
//! The window shows the framework's one sentence (`yantrik_app::for_mind::HIDDEN`) where a deck's
//! text would be: every slide's title, body and notes, on the canvas, in the rail's thumbnails and
//! on the presenter view's next-slide card. The deck's name, its file, its slide count and the
//! position stay for a deck an agent may be shown, since the Mind reads them through `describe`
//! anyway; for a deck it is not shown, the name and file are `(hidden)` here too and the notice is
//! the form an agent reads. The canvas takes no typing,
//! a search finds nothing, and what would change the deck or read or write a file through the
//! window is refused ([`refuses`]).
//!
//! Visual only: `describe` and the intents are unchanged by `for_mind`, so a Mind still reads and
//! writes decks through them, under the agent rules.

use crate::command::{Command, WindowAction};

/// A command a Mind's window cannot run, by the framework's read-only rule. Each is refused
/// before it touches the deck or the model:
/// - typing, the slide buttons (add, delete, duplicate, move), the layout and theme, undo and redo
///   would change the deck, or put the masked sentence over it;
/// - Save, Open and the exports would read or write a file as the person;
/// - speaker notes would carry the slide's text out to the Mind;
/// - a launch's file would be read as the person, past the agent rule.
///
/// What is left is looking: moving between slides, the presenter view, the gallery, the clock and
/// search (which finds nothing). The intents offer their own changes and reach them as
/// `Command::Surface`, which this does not match.
pub fn refuses(command: &Command) -> bool {
    match command {
        Command::Canvas(_) | Command::Launch(_) => true,
        Command::Window { action, .. } => matches!(
            action,
            WindowAction::AddSlide
                | WindowAction::DeleteSlide
                | WindowAction::DuplicateSlide
                | WindowAction::MoveUp
                | WindowAction::MoveDown
                | WindowAction::SetLayout(_)
                | WindowAction::SetTheme(_)
                | WindowAction::Save
                | WindowAction::OpenRecent
                | WindowAction::ExportMarkdown
                | WindowAction::ExportOutline
                | WindowAction::Undo
                | WindowAction::Redo
                | WindowAction::GenerateNotes
        ),
        _ => false,
    }
}
