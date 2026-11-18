//! Email as drawn for a Mind (`Boot::for_mind`): Mind View, where `read_mind_view` turns every line
//! on the window into text for an agent.
//!
//! The window shows the framework's one sentence (`yantrik_app::for_mind::HIDDEN`) in place of
//! every sender, address, subject, preview, body, summary and attachment name, in the list, the
//! reader and the thread. `describe` withholds the same in Private mode, so the window gives no
//! more than it does. Folders, counts, dates and flags stay.
//!
//! Visual only: `describe` and the intents are unchanged by `for_mind`.
//!
//! Where it would carry mail out, the command is refused ([`refuses`]): send, reply and forward.
//! A search matches nothing, since nothing it could match is shown. The person's saved draft is
//! never opened, saved over or cleared from here, and nothing the model says is drawn.

use crate::command::Command;

/// A command a Mind's window cannot run, by the framework's read-only rule:
/// - every ask of the companion (summary, suggested reply, draft, enhance, classify) sends the
///   message or the composer to the model, and what comes back is drawn in the window, in a notice
///   that can quote the mail, or over the composer's body;
/// - Open original writes the sender's HTML to a file and opens it;
/// - Send, Reply and Forward carry mail out or quote it;
/// - Save draft and the composer's own edits would write over the person's draft with text the
///   window cannot show;
/// - Delete, Archive and the account setup change the mailbox or the person's accounts, and the
///   intents offer none of them.
///
/// What the intents offer (open, search, mark read, flag, select folder, compose, sign in) is not
/// listed, so it runs as it does anywhere.
pub fn refuses(command: &Command) -> bool {
    matches!(
        command,
        Command::OpenOriginal
            | Command::Send(_)
            | Command::Reply { .. }
            | Command::Forward
            | Command::SaveDraft(_)
            | Command::Edited { .. }
            | Command::Ask(_)
            | Command::Delete(_)
            | Command::Archive(_)
            | Command::AddAccount
            | Command::TestConnection(_)
            | Command::SaveAccount(_)
    )
}
