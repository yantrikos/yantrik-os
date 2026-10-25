//! The parity pin: the action table and `describe` exactly as the Text Editor published them
//! before the framework. The table below was extracted mechanically from the old
//! `apps/text-editor/src/main.rs` at origin/main (`surface`, `act`, `arg`, the `Param`s), not
//! copied by hand from the new one, so a word changed in `intents/` fails here.

use serde_json::json;
use yantrik_app::app::App as _;
use yantrik_app::intents::Intents;

use super::{act, started, state, summary};
use crate::TextEditor;

struct Golden {
    name: &'static str,
    risk: &'static str,
    description: &'static str,
    /// Name, JSON type, required, description.
    params: &'static [(&'static str, &'static str, bool, &'static str)],
}

const BEFORE: &[Golden] = &[
    Golden {
        name: "new",
        risk: "standard",
        description: r#"Open a new tab and make it the active one, empty or holding the text given. Nothing is written to disk until `save_as` gives it a path. Tabs open until they would hold 64 MiB between them; the refusal then names a tab of yours to close."#,
        params: &[
            ("text", "string", false, r#"What the new tab holds, up to 1 MiB and 20,000 lines of UTF-8 with no control characters. Leave it out for an empty tab."#),
        ],
    },
    Golden {
        name: "open",
        risk: "standard",
        description: r#"Open the UTF-8 text file at this path in a tab and make it active; a file already open is brought forward instead of opened twice. The file is not changed."#,
        params: &[
            ("path", "string", true, r#"Absolute path to the file, or one starting `~/`. Up to 1 MiB of UTF-8 text; a binary or larger file is refused and nothing is opened."#),
        ],
    },
    Golden {
        name: "read",
        risk: "safe",
        description: r#"Read an open tab's text one page at a time, changing nothing: the tab in front or another by number. A file not open yet is `open`ed first. The answer names the `from_line` of the next page."#,
        params: &[
            ("from_line", "number", false, r#"The first line to return, counting from 1, as the last answer's `how_to_see_the_rest` names it. Leave it out to start at line 1."#),
            ("tab", "number", false, r#"Which open tab to read, counting from 0 as `describe.tabs` lists them; it is not brought forward. Leave it out to read the tab in front."#),
        ],
    },
    Golden {
        name: "save",
        risk: "standard",
        description: r#"Write the active tab back to the file it was opened from and check the bytes on disk. A tab with no file is refused — give `save_as` a path. When the editor is asking about unsaved changes, this is the answer that writes the tab and closes it."#,
        params: &[],
    },
    Golden {
        name: "save_as",
        risk: "standard",
        description: r#"Write the active tab to this path and keep the tab on it from now on. A file already at that path is refused unless `overwrite` says to replace it."#,
        params: &[
            ("path", "string", true, r#"Absolute path of the file to write, or one starting `~/`. Its folder must exist; a file already at the path needs `overwrite=true`."#),
            ("overwrite", "boolean", false, r#"Replace a file that is already at that path. Leave it out and an existing file is refused rather than overwritten."#),
        ],
    },
    Golden {
        name: "show",
        risk: "standard",
        description: r#"Bring the Editor window to the front. Nothing in any tab is read or changed."#,
        params: &[],
    },
    Golden {
        name: "close",
        risk: "standard",
        description: r#"Close the active tab. A tab with unsaved changes is not closed: the window asks, and `save`, `discard` or `cancel` answers it; an agent is refused such a tab when the person opened it. The last tab closed leaves an empty one."#,
        params: &[],
    },
    Golden {
        name: "cancel",
        risk: "standard",
        description: r#"Dismiss whichever dialog is open and leave every tab exactly as it was. Nothing is closed, saved or discarded."#,
        params: &[],
    },
    Golden {
        name: "discard",
        risk: "sensitive",
        description: r#"Throw away the unsaved changes in the tab the window is asking about and close it. The text that was never saved is gone and is not recoverable."#,
        params: &[],
    },
    Golden {
        name: "find",
        risk: "standard",
        description: r#"Search the active tab for this text, open the find bar, and select the first match. It reads the document and changes nothing in it."#,
        params: &[
            ("text", "string", true, r#"The text to look for, matched literally. Case is ignored unless the find bar's match-case box is ticked; an empty query clears the matches."#),
        ],
    },
    Golden {
        name: "find-next",
        risk: "standard",
        description: r#"Move the selection to the next match of the current `find` query, wrapping round at the end of the document. Nothing is changed."#,
        params: &[],
    },
    Golden {
        name: "find-prev",
        risk: "standard",
        description: r#"Move the selection to the previous match of the current `find` query, wrapping round at the start of the document. Nothing is changed."#,
        params: &[],
    },
    Golden {
        name: "replace_text",
        risk: "standard",
        description: r#"Set the text that `replace` and `replace-all` will put in place of a match. By itself it changes nothing in the document."#,
        params: &[
            ("text", "string", true, r#"What each match becomes. An empty string is allowed and deletes the match instead."#),
        ],
    },
    Golden {
        name: "replace",
        risk: "standard",
        description: r#"Replace the one match the selection is on with the `replace_text` text. The tab is changed in the window; nothing is written until `save`."#,
        params: &[],
    },
    Golden {
        name: "replace-all",
        risk: "standard",
        description: r#"Replace every match of the current `find` query in the active tab at once. The tab is changed in the window; nothing is written until `save`."#,
        params: &[],
    },
    Golden {
        name: "set_content",
        risk: "sensitive",
        description: r#"Replace everything in the active tab with this text. The file on disk is untouched until `save`; whatever was in the tab and unsaved is gone. To start a document with text, `new` with `text` does it without replacing anything."#,
        params: &[
            ("text", "string", true, r#"The tab's entire new text, up to 1 MiB and 20,000 lines of UTF-8 with no control characters. An empty string empties the tab."#),
        ],
    },
    Golden {
        name: "append",
        risk: "standard",
        description: r#"Add this text to the end of the active tab; on a new, empty tab it is the whole text. Nothing already in the tab is changed, and the file on disk is untouched until `save`."#,
        params: &[
            ("text", "string", true, r#"The text to add, exactly as given: start it with a newline to begin a new line. The whole tab has to stay within 1 MiB and 20,000 lines."#),
        ],
    },
    Golden {
        name: "undo",
        risk: "standard",
        description: r#"Take back the last change to the active tab, including a whole-tab `set_content` or `replace-all`. Up to 64 steps are kept per tab and none of it touches the file."#,
        params: &[],
    },
    Golden {
        name: "redo",
        risk: "standard",
        description: r#"Put back the change `undo` took off the active tab. Editing the tab in any other way discards what redo was holding."#,
        params: &[],
    },
    Golden {
        name: "select_tab",
        risk: "standard",
        description: r#"Make one of the open tabs the active one, by its position in `describe.tabs`. Nothing is saved, closed or changed."#,
        params: &[
            ("index", "number", true, r#"Which tab, counting from 0 in the order `describe.tabs` lists them; `describe.active_tab` is the one in front now."#),
        ],
    },
];

#[test]
fn the_action_table_is_the_one_the_editor_always_published() {
    let mut table = Intents::<TextEditor>::new();
    TextEditor::intents(&mut table);
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
    // The framework's own hand-over between two launches; refused to agents, graded safe.
    expected.push("launch".into());
    assert_eq!(published, expected);
}

/// Every key `describe` gave, with its value for a fresh editor: one empty tab.
#[test]
fn describe_has_the_same_keys_and_values() {
    let app = started();
    let s = state(&app);
    let mut keys: Vec<&str> = s.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut want = vec![
        "path", "title", "modified", "lines", "characters", "content", "content_cut", "read_with",
        "content_hidden", "bytes", "language", "tabs", "active_tab", "busy", "notice", "dialog", "dialog_is",
        "recovery", "cursor_line", "cursor_column", "find_query", "find_count", "replacement",
    ];
    want.sort_unstable();
    assert_eq!(keys, want);
    assert_eq!(
        s,
        json!({
            "path": null, "title": "Untitled", "modified": false, "lines": 1, "characters": 0, "content": "",
            "content_cut": false, "read_with": null, "content_hidden": null, "bytes": 0,
            "language": "Plain text",
            "tabs": [{ "name": "Untitled", "path": null, "modified": false, "opened_by": "person", "opened_by_you": true }],
            "active_tab": 0, "busy": false, "notice": "", "dialog": 0, "dialog_is": "",
            "recovery": "Draft recovery ready", "cursor_line": 1, "cursor_column": 1, "find_query": "",
            "find_count": 0, "replacement": "",
        })
    );
    assert_eq!(summary(&app), "Text Editor — Untitled (no file yet), 1 line, saved");
}

/// The summary in the states it has words for.
#[test]
fn the_summary_says_what_it_always_said() {
    let app = started();
    act(&app, "new", json!({ "text": "one\ntwo\n" })).unwrap();
    assert_eq!(summary(&app), "Text Editor — Untitled (no file yet), 3 lines, unsaved · tab 2 of 2");
    act(&app, "new", json!({ "text": "x" })).unwrap();
    assert_eq!(
        summary(&app),
        "Text Editor — Untitled (no file yet), 1 line, unsaved · tab 3 of 3 (2 tabs unsaved)"
    );
    act(&app, "close", json!({})).unwrap();
    assert_eq!(
        summary(&app),
        "Text Editor — Untitled (no file yet), 1 line, unsaved · tab 3 of 3 (2 tabs unsaved) · asking about \
         unsaved changes: save, discard or cancel"
    );
    let s = state(&app);
    assert_eq!(s["dialog"], 3);
    assert_eq!(s["dialog_is"], "Untitled has unsaved changes.");
    act(&app, "cancel", json!({})).unwrap();
    assert_eq!(state(&app)["dialog_is"], "", "a question no longer asked is not described");
}

/// `docs/app-control.md` grades every one of these actions as the table does, in the editor's rows.
#[test]
fn the_control_doc_grades_every_action_as_the_table_does() {
    let doc = include_str!("../../../../docs/app-control.md");
    let mut graded: Vec<(String, String)> = Vec::new();
    for row in doc.lines().filter(|l| l.starts_with("| editor |")) {
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
