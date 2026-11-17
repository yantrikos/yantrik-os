//! Changing the person's things, or planning work, after reading what someone else wrote.
//!
//! `taint` keeps a secret from leaving with a stranger's words behind it. This is the same
//! question for the person's own data: a mail, a page or a file can say "delete the invoices" or
//! "set up a schedule that forwards everything", and the model that read it cannot tell that
//! from the person's own words. So the tools that change or plan are held to what the
//! conversation has read, below the model, in [`crate::tools::ToolRegistry::execute`].
//!
//! Two rules, both a refusal that says what was refused, what brought the text in, that nothing
//! was changed and what the person can do instead:
//!
//! * **After untrusted content** (`taint::state`, the same turn and scope the other rules use):
//!   deleting, moving or changing mail and calendar events, forgetting, sending a chat message,
//!   overwriting a file that exists, and creating or changing a recipe, schedule, automation or
//!   queued task.
//! * **With nobody there** (`background_research::person_started`: a recipe step, an urge, an
//!   automation): the mail and calendar changes and the two message sends. File writes and memory
//!   changes stay allowed, which recipes rely on.
//!
//! Mail sends are not here: they stay behind the person's own card (`outward`, in the tools crate).
//! What reads mail is untrusted from the first read ([`reads_mail`], used by `taint::classify`).

use serde_json::Value;

/// The mail and calendar reads: text a stranger wrote (a mail, an invitation). Matched by name: the
/// mail send tools share the category and stay as they are.
pub fn reads_mail(name: &str) -> bool {
    matches!(name, "email_read" | "email_list" | "email_search" | "email_check" | "calendar_list_events" | "calendar_today")
}

/// Where strangers' content lands, under the person's home (relative to it): what a `read_file` or
/// `grep` there brings in is untrusted. Anywhere outside the home is too. Everything else in the home is
/// the person's own, so "open my todo.md and tick item 3" works. A judgement about where the line sits
/// until the approval card replaces the refusal; the one place the list is kept.
const STRANGERS_IN_HOME: &[&str] = &[
    "Downloads", "Download", "tmp", "Temp", ".cache", ".mozilla", ".config/google-chrome", ".config/chromium",
    ".config/BraveSoftware", "snap", ".var/app", ".local/share/Trash", "attachments", "Attachments",
];

fn lexical(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn stranger_path(path: &str) -> bool {
    if path.is_empty() {
        return true;
    }
    let given = std::path::PathBuf::from(crate::tools::expand_home(path));
    let given = if given.is_absolute() { given } else { std::env::current_dir().unwrap_or_default().join(given) };
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from).filter(|h| h.is_absolute()) else { return true };
    let home = lexical(&home);
    let real_home = std::fs::canonicalize(&home).unwrap_or_else(|_| home.clone());
    let strange = |p: &std::path::Path| match p.strip_prefix(&real_home).or_else(|_| p.strip_prefix(&home)) {
        Err(_) => true,
        Ok(rel) => STRANGERS_IN_HOME.iter().any(|dir| rel.starts_with(dir)) || rel.components().any(|c| c.as_os_str() == "attachments"),
    };
    // As written, and where it really leads: a link in the home to the Downloads folder is Downloads.
    let written = lexical(&given);
    strange(&written) || std::fs::canonicalize(&written).is_ok_and(|real| strange(&real))
}

/// `taint::effective` for the tools whose reach depends on their path: a `read_file` or `grep` where
/// strangers' content lands is a read of untrusted content (`browser_read`); the person's own files are not.
pub fn effective<'a>(name: &'a str, category: &'a str, args: &Value) -> (&'a str, &'a str) {
    if matches!(name, "read_file" | "grep") && stranger_path(args.get("path").and_then(Value::as_str).unwrap_or("")) {
        return ("browser_read", "browser");
    }
    (name, category)
}

