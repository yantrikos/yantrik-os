//! The parity pin: the action table and `describe` exactly as Email published them before the
//! framework. The table below was extracted mechanically from the old `apps/email/src/main.rs`
//! (`publish_control`, at 649ab69d), not copied by hand from the new one, so a word changed in
//! `intents.rs` fails here. The grades are also held to `docs/app-control.md`'s rows for email.

use serde_json::{json, Value};
use yantrik_app::app::App as _;
use yantrik_app::intents::Intents;

use super::{act, start_with, started, state, summary, with_inbox};
use crate::backend::fake::{FakeMail, UNREACHABLE};
use crate::Email;

struct Golden {
    name: &'static str,
    risk: &'static str,
    description: &'static str,
    /// Name, required, description.
    params: &'static [(&'static str, bool, &'static str)],
}

const BEFORE: &[Golden] = &[
    Golden {
        name: "open_message",
        risk: "standard",
        description: r#"Open a message by row number, subject or sender"#,
        params: &[
            ("which", true, r#"The row number from `messages`, counting from 1 — or part of the subject, the sender's name, or their address"#),
        ],
    },
    Golden {
        name: "select_folder",
        risk: "standard",
        description: r#"Switch to another mailbox"#,
        params: &[
            ("folder", true, r#"Folder name, e.g. INBOX, Sent, Archive"#),
        ],
    },
    Golden {
        name: "search",
        risk: "standard",
        description: r#"Search the mailbox"#,
        params: &[
            ("query", true, r#""#),
        ],
    },
    Golden {
        name: "mark_read",
        risk: "standard",
        description: r#"Mark a message as read"#,
        params: &[
            ("which", false, r#"Row number, subject or sender; omit for the open message"#),
        ],
    },
    Golden {
        name: "flag",
        risk: "standard",
        description: r#"Flag a message"#,
        params: &[
            ("which", false, r#"Row number, subject or sender; omit for the open message"#),
        ],
    },
    Golden {
        name: "compose",
        risk: "standard",
        description: r#"Open the composer with a draft filled in. Does NOT send — the user reviews and sends it."#,
        params: &[
            ("to", true, r#""#),
            ("subject", true, r#""#),
            ("body", true, r#""#),
        ],
    },
    Golden {
        name: "begin_google_sign_in",
        risk: "sensitive",
        description: r#"Open Google's sign-in page to add a Gmail account. Cannot finish on its own — the user chooses the account and grants access in their browser."#,
        params: &[],
    },
];

#[test]
fn the_action_table_is_the_one_email_always_published() {
    let mut table = Intents::<Email>::new();
    Email::intents(&mut table);
    let specs = table.specs();
    let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
    let before: Vec<&str> = BEFORE.iter().map(|g| g.name).collect();
    assert_eq!(names, before, "the same actions, in the same order");
    for (spec, old) in specs.iter().zip(BEFORE) {
        assert_eq!(spec.description, old.description, "`{}`'s description", old.name);
        assert_eq!(spec.permission, old.risk, "`{}` is graded as it was", old.name);
        assert!(!spec.deferred, "`{}` answers when its work is done, as it did", old.name);
        assert!(!spec.stateless && spec.expected_seconds.is_none() && spec.explainer.is_none());
        let params: Vec<(&str, bool, &str)> =
            spec.params.iter().map(|p| (p.name.as_str(), p.required, p.description.as_str())).collect();
        assert_eq!(params, old.params.to_vec(), "`{}`'s arguments", old.name);
        for p in &spec.params {
            assert_eq!(p.kind, "string", "`{}.{}` is text, as it was", old.name, p.name);
            assert!(p.default.is_none() && p.values.is_empty() && p.items.is_none());
        }
    }
}

#[test]
fn describe_publishes_those_actions_and_the_frameworks_launch() {
    let app = started(&with_inbox());
    let published: Vec<String> = app.describe()["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap().to_string())
        .collect();
    let mut expected: Vec<String> = BEFORE.iter().map(|g| g.name.to_string()).collect();
    // The framework's own hand-over between two launches; refused to agents, graded safe.
    expected.push("launch".into());
    assert_eq!(published, expected);
}

/// `docs/app-control.md` grades every published action; its email rows say what this table does.
#[test]
fn the_grades_are_the_ones_docs_app_control_lists() {
    let doc = include_str!("../../../../docs/app-control.md");
    let mut listed = Vec::new();
    for line in doc.lines().filter(|l| l.starts_with("| email |")) {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        let (actions, grade) = (cells[2], cells[3]);
        for action in actions.split(',').map(|a| a.trim().trim_matches('`')) {
            // The services' method table names `email.*` methods, not the app's actions.
            if !action.starts_with("email.") {
                listed.push((action.to_string(), grade.to_string()));
            }
        }
    }
    let mut published: Vec<(String, String)> =
        BEFORE.iter().map(|g| (g.name.to_string(), g.risk.to_string())).collect();
    listed.sort();
    published.sort();
    assert_eq!(listed, published, "docs/app-control.md and the table agree");
}

/// The keys `describe` gave with the mail service unreachable: the four the base always carries,
/// and `has_account` null rather than false.
#[test]
fn an_unreachable_service_is_described_as_it_always_was() {
    let app = started(&FakeMail::unreachable());
    assert_eq!(summary(&app), format!("Email — the mail service could not be reached: {UNREACHABLE}"));
    assert_eq!(
        state(&app),
        json!({
            "service": "unreachable",
            "notice": format!("The mail service could not be reached: {UNREACHABLE}"),
            "account_store": "",
            "has_account": null,
        })
    );
}

#[test]
fn a_service_with_no_account_is_described_as_it_always_was() {
    let app = started(&FakeMail::no_account());
    assert_eq!(
        summary(&app),
        "Email — the mail service is running and no account is configured in /home/p/.config/yantrik/email.json"
    );
    assert_eq!(
        state(&app),
        json!({
            "service": "up",
            "notice": "",
            "account_store": "/home/p/.config/yantrik/email.json",
            "has_account": false,
        })
    );
}

/// Every key `describe` gave once there is a mailbox, with its value for INBOX holding three
/// messages and nothing open.
#[test]
fn a_mailbox_is_described_with_the_same_keys_and_values() {
    let app = started(&with_inbox());
    let s = state(&app);
    let mut keys: Vec<&str> = s.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut want = vec![
        "service", "notice", "account_store", "has_account", "account", "folder", "triage", "counts",
        "counts_reason", "listed", "composing", "search_query", "open_message", "folders", "messages",
    ];
    want.sort_unstable();
    assert_eq!(keys, want);
    assert_eq!(summary(&app), "Email — INBOX, 2 unread of 3");
    assert_eq!(s["service"], "up");
    assert_eq!(s["notice"], "");
    assert_eq!(s["account_store"], "/home/p/.config/yantrik/email.json");
    assert_eq!(s["has_account"], true);
    assert_eq!(s["account"], "you@example.com");
    assert_eq!(s["folder"], "INBOX");
    assert_eq!(s["triage"], "All");
    assert_eq!(s["counts"], json!({ "unread": 2, "total": 3 }));
    assert_eq!(s["counts_reason"], Value::Null);
    assert_eq!(s["listed"], 3);
    assert_eq!(s["composing"], false);
    assert_eq!(s["search_query"], "");
    assert_eq!(s["open_message"], Value::Null);
    assert_eq!(
        s["folders"],
        json!([
            { "name": "INBOX", "counts": { "unread": 2, "total": 3 }, "reason": null },
            { "name": "Archive", "counts": { "unread": 0, "total": 10 }, "reason": null },
            { "name": "Receipts", "counts": null, "reason": "the mail server did not report it: STATUS timed out" },
        ])
    );
    let first = &s["messages"][0];
    let mut keys: Vec<&str> = first.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["address", "date", "flagged", "from", "read", "row", "subject"]);
    assert_eq!(first["row"], 1);
    assert_eq!(first["subject"], "Lunch on Friday");
    assert_eq!(first["from"], "Priya R.");
    assert_eq!(first["address"], "Priya R. <priya@example.com>");
    assert_eq!((first["read"].clone(), first["flagged"].clone()), (json!(false), json!(false)));
    // The date is said the way a person says one, relative to today (`words::list_date`).
    assert_eq!(first["date"], crate::words::list_date("2026-10-20 10:35", chrono::Local::now().date_naive()));
    assert_eq!(s["messages"].as_array().unwrap().len(), 3);
}

/// The summary in the states it has its own sentence for, and `describe` before anything has
/// answered: the state the app has always started in.
#[test]
fn the_summary_says_what_it_always_said() {
    let (app, _) = start_with(&with_inbox(), yantrik_app::mind::fake::FakeMind::new());
    assert_eq!(summary(&app), "Email — the mail service could not be reached: the mail service has not been asked yet");
    assert_eq!(state(&app)["has_account"], Value::Null, "nothing has answered yet");
    super::quiet(&app);
    act(&app, "select_folder", json!({ "folder": "Receipts" })).unwrap();
    assert_eq!(
        summary(&app),
        "Email — Receipts, counts unavailable (the mail server did not report it: STATUS timed out)"
    );
    assert_eq!(state(&app)["counts"], Value::Null);
    assert_eq!(state(&app)["counts_reason"], "the mail server did not report it: STATUS timed out");

    act(&app, "select_folder", json!({ "folder": "inbox" })).unwrap();
    act(&app, "search", json!({ "query": "lunch" })).unwrap();
    assert_eq!(summary(&app), "Email — INBOX, 3 results for \u{201c}lunch\u{201d}");

    act(&app, "open_message", json!({ "which": 1 })).unwrap();
    assert_eq!(summary(&app), "Email — INBOX, reading \u{201c}Lunch on Friday\u{201d} from Priya R.");

    act(&app, "compose", json!({ "to": "", "subject": "Hi", "body": "Hello" })).unwrap();
    assert_eq!(summary(&app), "Email — composing to (nobody yet)");
    act(&app, "compose", json!({ "to": "ananya@example.com", "subject": "Hi", "body": "Hello" })).unwrap();
    assert_eq!(summary(&app), "Email — composing to ananya@example.com");
}
