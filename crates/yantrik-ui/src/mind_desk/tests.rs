//! The projection: which Mind, what of it, and nothing the chrome must not show.

use serde_json::json;
use yantrik_harness::AgentId;
use yantrik_ipc_contracts::mind_desk::{Account, DeskSnapshot, Evidence, MindState, Outcome};

use super::project::{focus, project, with_account};
use crate::agents::model::{Agent, AgentMeta, CallState, Card, Item, Provenance, State, Turn, Usage};

fn card(name: &str, state: CallState, provenance: Provenance, at: u64) -> Card {
    let args = json!({ "command": "cat ~/.ssh/id_ed25519" });
    let mut c = Card::new(&format!("c{at}"), name, "/home/pranab/Documents/Ridge/secret.md", args, provenance, at);
    c.state = state;
    c.summary = "permission denied for /home/pranab/.ssh".into();
    c.preview = "cat ~/.ssh/id_ed25519".into();
    if state != CallState::Running {
        c.ended = Some(at + 1);
    }
    c
}

fn agent(mind: &str, conversation: &str, state: State, touched: u64, cards: Vec<Card>, open: bool) -> Agent {
    let mut meta = AgentMeta::new(AgentId::new(&mind.to_lowercase(), conversation), mind);
    meta.title = "Build the secret project in ~/Documents/Ridge".into();
    Agent {
        meta,
        state,
        since: touched,
        status: String::new(),
        turns: vec![Turn {
            n: 1,
            prompt: "Please use the password hunter2 to log in".into(),
            started: 1_000,
            ended: (!open).then_some(1_300),
            ok: (!open).then_some(true),
            lost: false,
            origin: Default::default(),
            items: cards.into_iter().map(Item::Card).collect(),
            events: true,
            trail_seq: 0,
        }],
        usage: Usage::default(),
        refused: 0,
        refusals: Vec::new(),
        refusals_shown: None,
        erasures: Vec::new(),
        approvals_asked: 0,
        approvals_answered: 0,
        pending_approvals: Vec::new(),
        job_waits: false,
        seq: touched,
        touched,
        next_turn: 2,
    }
}

fn snapshot(agents: &[Agent]) -> DeskSnapshot {
    project(agents, 1_200, &["Blender".to_string(), "Files".to_string()])
}

#[test]
fn the_desk_follows_the_mind_that_needs_the_person_then_the_one_at_work_then_the_latest() {
    let waiting = agent("Pi", "c-1", State::WaitingForYou, 10, vec![], true);
    let working = agent("Hermes", "c-2", State::RunningTool, 50, vec![], true);
    let done = agent("DeepSeek", "c-3", State::Done, 90, vec![], false);
    let all = [done, working, waiting];
    assert_eq!(focus(&all).unwrap().meta.mind, "Pi", "the one that needs you, however old");

    let working = agent("Hermes", "c-2", State::RunningTool, 50, vec![], true);
    let done = agent("DeepSeek", "c-3", State::Done, 90, vec![], false);
    let pair = [done, working];
    assert_eq!(focus(&pair).unwrap().meta.mind, "Hermes", "the one at work over a newer one at rest");

    let older = agent("Pi", "c-1", State::Done, 10, vec![], false);
    let newer = agent("Hermes", "c-2", State::Failed, 20, vec![], false);
    let rest = [older, newer];
    assert_eq!(focus(&rest).unwrap().meta.mind, "Hermes", "among those at rest, the one heard from last");

    assert!(focus(&[]).is_none());
    assert_eq!(snapshot(&[]).mind, None, "no Mind known is said as none, not invented");
}

#[test]
fn the_tool_is_its_name_with_who_vouches_and_how_it_went() {
    let calls = vec![
        card("files.read", CallState::Ok, Provenance::Verified, 1_010),
        card("blender.add_object", CallState::Running, Provenance::Reported, 1_020),
    ];
    let mut a = agent("Hermes", "main", State::RunningTool, 1_100, calls, true);
    a.status = "Shaping the last ascent.".into();
    let mind = snapshot(&[a]).mind.unwrap();
    let tool = mind.tool.unwrap();
    assert_eq!((tool.name.as_str(), tool.evidence, tool.outcome), ("blender.add_object", Evidence::Reported, Outcome::Running));
    assert_eq!(mind.state, MindState::RunningTool);
    assert_eq!(mind.name, "Hermes");
    assert_eq!(mind.counts.calls, 2);
    assert_eq!(mind.elapsed_secs, Some(200), "as of the snapshot, from the turn's start");
    let said = mind.narration.unwrap();
    assert_eq!(said.text, "Shaping the last ascent.");
    assert_eq!(said.heard.len(), 5, "a local HH:MM: {}", said.heard);
}

