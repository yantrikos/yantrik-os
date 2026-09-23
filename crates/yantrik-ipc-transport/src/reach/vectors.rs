//! `deploy/yantrik-os/reach-vectors.json`: the reach's rule written out for every other door to
//! replay — the Python surface SDK first (`sdk/python/tests/test_reach.py`).
//!
//! Two sections:
//!
//! * `within` — a reach, one call to it (the app, the action, the grade its surface publishes, and
//!   the arguments as sent), and the exact refusal, or `null` when the reach lets it through.
//!   `opens` beside it says which app each name an opening act carries opens, standing in for the
//!   shell's own resolution (`resolve_opened_apps_with`).
//! * `standing` — what the shell answered for a token, or why it did not answer (`unanswered`),
//!   and what a door decides from it: the reach the call is held to (`held_to`, `null` for a plain
//!   agent), or the exact refusal.
//!
//! Generated, never edited: `reach_vectors_are_what_this_rule_decides` fails the build when the
//! file and this rule differ. After a deliberate change, rewrite it:
//!
//! ```text
//! YANTRIK_WRITE_VECTORS=1 cargo test -p yantrik-ipc-transport --lib reach_vectors_write
//! ```

use serde_json::{json, Value};

use super::*;

/// Which app each name opens, as far as the vectors need: our apps by id, case and one alias, and
/// the desktop's own things — screens, the launcher, the browser, the shell — as no app at all.
const OPENS: [(&str, Option<&str>); 13] = [
    ("notes", Some("notes")),
    ("Notes", Some("notes")),
    ("calendar", Some("calendar")),
    ("editor", Some("editor")),
    ("text-editor", Some("editor")),
    ("documents", Some("documents")),
    ("terminal", Some("terminal")),
    ("settings", None),
    ("problems", None),
    ("files", None),
    ("browser", None),
    ("launchpad", None),
    ("shell", None),
];

fn opens(name: &str) -> Option<String> {
    OPENS.iter().find(|(n, _)| *n == name).and_then(|(_, app)| app.map(str::to_string))
}

fn role(agent: &str, id: &str, name: &str, surfaces: &[&str], ceiling: &str) -> Reach {
    Reach {
        agent: agent.into(),
        role: id.into(),
        name: name.into(),
        surfaces: surfaces.iter().map(|s| s.to_string()).collect(),
        ceiling: ceiling.into(),
    }
}

/// The shipped roles whose reaches differ in kind, and two of a person's own.
fn reaches() -> Vec<Reach> {
    vec![
        role("deepseek:c-p1an", "planner", "Planner", &["calendar", "notes"], "safe"),
        role("deepseek:c-rev1", "reviewer", "Reviewer", &["editor", "documents", "notes"], "safe"),
        role("pi:c-c0de", "coder", "Coder", &["shell.agent_*", "shell.editor_*", "editor"], "sensitive"),
        role("pi:c-red7", "red-team", "Red team", &[], "safe"),
        role("pi:c-read", "reader", "Reader", &["notes.list_*", "notes.read_note"], "safe"),
        role("pi:c-open", "opener", "Opener", &["shell.open_app"], "standard"),
        role("pi:c-typo", "typo", "Typo", &["notes"], "sensitve"),
    ]
}

