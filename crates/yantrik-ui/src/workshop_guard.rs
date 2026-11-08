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

/// Hand the companion's tools the rule, so that none of them starts a program in a workshop
/// (`yantrik_companion_tools::workshop_gate`; #1124, N1). Called once at start.
pub fn install_for_tools() {
    yantrik_companion_tools::workshop_gate::install(|path| planted(&workshop::record_path(), path));
    // And the file tools read nothing under a tainted workshop (N4).
    yantrik_companion_tools::install_tainted_read_refusal(|path| refuse_tainted_read(&workshop::record_path(), path));
}

/// Why `path` may not be read by a Mind or the companion: it is under a workshop that holds what a
/// conversation which had read private state wrote (#1124, N4). Read there, it would put injected
/// text into a conversation that is not marked tainted, and its next `curl` would go unasked in
/// `auto`. The sealed door reads it and taints the reader (`agent_run_sealed`); this refusal needs
/// no attribution of the caller, which a tokenless bridge does not have. The person's own reading
/// (Files, the Editor) is not a Mind's and is not refused here.
pub fn refuse_tainted_read(record: &Path, path: &Path) -> Option<String> {
    let real = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    workshop::tainted_folders(record).into_iter().find_map(|folder| {
        let folder = Path::new(&folder);
        (path.starts_with(folder) || real.starts_with(folder)).then(|| {
            "that is in a workshop that holds what a conversation which had read private state wrote; \
             read it with `agent_run_sealed`, which marks the conversation that does. Nothing was read."
                .to_string()
        })
    })
}

/// Does an unsealed command, started in `cwd`, reach a tainted workshop's files? True when it starts in
/// the workshop's folder or above it, or names the folder or the folder it is in, or the home with
/// a tool that walks it. Over-inclusive on purpose: the answer only taints the conversation.
pub fn reaches_tainted(record: &Path, home: &Path, command: &str, cwd: &Path) -> Option<String> {
    let walks = ["-r", "-R", "find ", "rg ", "grep", "ag ", "tree", "locate"].iter().any(|t| command.contains(t));
    for folder in workshop::tainted_folders(record) {
        let folder = Path::new(&folder);
        let name = folder.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        let parent = folder.parent().unwrap_or(folder);
        let parent_rel = parent.strip_prefix(home).ok().and_then(|p| p.to_str()).unwrap_or_default();
        let names = |p: &Path, rel: &str| {
            command.contains(&p.to_string_lossy().to_string())
                || (!rel.is_empty() && (command.contains(&format!("~/{rel}")) || command.contains(&format!("$HOME/{rel}")) || command.contains(&format!("${{HOME}}/{rel}"))))
        };
        let home_walk = walks && ["~", "$HOME", "${HOME}"].iter().any(|h| command.contains(h)) || (walks && command.contains(&*home.to_string_lossy()));
        if folder.starts_with(cwd) || names(folder, &format!("{parent_rel}/{name}")) || names(parent, parent_rel) || home_walk {
            return Some(format!("workshop {name} (it holds what a conversation that had read private state wrote)"));
        }
    }
    None
}

/// The red line for the card of an unsealed command, or `None`: the commands the shell runs or
/// types unsealed (`agent_run`, `agent_input`, the Terminal's `run` and the rest), from a tainted
/// conversation, while the Mind holds a workshop or any workshop is tainted. Whether the command
/// touches a workshop cannot be told from its text (`cd ~/Doc*/sh*`), so the person is told what
/// the risk is, and is given no standing yes (#1124, N3).
pub fn unsealed_card_note(record: &Path, app: &str, action: &str, agent: &str, conversation_tainted: bool) -> Option<String> {
    let unsealed = (app == "shell" && matches!(action, "agent_run" | "agent_input")) || app == "terminal";
    if !unsealed || !conversation_tainted {
        return None;
    }
    let held = workshop::load(record).unwrap_or_default().iter().any(|g| g.agent == agent);
    (held || !workshop::tainted_folders(record).is_empty()).then(|| {
        "This runs outside the sandbox, and files a Mind wrote may run with it: a package hook, a git setting, \
         a script in a workshop. Read what it does, not only its name."
            .to_string()
    })
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
    fn a_tainted_workshop_is_not_read_and_a_command_that_may_reach_it_is_noticed() {
        let (h, rec, shop) = setup();
        std::fs::write(shop.join("notes.md"), "x").unwrap();
        // Not tainted yet: readable.
        assert!(refuse_tainted_read(&rec, &shop.join("notes.md")).is_none());
        workshop::mark_tainted(&rec, "hermes").unwrap();
        assert!(refuse_tainted_read(&rec, &shop.join("notes.md")).unwrap().contains("agent_run_sealed"));
        assert!(refuse_tainted_read(&rec, &shop.join("not-yet.md")).is_some());
        assert!(refuse_tainted_read(&rec, &h.path().join("Documents")).is_none(), "the folder above is not under it");
        // A link out of the home into it is judged by where it leads.
        std::os::unix::fs::symlink(shop.join("notes.md"), h.path().join("l.md")).unwrap();
        assert!(refuse_tainted_read(&rec, &h.path().join("l.md")).is_some());
        // Commands: the grep over the Documents, the folder above, the cwd above; not an unrelated one.
        let docs = h.path().join("Documents");
        assert!(reaches_tainted(&rec, h.path(), "grep -ri meeting ~/Documents", h.path()).is_some());
        assert!(reaches_tainted(&rec, h.path(), "grep -ri meeting .", &docs).is_some());
        assert!(reaches_tainted(&rec, h.path(), "grep -r x ~", &docs.join("elsewhere")).is_some());
        assert!(reaches_tainted(&rec, h.path(), "ls /usr", &h.path().join("Downloads")).is_none());
    }

    #[test]
    fn an_unsealed_card_from_a_tainted_conversation_warns_when_a_workshop_is_in_play() {
        let (_h, rec, _shop) = setup();
        let note = unsealed_card_note(&rec, "shell", "agent_run", "hermes", true).unwrap();
        assert!(note.contains("a Mind wrote"));
        assert!(unsealed_card_note(&rec, "terminal", "run", "other", true).is_none(), "no workshop tainted, none held");
        assert!(unsealed_card_note(&rec, "shell", "agent_run", "hermes", false).is_none(), "an untainted conversation");
        assert!(unsealed_card_note(&rec, "shell", "agent_run_sealed", "hermes", true).is_none());
        workshop::mark_tainted(&rec, "hermes").unwrap();
        assert!(unsealed_card_note(&rec, "terminal", "run", "other", true).is_some(), "a workshop is tainted");
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
