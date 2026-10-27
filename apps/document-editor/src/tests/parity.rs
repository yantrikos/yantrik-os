//! The parity pin: the action table and `describe` exactly as yDoc published them before the
//! framework. The table below was extracted mechanically from the old
//! `apps/document-editor/src/main.rs` at origin/main (`publish_control`'s `Action::new` chains),
//! not copied by hand from the new one, so a word changed in `intents.rs` fails here.

use serde_json::json;
use yantrik_app::app::App as _;
use yantrik_app::intents::Intents;

use super::{started, state, summary};
use crate::YDoc;

struct Golden {
    name: &'static str,
    risk: &'static str,
    description: &'static str,
    /// Name, JSON type, required, description.
    params: &'static [(&'static str, &'static str, bool, &'static str)],
}

const BEFORE: &[Golden] = &[
    Golden {
        name: "open",
        risk: "standard",
        description: r#"Open a Markdown document by path"#,
        params: &[
            ("path", "string", true, r#""#),
            ("discard", "boolean", false, r#"Open even though this document has unsaved changes, losing them"#),
        ],
    },
    Golden {
        name: "new",
        risk: "standard",
        description: r#"Start an empty document"#,
        params: &[],
    },
    Golden {
        name: "save",
        risk: "standard",
        description: r#"Write the document to the file it came from"#,
        params: &[],
    },
    Golden {
        name: "save_as",
        risk: "standard",
        description: r#"Write the document to a path and keep it there"#,
        params: &[
            ("path", "string", true, r#""#),
            ("overwrite", "boolean", false, r#"Replace a file that is already at that path"#),
        ],
    },
    Golden {
        name: "set_content",
        risk: "standard",
        description: r#"Replace the document with this Markdown"#,
        params: &[
            ("text", "string", true, r#""#),
        ],
    },
    Golden {
        name: "append",
        risk: "standard",
        description: r#"Add Markdown to the end of the document"#,
        params: &[
            ("text", "string", true, r#""#),
        ],
    },
    Golden {
        name: "find",
        risk: "safe",
        description: r#"Search the document and show the matches"#,
        params: &[
            ("query", "string", true, r#""#),
        ],
    },
    Golden {
        name: "replace_all",
        risk: "standard",
        description: r#"Replace every match in the document"#,
        params: &[
            ("find", "string", true, r#""#),
            ("with", "string", true, r#""#),
        ],
    },
    Golden {
        name: "export_markdown",
        risk: "standard",
        description: r#"Write a Markdown copy to a path, leaving the document in its own file"#,
        params: &[
            ("path", "string", true, r#""#),
        ],
    },
    Golden {
        name: "show",
        risk: "safe",
        description: r#"Bring the yDoc window forward"#,
        params: &[],
    },
];

#[test]
fn the_action_table_is_the_one_ydoc_always_published() {
    let mut table = Intents::<YDoc>::new();
    YDoc::intents(&mut table);
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

/// Every key `describe` gave, with its value for a fresh document.
#[test]
fn describe_has_the_same_keys_and_values() {
    let app = started();
    let s = state(&app);
    let mut keys: Vec<&str> = s.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut want = vec![
        "format", "path", "title", "content_hidden", "dirty", "recovered_draft", "words", "chars", "outline",
        "content", "content_chars_shown", "content_chars_total", "find_query", "find_matches", "prompt_open",
        "notice",
    ];
    want.sort_unstable();
    assert_eq!(keys, want);
    assert_eq!(
        s,
        json!({
            "format": "markdown", "path": null, "title": "Untitled", "content_hidden": null, "dirty": false,
            "recovered_draft": false, "words": 0, "chars": 0, "outline": [], "content": "",
            "content_chars_shown": 0, "content_chars_total": 0, "find_query": "", "find_matches": 0,
            "prompt_open": false, "notice": "",
        })
    );
    assert_eq!(summary(&app), "yDoc \u{2014} \u{201c}Untitled\u{201d}, 0 words, saved \u{b7} no file yet");
}

/// The values for a document with a heading, a body and a find: what a mind reads.
#[test]
fn describe_reports_a_documents_title_outline_and_matches() {
    let app = started();
    super::act(&app, "set_content", json!({ "text": "# Plan\n\nOne two.\n\n## Part\nTwo again.\n" })).unwrap();
    super::act(&app, "find", json!({ "query": "two" })).unwrap();
    let s = state(&app);
    assert_eq!(s["title"], "Plan");
    assert_eq!(s["dirty"], true);
    assert_eq!(s["words"], 8);
    assert_eq!(s["outline"][0], json!({ "title": "Plan", "level": 1, "line": 1, "offset": 0 }));
    assert_eq!(s["outline"][1]["title"], "Part");
    assert_eq!((s["find_query"].clone(), s["find_matches"].clone()), (json!("two"), json!(2)));
    assert_eq!(s["content_chars_shown"], s["content_chars_total"]);
    assert_eq!(summary(&app), "yDoc \u{2014} \u{201c}Plan\u{201d}, 8 words, unsaved \u{b7} no file yet");
}

/// `docs/app-control.md` grades every one of these actions as the table does, in yDoc's rows.
#[test]
fn the_control_doc_grades_every_action_as_the_table_does() {
    let doc = include_str!("../../../../docs/app-control.md");
    let mut graded: Vec<(String, String)> = Vec::new();
    for row in doc.lines().filter(|l| l.starts_with("| documents |")) {
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
