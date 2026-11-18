//! What the Mind may ask Download Manager by name: the same ten actions as before, with the same
//! names, descriptions, parameters and grades, in the same order, each the command a button runs
//! where there is a button for it.
//!
//! What this app can say that nothing else can: whether the file arrived, where it is, and whether
//! its hash matches. An agent asked to fetch something had to shell out to `curl` and then work out
//! for itself whether the result was whole; here it queues the URL, watches one number, and reads
//! back a path and a SHA-256 it did not have to compute.
//!
//! # Settling
//!
//! `add`, `pause`, `resume`, `cancel`, `retry`, `verify`, `pause_all` and `resume_all` still
//! declare `defers`: each hands the work to a thread and returns long before the file exists, and
//! a caller that treated the call as the result would report a 4 GB image as downloaded the
//! instant it started. Each runs on the engine's desk now, so each is answered when its own call
//! has landed (`Settle::Explicit`), in the command that carries it (`answers.rs`): with the row as
//! the engine held it just after that call, or with the call's own refusal.

use std::time::Duration;

use serde_json::Value;
use yantrik_app::control::{Action, Param};
use yantrik_app::intents::{Args, Intent, Intents, Settle};
use yantrik_app::Refused;

use crate::command::Command;
use crate::model::Model;
use crate::Downloads;

/// How long a call to the engine may take to land before the Mind is told it did not.
pub const WITHIN: Duration = Duration::from_secs(14);

/// The number of a download, in the words the old surface refused a missing one with.
fn id(a: &Args<'_>) -> Result<i32, Refused> {
    a.raw()
        .get("id")
        .and_then(Value::as_i64)
        .map(|id| id as i32)
        .ok_or_else(|| Refused::new("missing_argument", "`id` must be the number of a download, as `describe` reports it"))
}

fn text(a: &Args<'_>, name: &str) -> String {
    a.raw().get(name).and_then(Value::as_str).unwrap_or_default().to_string()
}

pub fn table(table: &mut Intents<Downloads>) {
    table.add(answered(
        Intent::new(
            // `sensitive`: it fetches a URL the caller chose and writes the result as the person.
            // In `ask` mode an agent's download shows the person the URL and the folder first.
            Action::new("add", "Download a URL to this machine")
                .risk("sensitive")
                .fetches_address("url")
                .defers()
                .arg(Param::text("url").describe("The http(s) URL to fetch"))
                .arg(
                    Param::text("save_dir")
                        .optional()
                        .describe("Directory to save into; defaults to ~/Downloads"),
                )
                .arg(
                    Param::text("sha256")
                        .optional()
                        .describe("Expected SHA-256; the file is checked against it when it finishes"),
                ),
            |a| {
                Ok(Command::Add {
                    url: text(a, "url").trim().to_string(),
                    sha256: text(a, "sha256"),
                    save_dir: text(a, "save_dir"),
                })
            },
        ),
    ));
    table.add(answered(
        // Defers, and the live run is what proved it: pausing sets a flag the worker reads at its
        // next chunk boundary, so the row this returns can still say `downloading`. The stop is
        // certain; its arrival is not instant.
        Intent::new(
            Action::new("pause", "Stop a running download, keeping what has arrived")
                .defers()
                .arg(Param::integer("id")),
            |a| Ok(Command::Pause(id(a)?)),
        ),
    ));
    table.add(answered(Intent::new(
        Action::new("resume", "Continue a paused or failed download").defers().arg(Param::integer("id")),
        |a| Ok(Command::Resume(id(a)?)),
    )));
    table.add(answered(
        // `sensitive`, not `standard`: it interrupts work in progress and deletes the partial file.
        // Not `dangerous`: nothing a person made is lost, only bytes that can be fetched again.
        Intent::new(
            Action::new(
                "cancel",
                "Stop a download and delete the partial file. Deleting it cannot be undone; the \
                 download can only be started again from the beginning.",
            )
            .risk("sensitive")
            .defers()
            .arg(Param::integer("id")),
            |a| Ok(Command::Cancel(id(a)?)),
        ),
    ));
    table.add(answered(Intent::new(
        // An agent retries only what did not finish (`calls::retry`).
        Action::new("retry", "Start a failed download again from the beginning").defers().arg(Param::integer("id")),
        |a| Ok(Command::Retry(id(a)?)),
    )));
    table.add(answered(Intent::new(
        Action::new("verify", "Hash a finished file again and compare it with a checksum")
            .defers()
            .arg(Param::integer("id"))
            .arg(
                Param::text("sha256")
                    .optional()
                    .describe("Expected SHA-256; omit to re-check the one already recorded"),
            ),
        |a| {
            let sha256 = a.raw().get("sha256").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(str::to_string);
            Ok(Command::Verify { id: id(a)?, sha256 })
        },
    )));
    table.add(answered(Intent::new(
        Action::new("pause_all", "Stop every running download").defers(),
        |_| Ok(Command::PauseAll),
    )));
    table.add(answered(Intent::new(
        Action::new("resume_all", "Continue every paused download").defers(),
        |_| Ok(Command::ResumeAll),
    )));
    table.add(answered(Intent::new(
        Action::new("clear_completed", "Drop finished downloads from the list, keeping the files"),
        |_| Ok(Command::ClearCompleted),
    )));
    table.add(answered(Intent::new(
        Action::new("open_folder", "Show a download's folder in the file manager").arg(Param::integer("id")),
        |a| Ok(Command::OpenFolder(id(a)?)),
    )));
}

/// An intent answered when its own call has landed, within [`WITHIN`], with what that call found.
fn answered(intent: Intent<Downloads>) -> Intent<Downloads> {
    intent.settles(Settle::Explicit { within: WITHIN }).reply(answer)
}

/// What an intent answers: what `update` wrote in the command that settled it.
fn answer(m: &Model) -> Value {
    m.answer.clone()
}
