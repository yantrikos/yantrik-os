//! The parity pin: the action table and `describe` exactly as yPresent published them before the
//! framework. The table below was extracted mechanically from the old
//! `apps/presentation/src/main.rs` at 4e24dff7 (`publish_control`'s `Action::new` chains: name,
//! description, `.risk`, and each `.arg(Param::…)` with its `.describe` and `.optional`), not copied
//! by hand from the new one, so a word changed in `intents.rs` fails here.
//!
//! One name is not the old one: `present`, which the framework reserves for the desktop's own
//! hand-over (`yantrik_app::intents::RESERVED`). It is `presenter_view` now, with the same
//! description, `mode` parameter and grade; `RENAMED` is the one place that says so.

use serde_json::json;
use yantrik_app::app::App as _;
use yantrik_app::intents::Intents;

use super::{started, state, summary};
use crate::YPresent;

struct Golden {
    name: &'static str,
    risk: &'static str,
    description: &'static str,
    /// Name, JSON type, required, description.
    params: &'static [(&'static str, &'static str, bool, &'static str)],
}

/// The one parameter added since: `save_as`'s optional `overwrite`, worded as Document Editor's.
/// The framework's save refuses a file already there, so without it an agent could not replace one.
const ADDED: &str = "overwrite";

/// The old names the framework does not allow an app to keep, and what they are now.
const RENAMED: &[(&str, &str)] = &[("present", "presenter_view")];

/// What an action is called now.
fn now(old: &str) -> &str {
    RENAMED.iter().find(|(was, _)| *was == old).map_or(old, |(_, is)| is)
}

