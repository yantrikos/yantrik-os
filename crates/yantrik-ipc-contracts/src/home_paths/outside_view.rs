//! What a command a Mind drives sees of the person's home (#1265).
//!
//! The guard is the view, not the command's words: a check over text fails open (a glob, a variable
//! or a `cd` gets past it). In the outside view the home is a fresh tmpfs, and only these are put
//! back in it:
//!
//! - **read-write:** the home's top-level entries that are not hidden (no leading `.`, and not
//!   `bin` or `snap`, the lines `writes.rs` draws for writes) and are not links;
//! - **read-only:** the named [`TOOLCHAIN`] list (`view_places`), and git's shown settings
//!   (`view_git`), which are synthesized, never the files;
//! - **read-only, for one job:** a place the call named in `reach`, which the person was asked for.
//!
//! Everything else is absent, so an unknown `~/.newthing` is as absent as `~/.ssh`. The places of
//! [`PROTECTED`](super::PROTECTED) that sit inside a visible folder (`Pictures/Screenshots`...) get
//! an empty tmpfs over them, derived from that list and laid last, after `reach`, whether or not the
//! folder exists yet; that is a second layer, not the guard. `/tmp` is a private tmpfs per job:
//! the host's `/tmp` is never bound, and a working directory outside the home is one the job makes
//! in its own. `.cache` is not in the home at all.

use std::path::{Path, PathBuf};

use super::view_git::git_words;
pub use super::view_places::{account_home, TOOLCHAIN};
use super::view_places::toolchain_words;
use super::{is_protected, where_programs_look, PROTECTED};

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
        .filter(|p| !where_programs_look(p, home) && !is_protected(p))
        .collect();
    out.sort();
    out
}

/// A real folder (not a link) inside `dir` spelled as `part` in any case: `Pictures` and `pictures`
/// are two folders on a case-sensitive disk.
fn folders_named(dir: &Path, part: &str) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    read.flatten()
        .filter(|e| e.file_name().to_string_lossy().to_lowercase() == part)
        .map(|e| e.path())
        .filter(|p| p.symlink_metadata().is_ok_and(|m| m.is_dir()))
        .collect()
}

/// `pictures` as `Pictures`: how the folder is spelled when nothing of it is on disk yet.
fn titled(part: &str) -> String {
    let mut c = part.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// The places of `PROTECTED` that sit inside a visible folder, where they are on disk (the list is
/// lower case; the folder is `Screenshots`), and where they will be when the folder is made later:
/// a folder that is not there yet gets its mount point made in the view, so one the person's tools
/// create after a long job started is hidden too. A place whose first part is not shown (`snap`) is
/// not here: it is not in the view at all.
pub fn derived_masks(home: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for place in PROTECTED.iter().filter(|p| p.contains('/') && !p.starts_with('.')) {
        let parts: Vec<&str> = place.split('/').collect();
        if !shown(parts[0]) {
            continue;
        }
        let mut at = vec![home.to_path_buf()];
        for part in &parts[..parts.len() - 1] {
            at = at.iter().flat_map(|d| folders_named(d, part)).collect();
        }
        let last = parts[parts.len() - 1];
        for parent in at {
            let mut spelled = folders_named(&parent, last);
            if spelled.is_empty() && parent.symlink_metadata().is_ok() && !parent.join(titled(last)).exists() {
                spelled.push(parent.join(titled(last)));
            }
            out.extend(spelled);
        }
    }
    out
}

/// Where a working directory is allowed, or why not. `Ok(Some(dir))` is a directory the job makes
/// in its own private `/tmp`; the host's is never bound. A working directory is a place under the
/// home that the view shows, or a directory inside the job's own `/tmp`; where it really is decides,
/// whatever its text says.
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
    let hidden = |p: &Path| where_programs_look(p, home) || is_protected(p);
    if (in_home && (hidden(cwd) || hidden(&real))) || super::workshops::holds(&real, home) {
        return refuse("it is in a hidden or protected place in the home, which a command's view does not hold; run it from a visible folder");
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

/// A place named in `reach`: a folder or file inside the home, written absolute (or `~/…`), that
/// exists, that **is the place it is written as** (a link is refused, with the place it leads to),
/// and that is not above another protected place: naming `~/.ssh` is the card's question, naming
/// `~/.config` would put `~/.config/gh` and the rest in the view without the card saying so.
/// Never the home itself and never a workshop. The one function the card, the job and the bind all
/// use, so what the person is shown is what is checked, and the checked path is the one bound.
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
    let Ok(rest) = real.strip_prefix(&real_home) else {
        return Err(bad("only a place inside the home can be named"));
    };
    if rest.as_os_str().is_empty() || super::workshops::holds(&real, home) {
        return Err(bad("only a place inside the home can be named"));
    }
    // Compared case-folded, and against the masks that will be laid as well as the list.
    let lower = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());
    let (real_l, home_l) = (lower(&real), lower(&real_home));
    let holds_listed = PROTECTED.iter().any(|p| {
        let at = home_l.join(p);
        at != real_l && at.starts_with(&real_l)
    });
    let holds_mask = derived_masks(&real_home).iter().any(|m| lower(m) != real_l && lower(m).starts_with(&real_l));
    match holds_listed || holds_mask {
        true => Err(bad("it holds other protected places; name the one you need")),
        false => Ok(real),
    }
}

