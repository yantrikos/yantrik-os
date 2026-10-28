//! The parity pin: the action table and `describe` exactly as Terminal published them before the
//! framework. The table below was extracted mechanically from the old `surface()` in
//! `apps/terminal/src/main.rs` (at fe2b8b41): the old window test was run with a dump of each
//! published `Action` (description, grade, `deferred`, parameters) and of the keys of `view`, and
//! the strings are what rustc evaluated, not copied by hand from the new intents, so a word
//! changed in `intents.rs` fails here.

use serde_json::{json, Value};
use yantrik_app::intents::Intents;

use super::{open, start_with, PROMPT};
use crate::Terminal;

struct Golden {
    name: &'static str,
    risk: &'static str,
    description: &'static str,
    deferred: bool,
    /// Name, JSON type, required, description.
    params: Vec<(&'static str, &'static str, bool, &'static str)>,
}

fn before() -> Vec<Golden> {
    vec![
        Golden {
            name: "run",
            risk: "sensitive",
            description: r####"Type a command line into the active shell and press Return. It does not wait for the command to finish: the answer carries what the shell printed in the moment after, `running` names anything still going, and `describe` shows the rest as it arrives. There is no exit status — ask for one with `; echo exit=$?`. What it runs can do anything you can."####,
            deferred: true,
            params: vec![("command", "string", true, r####"One command line, exactly as it would be typed; a newline is added. It is interpreted by the shell, so pipes, redirection and `cd` all work, and `cd` moves this tab for every command after it."####)],
        },
        Golden {
            name: "send_input",
            risk: "sensitive",
            description: r####"Send raw bytes to the active shell without pressing Return — for answering a prompt a running command is waiting on, or sending a control character like \u0003 (Ctrl-C) to interrupt it. What it runs can do anything you can."####,
            deferred: true,
            params: vec![("text", "string", true, r####"The exact characters to send, with no newline added: end it with \n to submit a line. \u0003 interrupts, \u0004 is end-of-input. Up to 64 KiB."####)],
        },
        Golden {
            name: "new_tab",
            risk: "sensitive",
            description: r####"Start another interactive shell in a new tab, in the same directory as the tab that is active now, and make the new one active. Up to 32 tabs."####,
            deferred: false,
            params: vec![],
        },
        Golden {
            name: "open_directory",
            risk: "sensitive",
            description: r####"Start an interactive shell in a new tab whose working directory is this path, make it active, and bring the Terminal window to the front."####,
            deferred: false,
            params: vec![("directory", "string", true, r####"An absolute path to an existing folder; symlinks are resolved. It is refused if it is relative, missing, or not a folder."####)],
        },
    ]
}

/// The keys of `describe` with a shell open, as the old `view` set them.
const KEYS: [&str; 13] = [
    "active_tab",
    "alive",
    "directory",
    "error",
    "execution",
    "notice",
    "recent_output",
    "running",
    "scrollback",
    "shell_exit_code",
    "shell_pid",
    "tab_directories",
    "tabs",
];

#[test]
fn the_four_actions_are_published_as_they_were() {
    let mut table = Intents::<Terminal>::new();
    <Terminal as yantrik_app::App>::intents(&mut table);
    let specs = table.specs();
    assert_eq!(specs.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["run", "send_input", "new_tab", "open_directory"]);
    for (spec, golden) in specs.iter().zip(before()) {
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

#[test]
fn only_run_and_send_input_are_open_ended_and_deferred() {
    let mut table = Intents::<Terminal>::new();
    <Terminal as yantrik_app::App>::intents(&mut table);
    for spec in table.specs() {
        let open_ended = spec.description.contains(yantrik_app::control::OPEN_ENDED);
        let types_into_a_shell = matches!(spec.name.as_str(), "run" | "send_input");
        assert_eq!(open_ended, types_into_a_shell, "{}", spec.name);
        assert_eq!(spec.schema()["settles"] == json!("later"), types_into_a_shell, "{}", spec.name);
        assert_eq!(spec.permission, "sensitive", "{} is not raised or lowered", spec.name);
    }
}

/// What a caller sees in `describe`: the four, and the framework's `launch`, which every app on
/// the framework publishes.
#[test]
fn describe_lists_the_four_and_the_frameworks_launch() {
    let f = open();
    let actions = f.app.describe()["actions"].as_array().cloned().unwrap_or_default();
    let mut names: Vec<String> = actions.iter().filter_map(|a| a["name"].as_str().map(String::from)).collect();
    let mut expected: Vec<String> = before().iter().map(|g| g.name.to_string()).collect();
    expected.push("launch".to_string());
    names.sort();
    expected.sort();
    assert_eq!(names, expected);
    for golden in before() {
        let action = actions.iter().find(|a| a["name"] == json!(golden.name)).expect("published");
        assert_eq!(action["permission"], json!(golden.risk), "{}", golden.name);
    }
}

/// docs/app-control.md's rows for Terminal say what the app is and what it publishes.
#[test]
fn the_guides_rows_are_still_true() {
    let doc = include_str!("../../../../docs/app-control.md");
    let surface = doc.lines().find(|l| l.starts_with("| `terminal` |")).expect("the surfaces table names it");
    let cells: Vec<&str> = surface.split('|').map(str::trim).collect();
    assert_eq!(cells[2], "`app-terminal.sock`", "the socket is the id's: {surface}");
    assert_eq!(cells[4], "a shell: run commands", "{surface}");
    let row = doc.lines().find(|l| l.starts_with("| terminal | `run`")).expect("the grades table names its actions");
    let cells: Vec<&str> = row.split('|').map(str::trim).collect();
    let listed: Vec<&str> = cells[2].split(',').map(|n| n.trim().trim_matches('`')).collect();
    let all: Vec<&str> = before().iter().map(|g| g.name).collect();
    assert_eq!(listed, all, "every action once, in order");
    assert_eq!(cells[3], "sensitive", "at the grade the code holds");
}

fn keys(state: &Value) -> Vec<String> {
    let mut keys: Vec<String> = state.as_object().expect("an object").keys().cloned().collect();
    keys.sort();
    keys
}

#[test]
fn a_shell_at_a_prompt_is_described_as_it_was() {
    let f = open();
    f.until("the probe", |f| f.summary().contains("idle at a prompt"));
    let d = f.app.describe();
    let s = &d["state"];
    assert_eq!(keys(s), KEYS.iter().map(|k| k.to_string()).collect::<Vec<_>>());
    let summary = d["summary"].as_str().unwrap();
    assert!(summary.starts_with("Terminal \u{2014} /"), "{summary}");
    assert!(summary.ends_with(", shell idle at a prompt"), "{summary}");
    assert_eq!(s["execution"], json!("interactive_pty"));
    assert_eq!(s["tabs"], json!(1));
    assert_eq!(s["active_tab"], json!(0));
    assert_eq!(s["alive"], json!(true));
    assert_eq!(s["running"], Value::Null);
    assert_eq!(s["shell_exit_code"], Value::Null);
    assert_eq!(s["scrollback"], json!(0));
    assert_eq!(s["notice"], json!(""));
    assert_eq!(s["directory"], s["tab_directories"][0]);
    assert_eq!(s["shell_pid"], json!(f.active_pid()));
    assert!(s["recent_output"].as_str().unwrap().contains(PROMPT));
}

#[test]
fn several_tabs_and_a_running_job_are_described_as_they_were() {
    let f = open();
    f.open_tabs(1);
    f.start_job("sleep 300");
    f.until("the probe", |f| f.summary().contains("running: sleep"));
    let summary = f.summary();
    assert!(summary.contains(", running: sleep, tab 2 of 2"), "{summary}");
    assert_eq!(f.state()["running"], json!("sleep"));
    assert_eq!(f.state()["tabs"], json!(2));
    assert_eq!(f.state()["active_tab"], json!(1));
    assert_eq!(f.state()["tab_directories"].as_array().unwrap().len(), 2);
}

#[test]
fn an_exited_shell_says_how() {
    let f = open();
    f.type_until("exit 7\r", "exit 7");
    f.until("the exit", |f| f.summary().contains("shell exited (status 7)"));
    assert!(f.summary().contains("`new_tab` starts another"), "{}", f.summary());
    assert_eq!(f.state()["shell_exit_code"], json!(7));
    assert_eq!(f.state()["alive"], json!(false));
    assert_eq!(f.app.ui(|ui| ui.get_status().to_string()), "Shell exited \u{b7} status 7");
}

#[test]
fn with_no_shell_it_says_so_with_four_keys() {
    let f = open();
    f.app.run(crate::Command::CloseTab(0));
    f.until("the tab gone", |f| f.tabs() == 0);
    let d = f.app.describe();
    assert_eq!(d["summary"], json!("Terminal \u{2014} no shell is open; `new_tab` starts one"));
    assert_eq!(keys(&d["state"]), ["alive", "execution", "notice", "tabs"]);
    assert_eq!(d["state"]["tabs"], json!(0));
    assert_eq!(f.app.ui(|ui| ui.get_status().to_string()), "No sessions");
}

#[test]
fn a_launch_with_a_link_or_a_file_types_nothing() {
    use std::ffi::OsString;
    let f = start_with(vec![OsString::from("yantrik://terminal/run?command=echo%20LINK_TYPED"), OsString::from("/etc/hostname")]);
    f.app.advance(std::time::Duration::from_millis(200));
    f.quiet();
    assert!(!f.screen().contains("LINK_TYPED"), "a link's intent is never typed into a shell");
    assert!(f.screen().trim_end().ends_with(PROMPT.trim_end()), "{}", f.screen());
}
