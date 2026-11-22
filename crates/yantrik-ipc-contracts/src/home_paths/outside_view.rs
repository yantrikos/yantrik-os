//! What a command a Mind drives sees of the person's home (#1265).
//!
//! The guard is the view, not the command's words: a check over text fails open (a glob, a variable
//! or a `cd` gets past it). In the outside view the home is a fresh tmpfs, and only these are put
//! back in it:
//!
//! - **read-write:** the home's top-level entries that are not hidden (no leading `.`, and not
//!   `bin` or `snap`, the lines `writes.rs` draws for writes) and are not links. `Pictures` and
//!   `Videos` are put back child by child, every child but `Screenshots` and `Recordings`, so no
//!   mount point is ever made in the person's own folders and nothing masks a place a `reach` names;
//! - **read-only:** the named [`TOOLCHAIN`] list (`view_places`), and git's shown settings
//!   (`view_git`), which are synthesized, never the files, and the environment (`view_env`): cleared, then an allow list;
//! - **read-only, for one job:** a place the call named in `reach`, which the person was asked for,
//!   bound from a descriptor opened without following a link ([`held`](super::held)). A place inside
//!   a visible entry is not a `reach` place: it is already visible to the command (or, for a
//!   protected one such as `Pictures/Screenshots`, never shown), and it is the only kind another
//!   command of the same Mind could swap in the moment before `bwrap` binds it.
//!
//! Everything else is absent, so an unknown `~/.newthing` is as absent as `~/.ssh`. A link at a
//! protected place inside a visible folder (`Pictures/Screenshots`, resolving or not) stops the
//! command with a sentence naming it. `/tmp` is a private tmpfs per job: the host's `/tmp` is never
//! bound, and a working directory outside the home is one the job makes in its own. `.cache` is not
//! in the home at all. Every judgment here uses the account's home it is given, never `$HOME`.

use std::path::{Path, PathBuf};

use super::held::{open_beneath, where_is, Held};
use super::view_git::git_words;
pub use super::view_places::{account_home, TOOLCHAIN};
#[cfg(any(test, feature = "test-seams"))]
pub use super::view_places::account_home_for_tests;
use super::view_places::{toolchain_words, Seen, Verdict};
use super::{is_protected_in, where_programs_look, PROTECTED};

/// What a failed command is told when it touched a place the view does not hold.
pub const MISSING_HINT: &str =
    "A hidden place in the home (a dot-folder: settings, keys, app data) is not in this command's view; to use one, run it with `agent_run_reach` (`run_command_with_reach` in a tool list) and the person is asked.";

/// The part of the tool text that says what a command sees, and what it can still change.
pub const VIEW_TEXT: &str =
    "A command sees your visible folders. Your settings, keys and app data are not there. \
    A command can change files in the visible folders, including a project's own settings and hooks \
    (`.git/hooks`, `.envrc`, editor tasks), which run later as you. To use a hidden place, ask the person.";

/// `Some(MISSING_HINT)` for a command that exited non-zero with a "No such file" about a path with a
/// hidden part in it. Advice for a Mind, never the guard.
pub fn hint_for(code: i32, tail: &str) -> Option<&'static str> {
    let missing = tail.lines().any(|l| {
        l.contains("No such file or directory") && (l.contains("/.") || l.contains("~/."))
    });
    (code != 0 && missing).then_some(MISSING_HINT)
}

fn text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn bind(flag: &str, from: &Path, to: &Path) -> [String; 3] {
    [flag.to_string(), text(from), text(to)]
}

/// Whether a top-level name of the home is shown: not hidden, not `bin` or `snap` (every snap
/// app's data is in `snap`), not a protected name, and not a side file of one (`memory.db-wal`,
/// `memory.db-shm`, `memory.db-journal`).
fn shown(name: &str) -> bool {
    let lower = name.to_lowercase();
    let protected_name = PROTECTED.iter().any(|p| {
        !p.contains('/') && (lower == *p || lower.strip_prefix(*p).is_some_and(|rest| rest.starts_with(['-', '.'])))
    });
    !lower.starts_with('.') && lower != "bin" && lower != "snap" && !protected_name
}

