//! The parity pin: the action table and `describe` exactly as Snippets published them before the
//! framework. The table and the key lists below were extracted mechanically from the old
//! `apps/snippet-manager/src/main.rs` at 2be8dd24 (`publish_control`), with the string literals
//! evaluated as rustc reads them, not copied by hand from the new intents, so a word changed in
//! `intents.rs` fails here.

use serde_json::{json, Value};
use yantrik_app::app::App as _;
use yantrik_app::intents::Intents;

use super::open;
use crate::SnippetManager;

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
        description: r#"Keep a new snippet"#,
        params: &[
            ("title", "string", true, r#""#),
            ("language", "string", true, r#"Rust, Python, Shell, …"#),
            ("code", "string", true, r#"The snippet itself"#),
            ("tags", "string", false, r#"Comma-separated"#),
        ],
    },
    Golden {
        name: "save",
        risk: "standard",
        description: r#"Change a snippet that is already kept"#,
        params: &[
            ("id", "string", true, r#"Its id, or its title"#),
            ("title", "string", false, r#""#),
            ("language", "string", false, r#""#),
            ("code", "string", false, r#""#),
            ("tags", "string", false, r#""#),
            ("favorite", "boolean", false, r#""#),
            ("collection", "string", false, r#"A collection by name or id; empty files it under none"#),
        ],
    },
    Golden {
        name: "open",
        risk: "safe",
        description: r#"Show a snippet in the editor"#,
        params: &[
            ("id", "string", true, r#"Its id, or its title"#),
        ],
    },
    Golden {
        name: "search",
        risk: "safe",
        description: r#"Narrow the list to what matches"#,
        params: &[
            ("query", "string", true, r#"Matched against title, code, tags and language"#),
        ],
    },
    Golden {
        name: "copy",
        risk: "standard",
        description: r#"Put a snippet's code on the clipboard"#,
        params: &[
            ("id", "string", true, r#"Its id, or its title"#),
        ],
    },
    Golden {
        name: "delete",
        risk: "sensitive",
        description: r#"Throw a snippet away. There is no trash to take it out of: it cannot be undone."#,
        params: &[
            ("id", "string", true, r#"Its id, or its title"#),
        ],
    },
    Golden {
        name: "toggle_favorite",
        risk: "standard",
        description: r#"Star or unstar a snippet"#,
        params: &[
            ("id", "string", true, r#"Its id, or its title"#),
        ],
    },
    Golden {
        name: "import",
        risk: "standard",
        description: r#"Keep a file as a snippet, or read back an export"#,
        params: &[
            ("path", "string", true, r#"A source file, or a file `export` wrote"#),
        ],
    },
    Golden {
        name: "show",
        risk: "safe",
        description: r#"Bring the window forward"#,
        params: &[],
    },
];

const KEYS: &[&str] = &["total", "matching", "shown", "snippets", "favorites", "query", "tag", "collection", "collections", "open", "open_id", "open_language", "open_favorite", "open_lines", "store", "copied_recently", "notice"];
const ROW_KEYS: &[&str] = &["id", "title", "language", "tags", "favorite", "collection", "lines", "preview"];

#[test]
fn the_nine_actions_are_published_as_they_were() {
    let mut table = Intents::<SnippetManager>::new();
    SnippetManager::intents(&mut table);
    let specs = table.specs();
    let names: Vec<&str> = specs.iter().map(|a| a.name.as_str()).collect();
    let before: Vec<&str> = BEFORE.iter().map(|g| g.name).collect();
    assert_eq!(names, before, "the same actions, in the same order");
    for (spec, golden) in specs.iter().zip(BEFORE) {
        let name = golden.name;
        assert_eq!(spec.description, golden.description, "{name}: description");
        assert_eq!(spec.permission, golden.risk, "{name}: grade");
        assert!(!spec.deferred, "{name}: answered on return, as before");
        assert!(!spec.stateless && spec.expected_seconds.is_none() && spec.explainer.is_none(), "{name}");
        let params: Vec<(&str, &str, bool, &str)> = spec
            .params
            .iter()
            .map(|p| (p.name.as_str(), p.kind, p.required, p.description.as_str()))
            .collect();
        assert_eq!(params, golden.params, "{name}: parameters");
    }
}

/// What a caller sees in `describe`: the nine, and the framework's `launch`, which every app on
/// the framework publishes. (`present`, the framework's other one, is added by the runner when it
/// serves the surface.)
#[test]
fn describe_lists_the_nine_and_the_frameworks_launch() {
    let f = open();
    let actions = f.describe()["actions"].as_array().cloned().unwrap_or_default();
    let mut names: Vec<String> = actions.iter().filter_map(|a| a["name"].as_str().map(String::from)).collect();
    let mut expected: Vec<String> = BEFORE.iter().map(|g| g.name.to_string()).collect();
    expected.push("launch".to_string());
    names.sort();
    expected.sort();
    assert_eq!(names, expected);
    for golden in BEFORE {
        let action = actions.iter().find(|a| a["name"] == json!(golden.name)).expect("published");
        assert_eq!(action["permission"], json!(golden.risk), "{}", golden.name);
        assert_eq!(action["settles"], json!("on return"), "{}", golden.name);
    }
}

/// docs/app-control.md's grade table says what the app publishes, row for row, and its surface
/// row names the socket the framework serves.
#[test]
fn the_guide_is_still_true() {
    let doc = include_str!("../../../../docs/app-control.md");
    let mut listed = Vec::new();
    for row in doc.lines().filter(|l| l.starts_with("| snippets | `")) {
        let cells: Vec<&str> = row.split('|').map(str::trim).collect();
        let grade = cells[3];
        for name in cells[2].split(',').map(|n| n.trim().trim_matches('`')) {
            let golden = BEFORE.iter().find(|g| g.name == name).unwrap_or_else(|| panic!("{name} is not an action"));
            assert_eq!(golden.risk, grade, "{name}");
            listed.push(name);
        }
    }
    let mut all: Vec<&str> = BEFORE.iter().map(|g| g.name).collect();
    all.sort();
    listed.sort();
    assert_eq!(listed, all);
    assert!(doc.contains("| `snippets` | `app-snippets.sock` | `snippet-manager` |"));
}

fn keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value.as_object().expect("an object").keys().cloned().collect();
    keys.sort();
    keys
}

fn sorted(list: &[&str]) -> Vec<String> {
    let mut keys: Vec<String> = list.iter().map(|k| k.to_string()).collect();
    keys.sort();
    keys
}

/// The keys as before, and the one added on purpose: `recovery`, what the recovery bar offers
/// back (titles only), as Notes' `describe.recovery`.
fn described() -> Vec<String> {
    let mut keys = KEYS.to_vec();
    keys.push("recovery");
    sorted(&keys)
}

#[test]
fn an_empty_store_is_described_as_it_was() {
    let f = open();
    let d = f.describe();
    assert_eq!(d["summary"], json!("Snippets \u{2014} nothing kept yet"));
    let s = &d["state"];
    assert_eq!(keys(s), described());
    assert_eq!(
        *s,
        json!({
            "total": 0, "matching": 0, "shown": 0, "snippets": [], "favorites": 0,
            "query": "", "tag": "", "collection": "All Snippets", "collections": [],
            "open": null, "open_id": null, "open_language": null, "open_favorite": null, "open_lines": null,
            "store": f.path().display().to_string(), "copied_recently": false, "notice": "", "recovery": [],
        })
    );
}

#[test]
fn a_store_with_a_snippet_open_is_described_as_it_was() {
    let f = open();
    let (journal, borrow) = f.seeded();
    f.act("toggle_favorite", json!({ "id": journal.to_string() })).unwrap();
    f.act("open", json!({ "id": "journal" })).unwrap();
    let d = f.describe();
    assert_eq!(d["summary"], json!("Snippets \u{2014} \u{201c}Tail the journal\u{201d}, 2 kept"));
    let s = &d["state"];
    assert_eq!(keys(s), described());
    for row in s["snippets"].as_array().unwrap() {
        assert_eq!(keys(row), sorted(ROW_KEYS));
    }
    assert_eq!(s["total"], json!(2));
    assert_eq!(s["favorites"], json!(1));
    assert_eq!(s["open"], json!("Tail the journal"));
    assert_eq!(s["open_id"], json!(journal));
    assert_eq!(s["open_language"], json!("Shell"));
    assert_eq!(s["open_favorite"], json!(true));
    assert_eq!(s["open_lines"], json!(1));
    let row = s["snippets"].as_array().unwrap().iter().find(|r| r["id"] == json!(borrow)).cloned().unwrap();
    assert_eq!(
        row,
        json!({
            "id": borrow, "title": "Borrow checker dance", "language": "Rust", "tags": ["rust", "borrow"],
            "favorite": false, "collection": "All Snippets", "lines": 1,
            "preview": "let mut guard = state.borrow_mut();",
        })
    );
    // A search narrows the summary as it did.
    f.act("search", json!({ "query": "BORROW" })).unwrap();
    assert_eq!(
        f.describe()["summary"],
        json!("Snippets \u{2014} \u{201c}Tail the journal\u{201d}, 1 of 2 match \u{201c}BORROW\u{201d}")
    );
    f.app.run(crate::Command::Select(-1));
    assert_eq!(f.describe()["summary"], json!("Snippets \u{2014} 1 of 2 match \u{201c}BORROW\u{201d}, none open"));
    f.act("search", json!({ "query": " " })).unwrap();
    assert_eq!(f.describe()["summary"], json!("Snippets \u{2014} 2 kept, none open"));
}
