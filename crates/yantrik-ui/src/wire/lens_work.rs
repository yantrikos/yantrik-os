//! What the Lens's conversation shows of the runs it started: the work card under a reply, and the
//! strip above the composer (Chat v2).
//!
//! Everything here is read from the Agents store, which every call, answer and refusal already
//! passes through; nothing asks the mind and nothing is composed. The state is one of the eight
//! words every surface uses, and the one activity line is a call the store recorded, or "Last
//! update 10:42" when nothing is observable. There is no progress figure because nothing measures
//! one. "Paused" is in the vocabulary but is not drawn yet: no run can be paused today, so none is
//! ever called that.

use crate::agents::model::{Agent, CallState, State, Turn};
use crate::agents::{progress, RowKey};

/// How many of a conversation's runs get a card. A card is looked up by the id a reply carries, so
/// this only has to reach as far back as the transcript does.
const CARDS: usize = 24;

/// One run, as the card and the strip say it. Plain data, so it can be tested without a window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Work {
    pub run: String,
    pub title: String,
    pub mind: String,
    /// queued | working | needs-you | paused | finished | stopped | failed | lost
    pub state: &'static str,
    pub label: &'static str,
    pub activity: String,
    pub live: bool,
    pub can_view_desk: bool,
    pub can_review: bool,
}

/// The state words, keyed as the Slint side spells them.
fn words(state: &str) -> &'static str {
    match state {
        "queued" => "Queued",
        "working" => "Working",
        "needs-you" => "Needs you",
        "paused" => "Paused",
        "finished" => "Finished",
        "stopped" => "Stopped",
        "failed" => "Couldn\u{2019}t finish",
        _ => "Connection lost",
    }
}

/// Where a run stands, from the agent and the one turn.
fn state_of(a: &Agent, t: &Turn) -> &'static str {
    if t.open() {
        match a.state {
            State::HarnessGone => "lost",
            State::WaitingForYou => "needs-you",
            State::Idle => "queued",
            _ => "working",
        }
    } else if t.lost {
        "stopped"
    } else if t.ok == Some(false) {
        "failed"
    } else {
        "finished"
    }
}

/// The one line under the state: a recorded call, or when anything was last heard.
fn activity_of(a: &Agent, t: &Turn, state: &str, now: u64) -> String {
    let calls = t.cards().count();
    let last_update = || format!("Last update {}", super::agents::clock(if t.open() { a.touched } else { t.ended.unwrap_or(t.started) }));
    match state {
        "needs-you" => "Waiting for your answer".to_string(),
        "queued" => "Not started yet".to_string(),
        "working" => match progress::of(a, now) {
            Some(p) => match (p.running, p.recent.last()) {
                (Some(running), _) => format!("Running {running}"),
                // The mind's own latest status line ("Grounding from memory…", "Thinking… (60 s)")
                // outranks the last finished call, which says what it did, not what it does now.
                // A tool_start clears it in the store, so a call in progress always wins.
                (None, _) if !a.status.trim().is_empty() => progress::brief(a.status.trim(), 120),
                (None, Some(last)) => last.clone(),
                (None, None) if a.state == State::Thinking => "Thinking".to_string(),
                (None, None) => last_update(),
            },
            None => last_update(),
        },
        "finished" if calls > 0 => format!("{calls} call{} recorded", if calls == 1 { "" } else { "s" }),
        "failed" => match t.cards().filter(|c| c.state == CallState::Failed).last() {
            Some(c) => format!("{} failed", c.name),
            None => last_update(),
        },
        _ => last_update(),
    }
}

/// One run's card.
pub fn work_of(a: &Agent, t: &Turn, now: u64) -> Work {
    let state = state_of(a, t);
    let live = matches!(state, "queued" | "working" | "needs-you");
    Work {
        run: RowKey::run(&a.meta.id, t.n).id(),
        title: progress::brief(&t.prompt, 120),
        mind: a.meta.mind.clone(),
        state,
        label: words(state),
        activity: activity_of(a, t, state, now),
        live,
        // A desk exists to watch only while the run is live.
        can_view_desk: live,
        // The calls are recorded in the run's pane; there is nothing to review on an empty run.
        can_review: state == "finished" && t.cards().next().is_some(),
    }
}

/// The runs of `a` that the Agents screen lists, oldest first, the last few.
pub fn works_of(a: &Agent, now: u64) -> Vec<Work> {
    let mut all: Vec<Work> = a.turns.iter().filter(|t| a.is_run(t)).map(|t| work_of(a, t, now)).collect();
    let skip = all.len().saturating_sub(CARDS);
    all.drain(..skip);
    all
}

