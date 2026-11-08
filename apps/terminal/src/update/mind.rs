//! What a Mind may do through Terminal (#1124, H1).
//!
//! The person's own shell sees the workshops root and has `sudo`; a Mind's input must never reach
//! it. A Mind's `new_tab` and `open_directory` start a shell in the outside view (`outside.rs`),
//! and its `run` and `send_input` go only to a tab that was started that way. Into any other tab
//! they are refused, in a sentence that says what to do. The command's text is never read: a `cd`
//! in the same line, or a directory that changes after a check, defeats any such match, and the
//! view makes it unnecessary.

use yantrik_app::effects::Effects;

use super::{refuse, surface, tabs};
use crate::command::{Command, Why};
use crate::model::Model;
use crate::Terminal;

type Fx = Effects<Terminal>;

/// The sentence for input aimed at a shell the Mind did not open.
pub const NOT_YOURS: &str = "This tab is the person's own shell, which a Mind does not type into. \
    `new_tab` starts one of your own (a shell that cannot see Minds' workshops and has no sudo), \
    and `run` and `send_input` go to it once it is the active tab. Nothing was sent.";

/// Is the active tab one a Mind may type into?
pub fn active_is_outside(m: &Model) -> bool {
    m.tab().is_some_and(|t| t.outside)
}

pub fn apply(m: &mut Model, fx: &mut Fx, command: Command) {
    match command {
        Command::Run(_) | Command::SendInput(_) if m.tab().is_some() && !active_is_outside(m) => refuse(m, fx, NOT_YOURS),
        Command::Run(line) => surface::run(m, fx, line),
        Command::SendInput(text) => surface::send_input(m, fx, text),
        Command::NewTab => tabs::new_tab(m, fx, Why::NewTab, true),
        Command::OpenDirectory(given) => surface::open_directory(m, fx, given, true),
        _ => refuse(m, fx, "That is not something a Mind asks Terminal for."),
    }
}
