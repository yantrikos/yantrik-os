//! ySheets as drawn for a Mind (`Boot::for_mind`): Mind View, where `read_mind_view` turns every
//! line on the window into text for an agent.
//!
//! The window shows the framework's one sentence (`yantrik_app::for_mind::HIDDEN`) where a cell's
//! contents would be: in every cell of the grid, and in the formula bar. The sheet's name, its
//! size, the selection and the counts stay, since the Mind reads them through `describe` anyway;
//! for a sheet an agent is not shown, `describe` withholds them, so the window does too: every cell
//! is the mask (empty ones as well), the bar is the mask, the status line is the mask, and the
//! selection and the undo buttons say nothing (`view::render::Mask`).
//! The formula bar takes no typing, and what would change the sheet, or read or write a file
//! through the window, is refused ([`refuses`]).
//!
//! Visual only: `describe` and the intents are unchanged by `for_mind`, so a Mind still reads and
//! writes the sheet through them, under the agent rules.

use crate::command::Command;
use crate::model::Model;

/// A command a Mind's window cannot run, by the framework's read-only rule. Each is refused before
/// it touches the sheet or the model:
/// - typing into the formula bar, which would change a cell the window cannot show, or put the
///   masked sentence in it;
/// - undo and redo, which would change cells;
/// - Open, Save and Save As, which would read or write a file as the person;
/// - a launch's file, which would be read as the person, past the agent rule.
///
/// What is left is looking: selecting a cell, with the formula bar as the window drew it. The
/// intents offer their own changes and reach them as `Command::Surface`, which this does not match.
pub fn refuses(m: &Model, command: &Command) -> bool {
    // Anything in the bar that is not what the window put there was typed by someone.
    let typed = |text: &String| *text != m.field_text();
    match command {
        Command::Launch(_) | Command::Confirm { .. } => true,
        Command::Cell { typed: text, .. } | Command::Formula { typed: text, .. } => typed(text),
        Command::Header { id, typed: text } => {
            matches!(id.as_str(), "open" | "save" | "save-as" | "undo" | "redo") || typed(text)
        }
        _ => false,
    }
}