/// The top-level entries of `home` that a command sees, read-write.
pub fn visible_entries(home: &Path) -> Vec<PathBuf> {
    let Ok(dir) = std::fs::read_dir(home) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = dir
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(shown))
        // A link is skipped, not followed: `bwrap` follows one in a bind's source when it mounts, so
        // an entry swapped for a link between this look and the mount would be bound from its target.
        .filter(|e| e.file_type().is_ok_and(|t| !t.is_symlink()))
        .map(|e| e.path())
        .filter(|p| !where_programs_look(p, home) && !is_protected_in(p, Some(home)))
        .collect();
    out.sort();
    out
}

/// The places of `PROTECTED` that sit inside a visible folder, as (folder, place) in the list's own
/// lower case (`pictures`, `screenshots`). Always two parts: a test holds the list to that, because
/// the view is made child by child for exactly these. A place whose first part is not shown (`snap`)
/// is not here: it is not in the view at all.
fn inside_places() -> Vec<(&'static str, &'static str)> {
    PROTECTED
        .iter()
        .filter(|p| p.contains('/') && !p.starts_with('.'))
        .filter_map(|p| p.split_once('/'))
        .filter(|(folder, _)| shown(folder))
        .collect()
}

/// Whether a name is, in any case, one of the protected places in the folder `folder` (spelled as on
/// disk).
fn protected_child(folder: &str, name: &str) -> bool {
    let (folder, name) = (folder.to_lowercase(), name.to_lowercase());
    inside_places().iter().any(|(f, place)| *f == folder && *place == name)
}

/// The protected places inside a visible folder as they are on disk, whatever they are (a folder, a
/// file, a link that resolves or one that dangles), in any case.
pub fn masked_places(home: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(home) else { return Vec::new() };
    let mut out = Vec::new();
    for top in read.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())) {
        let folder = top.file_name().to_string_lossy().into_owned();
        let Ok(inner) = std::fs::read_dir(top.path()) else { continue };
        out.extend(inner.flatten().filter(|c| protected_child(&folder, &c.file_name().to_string_lossy())).map(|c| c.path()));
    }
    out
}

/// The most loose files bound out of one of the split folders; over it, the loose files are hidden
/// and the command sees the folders only. Never a reason to refuse a command.
pub const MAX_CHILD_FILES: usize = 256;
/// The most child folders bound out of one of the split folders (they are words on `bwrap`'s command
/// line); the rest, in name order, are hidden.
pub const MAX_CHILD_FOLDERS: usize = 2048;

/// A folder made child by child: the words that open it, the children as (look again, words), and
/// the words that close it.
struct Split {
    open: Vec<String>,
    kids: Vec<(Seen, [String; 3])>,
    close: Vec<String>,
}

