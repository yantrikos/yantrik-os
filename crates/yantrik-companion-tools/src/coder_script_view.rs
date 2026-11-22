//! What a script run is given (#1265, rounds 10 and 11).
//!
//! - **Places, always bound.** `deps/python` and `deps/node_modules` of the account's dependencies
//!   folder are made when missing (through the descriptor code of `coder_promote`, under its lock) and
//!   bound read-only over `<scripts>/.deps` and `<scripts>/node_modules` on every run: a bind is never
//!   skipped, so those names are never writable folders of the workspace. Each destination is checked
//!   first (`lstat`): a link, or anything that is not a folder, refuses the run.
//! - **The scripts folder is read-only** in the sandbox (`spawn::build_bwrap_args`), only the output
//!   folder is writable; Python runs with `PYTHONSAFEPATH=1`.
//! - **The environment** is the allow list plus the door's own named variables, and the Mind's own
//!   `env` after the manifest's drop list and `PATH` (`mind_env`); the door's names win.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::coder_deps::{deps_dir, deps_root_in, promoted_to, staging_in};

/// The two places of the scripts folder that the dependencies are bound over.
const DESTINATIONS: [&str; 2] = [".deps", "node_modules"];

/// What a script run is given besides the allow list: the dependencies as read-only binds (`source`,
/// `dest` in the sandbox) and the environment's extra names.
pub struct ScriptView {
    pub reads: Vec<(PathBuf, PathBuf)>,
    pub extra: Vec<(String, String)>,
}

/// The Mind's own `env` for a script, kept and dropped: the names that change where a program loads
/// code or settings from (`view_env::from_manifest`) and `PATH` are dropped, and each is logged.
pub fn mind_env(env: &HashMap<String, String>) -> Vec<(String, String)> {
    let (kept, mut dropped) = yantrik_ipc_contracts::home_paths::view_env::from_manifest(env.clone());
    let (kept, path): (Vec<_>, Vec<_>) = kept.into_iter().partition(|(k, _)| k != "PATH");
    dropped.extend(path.into_iter().map(|(k, _)| k));
    for name in &dropped {
        tracing::warn!(variable = %name, "A script_run env variable was dropped: it changes where a program loads code or settings from");
    }
    kept
}

/// Refuse a bind destination of the workspace that is a link or not a folder; make a missing one.
fn check_destinations(scripts: &Path) -> Result<(), String> {
    let dir = crate::coder_dirfd::Dir::open(scripts)?;
    for name in DESTINATIONS {
        let place = scripts.join(name);
        match place.symlink_metadata() {
            Ok(m) if m.file_type().is_dir() => {}
            Ok(_) => return Err(format!("{} is a link or not a folder, so no script was run; remove it and run again", place.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => dir.mkdir(OsStr::new(name), 0o755)?,
            Err(e) => return Err(format!("{}: {e}", place.display())),
        }
    }
    Ok(())
}

/// The view of a script run for the account `home`, with `scripts` its workspace, `output` its
/// writable folder and `mind` the `env` the Mind passed; or why the run must not start.
pub fn script_view(home: &Path, output: &Path, scripts: &Path, language: &str, mind: &HashMap<String, String>) -> Result<ScriptView, String> {
    crate::coder_promote::ensure_folders(&deps_root_in(home), &["python", "node_modules"], &staging_in(home))
        .map_err(|why| format!("Script run refused: the dependencies folder could not be made ready: {why}"))?;
    check_destinations(scripts).map_err(|why| format!("Script run refused: {why}"))?;
    let reads = [("python", deps_dir(scripts)), ("node", scripts.join("node_modules"))].map(|(l, dest)| (promoted_to(home, l), dest)).to_vec();
    let text = |p: &Path| p.to_string_lossy().into_owned();
    // The Mind's names first: the door's own, after them, win.
    let mut extra = mind_env(mind);
    extra.extend([("YANTRIK_OUTPUT_DIR".to_string(), text(output)), ("YANTRIK_SCRIPTS_DIR".to_string(), text(scripts))]);
    if language == "python" {
        extra.extend([("PYTHONPATH".to_string(), text(&deps_dir(scripts))), ("PYTHONSAFEPATH".to_string(), "1".to_string())]);
    }
    Ok(ScriptView { reads, extra })
}
