//! What the shell and Files will not run or open from a workshop (#1120 "Planting"; #1124, H2).
//!
//! A Mind builds in its workshop with no card per command, so it can write there whatever it
//! likes: a `package.json` with a `preinstall` hook, a `.git/config` with `core.fsmonitor`, an
//! `.envrc`, a `.desktop` file, a script. Written there it is inert. What turns it into harm is the
//! next, unsealed step: a card for `npm install` whose command line looks harmless, a prompt that
//! runs `git status` in the folder, a double-click in Files. This is the rule that stops the
//! steps the shell takes itself:
//!
//! - **`agent_run` (unsealed)** is refused when its working directory is inside a workshop, or its
//!   command names one. Nothing is carded instead: a card whose line hides a payload is the very
//!   thing that fails, and the sealed path is the Mind's way to work there.
//! - **Files and the launcher** will not open anything that came from a workshop in an app that
//!   runs it (a desktop entry's `Exec`, the browser), or launch a `.desktop` shortcut that did.
//!   Reading it in the Editor, the Images app or the media player is allowed: those show it.
//!
//! "From a workshop" is [`planted`]: under a folder the record grants, or carrying the shell's
//! xattr (`yantrik_agent_terminal::planted`), which follows a file moved out. The location decides
//! first, because a sealed command can write anything inside the folder but cannot move it.
//!
//! What this does not reach: the person's own interactive shell. A prompt that runs `git status`
//! inside a workshop is theirs, and nothing in the shell stands between them and their terminal.

use std::path::Path;

use crate::workshop;

const WHY: &str = "It is in, or came from, a Mind's workshop, and what a Mind writes there is not run unsealed or \
                   opened as an app without the person reading it first. Open it in the Editor to read it; \
                   a Mind builds there with `run_sealed_command`.";

/// Why `path` counts as made in a workshop, if it does.
pub fn planted(record: &Path, path: &Path) -> Option<String> {
    let real = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    for g in workshop::load(record).unwrap_or_default() {
        let folder = Path::new(&g.folder);
        if path.starts_with(folder) || real.starts_with(folder) {
            return Some(format!("{} is inside {}'s workshop.", path.display(), g.agent));
        }
    }
    yantrik_agent_terminal::marked_or_under_marked(&real)
        .then(|| format!("{} carries the mark of a Mind's workshop.", path.display()))
}

/// For an unsealed command: refuse a working directory in a workshop, and a command line that
/// names one.
pub fn refuse_unsealed(record: &Path, command: &str, cwd: &Path) -> Result<(), String> {
    if let Some(what) = planted(record, cwd) {
        return Err(format!(
            "{what} An unsealed command does not start in a workshop: use `run_sealed_command` there. {}",
            "If it needs the network, say so to the person and let them decide; nothing was run."
        ));
    }
    for g in workshop::load(record).unwrap_or_default() {
        let name = Path::new(&g.folder).file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if command.contains(&g.folder) || (!name.is_empty() && command.contains(&format!("Documents/{name}"))) {
            return Err(format!(
                "this command names {}'s workshop ({name}). What is in a workshop is not run unsealed: \
                 use `run_sealed_command`. Nothing was run.",
                g.agent
            ));
        }
    }
    Ok(())
}

/// For Files and the launcher: why `path` may not be opened in an app that runs it.
pub fn refuse_run_or_open_as_app(record: &Path, path: &Path) -> Option<String> {
    planted(record, path).map(|what| format!("{what} {WHY}"))
}

/// The same, against the shell's own record.
pub fn refuse_open(path: &Path) -> Option<String> {
    refuse_run_or_open_as_app(&workshop::record_path(), path)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let h = tempfile::tempdir().unwrap();
        std::fs::create_dir(h.path().join("Documents")).unwrap();
        let rec = h.path().join("workshops.json");
        let shop = workshop::check_folder(h.path(), "~/Documents/shop", true).unwrap();
        workshop::grant(&rec, "hermes", &shop, 1).unwrap();
        (h, rec, shop)
    }

    #[test]
    fn an_unsealed_command_does_not_start_in_a_workshop_or_name_one() {
        let (h, rec, shop) = setup();
        std::fs::create_dir(shop.join("app")).unwrap();
        assert!(refuse_unsealed(&rec, "npm install", &shop).is_err());
        assert!(refuse_unsealed(&rec, "npm install", &shop.join("app")).is_err());
        // From elsewhere, naming it by path or by the Documents form.
        let elsewhere = h.path().join("Documents");
        assert!(refuse_unsealed(&rec, &format!("bash {}/build.sh", shop.display()), &elsewhere).is_err());
        assert!(refuse_unsealed(&rec, "cd ~/Documents/shop && npm i", &elsewhere).is_err());
        assert!(refuse_unsealed(&rec, "cd $HOME/Documents/shop; git status", &elsewhere).is_err());
        // An ordinary command somewhere else is untouched.
        assert!(refuse_unsealed(&rec, "ls", &elsewhere).is_ok());
        assert!(refuse_unsealed(&rec, "cd ~/Documents/shop2 && ls", &elsewhere).is_err(), "a prefix of a longer name is refused too: the rule over-refuses, never under");
    }

    #[test]
    fn nothing_from_a_workshop_opens_as_an_app() {
        let (h, rec, shop) = setup();
        std::fs::write(shop.join("x.desktop"), "[Desktop Entry]\nExec=sh -c evil\n").unwrap();
        assert!(refuse_run_or_open_as_app(&rec, &shop.join("x.desktop")).unwrap().contains("workshop"));
        assert!(refuse_run_or_open_as_app(&rec, &shop.join("not-yet.desktop")).is_some());
        let mine = h.path().join("Documents/mine.txt");
        std::fs::write(&mine, "x").unwrap();
        assert!(refuse_run_or_open_as_app(&rec, &mine).is_none(), "the person's own files are not touched");
    }

    #[test]
    fn a_marked_file_moved_out_is_still_a_workshop_file() {
        let (h, rec, shop) = setup();
        let f = shop.join("made.desktop");
        std::fs::write(&f, "x").unwrap();
        if !yantrik_agent_terminal::mark(&f) {
            eprintln!("skipped: this filesystem takes no user xattrs");
            return;
        }
        let out = h.path().join("Documents/made.desktop");
        std::fs::rename(&f, &out).unwrap();
        assert!(refuse_run_or_open_as_app(&rec, &out).unwrap().contains("mark"));
    }
}