/// A folder that a view makes child by child: its protected places are left out, so nothing is made
/// in the person's folder and no mask can conflict with a `reach`. The folder is a tmpfs of the
/// view with each child bound into it, then remounted read-only, so a file a command writes directly
/// in it fails loudly instead of being kept in the view only. Only child **folders** are bound, and
/// at most [`MAX_CHILD_FILES`] loose files (none over that); a link is neither bound nor followed.
/// `Err` when a protected place in it is a link, whether it resolves or leads nowhere: a view that
/// did not say so would show what the link leads to.
fn child_by_child(home: &Path, entry: &Path) -> Result<Split, String> {
    let folder = entry.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut split = Split { open: vec!["--tmpfs".to_string(), text(entry)], kids: Vec::new(), close: vec!["--remount-ro".to_string(), text(entry)] };
    let Ok(read) = std::fs::read_dir(entry) else { return Ok(split) };
    let mut children: Vec<_> = read.flatten().collect();
    children.sort_by_key(|c| c.file_name());
    let (mut folders, mut files) = (Vec::new(), Vec::new());
    for child in children {
        let (path, name) = (child.path(), child.file_name().to_string_lossy().into_owned());
        let kind = child.file_type();
        if protected_child(&folder, &name) {
            if kind.as_ref().is_ok_and(|t| t.is_symlink()) {
                return Err(format!(
                    "Error: {} is a link, and a command is not started with a protected place that is one (it is neither shown nor hidden by a link). Nothing was run.",
                    path.display()
                ));
            }
            continue;
        }
        let Ok(kind) = kind else { continue };
        if kind.is_symlink() || is_protected_in(&path, Some(home)) {
            continue;
        }
        match kind.is_dir() {
            true => folders.push(path),
            false => files.push(path),
        }
    }
    folders.truncate(MAX_CHILD_FOLDERS);
    if files.len() > MAX_CHILD_FILES {
        files.clear();
    }
    for path in folders.into_iter().chain(files) {
        if let Some(at) = Seen::of(&path) {
            let words = bind("--bind-try", &path, &path);
            split.kids.push((at, words));
        }
    }
    Ok(split)
}

/// Where a working directory is allowed, or why not. `Ok(Some(dir))` is a directory the job makes
/// in its own private `/tmp`; the host's is never bound. A working directory is the home itself, a
/// place **inside a shown entry** (the list [`visible_entries`] makes, not whatever is not on a deny
/// list), or a directory inside the job's own `/tmp`; where it really is decides, whatever its text
/// says.
fn check_cwd(home: &Path, cwd: &Path) -> Result<Option<PathBuf>, String> {
    let refuse = |why: &str| Err(format!("Error: {}: {why}. Nothing was run.", cwd.display()));
    let (tmp, real_home) = (Path::new("/tmp"), home.canonicalize().unwrap_or_else(|_| home.to_path_buf()));
    let real = cwd.canonicalize().ok();
    let in_home = cwd.starts_with(home) || real.as_ref().is_some_and(|r| r.starts_with(&real_home));
    if cwd.starts_with(tmp) && !in_home {
        if cwd.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return refuse("a working directory under /tmp is named with no `..`");
        }
        let tmp_real = tmp.canonicalize().unwrap_or_else(|_| tmp.to_path_buf());
        if real.as_ref().is_some_and(|r| !r.starts_with(&tmp_real)) {
            return refuse("it leads outside /tmp, and a command's /tmp is its own; run it from a visible folder");
        }
        // A link on the way that leads nowhere is not a place yet: refused, not made in the view.
        if real.is_none() && cwd.ancestors().any(|a| a.symlink_metadata().is_ok_and(|m| m.is_symlink())) {
            return refuse("a link on the way leads nowhere, and a command's /tmp is its own");
        }
        return Ok((cwd != tmp).then(|| cwd.to_path_buf()));
    }
    let real = real.unwrap_or_else(|| cwd.to_path_buf());
    let hidden = |p: &Path| where_programs_look(p, home) || is_protected_in(p, Some(home));
    if in_home {
        let entries = visible_entries(&real_home);
        // Both as written and where it really is: a link on the way is not a place the view shows
        // (the entry is skipped, so the path does not exist in the view), whatever it leads to.
        let written = cwd.strip_prefix(home).map(|rest| real_home.join(rest)).unwrap_or_else(|_| cwd.to_path_buf());
        let inside = |p: &Path| p == real_home || entries.iter().any(|e| p.starts_with(e));
        let shown_place = inside(&real) && inside(&written);
        if hidden(cwd) || hidden(&real) || !shown_place {
            return refuse("it is not in a folder the command's view shows (a hidden or protected place, a link, or a folder the view leaves out); run it from a visible folder");
        }
    }
    if super::workshops::holds(&real, home) {
        return refuse("it is in a Mind's workshop, which a command's view does not hold; run it from a visible folder");
    }
    Ok(None)
}

/// The most places one call may name.
pub const MAX_REACH: usize = 4;