#[test]
fn with_nothing_running_it_is_the_last_call_and_the_counts_are_the_runs() {
    let calls = vec![
        card("a", CallState::Ok, Provenance::Verified, 1_010),
        card("b", CallState::Failed, Provenance::Verified, 1_020),
        card("c", CallState::Interrupted, Provenance::Reported, 1_030),
        card("d", CallState::Failed, Provenance::Reported, 1_040),
    ];
    let a = agent("Hermes", "main", State::Failed, 1_100, calls, false);
    let mind = snapshot(&[a]).mind.unwrap();
    let tool = mind.tool.unwrap();
    assert_eq!((tool.name.as_str(), tool.outcome), ("d", Outcome::Failed));
    assert_eq!((mind.counts.calls, mind.counts.failed, mind.counts.interrupted), (4, 2, 1));
    assert_eq!(mind.elapsed_secs, Some(300), "a finished run lasted from its start to its end");
}

#[test]
fn nothing_the_chrome_must_not_show_leaves_the_shell() {
    let calls = vec![card("agent_run", CallState::Running, Provenance::Verified, 1_010)];
    let mut a = agent("Hermes", "main", State::RunningTool, 1_100, calls, true);
    a.status = "Reading /home/pranab/Documents/Ridge/secret.md".into();
    a.meta.model = "deepseek-v4.1-flash via ollama-cloud".into();
    let wire = serde_json::to_string(&snapshot(&[a])).unwrap();
    for leak in [
        "secret", "hunter2", "password", "/home", "~/", ".ssh", "id_ed25519", "cat ", "Ridge", "permission denied", "command",
    ] {
        assert!(!wire.contains(leak), "`{leak}` reached the desk: {wire}");
    }
    assert!(wire.contains("agent_run") && wire.contains("deepseek-v4.1-flash"), "{wire}");
}

#[test]
fn a_command_line_put_where_a_name_goes_is_not_shown_as_a_tool() {
    let calls = vec![card("cat ~/.ssh/id_ed25519", CallState::Running, Provenance::Reported, 1_010)];
    let a = agent("Hermes", "main", State::RunningTool, 1_100, calls, true);
    let mind = snapshot(&[a]).mind.unwrap();
    assert_eq!(mind.tool, None, "no name is better than that one");
    assert_eq!(mind.counts.calls, 1, "though the call is still counted");
}

#[test]
fn the_minds_own_line_is_shown_while_it_is_working_and_not_after() {
    let mut working = agent("Hermes", "main", State::Thinking, 1_100, vec![], true);
    working.status = "Waiting on the docs.".into();
    assert_eq!(snapshot(std::slice::from_ref(&working)).mind.unwrap().narration.unwrap().text, "Waiting on the docs.");

    for state in [State::Done, State::Failed, State::Idle, State::HarnessGone] {
        let mut a = agent("Hermes", "main", state, 1_100, vec![], false);
        a.status = "Old words.".into();
        assert_eq!(snapshot(&[a]).mind.unwrap().narration, None, "{state:?}");
    }
    // A line with a path in it is not shown, whoever said it.
    working.status = "Writing ~/Documents/ridge/notes.md".into();
    assert_eq!(snapshot(&[working]).mind.unwrap().narration, None);
}

#[test]
fn the_model_is_what_the_harness_said_or_nothing() {
    let mut a = agent("Hermes", "main", State::Idle, 1_100, vec![], false);
    assert_eq!(snapshot(std::slice::from_ref(&a)).mind.unwrap().model, None);
    a.usage.model = "claude-opus-5".into();
    assert_eq!(snapshot(std::slice::from_ref(&a)).mind.unwrap().model.as_deref(), Some("claude-opus-5"));
    a.meta.model = "/opt/models/private.gguf".into();
    assert_eq!(snapshot(&[a]).mind.unwrap().model.as_deref(), Some("claude-opus-5"), "a path is not a model name");
}

#[test]
fn the_account_comes_from_the_harness_and_the_apps_are_named_plainly() {
    let a = agent("Yantrik Mind", "main", State::Idle, 1_100, vec![], false);
    let apps = ["Blender".to_string(), "/home/me/x".to_string(), "Files".to_string()];
    let snap = project(&[a], 1_200, &apps);
    assert_eq!(snap.mind.as_ref().unwrap().account, Account::Unknown, "the store does not know it");
    let snap = with_account(snap, |harness| if harness == "yantrik mind" { Account::Own } else { Account::Person });
    assert_eq!(snap.mind.unwrap().account, Account::Own);
    assert_eq!(snap.apps, ["Blender", "Files"], "a name that is a path is left out");
}