/// One call each: `(app, action, the grade its surface publishes, args as sent)`.
fn calls() -> Vec<(&'static str, &'static str, &'static str, Value)> {
    let open = |name: Value| json!({ "name": name });
    vec![
        ("shell", "open_app", "standard", open(json!("notes"))),
        ("shell", "open_app", "standard", open(json!("Notes"))),
        ("shell", "open_app", "standard", open(json!(" calendar "))),
        ("shell", "open_app", "standard", open(json!("text-editor"))),
        ("shell", "open_app", "standard", open(json!("terminal"))),
        ("shell", "open_app", "standard", open(json!("settings"))),
        ("shell", "open_app", "standard", open(json!("problems"))),
        ("shell", "open_app", "standard", open(json!("browser"))),
        ("shell", "open_app", "standard", open(json!("launchpad"))),
        ("shell", "open_app", "standard", open(json!("no-such-app"))),
        ("shell", "open_app", "standard", open(json!(7))),
        ("shell", "open_app", "standard", open(json!(""))),
        ("shell", "open_app", "standard", json!({})),
        ("shell", "show_app", "standard", open(json!("notes"))),
        ("shell", "show_app", "standard", open(json!("terminal"))),
        ("shell", "close_window", "standard", json!({ "title": "Notes" })),
        ("shell", "agent_run", "sensitive", json!({ "command": "ls" })),
        ("shell", "editor_open", "standard", json!({})),
        ("shell", "request_approval", "safe", json!({ "app": "notes", "action": "new_note" })),
        ("shell", "files_delete", "dangerous", json!({ "name": "x" })),
        ("notes", "list_notes", "safe", json!({})),
        ("notes", "read_note", "safe", json!({ "title": "Plan" })),
        ("notes", "new_note", "standard", json!({})),
        ("notes", "open_app", "standard", open(json!("notes"))),
        ("notes", "nuke", "catastrophic", json!({})),
        ("editor", "save", "standard", json!({})),
        ("files", "move", "safe", json!({})),
    ]
}

fn within_section() -> Vec<Value> {
    let mut out = Vec::new();
    for reach in reaches() {
        for (app, action, grade, args) in calls() {
            let refusal = within_call_with(&reach, app, action, grade, &args, &opens).err();
            out.push(json!({
                "reach": reach,
                "app": app,
                "action": action,
                "grade": grade,
                "args": args,
                "refusal": refusal,
            }));
        }
    }
    out
}

fn standing_section() -> Vec<Value> {
    let planner = reaches().remove(0);
    let answers = [
        json!({ "standing": "held", "reach": planner }),
        json!({ "standing": "plain" }),
        json!({ "standing": "unknown" }),
        json!({ "standing": "sideways" }),
        json!({ "standing": "held" }),
        json!({ "standing": "held", "reach": { "agent": "pi:c-1" } }),
        json!("spent"),
    ];
    let mut out: Vec<Value> = answers
        .into_iter()
        .map(|answer| {
            let decided = decided(Standing::from_json(&answer));
            json!({
                "answer": answer,
                "held_to": decided.as_ref().ok().cloned().flatten(),
                "refusal": decided.err(),
            })
        })
        .collect();
    for why in [
        "Connection failed (/run/user/1000/yantrik/app-shell.sock): No such file or directory (os error 2)",
        "the process answering as the shell is /usr/bin/python3 (pid 4242), not the desktop's own yantrik-ui, so it was not asked",
    ] {
        let decided = decided(Err(why.to_string()));
        out.push(json!({ "unanswered": why, "held_to": null, "refusal": decided.err() }));
    }
    out
}

fn document() -> String {
    let header = json!([
        "Generated. Do not hand-edit: YANTRIK_WRITE_VECTORS=1 cargo test -p yantrik-ipc-transport --lib reach_vectors_write",
        "",
        "An agent's reach (crates/yantrik-ipc-transport/src/reach.rs), written out for every door to replay: the Python",
        "SDK replays both sections.",
        "",
        "within: a `reach`, and one call to it — the `app`, the `action`, the `grade` its surface publishes and the `args`",
        "as sent. `refusal` is the exact sentence, or null when the reach lets the call through. `opens` says which app each",
        "name an opening act (shell.open_app, shell.show_app) carries opens; a name not in it opens no app.",
        "",
        "standing: what the shell answered to agent.reach for a token (`answer`), or why it did not answer",
        "(`unanswered`), and what a door decides from it: the reach the call is held to (`held_to`, null for a live",
        "agent with no role), or the exact `refusal`.",
    ]);
    let opens: serde_json::Map<String, Value> =
        OPENS.iter().map(|(name, app)| (name.to_string(), json!(app))).collect();
    let lines = |items: &[Value]| items.iter().map(|v| format!("    {v}")).collect::<Vec<_>>().join(",\n");
    format!(
        "{{\n  \"_\": {},\n  \"opens\": {},\n  \"within\": [\n{}\n  ],\n  \"standing\": [\n{}\n  ]\n}}\n",
        serde_json::to_string_pretty(&header).unwrap().replace('\n', "\n  "),
        Value::Object(opens),
        lines(&within_section()),
        lines(&standing_section()),
    )
}