/// A place written as `~` or `~/x` is the home's; anything else is as written.
pub fn expand(home: &Path, written: &str) -> PathBuf {
    let w = written.trim();
    match w.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if w == "~" => home.to_path_buf(),
        None => PathBuf::from(w),
    }
}

/// What a place named in `reach` must not be, judged on the path that is really there: not the home
/// or a workshop, not **inside a shown entry** (it is already visible to the command, or protected
/// and never shown, and it is the one kind another command could swap), and not above another
/// protected place (naming `~/.config` would put `~/.config/gh` and the rest in the view without
/// the card saying so). Compared case-folded, and against the protected places that are on disk as
/// well as the list.
fn check_reach_path(home: &Path, real_home: &Path, real: &Path) -> Result<(), String> {
    let Ok(rest) = real.strip_prefix(real_home) else {
        return Err("only a place inside the home can be named".to_string());
    };
    if rest.as_os_str().is_empty() || super::workshops::holds(real, home) {
        return Err("only a place inside the home can be named".to_string());
    }
    let lower = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());
    let (real_l, home_l) = (lower(real), lower(real_home));
    if masked_places(real_home).iter().any(|m| lower(m).starts_with(&real_l) || real_l.starts_with(lower(m))) {
        return Err("it is a protected place inside a visible folder, which a command is never shown".to_string());
    }
    if visible_entries(real_home).iter().any(|e| real.starts_with(e)) {
        return Err("it is already visible to the command".to_string());
    }
    let holds_listed = PROTECTED.iter().any(|p| {
        let at = home_l.join(p);
        at != real_l && at.starts_with(&real_l)
    });
    match holds_listed {
        true => Err("it holds other protected places; name the one you need".to_string()),
        false => Ok(()),
    }
}

/// A place named in `reach`: a folder or file inside the home, written absolute (or `~/…`), that
/// exists, that **is the place it is written as** (a link is refused, with the place it leads to),
/// and that passes [`check_reach_path`]. The one function the card, the job and the bind all use, so
/// what the person is shown is what is checked; the bind then opens it without following a link
/// and checks what it opened (`view_args`).
pub fn reach_place(home: &Path, place: &Path) -> Result<PathBuf, String> {
    let bad = |why: &str| format!("Error: reach {}: {why}. Nothing was run.", place.display());
    if !place.is_absolute() || place.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(bad("name it as an absolute path with no `..`"));
    }
    let real = place.canonicalize().map_err(|_| bad("it does not exist"))?;
    let real_home = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    let as_written = match place.strip_prefix(home) {
        Ok(rest) => real_home.join(rest),
        Err(_) => place.to_path_buf(),
    };
    if as_written != real {
        return Err(bad(&format!("it is a link, or is spelled differently from where it is; name the place it leads to ({})", real.display())));
    }
    check_reach_path(home, &real_home, &real).map_err(|why| bad(&why))?;
    Ok(real)
}

/// The places of a `reach` argument as the view takes them: at most [`MAX_REACH`], `~` expanded, each
/// checked by [`reach_place`]. `Err` is the sentence to return as is.
pub fn reach_list(home: &Path, written: &[String]) -> Result<Vec<PathBuf>, String> {
    if written.is_empty() || written.len() > MAX_REACH {
        return Err(format!("Error: `reach` names between 1 and {MAX_REACH} places. Nothing was run."));
    }
    written.iter().map(|w| reach_place(home, &expand(home, w))).collect()
}

