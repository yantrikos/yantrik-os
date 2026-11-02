//! `files_move {path, to_folder}`: put a file or folder into another folder, by explicit paths.
//!
//! A Mind has no selection, clipboard or cut-and-paste (#1033), so "move trip-plan.md into
//! Trips" had no verb: it made the folder, called `files_select` three times and gave up (#1059).
//! This is that verb. It is its own action rather than a `to_folder` on `files_rename`, because a
//! rename changes a name inside one folder and the card the person approves should say "moves
//! this into that", not "renames".
//!
//! - **Never replaces.** The Bench's `renameat2(RENAME_NOREPLACE)` (`fileops::rename_no_replace`);
//!   a name already in the folder is refused with a sentence saying so, and nothing moves.
//! - **Same gates as `files_rename`** (`control_files_mind`): the entry and the folder must be in
//!   the person's home outside its protected places, links followed; the new name must be one the
//!   Mind may make there; everything below a moved folder must be able to land. Checked on the UI
//!   thread, and again on the worker on what the paths are by then.
//! - **Not into itself.** A folder is never moved into its own subfolder, nor into the folder it
//!   is already in.
//! - **Across disks it refuses**, plainly, with the source untouched: a copy-then-delete is not a
//!   move that cannot lose data, and the Mind is told to say so rather than retry.
//! - **Touches no screen.** No window, selection, clipboard or cursor is read or changed; the
//!   handler has no `App`.

use std::path::{Component, Path, PathBuf};

use serde_json::{json, Value};
use yantrik_app_runtime::control::{Action, Param};

use crate::control_files_cursor::{self as cursor, entry_in, spelled, text};
use crate::control_files_mind as mind;

/// The action's declaration.
pub(crate) fn action() -> Action {
    Action::new(
        "files_move",
        "Move a file or folder into another folder, by path. Never replaces: if the folder already \
         has that name, it is refused and nothing moves. Touches nobody's screen. Answers with where \
         the item now is. (To make the folder first: files_new_folder; to rename in place: files_rename)",
    )
    .defers()
    .risk("sensitive")
    // The card says what moves and where, whole: a person cannot judge a move they cannot see.
    .explain(|args| {
        let (from, to) = (mind::plain(text(args, "path")), mind::plain(text(args, "to_folder")));
        if from.trim().is_empty() || to.trim().is_empty() {
            String::new()
        } else {
            // The destination first, and each value quoted: the shell cuts a card's sentence at
            // 600 characters, and what falls off must not be the part that says where it goes.
            format!("Moves into \"{}\": \"{}\"", to.trim(), from.trim())
        }
    })
    .arg(Param::text("path").describe("The file or folder to move, e.g. ~/Desktop/trip-plan.md"))
    .arg(Param::text("to_folder").describe("The folder to move it into, e.g. ~/Desktop/Trips; it must exist"))
}

/// What a move would do, decided.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    pub src: PathBuf,
    pub dst: PathBuf,
}

pub(crate) fn no_dots(label: &str) -> Result<(), String> {
    if Path::new(label).components().any(|c| c == Component::ParentDir) {
        return Err(format!("`{label}` has `..` in it; give the path without it"));
    }
    Ok(())
}

/// The folders the person's desktop and apps are built around, directly in the home: moving one
/// takes the place away from everything that looks there.
const HOME_FOLDERS: [&str; 9] =
    ["Desktop", "Documents", "Downloads", "Music", "Pictures", "Public", "Templates", "Videos", "Archive"];

fn is_a_home_folder(src: &Path, home: &Path) -> bool {
    let real_home = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    src.parent().is_some_and(|p| p == home || p == real_home)
        && src.file_name().is_some_and(|n| HOME_FOLDERS.iter().any(|f| n.to_string_lossy().eq_ignore_ascii_case(f)))
}

/// Decide the move from `path` and `to_folder`, with `home` as the person's home. `for_mind`
/// applies the Mind's rules (`control_files_mind`), which the person's own call is not held to.
pub(crate) fn plan(args: &Value, home: &Path, for_mind: bool) -> Result<Plan, String> {
    plan_checked(args, home, for_mind).map_err(mind::in_words)
}

/// The longest path the approval card can show whole beside another (it is cut at 600 characters).
const CARD_PART: usize = 250;

