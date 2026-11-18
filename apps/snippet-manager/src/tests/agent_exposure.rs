//! What an agent can learn of a snippet's code, and what it can make Snippets read: the security
//! fixes of `fix/snippets-agent-exposure`. A snippet may be a password, a key or a token, so a
//! sentinel in its code is followed through what an agent reads.
//!
//! - `describe` carries no line of any snippet's code (the list's `preview` was its first line);
//! - an agent's `search` matches titles, tags and language, never code, so its counts cannot be
//!   used to read a secret back a character at a time;
//! - an agent's `import` names only a file in the person's home, outside its protected places
//!   (`home_paths`, #443, as Images' `open`);
//! - `new`, `save`, `open` and `copy` answer with neither the code nor its length, and `describe`
//!   and the rail carry no line count;
//! - an agent's `copy` runs only as a call the person allowed: the clipboard is where the same
//!   agent could read the whole code back.

use serde_json::{json, Value};
use yantrik_testing::Call;

use super::{open, Fixture};
use crate::Command;

const SENTINEL: &str = "SENTINEL-agent-7f3a";

/// An agent's call, through the real dispatch, answered once its work has landed.
fn as_agent(f: &Fixture, name: &str, args: Value) -> Result<Value, String> {
    agent_call(f, Call::new(name, args).as_agent("mind"))
}

/// The same, as a call the person allowed on a card.
fn as_agent_allowed(f: &Fixture, name: &str, args: Value) -> Result<Value, String> {
    agent_call(f, Call::new(name, args).as_agent("mind").granted())
}

fn agent_call(f: &Fixture, call: Call) -> Result<Value, String> {
    let pending = f.app.act_as(call)?;
    f.quiet();
    pending.settle()
}

fn a_secret(f: &Fixture) -> i32 {
    f.keep("Deploy key", "Shell", &format!("export TOKEN={SENTINEL}\n# rotated {SENTINEL}"), "deploy, secrets")
}

#[test]
fn describe_carries_no_line_of_any_snippets_code() {
    let f = open();
    let id = a_secret(&f);
    f.act("open", json!({ "id": id.to_string() })).unwrap();
    let d = f.describe();
    assert!(!d.to_string().contains(SENTINEL), "{d}");
    let row = d["state"]["snippets"][0].clone();
    assert_eq!(row["title"], json!("Deploy key"), "what it is called stays");
    assert_eq!(row["tags"], json!(["deploy", "secrets"]));
    assert!(row.get("preview").is_none(), "{row}");
    assert!(row.get("lines").is_none() && d["state"].get("open_lines").is_none(), "no line count: {d}");
    // The window, the person's own, still shows the first line in the list.
    assert!(f.app.ui(|ui| slint::Model::row_data(&ui.get_snippets(), 0).unwrap().preview.to_string()).contains(SENTINEL));
}

#[test]
fn an_agents_search_never_matches_code() {
    let f = open();
    a_secret(&f);
    f.keep("Journal", "Shell", "journalctl -f", "logs");
    for probe in [SENTINEL, "TOKEN=S", "rotated", "export"] {
        let answer = as_agent(&f, "search", json!({ "query": probe })).unwrap();
        assert_eq!(answer["matching"], json!(0), "{probe}: {answer}");
        assert_eq!(f.state()["matching"], json!(0), "{probe}: and the list it leaves says the same");
    }
    // What a snippet is called still finds it.
    for probe in ["deploy key", "secrets", "shell"] {
        let answer = as_agent(&f, "search", json!({ "query": probe })).unwrap();
        assert!(answer["matching"].as_u64().unwrap() >= 1, "{probe}: {answer}");
    }
    // The person's own search matches the code, from `yos` and in the window.
    let answer = f.act("search", json!({ "query": SENTINEL })).unwrap();
    assert_eq!(answer["matching"], json!(1));
    f.app.run(Command::Search(format!("TOKEN={SENTINEL}")));
    assert_eq!(f.app.ui(|ui| slint::Model::row_count(&ui.get_snippets())), 1);
    assert!(!f.describe().to_string().contains("\"preview\""), "no preview, whoever searched");
}

