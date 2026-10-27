//! What a mind may ask yDoc by name: the ten actions worded, argued and graded as they always
//! were, each the command its window action runs, in the order yDoc has always published them
//! (tests/parity.rs pins it).
//!
//! Every answer is built from that call's own work (update/surface.rs) and settled when it has
//! landed (`Settle::Explicit`), so nothing the person does meanwhile, and no other call, becomes
//! this call's answer.

use std::time::Duration;

use serde_json::Value;
use yantrik_app::control::{Action, Param};
use yantrik_app::intents::{Args, Intent, Intents, Settle};
use yantrik_app::Refused;

use crate::caller::Caller;
use crate::command::{Act, Command};
use crate::model::Model;
use crate::YDoc;

/// How long a call may wait for its file work. The old window waited at most 800 ms inside a 3 s
/// round trip; a save of the 1 MiB a document may hold is far below either.
const ANSWER_WITHIN: Duration = Duration::from_secs(3);

/// A text argument as given. The dispatch has checked it is there; whitespace in it is the
/// caller's, since `set_content` and `append` take Markdown whose spaces and newlines matter.
fn text(a: &Args<'_>, name: &str) -> String {
    a.raw().get(name).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn flag(a: &Args<'_>, name: &str) -> bool {
    a.raw().get(name).and_then(Value::as_bool).unwrap_or(false)
}

/// Declares an intent: its arguments read into an [`Act`], raised with who is calling, answered
/// once its work has landed.
fn add(table: &mut Intents<YDoc>, action: Action, parse: impl Fn(&Args<'_>) -> Act + 'static) {
    table.add(
        Intent::new(action, move |a| Ok::<Command, Refused>(Command::Surface { act: parse(a), caller: Caller::now() }))
            .settles(Settle::Explicit { within: ANSWER_WITHIN })
            .reply(|m: &Model| m.answer.clone()),
    );
}

pub fn table(table: &mut Intents<YDoc>) {
    add(
        table,
        Action::new("open", "Open a Markdown document by path")
            .arg(Param::text("path"))
            .arg(
                Param::flag("discard")
                    .optional()
                    .describe("Open even though this document has unsaved changes, losing them"),
            ),
        |a| Act::Open { path: text(a, "path"), discard: flag(a, "discard") },
    );

    add(table, Action::new("new", "Start an empty document"), |_| Act::New);

    add(table, Action::new("save", "Write the document to the file it came from"), |_| Act::Save);

    add(
        table,
        Action::new("save_as", "Write the document to a path and keep it there")
            .arg(Param::text("path"))
            .arg(Param::flag("overwrite").optional().describe("Replace a file that is already at that path")),
        |a| Act::SaveAs { path: text(a, "path"), overwrite: flag(a, "overwrite") },
    );

    add(
        table,
        Action::new("set_content", "Replace the document with this Markdown").arg(Param::text("text")),
        |a| Act::SetContent { text: text(a, "text") },
    );

    add(
        table,
        Action::new("append", "Add Markdown to the end of the document").arg(Param::text("text")),
        |a| Act::Append { text: text(a, "text") },
    );

    add(
        table,
        Action::new("find", "Search the document and show the matches").arg(Param::text("query")).risk("safe"),
        |a| Act::Find { query: text(a, "query") },
    );

    add(
        table,
        Action::new("replace_all", "Replace every match in the document")
            .arg(Param::text("find"))
            .arg(Param::text("with")),
        |a| Act::ReplaceAll { find: text(a, "find"), with: text(a, "with") },
    );

    add(
        table,
        Action::new("export_markdown", "Write a Markdown copy to a path, leaving the document in its own file")
            .arg(Param::text("path")),
        |a| Act::ExportMarkdown { path: text(a, "path") },
    );

    // `show` exists for the handover: a second launch with no file asks the running window to
    // come forward. The framework's `present` brings it forward; this answers that it is there.
    add(table, Action::new("show", "Bring the yDoc window forward").risk("safe"), |_| Act::Show);
}