fn plan_checked(args: &Value, home: &Path, for_mind: bool) -> Result<Plan, String> {
    for (key, what) in [("path", "the path"), ("to_folder", "the folder")] {
        let long = text(args, key).chars().count();
        if for_mind && long > CARD_PART {
            return Err(format!(
                "{what} is {long} characters, more than the approval card can show whole ({CARD_PART}); give a shorter path"
            ));
        }
    }
    let (parent, name) = entry_in(args)?;
    crate::fileops::name(&name)?;
    let to = text(args, "to_folder");
    if to.is_empty() {
        return Err("give `to_folder`, the folder to move it into".into());
    }
    let to = spelled(to)?.trim_end_matches('/');
    let to = if to.is_empty() { "/" } else { to };
    no_dots(&parent)?;
    no_dots(to)?;
    if for_mind {
        mind::entry_verdict(&parent, &name, home)?;
        // The source must be somewhere the Mind could make it, as a rename's old name is: not in a
        // hidden folder or a place programs read their settings or startup from (`~/.config`,
        // `~/.local/bin`, Trash, a browser profile). Moving out of one is as much a change to it
        // as moving in, and `files_rename` never allowed it.
        mind::make_verdict(&parent, &name, home)?;
        mind::open_verdict(to, home)?;
        mind::make_verdict(to, &name, home)?;
    }
    let from_dir = crate::filebrowser::expand_home(&parent)
        .canonicalize()
        .map_err(|e| format!("{parent} cannot be read: {e}"))?;
    let src = from_dir.join(&name);
    if std::fs::symlink_metadata(&src).is_err() {
        return Err(format!("nothing is at {}", src.display()));
    }
    if for_mind && is_a_home_folder(&src, home) {
        return Err(format!(
            "{} is one of the person's own folders; move what is in it, not the folder",
            src.display()
        ));
    }
    let into = crate::filebrowser::expand_home(to);
    let into = into.canonicalize().map_err(|_| mind::no_folder_at(to, home))?;
    if !into.is_dir() {
        return Err(format!("{to} is not a folder"));
    }
    if into == from_dir {
        return Err(format!("{name} is already in {to}; nothing to move"));
    }
    if std::fs::symlink_metadata(&src).is_ok_and(|m| m.is_dir()) && into.starts_with(&src) {
        return Err(format!("{name} cannot be moved into {to}, which is inside it"));
    }
    let dst = into.join(&name);
    if for_mind {
        yantrik_ipc_contracts::home_paths::may_land(&src, &dst, home)?;
    }
    Ok(Plan { src, dst })
}

/// Do the move and read it back. Never replaces; never copies.
pub(crate) fn perform(plan: &Plan) -> Result<Value, String> {
    let kind = if std::fs::symlink_metadata(&plan.src).is_ok_and(|m| m.is_dir()) { "folder" } else { "file" };
    let name = plan.dst.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let folder = plan.dst.parent().unwrap_or(Path::new("/")).display().to_string();
    if let Err(e) = yantrik_bench::rename::rename_no_replace(&plan.src, &plan.dst) {
        return Err(match (e.kind(), e.raw_os_error()) {
            (std::io::ErrorKind::AlreadyExists, _) => format!(
                "{folder} already has something called {name}; nothing was moved and nothing was replaced. \
                 Move it under another name (files_rename first) or to another folder"
            ),
            (_, Some(libc::EXDEV)) => format!(
                "{name} and {folder} are on different disks, and a move between disks is not done here \
                 because a copy that fails halfway could lose it; it was left where it was"
            ),
            _ => format!("{} could not be moved to {folder}: {e}; it was left where it was", plan.src.display()),
        });
    }
    let there = std::fs::symlink_metadata(&plan.dst).is_ok();
    Ok(json!({
        "moved": plan.src.display().to_string(),
        "now_at": plan.dst.display().to_string(),
        "kind": kind,
        "settled": there,
    }))
}

/// The handler. Checks on the calling thread, the disk on the worker.
pub(crate) fn run(args: &Value) -> Result<Value, String> {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let for_mind = cursor::caller_key().is_mind();
    plan(args, &home, for_mind)?;
    let args = args.clone();
    cursor::deferred(move || perform(&plan(&args, &home, for_mind)?))
}

#[cfg(test)]
#[path = "control_files_move_tests.rs"]
mod tests;
