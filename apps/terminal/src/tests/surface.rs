//! What the Mind is told: every action's answer is read from its own shell, the refusals are said
//! twice, and the grades and the gate's sentence are what they were.

use serde_json::{json, Value};
use yantrik_testing::Call;

use super::{open, AGENT};
use crate::command::Command;

fn keys(v: &Value) -> Vec<&str> {
    let mut keys: Vec<&str> = v.as_object().expect("an object").keys().map(String::as_str).collect();
    keys.sort();
    keys
}

const SHELL: [&str; 10] =
    ["alive", "directory", "error", "notice", "running", "screen_tail", "shell_exit_code", "shell_pid", "tab", "tabs"];

fn with(extra: &[&'static str]) -> Vec<&'static str> {
    let mut all: Vec<&'static str> = SHELL.to_vec();
    all.extend_from_slice(extra);
    all.sort();
    all
}

#[test]
fn run_answers_what_the_shell_printed_and_never_claims_the_command_finished() {
    let f = open();
    let pid = f.active_pid();
    let answer = f.act("run", json!({ "command": "printf 'SURFACE:%s\\n' ok" })).unwrap();
    assert_eq!(keys(&answer), with(&["finished", "output", "sent", "shell_answered"]));
    assert_eq!(answer["sent"], json!("printf 'SURFACE:%s\\n' ok"));
    assert_eq!(answer["shell_pid"], json!(pid), "the answer names the shell it typed into");
    assert_eq!(answer["alive"], json!(true));
    assert_eq!(answer["finished"], json!(false));
    assert_eq!(answer["shell_answered"], json!(true));
    assert_eq!(answer["tab"], json!(0));
    assert_eq!(answer["tabs"], json!(1));
    assert!(answer["output"].as_str().unwrap().contains("SURFACE"), "what appeared after the command was sent: {answer}");
    assert!(!answer["screen_tail"].as_str().unwrap().is_empty());
    // And the command really ran, checked against the screen and not the answer.
    f.until("the output", |f| f.screen().contains("SURFACE:ok"));
}

#[test]
fn run_refuses_a_blank_command_a_missing_shell_and_a_dead_one_in_the_sentences_it_always_used() {
    let f = open();
    let blank = f.act("run", json!({ "command": "   " })).unwrap_err();
    assert_eq!(blank, "`run` needs `command`: one command line for the active shell.");
    assert_eq!(f.notice(), blank, "said twice: to the caller, and on the notice line");

    f.type_until("exit 2\r", "exit 2");
    f.until("the exit", |f| !f.app.model(|m| m.session().is_some_and(|s| s.alive())));
    let dead = f.act("run", json!({ "command": "echo nope" })).unwrap_err();
    assert_eq!(dead, "The shell in this tab has exited; `new_tab` starts another.");
    assert_eq!(f.notice(), dead);

    f.app.run(Command::CloseTab(0));
    f.until("no tab", |f| f.tabs() == 0);
    let none = f.act("run", json!({ "command": "echo nope" })).unwrap_err();
    assert_eq!(none, "No shell is open in this Terminal; `new_tab` starts one.");
    let none = f.act("send_input", json!({ "text": "x" })).unwrap_err();
    assert_eq!(none, "No shell is open in this Terminal; `new_tab` starts one.");
}

#[test]
fn send_input_interrupts_a_job_and_says_what_was_running() {
    let f = open();
    f.start_job("sleep 300");
    f.until("the probe", |f| f.summary().contains("running: sleep"));
    let answer = f.act("send_input", json!({ "text": "\u{3}" })).unwrap();
    assert_eq!(keys(&answer), with(&["sent_bytes", "shell_answered", "was_running"]));
    assert_eq!(answer["sent_bytes"], json!(1));
    assert_eq!(answer["was_running"], json!("sleep"));
    f.until("the job to end", |f| !f.app.model(|m| m.session().is_some_and(|s| s.has_children())));
}

#[test]
fn send_input_refuses_nothing_to_send_and_more_than_it_can_queue() {
    let f = open();
    let empty = f.act("send_input", json!({ "text": "" })).unwrap_err();
    assert_eq!(empty, "`send_input` needs `text`: the exact characters to send to the shell.");
    let big = f.act("send_input", json!({ "text": "x".repeat(64 * 1024 + 1) })).unwrap_err();
    assert_eq!(big, "Paste is limited to 64 KiB.");
    assert_eq!(f.notice(), big);
}

#[test]
fn the_answer_is_the_shell_it_was_typed_into_not_the_tab_in_front_when_it_lands() {
    let f = open();
    f.open_tabs(1);
    f.app.run(Command::SelectTab(0));
    let (first, second) = (f.pids()[0], f.pids()[1]);
    let pending = f.app.act("run", json!({ "command": "echo FIRST_TAB_OUTPUT" })).unwrap();
    // The person moves to the other tab before the shell has answered.
    f.app.run(Command::SelectTab(1));
    let answer = f.settled(pending).unwrap();
    assert_eq!(answer["shell_pid"], json!(first), "typed into the first tab, so told about the first tab");
    assert!(answer["output"].as_str().unwrap().contains("FIRST_TAB_OUTPUT"), "{answer}");
    assert_eq!(answer["tab"], json!(0));
    assert_eq!(f.active_pid(), second);
    assert!(!f.screen().contains("FIRST_TAB_OUTPUT"), "and nothing was typed into the tab in front");
}

#[test]
fn the_tab_closed_while_the_shell_answers_is_still_the_one_described() {
    let f = open();
    f.open_tabs(1);
    let second = f.pids()[1];
    let pending = f.app.act("run", json!({ "command": "echo GONE_OUTPUT" })).unwrap();
    f.app.run(Command::CloseTab(1));
    let answer = f.settled(pending).unwrap();
    assert_eq!(answer["shell_pid"], json!(second), "the shell it was typed into, whether or not its tab is still open");
}

#[test]
fn two_calls_one_after_the_other_are_each_told_their_own_output() {
    let f = open();
    let one = f.act("run", json!({ "command": "echo ONE_MARK" })).unwrap();
    let two = f.act("run", json!({ "command": "echo TWO_MARK" })).unwrap();
    assert!(one["output"].as_str().unwrap().contains("ONE_MARK"));
    assert!(!one["output"].as_str().unwrap().contains("TWO_MARK"));
    assert!(two["output"].as_str().unwrap().contains("TWO_MARK"));
}

#[test]
fn new_tab_and_open_directory_answer_with_the_shells_they_started() {
    let f = open();
    let tab = f.act("new_tab", json!({})).unwrap();
    assert_eq!(keys(&tab), with(&[]));
    assert_eq!(tab["tabs"], json!(2));
    assert_eq!(tab["alive"], json!(true));
    let dir = super::folder(&f.paths, "answer");
    let opened = f.act("open_directory", json!({ "directory": dir.display().to_string() })).unwrap();
    assert_eq!(keys(&opened), with(&["asked_for"]));
    assert_eq!(opened["tabs"], json!(3));
}

/// A mind's token reaches the same four actions and is answered the same way; the open-ended two
/// ask first in ask mode, and a session rule or a person's grant lets them through.
#[test]
fn an_agent_is_answered_as_the_person_is_and_the_open_ended_actions_still_ask() {
    let f = open();
    let answer = f.agent("run", json!({ "command": "echo AGENT_MARK" })).unwrap();
    assert!(answer["output"].as_str().unwrap().contains("AGENT_MARK"));
    assert_eq!(f.agent("new_tab", json!({})).unwrap()["tabs"], json!(2));

    let asking = |name: &str, args: Value| Call::new(name, args).as_agent(AGENT).in_mode("ask");
    assert!(f.app.act_as(asking("run", json!({ "command": "echo NEVER_TYPED" }))).is_err(), "run asks in ask mode");
    assert!(f.app.act_as(asking("send_input", json!({ "text": "echo NEVER_TYPED\n" }))).is_err(), "so does send_input");
    f.quiet();
    assert!(!f.screen().contains("NEVER_TYPED"), "nothing reached the shell");
    let granted = f.app.act_as(asking("run", json!({ "command": "echo GRANTED_MARK" })).granted()).unwrap();
    assert!(f.settled(granted).unwrap()["output"].as_str().unwrap().contains("GRANTED_MARK"));
}

#[test]
fn the_notice_is_the_sentence_the_caller_was_told_and_is_cleared_by_the_next_tab() {
    let f = open();
    let blank = f.act("run", json!({ "command": "   " })).unwrap_err();
    assert_eq!(f.state()["notice"], json!(blank));
    assert!(f.summary().ends_with(&format!(" \u{b7} {blank}")), "{}", f.summary());
    f.act("new_tab", json!({})).unwrap();
    assert_eq!(f.notice(), "", "a tab that opened clears what the last refusal said");
}
