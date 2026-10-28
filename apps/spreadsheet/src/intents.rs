//! What a mind may ask ySheets by name: the actions worded, argued and graded as they always were,
//! each the command its window action runs, in the order ySheets has always published them
//! (tests/parity.rs pins it).
//!
//! Grades: reading is `safe`; changing the sheet, opening a file and writing a NEW file are
//! `standard`; replacing a file that is already on the disk is `save_over`, graded `sensitive`,
//! because that is the one that destroys something a person cannot get back. `save` refuses to
//! replace and names `save_over`, so the grade cannot be stepped around.
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
use crate::YSheets;

/// How long a call may wait for its work: a file read or written, or a sheet worked out. The old
/// surface answered inside a 3 s round trip; a sheet is at most 16 MiB of CSV.
const ANSWER_WITHIN: Duration = Duration::from_secs(8);

/// A text argument as given. The dispatch has checked it is there.
fn text(a: &Args<'_>, name: &str) -> String {
    a.raw().get(name).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn optional(a: &Args<'_>, name: &str) -> Option<String> {
    a.raw().get(name).and_then(Value::as_str).map(str::to_string)
}

/// Declares an intent: its arguments read into an [`Act`], raised with who is calling, answered
/// once its work has landed.
fn add(table: &mut Intents<YSheets>, action: Action, parse: impl Fn(&Args<'_>) -> Act + 'static) {
    table.add(
        Intent::new(action, move |a| Ok::<Command, Refused>(Command::Surface { act: parse(a), caller: Caller::now() }))
            .settles(Settle::Explicit { within: ANSWER_WITHIN })
            .reply(|m: &Model| m.answer.clone()),
    );
}

pub fn table(table: &mut Intents<YSheets>) {
    add(
        table,
        Action::new(
            "set_cell",
            "Type into one cell, as the person would: a number, text, or a formula starting with =. Answers with what the cell now holds and works out to.",
        )
        .arg(Param::text("cell").describe("The cell, like B3"))
        .arg(Param::text("text").describe("What to type; empty clears the cell")),
        |a| Act::SetCell { cell: text(a, "cell"), text: text(a, "text") },
    );

    add(
        table,
        Action::new("get_range", "Read a range of cells: what was typed in each, and what each works out to.")
            .arg(Param::text("range").describe("Like A1:C10, or one cell"))
            .risk("safe"),
        |a| Act::GetRange { range: text(a, "range") },
    );

    add(
        table,
        Action::new("select", "Select a cell, so the person sees it and the formula bar shows it.")
            .arg(Param::text("cell").describe("The cell, like B3")),
        |a| Act::Select { cell: text(a, "cell") },
    );

    add(table, Action::new("undo", "Undo the last change to a cell."), |_| Act::Undo);

    add(table, Action::new("redo", "Redo the change undo last took back."), |_| Act::Redo);

    add(
        table,
        Action::new("open", "Open a CSV file in this window. Refused while the sheet has unsaved changes.")
            .arg(Param::text("path").describe("An absolute path, or one starting with ~/")),
        |a| Act::Open { path: text(a, "path") },
    );

    add(
        table,
        Action::new(
            "save",
            "Save the sheet as a NEW CSV file. Refuses to replace a file that is already there; that is save_over.",
        )
        .arg(
            Param::text("path")
                .describe("Where to write, ending in .csv. Omit to use the sheet's own file, if it has none on disk yet")
                .optional(),
        ),
        |a| Act::Save { path: optional(a, "path") },
    );

    add(
        table,
        Action::new(
            "save_over",
            "Save the sheet as CSV, replacing the file that is already at the path. What was in that file is lost.",
        )
        .arg(
            Param::text("path")
                .describe("The file to replace, ending in .csv. Omit to save over the sheet's own file")
                .optional(),
        )
        .risk("sensitive"),
        |a| Act::SaveOver { path: optional(a, "path") },
    );
}