const BEFORE: &[Golden] = &[
    Golden {
        name: "open",
        risk: "standard",
        description: r#"Open a deck file in this window"#,
        params: &[
            ("path", "string", true, r#"Path to a .ydeck file"#),
        ],
    },
    Golden {
        name: "show",
        risk: "standard",
        description: r#"Bring the window forward"#,
        params: &[],
    },
    Golden {
        name: "save",
        risk: "standard",
        description: r#"Write the deck to its file"#,
        params: &[],
    },
    Golden {
        name: "save_as",
        risk: "standard",
        description: r#"Write the deck to a path of your choosing"#,
        params: &[
            ("path", "string", true, r#"Where to write the .ydeck file"#),
        ],
    },
    Golden {
        name: "new_deck",
        risk: "standard",
        description: r#"Start an empty deck"#,
        params: &[
            ("title", "string", false, r#"What to call it"#),
        ],
    },
    Golden {
        name: "add_slide",
        risk: "standard",
        description: r#"Add a slide and select it"#,
        params: &[
            ("title", "string", false, r#""#),
            ("body", "string", false, r#""#),
            ("notes", "string", false, r#""#),
            ("after", "number", false, r#"Insert after this slide index"#),
        ],
    },
    Golden {
        name: "set_slide",
        risk: "standard",
        description: r#"Change the text of one slide"#,
        params: &[
            ("index", "number", true, r#""#),
            ("title", "string", false, r#""#),
            ("body", "string", false, r#""#),
            ("notes", "string", false, r#""#),
        ],
    },
    Golden {
        name: "delete_slide",
        risk: "sensitive",
        description: r#"Remove a slide from the deck"#,
        params: &[
            ("index", "number", true, r#""#),
        ],
    },
    Golden {
        name: "move_slide",
        risk: "standard",
        description: r#"Move a slide to another position"#,
        params: &[
            ("from", "number", true, r#""#),
            ("to", "number", true, r#""#),
        ],
    },
    Golden {
        name: "go_to",
        risk: "standard",
        description: r#"Select a slide by index"#,
        params: &[
            ("index", "number", true, r#""#),
        ],
    },
    Golden {
        name: "next",
        risk: "standard",
        description: r#"The next slide"#,
        params: &[],
    },
    Golden {
        name: "previous",
        risk: "standard",
        description: r#"The previous slide"#,
        params: &[],
    },
    Golden {
        name: "present",
        risk: "standard",
        description: r#"Enter or leave the full-screen presenter view"#,
        params: &[
            ("mode", "string", true, r#"on | fullscreen | true | start | off | windowed | exit | false | stop | toggle"#),
        ],
    },
    Golden {
        name: "export_markdown",
        risk: "standard",
        description: r#"Write the deck out as Markdown"#,
        params: &[
            ("path", "string", false, r#"Where to write it"#),
        ],
    },
];

#[test]
fn the_action_table_is_the_one_ypresent_always_published() {
    let mut table = Intents::<YPresent>::new();
    YPresent::intents(&mut table);
    let specs = table.specs();
    let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
    let before: Vec<&str> = BEFORE.iter().map(|g| now(g.name)).collect();
    assert_eq!(names, before, "the same actions, in the same order");
    for (spec, old) in specs.iter().zip(BEFORE) {
        assert_eq!(spec.description, old.description, "`{}`'s description", old.name);
        assert_eq!(spec.permission, old.risk, "`{}` is graded as it was", old.name);
        assert!(!spec.deferred, "`{}` answers when its work is done, as it did", old.name);
        assert!(!spec.stateless && spec.expected_seconds.is_none() && spec.explainer.is_none());
        let params: Vec<(&str, &str, bool, &str)> = spec
            .params
            .iter()
            .filter(|p| !(old.name == "save_as" && p.name == ADDED))
            .map(|p| (p.name.as_str(), p.kind, p.required, p.description.as_str()))
            .collect();
        assert_eq!(params, old.params.to_vec(), "`{}`'s arguments", old.name);
        for p in &spec.params {
            assert!(p.default.is_none() && p.values.is_empty() && p.items.is_none());
        }
    }
}

#[test]
fn save_as_has_the_one_added_parameter_worded_as_document_editors() {
    let mut table = Intents::<YPresent>::new();
    YPresent::intents(&mut table);
    let spec = table.specs().into_iter().find(|s| s.name == "save_as").unwrap();
    let added: Vec<_> = spec.params.iter().filter(|p| p.name == ADDED).collect();
    assert_eq!(added.len(), 1);
    let p = added[0];
    assert_eq!((p.kind, p.required, p.description.as_str()), ("boolean", false, "Replace a file that is already at that path"));
    assert_eq!(spec.permission, "standard", "and the grade is as it was");
}

#[test]
fn describe_publishes_those_actions_and_the_frameworks_launch() {
    let app = started();
    let published: Vec<String> =
        app.describe()["actions"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap().to_string()).collect();
    let mut expected: Vec<String> = BEFORE.iter().map(|g| now(g.name).to_string()).collect();
    // The framework's own hand-over between two launches; refused to agents, graded safe. (Its
    // `present` is added when the surface is served, which a test does not do.)
    expected.push("launch".into());
    published.iter().for_each(|p| assert!(expected.contains(p), "`{p}` is not one of the actions"));
    expected.iter().for_each(|e| assert!(published.contains(e), "`{e}` is not published"));
}

/// Every key `describe` gave for a deck in front, with its value for a fresh deck.
#[test]
fn describe_has_the_same_keys_and_values() {
    let app = started();
    let s = state(&app);
    let mut keys: Vec<&str> = s.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut want = vec![
        "file", "content_hidden", "dirty", "recovered", "slides", "index", "deck_title", "current", "outline",
        "outline_shown", "outline_total", "presenting", "theme", "timer", "timer_running", "search_matches",
        "saves_to", "recovery", "notice",
    ];
    want.sort_unstable();
    assert_eq!(keys, want);
    let (saves_to, recovery) = (s["saves_to"].clone(), s["recovery"].clone());
    assert!(saves_to.as_str().unwrap().ends_with("/untitled.ydeck"), "{saves_to}");
    assert!(recovery.as_str().unwrap().contains("/drafts/deck-"), "{recovery}");
    assert_eq!(
        s,
        json!({
            "file": null, "content_hidden": null, "dirty": false, "recovered": false, "slides": 1, "index": 0,
            "deck_title": "Untitled",
            "current": { "title": "Title Slide", "body": "Click to add subtitle", "notes": "", "layout": 0 },
            "outline": ["Title Slide"], "outline_shown": 1, "outline_total": 1, "presenting": false,
            "theme": "Slate", "timer": "00:00", "timer_running": false, "search_matches": 0,
            "saves_to": saves_to, "recovery": recovery, "notice": "",
        })
    );
    assert_eq!(summary(&app), "yPresent \u{2014} Untitled \u{b7} slide 1 of 1");
}

/// The values for a deck with slides, a search and an unsaved change: what a mind reads.
#[test]
fn describe_reports_the_slide_the_outline_the_matches_and_whether_it_is_saved() {
    let app = started();
    super::act(&app, "add_slide", json!({ "title": "Second", "body": "two words" })).unwrap();
    super::press(&app, crate::command::WindowAction::Search("two".into()));
    let s = state(&app);
    assert_eq!((s["slides"].clone(), s["index"].clone()), (json!(2), json!(1)));
    assert_eq!(s["current"], json!({ "title": "Second", "body": "two words", "notes": "", "layout": 1 }));
    assert_eq!(s["outline"], json!(["Title Slide", "Second"]));
    assert_eq!((s["search_matches"].clone(), s["dirty"].clone()), (json!(1), json!(true)));
    assert_eq!(summary(&app), "yPresent \u{2014} Untitled \u{b7} slide 2 of 2 \u{b7} unsaved");
}

/// The outline is capped, and the cap is said out loud.
#[test]
fn a_long_outline_is_capped_and_says_so() {
    let app = started();
    for n in 0..120 {
        super::act(&app, "add_slide", json!({ "title": format!("s{n}") })).unwrap();
    }
    let s = state(&app);
    assert_eq!(s["outline"].as_array().unwrap().len(), 100);
    assert_eq!((s["outline_shown"].clone(), s["outline_total"].clone()), (json!(100), json!(121)));
}

/// `docs/app-control.md` grades every one of these actions as the table does, in yPresent's rows.
#[test]
fn the_control_doc_grades_every_action_as_the_table_does() {
    let doc = include_str!("../../../../docs/app-control.md");
    let mut graded: Vec<(String, String)> = Vec::new();
    for row in doc.lines().filter(|l| l.starts_with("| presentation |")) {
        let cells: Vec<&str> = row.split('|').map(str::trim).collect();
        let (actions, grade) = (cells[2], cells[3]);
        for action in actions.split(',').map(|a| a.trim().trim_matches('`')) {
            graded.push((action.to_string(), grade.to_string()));
        }
    }
    graded.sort();
    let mut want: Vec<(String, String)> =
        BEFORE.iter().map(|g| (now(g.name).to_string(), g.risk.to_string())).collect();
    want.sort();
    assert_eq!(graded, want, "the doc lists every published action once, at its grade");
}