fn vectors_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/yantrik-os/reach-vectors.json")
}

/// Write the vectors out. Gated, because a test that rewrites its own expectation is not a test.
#[test]
fn reach_vectors_write() {
    if std::env::var("YANTRIK_WRITE_VECTORS").as_deref() != Ok("1") {
        return;
    }
    std::fs::write(vectors_path(), document()).expect("write the vectors");
}

#[test]
fn reach_vectors_are_what_this_rule_decides() {
    let path = vectors_path();
    let checked_in = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert!(
        checked_in == document(),
        "deploy/yantrik-os/reach-vectors.json is not what the reach decides any more. If the change is \
         deliberate, rewrite it and change every port with it:\n  \
         YANTRIK_WRITE_VECTORS=1 cargo test -p yantrik-ipc-transport --lib reach_vectors_write"
    );
}

/// The file says what #195 and #189 decided, not only whatever the code happened to do: the rows
/// the rules exist for, checked by meaning.
#[test]
fn the_vectors_hold_the_rules_they_exist_for() {
    let doc: Value = serde_json::from_str(&document()).expect("json");
    let row = |role: &str, app: &str, action: &str, args: Value| {
        doc["within"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["reach"]["role"] == role && r["app"] == app && r["action"] == action && r["args"] == args)
            .unwrap_or_else(|| panic!("no row for {role} {app}.{action} {args}"))["refusal"]
            .clone()
    };
    let open = |name: &str| json!({ "name": name });
    // The Planner opens Notes and Calendar, and brings Notes forward, at `safe`…
    assert_eq!(row("planner", "shell", "open_app", open("notes")), Value::Null);
    assert_eq!(row("planner", "shell", "open_app", open(" calendar ")), Value::Null);
    assert_eq!(row("planner", "shell", "show_app", open("notes")), Value::Null);
    // …and nothing else: no other app, no screen, not the launcher or the browser.
    for name in ["terminal", "settings", "problems", "browser", "launchpad", "text-editor", "no-such-app"] {
        assert!(row("planner", "shell", "open_app", open(name)).is_string(), "{name}");
    }
    // Opened, it reads and writes nothing.
    assert_eq!(row("planner", "notes", "list_notes", json!({})), Value::Null);
    assert!(row("planner", "notes", "new_note", json!({})).is_string());
    // An alias opens the app it names.
    assert_eq!(row("reviewer", "shell", "open_app", open("text-editor")), Value::Null);
    // A reach naming only some of an app's actions still names the app.
    assert_eq!(row("reader", "shell", "open_app", open("notes")), Value::Null);
    // `shell.agent_*` names no app, and the Red team none at all.
    assert!(row("coder", "shell", "open_app", open("terminal")).is_string());
    assert!(row("red-team", "shell", "open_app", open("notes")).is_string());
    // A ceiling off the ladder does not keep an app from opening, and opens nothing inside it.
    assert_eq!(row("typo", "shell", "open_app", open("notes")), Value::Null);
    assert!(row("typo", "notes", "list_notes", json!({})).is_string());

    let standing = doc["standing"].as_array().unwrap();
    assert_eq!(standing[0]["held_to"]["role"], "planner");
    assert_eq!((standing[1]["held_to"].clone(), standing[1]["refusal"].clone()), (Value::Null, Value::Null), "a plain agent");
    for refused in &standing[2..] {
        assert!(refused["refusal"].as_str().is_some_and(|r| r.starts_with("REACH: ")), "{refused}");
    }
}
