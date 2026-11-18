//! Which actions the built-in companion may run through `app_action` (#1311, round 6 of #1313).
//!
//! "Approve the effect, not the route to it": the four questions (`asks.rs`) guard what the
//! companion's own tools do to the person's mail, calendar, files, memory and plans, and a card the
//! person pressed is the only thing that lets one go ahead. `app_action` is one tool name for every
//! app's every action, so an app that publishes the same effect under its own action name was a
//! second road to it, with no card in full bypass. Rounds 5 and 6 first guarded that road with a
//! list of refused names, and a list of names fails open: it was keyed on folder names no socket
//! binds, and a command line, a calendar event of the companion's own and a note's tags walked past
//! it. So the guard is the other way round: an allow list, decided at run time.
//!
//! An action runs on this road only if the app publishes it graded `safe`, or it is on the reviewed
//! list ([`crate::app_reviewed::REVIEWED`]: an (id, action) pair with the reason it stays). Anything
//! else, whether an app, an adapter or an action nobody has looked at, is refused with a sentence that
//! names the companion's own asking tool where one exists ([`INSTEAD`]) and otherwise says the person
//! can do it in the app. No card is raised on this road.
//!
//! The key is the id the app's socket is bound under (`editor`, `documents`, `snippets`), not a
//! folder name or an alias: [`bound_id_in`] reads the socket directory (an alias is a symlink to the
//! real socket, a real socket is its own id), and [`table_id`] reads the shell's own table, the
//! `.desktop` files' `X-Yantrik-Surface` and `X-Yantrik-Aliases` (`yantrik_shell_core::apps`, the
//! scan `surfaces::declared` is built on), when no socket is there yet.
//! `tests/app_guard_surface.rs` holds the reviewed list to the apps' own source.

use serde_json::Value;
use yantrik_shell_core::apps::{self, DesktopEntry};

use crate::app_reach::published;
pub use crate::app_reviewed::{is_reviewed, REVIEWED};

/// An action the companion has its own asking tool for: the sentence names it.
pub struct Instead {
    /// The id the app's socket is bound under.
    pub id: &'static str,
    pub action: &'static str,
    /// The companion tool that asks the person on a card first.
    pub tool: &'static str,
}

const fn instead(id: &'static str, action: &'static str, tool: &'static str) -> Instead {
    Instead { id, action, tool }
}

/// Refused pairs whose effect the companion's own tools make, asking first. Only for the sentence:
/// what is not on the reviewed list is refused whether it is here or not.
pub const INSTEAD: &[Instead] = &[
    instead("calendar", "delete_event", "calendar_delete_event"),
    instead("calendar", "update_event", "calendar_update_event"),
    instead("documents", "save", "write_file"),
    instead("documents", "save_as", "write_file"),
    instead("documents", "set_content", "write_file"),
    instead("documents", "export_markdown", "write_file"),
    instead("presentation", "save", "write_file"),
    instead("presentation", "save_as", "write_file"),
    instead("presentation", "export_markdown", "write_file"),
    instead("spreadsheet", "save_over", "write_file"),
    instead("editor", "save", "write_file"),
    instead("editor", "save_as", "write_file"),
    instead("editor", "set_content", "write_file"),
    instead("shell", "run_recipe", "run_recipe"),
];

/// The id `name`'s socket is bound under in `dir`, when there is a socket: an alias is a symlink
/// (`app-<alias>.sock` to `app-<id>.sock`) and is read to its id, and a socket that is not a link
/// is bound under `name` itself. `None` when nothing is there (not started, or no such directory).
pub(crate) fn bound_id_in(dir: &std::path::Path, name: &str) -> Option<String> {
    let path = dir.join(format!("app-{name}.sock"));
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.file_type().is_symlink() {
        return Some(name.to_string());
    }
    let target = std::fs::read_link(&path).ok()?;
    let file = target.file_name()?.to_str()?;
    file.strip_prefix("app-")?.strip_suffix(".sock").map(str::to_string)
}

/// The id the shell's table gives `name`: the surface an entry declares under it, else the surface
/// an entry lists it as an alias of, else the folded name itself.
pub(crate) fn table_id(entries: &[DesktopEntry], name: &str) -> String {
    let key = apps::fold_name(name);
    if entries.iter().any(|e| e.surface.as_deref() == Some(key.as_str())) {
        return key;
    }
    entries.iter().find(|e| e.aliases.contains(&key)).and_then(|e| e.surface.clone()).unwrap_or(key)
}

/// The id this call is about: where the socket is bound if there is one, else the table's answer.
pub(crate) fn id_in(dir: &std::path::Path, entries: &dyn Fn() -> Vec<DesktopEntry>, app: &str) -> String {
    bound_id_in(dir, app.trim()).unwrap_or_else(|| table_id(&entries(), app))
}

fn sentence(id: &str, action: &str) -> String {
    let way = match INSTEAD.iter().find(|i| i.id == id && i.action == action) {
        Some(i) => format!("Use `{}`, which asks the person on a card first.", i.tool),
        None => format!("The companion has no tool that asks for this, so say what you would change and the person can do it in the {id} app."),
    };
    format!(
        "Refused: `{id}.{action}` is not graded safe and is not on the reviewed list of what the companion may run through `app_action`, and this road raises no card. {way} Nothing was changed."
    )
}

/// Whether the call may run: the action is reviewed, or the app publishes it graded `safe`. `view` is
/// the app's `describe`, or `None` when it did not answer (then only the reviewed list passes).
pub(crate) fn admit_in(
    dir: &std::path::Path,
    entries: &dyn Fn() -> Vec<DesktopEntry>,
    app: &str,
    action: &str,
    view: Option<&Value>,
) -> Result<(), String> {
    let id = id_in(dir, entries, app);
    if is_reviewed(&id, action) {
        return Ok(());
    }
    let grade = view.and_then(|v| published(v, action)).and_then(|a| a.get("permission")).and_then(Value::as_str);
    match grade {
        Some("safe") => Ok(()),
        _ => Err(sentence(&id, action)),
    }
}

/// [`admit_in`] against this session's socket directory and the installed `.desktop` files. Called
/// once the app is started, with what its `describe` says.
pub fn admit(app: &str, action: &str, view: Option<&Value>) -> Result<(), String> {
    admit_in(&yantrik_ipc_transport::server::socket_dir(), &apps::scan, app, action, view)
}

/// The refusal for a pair the companion has its own asking tool for, before anything is started.
/// Everything else waits for [`admit`], which needs the app's own grade.
pub fn refused_early(app: &str, action: &str) -> Option<String> {
    refused_early_in(&yantrik_ipc_transport::server::socket_dir(), &apps::scan, app, action)
}

pub(crate) fn refused_early_in(dir: &std::path::Path, entries: &dyn Fn() -> Vec<DesktopEntry>, app: &str, action: &str) -> Option<String> {
    // Reading the `.desktop` files is for a name that could be one of these only.
    if !INSTEAD.iter().any(|i| i.action == action) {
        return None;
    }
    let id = id_in(dir, entries, app);
    (INSTEAD.iter().any(|i| i.id == id && i.action == action) && !is_reviewed(&id, action)).then(|| sentence(&id, action))
}

#[cfg(test)]
#[path = "app_guard_tests.rs"]
mod tests;
