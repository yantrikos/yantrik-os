//! Which paths an agent is told (#443).
//!
//! The person sees every path: a library folder, the file mpv is playing. An agent is told a
//! path only when it is a file in the person's home outside its protected places, by the rule
//! every side shares (`yantrik_ipc_contracts::home_paths`), even a file the person is playing.
//! Any other path reads `(hidden)`.
//!
//! The rule reads the disk, so it is asked in the job that came across the path (the scan for
//! the folders, the player's report for the file it plays), and the verdict is kept beside the
//! path. What an agent is shown is decided from the verdict; a path with none yet is hidden.

use yantrik_ipc_contracts::home_paths;

/// What an agent reads in place of a path it may not see.
pub const HIDDEN: &str = "(hidden)";

/// Whether an agent may be told `path`. Reads the disk: from a job only.
pub fn for_agents(path: &str) -> bool {
    home_paths::may_read_file(path, &yantrik_foundation::place::home()).is_ok()
}

/// Refuse an agent's `play_file` of anything but a file in the home outside its protected
/// places, in the rule's own words. Reads the disk: from a job only, before the path is looked
/// at, so a refusal says nothing about whether a file is there.
pub fn may_play(path: &str) -> Result<(), String> {
    // The rule's sentence quotes the path the agent gave, which is untrusted: one line, plain.
    home_paths::may_read_file(path, &yantrik_foundation::place::home()).map_err(|why| crate::library::plain(&why))
}

/// `path` as this caller may see it, given whether agents may (`may`).
pub fn shown(path: &str, agent: bool, may: bool) -> String {
    if agent && !may {
        HIDDEN.to_string()
    } else {
        path.to_string()
    }
}