#[test]
fn an_agents_import_reads_only_a_file_in_the_home() {
    let dir = std::env::temp_dir().join(format!("snippets-agent-import-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let outside = dir.join("credentials.env");
    std::fs::write(&outside, format!("API_KEY={SENTINEL}\n")).unwrap();
    assert!(!outside.starts_with(yantrik_foundation::place::home()), "the test's file is outside the home");
    let f = open();

    let e = as_agent(&f, "import", json!({ "path": outside.display().to_string() })).unwrap_err();
    assert!(e.starts_with("an agent may import only a file in the person's home"), "{e}");
    let protected = as_agent(&f, "import", json!({ "path": "~/.ssh/id_snippets_probe" })).unwrap_err();
    assert!(protected.starts_with("an agent may import only a file in the person's home"), "{protected}");
    assert_eq!(f.state()["total"], json!(0), "nothing was read in");
    assert_eq!(f.notice(), "", "a refusal to the agent, not a failure on the person's screen");
    assert!(!f.describe().to_string().contains(SENTINEL));

    // The person names any file they can read, as before.
    let answer = f.act("import", json!({ "path": outside.display().to_string() })).unwrap();
    assert_eq!(answer["imported"], json!(1));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A PIN is four bytes: a length is a hint at a secret, whichever action reveals it. `new`, `save`,
/// `open` and `copy` each answer which snippet and what came of it, as the person and as an agent,
/// and no answer carries the code or a number that is its length.
#[test]
fn new_save_open_and_copy_answer_with_neither_the_code_nor_its_length() {
    let f = open();
    // Lengths no other number in an answer can be: not an id, a count, a total or a flag.
    let code = format!("{SENTINEL}-{}", "x".repeat(41));
    let changed = format!("{SENTINEL}-{}", "y".repeat(52));
    let made = f.act("new", json!({ "title": "Vault PIN", "language": "Other", "code": code })).unwrap();
    let id = made["id"].as_i64().unwrap().to_string();
    let mut answers = vec![("new", made)];
    // As the person, then as an agent.
    for agent in [false, true] {
        let call = |name: &str, args: Value| if agent { as_agent(&f, name, args) } else { f.act(name, args) };
        answers.push(("save", call("save", json!({ "id": id, "code": changed })).unwrap()));
        answers.push(("open", call("open", json!({ "id": id })).unwrap()));
        let copied = if agent { as_agent_allowed(&f, "copy", json!({ "id": id })) } else { f.act("copy", json!({ "id": id })) };
        answers.push(("copy", copied.unwrap()));
    }
    let lengths = [json!(code.len()), json!(changed.len())];
    for (name, answer) in &answers {
        assert!(!answer.to_string().contains(SENTINEL), "{name}: {answer}");
        assert!(answer.get("bytes").is_none(), "{name}: {answer}");
        let values = answer.as_object().unwrap().values();
        assert!(values.into_iter().all(|v| !lengths.contains(v)), "{name}, a size hint: {answer}");
    }
    assert_eq!(f.clipboard.texts(), [changed.clone(), changed], "and it was copied, both times");
}

/// The clipboard is readable by everything on the session, the agent included, so an agent's
/// `copy` would hand it the whole code with no card. It is refused unless the person allowed that
/// call; with the Allow it copies. The person's own copy is unchanged.
#[test]
fn an_agents_copy_needs_the_persons_allow() {
    let f = open();
    let id = a_secret(&f).to_string();
    let e = as_agent(&f, "copy", json!({ "id": id })).unwrap_err();
    assert_eq!(e, crate::intents::COPY_NEEDS_ALLOW);
    assert!(!e.contains(SENTINEL) && !e.contains("Deploy key"), "the refusal carries no code: {e}");
    assert!(f.clipboard.texts().is_empty(), "nothing reached the clipboard");
    assert_eq!(f.on_disk().snippets()[0].use_count, 0, "and nothing was counted");
    assert_eq!(f.notice(), "", "a refusal to the agent, not a failure on the person's screen");

    let answer = as_agent_allowed(&f, "copy", json!({ "id": id })).unwrap();
    assert_eq!(answer["copied"], json!("Deploy key"));
    assert_eq!(f.clipboard.texts().len(), 1, "allowed, it copies");
    // The person's own `yos act` and button are as they were.
    f.act("copy", json!({ "id": id })).unwrap();
    let n: i32 = id.parse().unwrap();
    f.app.ui(move |ui| ui.invoke_snip_copy(n));
    f.quiet();
    assert_eq!(f.clipboard.texts().len(), 3);
}