/// The places of a `reach` argument as the view takes them: at most [`MAX_REACH`], `~` expanded, each
/// checked by [`reach_place`]. `Err` is the sentence to return as is.
pub fn reach_list(home: &Path, written: &[String]) -> Result<Vec<PathBuf>, String> {
    if written.is_empty() || written.len() > MAX_REACH {
        return Err(format!("Error: `reach` names between 1 and {MAX_REACH} places. Nothing was run."));
    }
    written.iter().map(|w| reach_place(home, &expand(home, w))).collect()
}

/// The `bwrap` words that make the home of this view: `--tmpfs /tmp`, `--tmpfs $HOME`, then what is
/// put back, then the masks last. `Err` (with the sentence to return as is) for a working directory
/// that is not allowed or a `reach` that is not a place in the home, as it is now (checked again
/// here, at bind time, so a place swapped for a link since the card is refused).
///
/// `bwrap` binds a source by its path, so the time between this check and the mount is not nil. A
/// descriptor (`--bind-fd`) would close it, and the `bwrap` of the image is not known to have it;
/// the remaining window is written in SECURITY.md.
pub fn view_args(home: &Path, cwd: &Path, reach: &[PathBuf]) -> Result<Vec<String>, String> {
    let make = check_cwd(home, cwd)?;
    let reach: Vec<PathBuf> = reach.iter().map(|r| reach_place(home, r)).collect::<Result<_, _>>()?;
    let mut a: Vec<String> = ["--tmpfs", "/tmp"].map(String::from).to_vec();
    if let Some(dir) = make {
        a.extend(["--dir".to_string(), text(&dir)]);
    }
    a.extend(["--tmpfs".to_string(), text(home)]);
    // `-try`: a file that is gone by the time the view is made (a database's `-shm`, a lock) is not
    // one, and must not stop the command.
    for entry in visible_entries(home) {
        a.extend(bind("--bind-try", &entry, &entry));
    }
    a.extend(toolchain_words(home));
    a.extend(git_words(home));
    for place in reach {
        a.extend(bind("--ro-bind", &place, &place));
    }
    // Last, so no bind of a folder that holds one lays a fresh view over it.
    for mask in derived_masks(home) {
        a.extend(["--tmpfs".to_string(), text(&mask)]);
    }
    Ok(a)
}

#[cfg(all(test, unix))]
#[path = "outside_view_tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "view_round6_tests.rs"]
mod round6_tests;
