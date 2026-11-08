//! The companion's tools that start a program in a folder do not start one in a Mind's workshop
//! (#1120; #1124, N1).
//!
//! A Mind writes what it likes in its workshop: a `.git/config` with `core.fsmonitor`, a
//! `package.json` with a `preinstall` hook. Written there it is inert. A companion tool that runs
//! `git status` or a script in that folder runs it, unsealed, as the person, with the network and
//! no card. This crate does not know where the workshops are, so the shell installs the rule
//! once at start ([`install`]) and every tool that spawns a program with a working directory asks
//! [`refuse`] first. Before the shell installs it (a test, a tool run outside the shell) there is
//! no workshop to protect.

use std::path::Path;
use std::sync::OnceLock;

type Rule = Box<dyn Fn(&Path) -> Option<String> + Send + Sync>;

static RULE: OnceLock<Rule> = OnceLock::new();

/// Install the shell's rule: `Some(why)` when the path is in, or came from, a workshop. The first
/// call wins.
pub fn install(rule: impl Fn(&Path) -> Option<String> + Send + Sync + 'static) {
    let _ = RULE.set(Box::new(rule));
}

/// Why `dir` is a workshop (or in one), if it is.
pub fn in_workshop(dir: &str) -> Option<String> {
    RULE.get().and_then(|rule| rule(Path::new(dir)))
}

/// `Err(message)` for a tool to return as is when `dir` is in a workshop.
pub fn refuse(dir: &str) -> Result<(), String> {
    match in_workshop(dir) {
        Some(why) => Err(format!(
            "Error: {why} A tool does not run a program in a Mind's workshop; what a Mind wrote there is not \
             run unsealed. Nothing was run."
        )),
        None => Ok(()),
    }
}
