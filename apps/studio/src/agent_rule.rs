//! Which files an agent may name to Studio (#443).
//!
//! Studio runs as the person. `upscale` copied any picture the person can read into the gallery,
//! where every image tool reads it (the screenshots in `~/Pictures/Screenshots` included),
//! `open` handed any picture to the machine's viewer, `variations` read the record beside any
//! picture (security review of #942, finding 1). `set_backend`, which also read any file as a
//! workflow, is no longer an agent's to call at all: the framework refuses it outright
//! (`Agents::Refused`, intents.rs), so no rule about its files is needed here.
//! An agent now names only files in the person's home outside its protected places, by the rule
//! every side shares (`yantrik_ipc_contracts::home_paths`). A bare filename names a picture in the
//! gallery, which is Studio's own, and is resolved there as before. The person is not asked.
//!
//! The rule reads the disk (whether the path is there, a file, a link), so it is asked from the
//! job that resolves the name, never on the window's thread.

use std::path::PathBuf;

use yantrik_ipc_contracts::home_paths;

/// The argument of each action that names a file, if it takes one. `delete` is not here: it
/// moves only Studio's own pictures, inside the gallery, whoever asks.
pub fn names_a_file(action: &str) -> bool {
    matches!(action, "upscale" | "open" | "variations")
}

/// Whether this caller may run `action` naming `named`. `agent` is who asked, as the command knew
/// it (`Origin::Surface { agent: true }`); the person may name any file.
pub fn admit(agent: bool, action: &str, named: &str) -> Result<(), String> {
    admit_in(agent, action, named, &PathBuf::from(std::env::var_os("HOME").unwrap_or_default()))
}

/// [`admit`] with the home folder given, so a test can use a folder of its own.
pub fn admit_in(agent: bool, action: &str, named: &str, home: &std::path::Path) -> Result<(), String> {
    if !agent || !names_a_file(action) {
        return Ok(());
    }
    let named = named.trim();
    // Empty is each action's own refusal; a bare name is a picture in the gallery.
    if named.is_empty() || !(named.contains('/') || named.starts_with('~')) {
        return Ok(());
    }
    home_paths::may_read_file(named, home)
        .map_err(|why| format!("an agent names only a file in the person's home, outside its protected places: {why}"))
}