/// A `reach` place opened for the bind: through the home without following a link, then looked at
/// as the descriptor holds it (where it is, and the checks of [`check_reach_path`] again on that),
/// and kept in `held`. The `--ro-bind-fd` words that bind it.
fn reach_words(home: &Path, place: &Path, held: &mut Held) -> Result<[String; 3], String> {
    let bad = |why: &str| format!("Error: reach {}: {why}. Nothing was run.", place.display());
    let place = reach_place(home, place)?;
    let real_home = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    let fd = open_beneath(&real_home, &place).map_err(|why| bad(&why))?;
    let at = where_is(std::os::fd::AsRawFd::as_raw_fd(&fd)).ok_or_else(|| bad("it was removed while it was opened"))?;
    if at != place {
        return Err(bad("it is not the place that was checked any more"));
    }
    check_reach_path(home, &real_home, &at).map_err(|why| bad(&why))?;
    let number = held.keep(fd).map_err(|why| bad(&why))?;
    Ok(["--ro-bind-fd".to_string(), number.to_string(), text(&place)])
}

/// The `bwrap` words that make the home of this view: `--tmpfs /tmp`, `--tmpfs $HOME`, then what is
/// put back, and the descriptors those words name. `Err` (with the sentence to return as is) for a
/// working directory that is not allowed, a protected place that is a link, or a `reach` that is not
/// a place in the home any more.
///
/// A `reach` place is bound from the descriptor it was opened as, so it is the place that was checked
/// whatever happens to its path after. The top-level entries and the toolchain are in a home that no
/// view can write to (it is a tmpfs of its own in every view), and each is looked at again as the
/// last thing before the words are made, so the time in which any of them could change is the time
/// to make the words, not the time to walk the toolchain.
pub fn view_args(home: &Path, cwd: &Path, reach: &[PathBuf]) -> Result<(Vec<String>, Held), String> {
    let make = check_cwd(home, cwd)?;
    let mut a: Vec<String> = ["--tmpfs", "/tmp"].map(String::from).to_vec();
    if let Some(dir) = make {
        a.extend(["--dir".to_string(), text(&dir)]);
    }
    a.extend(["--tmpfs".to_string(), text(home)]);
    let (mut seen, mut entry_words, mut splits) = (Vec::new(), Vec::new(), Vec::new());
    for entry in visible_entries(home) {
        let split = entry.file_name().is_some_and(|n| inside_places().iter().any(|(f, _)| *f == n.to_string_lossy().to_lowercase()));
        match split {
            true => splits.push(child_by_child(home, &entry)?),
            // `-try`: a file that is gone by the time the view is made (a database's `-shm`, a
            // lock) is not one, and must not stop the command.
            false => entry_words.extend(bind("--bind-try", &entry, &entry)),
        }
        // The split folder too: its children are bound from paths below it.
        seen.extend(Seen::of(&entry));
    }
    let (tool_words, tool_seen) = toolchain_words(home);
    seen.extend(tool_seen);
    let git = git_words(home);
    // The slow part is over. The places named in `reach` are opened now, and every bound entry and
    // toolchain entry is looked at once more, as late as it can be.
    let mut held = Held::default();
    let mut reach_words_out = Vec::new();
    for place in reach {
        reach_words_out.extend(reach_words(home, place, &mut held)?);
    }
    if let Some(moved) = seen.iter().find(|s| !s.unchanged()) {
        return Err(format!("Error: {} changed while the view was being made. Nothing was run.", moved.path().display()));
    }
    a.extend(entry_words);
    // The split folders: each child that is still a plain child is bound, one that is gone is left
    // out (a download in progress), and the folder is then made read-only.
    for split in splits {
        a.extend(split.open);
        for (at, words) in split.kids {
            match at.verdict() {
                Verdict::Same => a.extend(words),
                Verdict::Gone => {}
                Verdict::Changed => return Err(format!("Error: {} changed while the view was being made. Nothing was run.", at.path().display())),
            }
        }
        a.extend(split.close);
    }
    a.extend(tool_words);
    a.extend(git);
    a.extend(reach_words_out);
    Ok((a, held))
}

#[cfg(all(test, unix))]
#[path = "outside_view_tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "view_round6_tests.rs"]
mod round6_tests;

#[cfg(all(test, unix))]
#[path = "view_round7_tests.rs"]
mod round7_tests;

#[cfg(all(test, unix))]
#[path = "view_round8_tests.rs"]
mod round8_tests;
