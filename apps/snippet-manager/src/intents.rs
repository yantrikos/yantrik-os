//! What the Mind, the launcher and `yos` may ask Snippets by name: the nine actions it always
//! published, with the same names, descriptions, parameters and grades, each the command its
//! button runs.
//!
//! Every one settles explicitly (`Settle::Explicit`), in the command where its own result lands:
//! a change when its write has landed, a copy when the clipboard took it and its count was kept,
//! an import when the file was read and kept. A failure is said on the notice line and is the
//! call's refusal, as the old `settle` helper made it.

use std::time::Duration;

use yantrik_app::control::{Action, Param};
use yantrik_app::intents::{Args, Intent, Intents, Settle};
use yantrik_app::Refused;

use crate::command::{Command, From};
use crate::model::Model;
use crate::store::Patch;
use crate::SnippetManager;

/// How long a call may wait for its write, its copy or its file. Each is local and quick; this
/// bounds one that is stuck.
const WITHIN: Duration = Duration::from_secs(20);

pub fn table(table: &mut Intents<SnippetManager>) {
    // Standard: it writes this app's own store and nothing else on the machine changes.
    add(table, Action::new("new", "Keep a new snippet")
        .arg(Param::text("title"))
        .arg(Param::text("language").describe("Rust, Python, Shell, \u{2026}"))
        .arg(Param::text("code").describe("The snippet itself"))
        .arg(Param::text("tags").describe("Comma-separated").optional())
        .risk("standard"), new);
    add(table, Action::new("save", "Change a snippet that is already kept")
        .arg(Param::text("id").describe("Its id, or its title"))
        .arg(Param::text("title").optional())
        .arg(Param::text("language").optional())
        .arg(Param::text("code").optional())
        .arg(Param::text("tags").optional())
        .arg(Param::flag("favorite").optional())
        .arg(
            Param::text("collection")
                .describe("A collection by name or id; empty files it under none")
                .optional(),
        )
        .risk("standard"), save);
    // Safe: it changes what is on screen and nothing that is stored (the typing in the editor is
    // kept before it moves, as every move of the editor does).
    add(table, Action::new("open", "Show a snippet in the editor")
        .arg(Param::text("id").describe("Its id, or its title"))
        .risk("safe"), |a| Ok(Command::Open(raw(a, "id"))));
    add(table, Action::new("search", "Narrow the list to what matches")
        .arg(Param::text("query").describe("Matched against title, code, tags and language"))
        .risk("safe"), |a| Ok(Command::Find(raw(a, "query"))));
    // Standard: the clipboard is readable by everything else on the session, so this is data
    // leaving the app — reversible, but not nothing.
    add(table, Action::new("copy", "Put a snippet's code on the clipboard")
        .arg(Param::text("id").describe("Its id, or its title"))
        .risk("standard"), |a| Ok(Command::CopyNamed(raw(a, "id"))));
    // Sensitive, not standard: this destroys something the person wrote and there is no trash to
    // take it out of. Container Manager grades `remove` the same way for the same reason — the
    // recoverable and the unrecoverable are not one grade.
    add(table, Action::new("delete", "Throw a snippet away. There is no trash to take it out of: it cannot be undone.")
        .arg(Param::text("id").describe("Its id, or its title"))
        .risk("sensitive"), |a| Ok(Command::DeleteNamed(raw(a, "id"))));
    add(table, Action::new("toggle_favorite", "Star or unstar a snippet")
        .arg(Param::text("id").describe("Its id, or its title"))
        .risk("standard"), |a| Ok(Command::FavoriteNamed(raw(a, "id"))));
    add(table, Action::new("import", "Keep a file as a snippet, or read back an export")
        .arg(Param::text("path").describe("A source file, or a file `export` wrote"))
        .risk("standard"), import);
    add(table, Action::new("show", "Bring the window forward").risk("safe"), |_| Ok(Command::Show));
}

/// Every intent the same way: answered from `Model::answer`, which the command that settles it
/// writes in the same command.
fn add(
    table: &mut Intents<SnippetManager>,
    action: Action,
    parse: impl Fn(&Args<'_>) -> Result<Command, Refused> + 'static,
) {
    table.add(
        Intent::new(action, parse)
            .settles(Settle::Explicit { within: WITHIN })
            .reply(|m: &Model| m.answer.clone()),
    );
}

/// One text argument, trimmed of nothing: code keeps its whitespace.
fn raw(a: &Args<'_>, key: &str) -> String {
    a.raw()[key].as_str().unwrap_or_default().to_string()
}

/// An argument that was actually given, as against one left out.
fn given(a: &Args<'_>, key: &str) -> Option<String> {
    a.raw()[key].as_str().map(str::to_string)
}

fn new(a: &Args<'_>) -> Result<Command, Refused> {
    let title = raw(a, "title");
    let code = raw(a, "code");
    if title.trim().is_empty() {
        return Err(Refused::new("missing_argument", "`title` is empty"));
    }
    if code.is_empty() {
        return Err(Refused::new("missing_argument", "`code` is empty; there is nothing to keep"));
    }
    let language = match raw(a, "language") {
        given if given.trim().is_empty() => "Other".to_string(),
        given => given,
    };
    Ok(Command::Add { title, language, code, tags: raw(a, "tags") })
}

fn save(a: &Args<'_>) -> Result<Command, Refused> {
    let patch = Patch {
        title: given(a, "title"),
        language: given(a, "language"),
        code: given(a, "code"),
        tags: given(a, "tags"),
        favorite: a.raw()["favorite"].as_bool(),
        collection: None,
    };
    Ok(Command::Change { id: raw(a, "id"), patch, collection: given(a, "collection") })
}

fn import(a: &Args<'_>) -> Result<Command, Refused> {
    let path = raw(a, "path");
    if path.trim().is_empty() {
        return Err(Refused::new("missing_argument", "`path` is empty"));
    }
    // `~/x` means what it does in a shell; anything else is taken as given.
    Ok(Command::Import { path: yantrik_foundation::home::expand(path.trim()), from: From::Surface })
}
