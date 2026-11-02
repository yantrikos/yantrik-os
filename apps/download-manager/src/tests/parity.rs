//! The parity pin: the action table and `describe` exactly as Download Manager published them
//! before the framework. The table below was extracted mechanically from the old
//! `apps/download-manager/src/main.rs` (`publish_control`, at 51c46525), with the string literals
//! evaluated as rustc reads them (a line ending in `\` joins the next, which is how `cancel`'s
//! description is one sentence), not copied by hand from the new intents, so a word changed in
//! `intents.rs` fails here. The keys of `describe` were extracted from the old `describe` closure
//! the same way, in the order it set them.

use serde_json::json;
use yantrik_app::app::App as _;
use yantrik_app::intents::Intents;

use super::{body, open, server::File};
use crate::Downloads;

struct Golden {
    name: &'static str,
    risk: &'static str,
    description: &'static str,
    /// Whether the action says the work is only started when it returns.
    deferred: bool,
    /// Name, JSON type, required, description.
    params: &'static [(&'static str, &'static str, bool, &'static str)],
}

const ID: (&str, &str, bool, &str) = ("id", "integer", true, "");

const BEFORE: &[Golden] = &[
    Golden {
        name: "add",
        risk: "sensitive",
        description: r#"Download a URL to this machine. It fetches the address in `url`."#,
        deferred: true,
        params: &[
            ("url", "string", true, r#"The http(s) URL to fetch"#),
            ("save_dir", "string", false, r#"Directory to save into; defaults to ~/Downloads"#),
            ("sha256", "string", false, r#"Expected SHA-256; the file is checked against it when it finishes"#),
        ],
    },
    Golden {
        name: "pause",
        risk: "standard",
        description: r#"Stop a running download, keeping what has arrived"#,
        deferred: true,
        params: &[ID],
    },
    Golden {
        name: "resume",
        risk: "standard",
        description: r#"Continue a paused or failed download"#,
        deferred: true,
        params: &[ID],
    },
    Golden {
        name: "cancel",
        risk: "sensitive",
        description: r#"Stop a download and delete the partial file. Deleting it cannot be undone; the download can only be started again from the beginning."#,
        deferred: true,
        params: &[ID],
    },
    Golden {
        name: "retry",
        risk: "standard",
        description: r#"Start a failed download again from the beginning"#,
        deferred: true,
        params: &[ID],
    },
    Golden {
        name: "verify",
        risk: "standard",
        description: r#"Hash a finished file again and compare it with a checksum"#,
        deferred: true,
        params: &[ID, ("sha256", "string", false, r#"Expected SHA-256; omit to re-check the one already recorded"#)],
    },
    Golden {
        name: "pause_all",
        risk: "standard",
        description: r#"Stop every running download"#,
        deferred: true,
        params: &[],
    },
    Golden {
        name: "resume_all",
        risk: "standard",
        description: r#"Continue every paused download"#,
        deferred: true,
        params: &[],
    },
    Golden {
        name: "clear_completed",
        risk: "standard",
        description: r#"Drop finished downloads from the list, keeping the files"#,
        deferred: false,
        params: &[],
    },
    Golden {
        name: "open_folder",
        risk: "standard",
        description: r#"Show a download's folder in the file manager"#,
        deferred: false,
        params: &[ID],
    },
];

/// The keys of `describe`'s state, in the order the old closure set them.
const KEYS: &[&str] = &[
    "downloads", "shown", "total", "active", "paused", "completed", "failed", "missing", "restored",
    "state_file", "notice", "speed", "save_dir",
];

#[test]
fn the_ten_actions_are_published_as_they_were() {
    let mut table = Intents::<Downloads>::new();
    Downloads::intents(&mut table);
    let specs = table.specs();
    let names: Vec<&str> = specs.iter().map(|a| a.name.as_str()).collect();
    let before: Vec<&str> = BEFORE.iter().map(|g| g.name).collect();
    assert_eq!(names, before, "the same actions, in the same order");
    for (spec, golden) in specs.iter().zip(BEFORE) {
        let name = golden.name;
        assert_eq!(spec.description, golden.description, "{name}: description");
        assert_eq!(spec.permission, golden.risk, "{name}: grade");
        assert_eq!(spec.expected_seconds, None, "{name}: expected seconds");
        assert_eq!(spec.deferred, golden.deferred, "{name}: deferred");
        assert!(!spec.stateless && spec.explainer.is_none(), "{name}");
        let params: Vec<(&str, &str, bool, &str)> =
            spec.params.iter().map(|p| (p.name.as_str(), p.kind, p.required, p.description.as_str())).collect();
        assert_eq!(params, golden.params, "{name}: parameters");
    }
}

/// What a caller sees in `describe`: the ten, and the framework's `launch`, which every app on
/// the framework publishes. (`present`, the framework's other one, is added by the runner when it
/// serves the surface.)
#[test]
fn describe_lists_the_ten_and_the_frameworks_launch() {
    let f = open(vec![]);
    let actions = f.app.describe()["actions"].as_array().cloned().unwrap_or_default();
    let mut names: Vec<String> = actions.iter().filter_map(|a| a["name"].as_str().map(String::from)).collect();
    let mut expected: Vec<String> = BEFORE.iter().map(|g| g.name.to_string()).collect();
    expected.push("launch".to_string());
    names.sort();
    expected.sort();
    assert_eq!(names, expected);
    for golden in BEFORE {
        let action = actions.iter().find(|a| a["name"] == json!(golden.name)).expect("published");
        assert_eq!(action["permission"], json!(golden.risk), "{}", golden.name);
    }
}

/// docs/app-control.md's rows for Download Manager say what the app is and what it publishes.
#[test]
fn the_guides_rows_are_still_true() {
    let doc = include_str!("../../../../docs/app-control.md");
    let surface = doc.lines().find(|l| l.starts_with("| `download-manager` |")).expect("the surfaces table names it");
    let cells: Vec<&str> = surface.split('|').map(str::trim).collect();
    assert_eq!(cells[2], "`app-download-manager.sock`", "the socket is the id's: {surface}");
    assert_eq!(cells[3], "`downloads`");
    let mut listed: Vec<(String, String)> = Vec::new();
    for row in doc.lines().filter(|l| l.starts_with("| download-manager | `")) {
        let cells: Vec<&str> = row.split('|').map(str::trim).collect();
        for name in cells[2].split(',') {
            listed.push((name.trim().trim_matches('`').to_string(), cells[3].to_string()));
        }
    }
    listed.sort();
    let mut all: Vec<(String, String)> = BEFORE.iter().map(|g| (g.name.to_string(), g.risk.to_string())).collect();
    all.sort();
    assert_eq!(listed, all, "every action once, at the grade the code holds");
}

fn keys(state: &serde_json::Value) -> Vec<String> {
    let mut keys: Vec<String> = state.as_object().expect("an object").keys().cloned().collect();
    keys.sort();
    keys
}

fn expected() -> Vec<String> {
    let mut keys: Vec<String> = KEYS.iter().map(|k| k.to_string()).collect();
    keys.sort();
    keys
}

#[test]
fn nothing_queued_is_described_as_it_was() {
    let f = open(vec![]);
    let d = f.app.describe();
    assert_eq!(d["summary"], json!("Downloads \u{2014} nothing queued"));
    let s = &d["state"];
    assert_eq!(keys(s), expected());
    let list = f.paths.data_dir().join("list/state.json");
    assert_eq!(
        *s,
        json!({
            "downloads": [],
            "shown": 0,
            "total": 0,
            "active": 0,
            "paused": 0,
            "completed": 0,
            "failed": 0,
            "missing": 0,
            "restored": 0,
            "state_file": list.to_string_lossy(),
            "notice": "",
            "speed": "0 B/s",
            "save_dir": f.default_dir().to_string_lossy(),
        })
    );
}

#[test]
fn a_finished_download_is_described_as_it_was() {
    let data = body(70_000);
    let f = open(vec![("/paper.pdf", File::plain(data.clone()))]);
    let id = f.download("/paper.pdf");
    let d = f.app.describe();
    assert_eq!(d["summary"], json!("Downloads \u{2014} 1 finished, nothing running"));
    assert_eq!(keys(&d["state"]), expected());
    let row = f.row(id);
    let path = f.default_dir().join("paper.pdf");
    assert_eq!(
        row,
        json!({
            "id": id,
            "filename": "paper.pdf",
            "status": "completed",
            "size": "68.4 KB / 68.4 KB",
            "path": path.to_string_lossy(),
            "url": f.server.url("/paper.pdf"),
            "percent": 100,
            "sha256": super::sha256_of(&data),
        })
    );
    assert_eq!((d["state"]["shown"].clone(), d["state"]["total"].clone(), d["state"]["completed"].clone()), (json!(1), json!(1), json!(1)));
    assert_eq!(std::fs::read(&path).unwrap(), data, "the file is the body, byte for byte");
}

#[test]
fn a_failed_download_leads_the_summary_as_it_did() {
    let f = open(vec![]);
    let answer = f.act("add", json!({ "url": f.server.url("/missing.bin") })).unwrap();
    let id = answer["id"].as_i64().unwrap();
    f.until("the server's refusal is in", |f| f.row(id)["status"] == "failed");
    assert_eq!(f.summary(), "Downloads \u{2014} 1 failed (missing.bin: server said HTTP 404), 0 active");
    assert_eq!(f.row(id)["error"], json!("server said HTTP 404"));
}