/// The run the strip describes: one that needs the person first, else the newest live one.
pub fn strip_of(works: &[Work]) -> Option<&Work> {
    works
        .iter()
        .rev()
        .find(|w| w.state == "needs-you")
        .or_else(|| works.iter().rev().find(|w| w.live))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::model::{AgentMeta, Card, Item, Provenance, TurnOrigin};
    use crate::agents::AgentId;

    fn agent(state: State) -> Agent {
        let id = AgentId::new("pi", AgentId::MAIN);
        Agent {
            meta: AgentMeta::new(id, "pi"),
            state,
            since: 0,
            status: String::new(),
            turns: Vec::new(),
            usage: Default::default(),
            refused: 0,
            refusals: Vec::new(),
            approvals_asked: 0,
            approvals_answered: 0,
            pending_approvals: Vec::new(),
            job_waits: false,
            seq: 0,
            touched: 100,
            next_turn: 2,
        }
    }

    fn turn(ended: Option<u64>, ok: Option<bool>, lost: bool, cards: &[(&str, CallState)]) -> Turn {
        let items = cards
            .iter()
            .map(|(name, state)| {
                let mut c = Card::new("c", name, "", serde_json::Value::Null, Provenance::Reported, 10);
                c.state = *state;
                Item::Card(c)
            })
            .collect();
        Turn {
            n: 1,
            prompt: "tidy the   photos folder".into(),
            started: 10,
            ended,
            ok,
            lost,
            origin: TurnOrigin::Chat,
            items,
            events: true,
            trail_seq: 0,
        }
    }

    #[test]
    fn a_run_says_one_of_the_eight_words_and_only_what_the_store_recorded() {
        let now = 200;
        let mut a = agent(State::RunningTool);
        let t = turn(None, None, false, &[("files.move", CallState::Running)]);
        a.turns.push(t);
        let w = work_of(&a, &a.turns[0], now);
        assert_eq!((w.state, w.label), ("working", "Working"));
        assert_eq!(w.title, "tidy the photos folder", "the title is what was asked, on one line");
        assert!(w.activity.starts_with("Running files.move"), "{}", w.activity);
        assert_eq!(w.run, "pi:main#1");
        assert!(w.can_view_desk && !w.can_review);

        a.state = State::WaitingForYou;
        let w = work_of(&a, &a.turns[0], now);
        assert_eq!((w.state, w.label, w.activity.as_str()), ("needs-you", "Needs you", "Waiting for your answer"));

        a.state = State::HarnessGone;
        assert_eq!(work_of(&a, &a.turns[0], now).label, "Connection lost");
    }

    #[test]
    fn a_status_line_is_the_activity_line_until_a_call_starts() {
        let mut a = agent(State::Thinking);
        a.turns.push(turn(None, None, false, &[("files.list", CallState::Ok)]));
        a.status = "Grounding from memory…".into();
        assert_eq!(work_of(&a, &a.turns[0], 200).activity, "Grounding from memory…");
        a.status = "Thinking… (60 s)".into();
        assert_eq!(work_of(&a, &a.turns[0], 200).activity, "Thinking… (60 s)", "replaced, not appended");

        // The store clears the status on tool_start and the running call is what is said.
        a.status.clear();
        a.state = State::RunningTool;
        a.turns[0].items.push(Item::Card(Card::new("c2", "files.move", "", serde_json::Value::Null, Provenance::Reported, 20)));
        assert!(work_of(&a, &a.turns[0], 200).activity.starts_with("Running files.move"));
        a.status = "Thinking… (90 s)".into();
        assert!(work_of(&a, &a.turns[0], 200).activity.starts_with("Running files.move"), "a running call outranks a status");
    }

    #[test]
    fn a_run_that_ended_is_finished_stopped_or_could_not_finish_and_never_guesses_which() {
        let a = agent(State::Done);
        let done = turn(Some(50), Some(true), false, &[("files.move", CallState::Ok), ("files.move", CallState::Ok)]);
        let w = work_of(&a, &done, 200);
        assert_eq!((w.state, w.activity.as_str()), ("finished", "2 calls recorded"));
        assert!(w.can_review && !w.can_view_desk, "changes to review, and no desk left to watch");

        let failed = turn(Some(50), Some(false), false, &[("files.move", CallState::Failed)]);
        let w = work_of(&a, &failed, 200);
        assert_eq!((w.label, w.activity.as_str()), ("Couldn\u{2019}t finish", "files.move failed"));

        let cut = turn(Some(50), None, true, &[]);
        let w = work_of(&a, &cut, 200);
        assert_eq!(w.label, "Stopped");
        assert!(w.activity.starts_with("Last update "), "unobservable work says when it was last heard: {}", w.activity);
        assert!(!w.can_review, "a run with nothing recorded has nothing to review");
    }

    #[test]
    fn the_strip_follows_the_run_that_needs_the_person_first() {
        let mk = |run: &str, state: &'static str, live: bool| Work {
            run: run.into(),
            title: String::new(),
            mind: "pi".into(),
            state,
            label: words(state),
            activity: String::new(),
            live,
            can_view_desk: live,
            can_review: false,
        };
        let works = vec![mk("a#1", "needs-you", true), mk("a#2", "working", true), mk("a#0", "finished", false)];
        assert_eq!(strip_of(&works).map(|w| w.run.as_str()), Some("a#1"));
        assert_eq!(strip_of(&works[1..]).map(|w| w.run.as_str()), Some("a#2"));
        assert!(strip_of(&works[2..]).is_none(), "no live run, no strip");
    }
}
