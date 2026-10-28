//! The parity pin: the action table and `describe` exactly as ySheets published them before the
//! framework. The table below was extracted mechanically from the old
//! `apps/spreadsheet/src/surface.rs` at origin/main (`actions()`'s `Action::new` chains, by
//! scratchpad/sheet/extract_golden.py), not copied by hand from the new one, so a word changed in
//! `intents.rs` fails here. The `describe` keys were extracted the same way from the old
//! `describe()` and `hidden_view()`.
//!
//! Nothing is renamed, added or dropped: the eight actions keep their names, order, descriptions,
//! parameters and grades. The framework publishes its own `present` and `launch` beside them.

use serde_json::json;
use yantrik_app::app::App as _;
use yantrik_app::intents::Intents;

use super::{act, started, state, summary};
use crate::YSheets;

struct Golden {
    name: &'static str,
    risk: &'static str,
    description: &'static str,
    /// Name, JSON type, required, description.
    params: &'static [(&'static str, &'static str, bool, &'static str)],
}

const BEFORE: &[Golden] = &[
    Golden {
        name: "set_cell",
        risk: "standard",
        description: r#"Type into one cell, as the person would: a number, text, or a formula starting with =. Answers with what the cell now holds and works out to."#,
        params: &[
            ("cell", "string", true, r#"The cell, like B3"#),
            ("text", "string", true, r#"What to type; empty clears the cell"#),
        ],
    },
    Golden {
        name: "get_range",
        risk: "safe",
        description: r#"Read a range of cells: what was typed in each, and what each works out to."#,
        params: &[
            ("range", "string", true, r#"Like A1:C10, or one cell"#),
        ],
    },
    Golden {
        name: "select",
        risk: "standard",
        description: r#"Select a cell, so the person sees it and the formula bar shows it."#,
        params: &[
            ("cell", "string", true, r#"The cell, like B3"#),
        ],
    },
    Golden {
        name: "undo",
        risk: "standard",
        description: r#"Undo the last change to a cell."#,
        params: &[],
    },
    Golden {
        name: "redo",
        risk: "standard",
        description: r#"Redo the change undo last took back."#,
        params: &[],
    },
    Golden {
        name: "open",
        risk: "standard",
        description: r#"Open a CSV file in this window. Refused while the sheet has unsaved changes."#,
        params: &[
            ("path", "string", true, r#"An absolute path, or one starting with ~/"#),
        ],
    },
    Golden {
        name: "save",
        risk: "standard",
        description: r#"Save the sheet as a NEW CSV file. Refuses to replace a file that is already there; that is save_over."#,
        params: &[
            ("path", "string", false, r#"Where to write, ending in .csv. Omit to use the sheet's own file, if it has none on disk yet"#),
        ],
    },
    Golden {
        name: "save_over",
        risk: "sensitive",
        description: r#"Save the sheet as CSV, replacing the file that is already at the path. What was in that file is lost."#,
        params: &[
            ("path", "string", false, r#"The file to replace, ending in .csv. Omit to save over the sheet's own file"#),
        ],
    },
];

/// The keys of the old `describe()`, in the order it wrote them (and `hidden_view()` wrote the same
/// sixteen).
const KEYS: &[&str] = &[
    "sheet", "path", "rows", "cols", "used", "selection", "dirty", "can_undo", "can_redo", "filled_cells",
    "error_cells", "first_error", "status", "notice", "hidden", "functions",
];

#[test]
fn the_action_table_is_the_one_ysheets_always_published() {
    let mut table = Intents::<YSheets>::new();
    YSheets::intents(&mut table);
    let specs = table.specs();
    let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
    let before: Vec<&str> = BEFORE.iter().map(|g| g.name).collect();
    assert_eq!(names, before, "the same actions, in the same order");
    for (spec, old) in specs.iter().zip(BEFORE) {
        assert_eq!(spec.description, old.description, "`{}`'s description", old.name);
        assert_eq!(spec.permission, old.risk, "`{}` is graded as it was", old.name);
        assert!(!spec.deferred, "`{}` answers when its work is done, as it did", old.name);
        assert!(!spec.stateless && spec.expected_seconds.is_none() && spec.explainer.is_none());
        let params: Vec<(&str, &str, bool, &str)> =
            spec.params.iter().map(|p| (p.name.as_str(), p.kind, p.required, p.description.as_str())).collect();
        assert_eq!(params, old.params.to_vec(), "`{}`'s arguments", old.name);
        for p in &spec.params {
            assert!(p.default.is_none() && p.values.is_empty() && p.items.is_none());
        }
    }
}

#[test]
fn describe_publishes_those_actions_and_the_frameworks_launch() {
    let app = started();
    let published: Vec<String> =
        app.describe()["actions"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap().to_string()).collect();
    let mut expected: Vec<String> = BEFORE.iter().map(|g| g.name.to_string()).collect();
    // The framework's own hand-over between two launches; refused to agents, graded safe. (Its
    // `present` is added when the surface is served, which a test does not do.)
    expected.push("launch".into());
    published.iter().for_each(|p| assert!(expected.contains(p), "`{p}` is not one of the actions"));
    expected.iter().for_each(|e| assert!(published.contains(e), "`{e}` is not published"));
}

#[test]
fn describe_has_the_same_keys_and_values() {
    let app = started();
    let s = state(&app);
    let mut keys: Vec<&str> = s.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut want = KEYS.to_vec();
    want.sort_unstable();
    assert_eq!(keys, want);
    let functions = s["functions"].clone();
    assert!(functions.as_array().unwrap().iter().any(|f| f == "SUM"), "{functions}");
    assert_eq!(
        s,
        json!({
            "sheet": "Untitled", "path": null, "rows": 50, "cols": 26, "used": "",
            "selection": { "cell": "A1", "typed": "", "value": null },
            "dirty": false, "can_undo": false, "can_redo": false, "filled_cells": 0, "error_cells": 0,
            "first_error": null, "status": "A1 · 50 rows × 26 columns · 0 cells filled",
            "notice": "", "hidden": null, "functions": functions,
        })
    );
    assert_eq!(summary(&app), "ySheets — Untitled · 50 rows × 26 columns · A1 selected · saved");
}

/// The values for a sheet with cells, an error, a selection and an unsaved change: what a mind reads.
#[test]
fn describe_reports_the_size_the_selection_the_counts_and_whether_it_is_saved() {
    let app = started();
    act(&app, "set_cell", json!({ "cell": "C2", "text": "=1/0" })).unwrap();
    act(&app, "set_cell", json!({ "cell": "A1", "text": "7" })).unwrap();
    act(&app, "select", json!({ "cell": "C2" })).unwrap();
    let s = state(&app);
    assert_eq!((s["rows"].clone(), s["cols"].clone()), (json!(50), json!(26)));
    assert_eq!(s["selection"]["cell"], "C2");
    assert_eq!(s["selection"]["typed"], "=1/0");
    assert_eq!(s["selection"]["value"]["error"], "#DIV/0!");
    assert_eq!((s["dirty"].clone(), s["can_undo"].clone(), s["can_redo"].clone()), (json!(true), json!(true), json!(false)));
    assert_eq!((s["filled_cells"].clone(), s["error_cells"].clone()), (json!(2), json!(1)));
    assert_eq!((s["first_error"].clone(), s["used"].clone()), (json!("C2"), json!("A1:C2")));
    assert_eq!(s["status"], "C2 shows #DIV/0!: dividing by zero.");
    assert_eq!(summary(&app), "ySheets — Untitled · 50 rows × 26 columns · C2 selected · unsaved changes");
}

/// `docs/app-control.md` grades every one of these actions as the table does, in ySheets' rows.
#[test]
fn the_control_doc_grades_every_action_as_the_table_does() {
    let doc = include_str!("../../../../docs/app-control.md");
    let mut graded: Vec<(String, String)> = Vec::new();
    for row in doc.lines().filter(|l| l.starts_with("| spreadsheet |")) {
        let cells: Vec<&str> = row.split('|').map(str::trim).collect();
        let (actions, grade) = (cells[2], cells[3]);
        for action in actions.split(',').map(|a| a.trim().trim_matches('`')) {
            graded.push((action.to_string(), grade.to_string()));
        }
    }
    graded.sort();
    let mut want: Vec<(String, String)> = BEFORE.iter().map(|g| (g.name.to_string(), g.risk.to_string())).collect();
    want.sort();
    assert_eq!(graded, want, "the doc lists every published action once, at its grade");
}

/// The surfaces table lists the app under the ids the launcher routes to it.
#[test]
fn the_surfaces_table_names_the_socket_and_the_aliases() {
    let doc = include_str!("../../../../docs/app-control.md");
    let row = doc.lines().find(|l| l.starts_with("| `spreadsheet` |")).expect("the app has a row");
    assert!(row.contains("`app-spreadsheet.sock`") && row.contains("`ysheets`") && row.contains("`sheets`"), "{row}");
    assert_eq!(YSheets::ID, "spreadsheet");
}
