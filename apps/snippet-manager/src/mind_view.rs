//! Snippets as drawn for a Mind (`Boot::for_mind`): Mind View, where `read_mind_view` turns every
//! line on the window into text for an agent.
//!
//! The window shows the framework's `HIDDEN` where a snippet's code would be, in the editor and in the list's
//! previews and rows, and keeps titles, tags and language. Nothing there can change or copy the
//! code: the commands that would are refused ([`refuses`]).

use yantrik_app::rail::RailCommand;

use crate::command::Command;

/// The code the window shows: the code itself for the person, and for a Mind the one sentence the
/// framework gives every app (`yantrik_app::for_mind::HIDDEN`).
pub fn shown(for_mind: bool, code: &str) -> String {
    yantrik_app::for_mind::shown(for_mind, code).into_owned()
}

/// A command a Mind's window cannot run (the framework's read-only rule refuses it). Typing and
/// saving would write the masked text over the real code; export writes every snippet to a file as the
/// person; creating, renaming and deleting a collection change the library, and no intent offers
/// them; copying and explaining would carry the
/// real code out of the window. The rail's chip sends the code to the Mind in its prompt; the rail
/// offers nothing here, and this is the second lock on the same door.
pub fn refuses(command: &Command) -> bool {
    matches!(
        command,
        Command::Edited(_)
            | Command::Save(_)
            | Command::Copy(_)
            | Command::CopyNamed(_)
            | Command::ExportAll
            | Command::CollectionCreate(_)
            | Command::CollectionRename(..)
            | Command::CollectionDelete(_)
            | Command::AiExplain
            | Command::Rail(RailCommand::Activate(_))
    )
}