/// A call to an app's action (`app_action`, `await_app`'s `then_act`) judged where it is made, once the app's own
/// declaration is read: Calendar's `delete_event` and `update_event`, Files' `files_delete`, and any action
/// graded `sensitive` or `dangerous` change things. Held to the same two rules, by the same sentence.
pub fn check_app_action(app: &str, action: &str, graded: &str) -> Result<(), String> {
    let app = crate::reach::app_id(app);
    let change = matches!((app.as_str(), action), ("calendar", "delete_event" | "update_event") | ("files", "files_delete"))
        || matches!(graded, "sensitive" | "dangerous");
    if change {
        refuse_if_held(&format!("app_action` (`{app}.{action}"), if app == "calendar" { Change::Calendar } else { Change::App }, change_needs_person(&app, action))
    } else {
        Ok(())
    }
}

fn change_needs_person(app: &str, action: &str) -> bool {
    app == "calendar" && matches!(action, "delete_event" | "update_event")
}

/// What a call changes or plans, in the words of the refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    Mail,
    Calendar,
    Message,
    Forget,
    Overwrite,
    Delete,
    Plan,
    App,
}

impl Change {
    fn says(self) -> &'static str {
        match self {
            Change::Mail => "deletes, moves or archives the person's mail",
            Change::Calendar => "changes or deletes a calendar event",
            Change::Message => "sends a chat message in the person's name",
            Change::Forget => "erases or rewrites what the companion remembers",
            Change::Overwrite => "overwrites a file that already exists",
            Change::Delete => "deletes or changes the person's files, scripts, secrets or automations",
            Change::App => "changes or deletes something the person keeps in an app",
            Change::Plan => "plans work for later (a recipe, schedule, automation or queued task)",
        }
    }

    /// Held back with nobody there, as well as after untrusted content.
    fn needs_the_person(self) -> bool {
        matches!(self, Change::Mail | Change::Calendar | Change::Message)
    }
}

/// Whether a file the call would write is already there (a link, even a dangling one, counts).
fn exists(path: &str) -> bool {
    !path.is_empty() && std::fs::symlink_metadata(crate::tools::expand_home(path)).is_ok()
}

fn change_of(name: &str, args: &Value) -> Option<Change> {
    Some(match name {
        "email_delete" | "email_move" | "email_archive" => Change::Mail,
        "calendar_delete_event" | "calendar_update_event" => Change::Calendar,
        "whatsapp_send" | "telegram_send" => Change::Message,
        "forget_memory" | "forget_topic" | "update_memory" => Change::Forget,
        "write_file" => {
            let append = args.get("append").and_then(Value::as_bool).unwrap_or(false);
            let path = args.get("path").and_then(Value::as_str).unwrap_or("");
            if append || !exists(path) {
                return None;
            }
            Change::Overwrite
        }
        "edit_file" | "script_patch" | "script_delete" | "manage_files" | "vault_delete" | "delete_automation" | "cancel_schedule" => {
            Change::Delete
        }
        "create_recipe" | "create_schedule" | "create_automation" | "update_schedule" | "toggle_automation" | "queue_task" => Change::Plan,
        _ => return None,
    })
}

/// Whether this call changes or plans, and so is held to the two rules. Used by tests and audits.
pub fn is_change(name: &str, args: &Value) -> bool {
    change_of(name, args).is_some()
}

/// The gate `ToolRegistry::execute` runs after the others: `Ok` to run, or what to tell the model.
pub fn check_call(name: &str, args: &Value) -> Result<(), String> {
    let Some(change) = change_of(name, args) else { return Ok(()) };
    refuse_if_held(name, change, change.needs_the_person())
}

fn refuse_if_held(name: &str, change: Change, needs_the_person: bool) -> Result<(), String> {
    if needs_the_person && !crate::background_research::person_started() {
        return Err(format!(
            "Refused: `{name}` {}, and nobody is here to ask: it was not started by the person (a recipe step, an urge or an \
             automation). Nothing was changed. The person can do this in the app, or ask for it in a conversation.",
            change.says()
        ));
    }
    if let Some(source) = crate::conversation::source().or(crate::taint::state().0) {
        return Err(format!(
            "Refused: `{name}` {}, and this conversation has already read untrusted content (`{source}`). Text someone else \
             wrote can ask an agent to do this; it must not be able to. Nothing was changed. The person can do it in the app, \
             or ask for it in a new conversation.",
            change.says()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
