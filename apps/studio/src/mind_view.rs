//! Studio as drawn for a Mind (`Boot::for_mind`): Mind View, where `read_mind_view` turns every
//! line on the window into text for an agent.
//!
//! What the person wrote and what the person made is the content here: the sentences in the
//! prompt boxes, the queue's labels, each picture's sentence and the record pane, and the
//! thumbnails. The window shows the framework's one sentence
//! (`yantrik_app::for_mind::HIDDEN`) where a sentence would be and draws no thumbnail. What would
//! change the fields, make or change a file, send a sentence to a backend or hand a file to the
//! desktop through the window is refused ([`refuses`]).
//!
//! What stays is what a Mind may know of any app of the person's: the backend and where it is,
//! the queue's shape, the pictures' names, seeds and sizes, the notice. Reading the folder again
//! is not content and is not refused. Cancelling is: it stops the person's own render, which
//! changes their work, and a Mind's copy may only look.

use crate::command::Command;

/// A command a Mind's window cannot run, by the framework's read-only rule:
/// - typing would put a sentence in a box the window cannot show;
/// - `generate`, `variations` and `upscale` write files as the person (and send a sentence to a
///   backend that may charge for it), `delete` moves them, and `set_backend` writes the person's
///   settings file;
/// - `open` and the folder button hand a file to the person's desktop;
/// - cancelling stops a render the person asked for.
pub fn refuses(command: &Command) -> bool {
    matches!(
        command,
        Command::Edit(..)
            | Command::Generate(_)
            | Command::Variations { .. }
            | Command::Upscale { .. }
            | Command::Open { .. }
            | Command::OpenFolder
            | Command::Delete { .. }
            | Command::SetBackend(_)
            | Command::Cancel(_)
    )
}
