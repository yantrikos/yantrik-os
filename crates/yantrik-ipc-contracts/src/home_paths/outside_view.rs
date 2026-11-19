//! What a command a Mind drives sees of the person's home (#1265).
//!
//! The guard is the view, not the command's words: a check over text fails open (a glob, a variable
//! or a `cd` gets past it). In the outside view the home is a fresh tmpfs, and only these are put
//! back in it:
//!
//! - **read-write:** the home's top-level entries that are not hidden (no leading `.`, and not
//!   `bin`, the line `writes.rs` draws for writes), and the job's working directory;
//! - **read-only:** the named [`TOOLCHAIN`] list;
//! - **read-only, for one job:** a place the call named in `reach`, which the person was asked for.
//!
//! Everything else is absent, so an unknown `~/.newthing` is as absent as `~/.ssh`. The four places
//! of [`PROTECTED`](super::PROTECTED) that sit inside a visible folder (`Pictures/Screenshots`...)
//! get an empty tmpfs over them, derived from that list; that is a second layer, not the guard.
//! `/tmp` is a private tmpfs per job, and so is `.cache` (it is not in the home at all).

use std::path::{Path, PathBuf};

use super::{is_protected, where_programs_look, PROTECTED};

/// The dot-places a build needs, read-only. A test holds that none is, or holds, a protected place.
pub const TOOLCHAIN: &[&str] = &[
    ".gitconfig",
    ".config/git",
    ".local/bin",
    ".cargo/bin",
    ".cargo/registry",
    ".cargo/git",
    ".rustup",
];

/// What a failed command is told when it touched a place the view does not hold.
pub const MISSING_HINT: &str =
    "A hidden place in the home (a dot-folder: settings, keys, app data) is not in this command's view; to use one, run it with `agent_run_reach` (`run_command_with_reach` in a tool list) and the person is asked.";

/// The part of the tool text that says what a command sees.
pub const VIEW_TEXT: &str =
    "A command sees your visible folders. Your settings, keys and app data are not there. \
    To use one, ask the person.";

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

/// Whether a top-level name of the home is shown: not hidden, not `bin`, not a protected name.
fn shown(name: &str) -> bool {
    let lower = name.to_lowercase();
    !lower.starts_with('.')
        && lower != "bin"
        && !PROTECTED.iter().any(|p| !p.contains('/') && *p == lower)
}

/// The top-level entries of `home` that a command sees, read-write.
pub fn visible_entries(home: &Path) -> Vec<PathBuf> {
    let Ok(dir) = std::fs::read_dir(home) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = dir
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(shown))
        // A visible name that is a link into a hidden place is not a way in.
        .map(|e| e.path())
        .filter(|p| !where_programs_look(p, home) && !is_protected(p))
        .collect();
    out.sort();
    out
}

/// The places of `PROTECTED` that sit inside a visible folder, where they really are on disk (the
/// list is lower case; the folder is `Screenshots`).
pub fn derived_masks(home: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for place in PROTECTED
        .iter()
        .filter(|p| p.contains('/') && !p.starts_with('.'))
    {
        // Every spelling of each part that is on disk: `Pictures` and `pictures` are two folders here.
        let mut at = vec![home.to_path_buf()];
        for part in place.split('/') {
            at = at
                .iter()
                .filter_map(|d| std::fs::read_dir(d).ok())
                .flat_map(|d| d.flatten())
                .filter(|e| e.file_name().to_string_lossy().to_lowercase() == part)
                .map(|e| e.path())
                .collect();
        }
        out.extend(at.into_iter().filter(|p| p.is_dir()));
    }
    out
}

/// Why a working directory is not granted, or `None`.
fn refused(home: &Path, cwd: &Path) -> Option<String> {
    let under_home = cwd.starts_with(home)
        || cwd.canonicalize().is_ok_and(|c| {
            c.starts_with(home.canonicalize().unwrap_or_else(|_| home.to_path_buf()))
        });
    if under_home && (where_programs_look(cwd, home) || is_protected(cwd)) {
        return Some(format!(
            "Error: {} is in a hidden or protected place in the home, which a command's view does not hold; run it from a visible folder. Nothing was run.",
            cwd.display()
        ));
    }
    None
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
/// exists, and that is not above another protected place: naming `~/.ssh` is the card's question,
/// naming `~/.config` would put `~/.config/gh` and the rest in the view without the card saying so.
/// Never the home itself and never a workshop.
pub fn reach_place(home: &Path, place: &Path) -> Result<PathBuf, String> {
    let bad = |why: &str| format!("Error: reach {}: {why}. Nothing was run.", place.display());
    if !place.is_absolute() || place.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(bad("name it as an absolute path with no `..`"));
    }
    let real = place.canonicalize().map_err(|_| bad("it does not exist"))?;
    let real_home = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    let Ok(rest) = real.strip_prefix(&real_home) else {
        return Err(bad("only a place inside the home can be named"));
    };
    if rest.as_os_str().is_empty() || super::workshops::holds(&real, home) {
        return Err(bad("only a place inside the home can be named"));
    }
    let holds_another = PROTECTED.iter().any(|p| {
        let at = real_home.join(p);
        at != real && at.starts_with(&real)
    });
    match holds_another {
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

/// The sentence on the card for a command that asks for `reach`: the place, the command, and that
/// it is this command only. Text of the call is shown as given, control characters left out.
pub fn card_text(command: &str, places: &[String]) -> String {
    let clean = |s: &str, n: usize| -> String { s.trim().chars().filter(|c| !c.is_control()).take(n).collect() };
    let shown: Vec<String> = places.iter().take(MAX_REACH).map(|p| format!("`{}`", clean(p, 120))).collect();
    format!(
        "A Mind wants to run `{}` with {} visible to it, read-only. Only this command.",
        clean(command, 300),
        shown.join(", ")
    )
}

/// The `bwrap` words that make the home of this view: `--tmpfs /tmp`, `--tmpfs $HOME`, then what is
/// put back. `Err` (with the sentence to return as is) for a working directory that is not allowed
/// or a `reach` that is not a place in the home.
pub fn view_args(home: &Path, cwd: &Path, reach: &[PathBuf]) -> Result<Vec<String>, String> {
    if let Some(why) = refused(home, cwd) {
        return Err(why);
    }
    let reach: Vec<PathBuf> = reach
        .iter()
        .map(|r| reach_place(home, r))
        .collect::<Result<_, _>>()?;
    let mut a: Vec<String> = ["--tmpfs", "/tmp", "--tmpfs"].map(String::from).to_vec();
    a.push(text(home));
    for entry in visible_entries(home) {
        a.extend(bind("--bind", &entry, &entry));
    }
    for rel in TOOLCHAIN {
        let at = home.join(rel);
        if at.symlink_metadata().is_ok() {
            a.extend(bind("--ro-bind", &at, &at));
        }
    }
    // A working directory in `/tmp` is the one thing of it a job keeps.
    if !cwd.starts_with(home) && cwd.starts_with("/tmp") && cwd.is_dir() {
        a.extend(bind("--bind", cwd, cwd));
    }
    for mask in derived_masks(home) {
        a.extend(["--tmpfs".to_string(), text(&mask)]);
    }
    for place in reach {
        a.extend(bind("--ro-bind", &place, &place));
    }
    Ok(a)
}

#[cfg(all(test, unix))]
#[path = "outside_view_tests.rs"]
mod tests;
