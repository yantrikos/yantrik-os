//! Notes as drawn for a Mind (`Boot::for_mind`): Mind View, where `read_mind_view` turns every
//! line on the window into text for an agent.
//!
//! The window shows the framework's one sentence (`yantrik_app::for_mind::HIDDEN`) where a note's
//! text would be: the editor, the list's previews, the rendered preview and the other version in
//! the comparison. Titles stay, since the Mind reads them through `describe` anyway. The editor
//! takes no typing, a search matches titles and fields and not text, and what would edit a note or
//! read or write a file through the window is refused ([`refuses`]).
//!
//! Visual only: `describe` and the intents are unchanged by `for_mind`, so a Mind still reads
//! notes through them.

use crate::command::{Command, WindowAction};
use crate::model::Model;

/// Whether `command` runs where a Mind is looking: this whole copy is drawn on a Mind's display
/// (`Boot::for_mind`), or the command came from a Mind's window beside the person's
/// (`WindowId::for_mind`). A call through the surface comes from no window and is not.
pub fn looking(m: &Model, command: &Command) -> bool {
    m.for_mind || matches!(command, Command::From { window, .. } if window.for_mind())
}

/// [`refuses`], and in a Mind's window beside the person's, *New window* too: it would open one on
/// the person's display, at a Mind's press.
pub fn refuses_in(m: &Model, command: &Command) -> bool {
    let new_window = matches!(command, Command::From { window, command }
        if window.for_mind() && matches!(**command, Command::Window(WindowAction::NewWindow)));
    refuses(command) || (new_window && !m.for_mind)
}

/// A command a Mind's window cannot run, by the framework's read-only rule. Each is refused before
/// it touches a note or the model:
/// - typing, undo, redo and the conflict and recovery choices would change the note or put the
///   masked sentence over it;
/// - save, import and export (and the dialog's confirm) would read or write a file as the person;
/// - Copy saves the note as a new note, which carries its real text into a second note;
/// - Trash, Restore, the notebook and tags fields and the recovery bar's Discard change the
///   library, and no one may do that through a window as the person. The intents offer the same
///   changes and reach them as `Command::Surface`, which this does not match.
pub fn refuses(command: &Command) -> bool {
    match command {
        Command::From { command, .. } => refuses(command),
        Command::Edit(_) | Command::Metadata { .. } => true,
        Command::Window(action) => matches!(
            action,
            WindowAction::Copy
                | WindowAction::Undo
                | WindowAction::Redo
                | WindowAction::Save
                | WindowAction::Import
                | WindowAction::Export
                | WindowAction::Confirm { .. }
                | WindowAction::KeepMine
                | WindowAction::TakeTheirs
                | WindowAction::RestoreDraft
                | WindowAction::DiscardDraft
                | WindowAction::Trash
                | WindowAction::Restore
                | WindowAction::Pin
        ),
        _ => false,
    }
}
