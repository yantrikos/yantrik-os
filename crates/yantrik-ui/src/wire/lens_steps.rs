//! The steps a mind is taking, live in its Lens bubble while it answers: "✓ Checked what I
//! remember", then "Looking at your calendar… · 24 s".
//!
//! On VM 541 a calendar question sat 72 s behind an empty bubble, a note 288 s, a search 147 s,
//! while every step the mind took was already in the Agents store. The Lens showed none of it
//! until the whole answer landed, and a mind at work looked like a mind that had hung.
//!
//! This follows the Lens's open turn in the store: the calls it made (cards), and the mind's own
//! status lines, which the store keeps only as the latest one, so the steps they named are
//! remembered here in the order they were first seen. Words come from `step_words` and nothing
//! else. When the turn ends the same steps become the reply's "Show actions" fold, which is how a
//! mind that writes no trail lines (the Mind) still says what it did.

use std::cell::RefCell;

use crate::agents::model::{Agent, ApprovalOutcome, CallState, Card, Item, Turn};
use crate::agents::{progress, AgentId};

use super::step_words::{self, Phrase};

/// How many finished steps sit above the current one.
const EARLIER: usize = 2;

/// From this long into a turn, the current line says how long it has been.
const SHOW_ELAPSED_SECS: u64 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepState {
    Running,
    Done,
    Failed,
    Waiting,
}

impl StepState {
    /// As `describe shell` and the Slint side spell it.
    pub fn word(self) -> &'static str {
        match self {
            StepState::Running => "running",
            StepState::Done => "done",
            StepState::Failed => "failed",
            StepState::Waiting => "waiting",
        }
    }
}

/// One step, as said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub text: String,
    pub state: StepState,
    /// Something done, not the mind thinking between steps; only these are folded.
    pub action: bool,
}

/// Something the turn did, in the order it was first seen.
#[derive(Clone, Debug, PartialEq)]
enum Seen {
    /// A call, by the harness's id for it; read back from the store each time, so it settles.
    Card(String),
    /// A step the mind's status line named.
    Status(Phrase),
}

/// The one turn the Lens is following, and what it has seen of it.
#[derive(Debug, Default)]
pub struct Tracker {
    turn: Option<(AgentId, u64)>,
    /// The conversation row of the bubble the steps belong to.
    row: Option<usize>,
    seen: Vec<Seen>,
    /// What was last said, for `describe shell`: the lines shown while it runs, the fold after.
    said: Vec<Step>,
}

impl Tracker {
    /// Follow `t` of `a`: note what is new in it, and give every step it has taken, oldest first.
    /// A different turn starts over.
    pub fn follow(&mut self, a: &Agent, t: &Turn) -> Vec<Step> {
        let key = (a.meta.id.clone(), t.n);
        if self.turn.as_ref() != Some(&key) {
            *self = Tracker { turn: Some(key), ..Tracker::default() };
        }
        for card in t.cards() {
            if !self.seen.iter().any(|s| matches!(s, Seen::Card(id) if *id == card.call)) {
                self.seen.push(Seen::Card(card.call.clone()));
            }
        }
        if t.open() {
            if let Some(phrase) = step_words::of_status(&a.status) {
                // A heartbeat repeats the line it last sent; only a new step is a step.
                if self.seen.last() != Some(&Seen::Status(phrase.clone())) {
                    self.seen.push(Seen::Status(phrase));
                }
            }
        }
        let open = t.open();
        let running_card = t.cards().any(Card::running);
        let last = self.seen.len().saturating_sub(1);
        self.seen
            .iter()
            .enumerate()
            .filter_map(|(i, seen)| match seen {
                Seen::Card(id) => {
                    // As shown: a call holding words the person erased reads with the marker.
                    let card = t.cards().find(|c| c.call == *id)?;
                    let card = card.shown();
                    let phrase = step_words::of_call(&card.name, &card.target, &card.args, &card.preview);
                    let state = match card.state {
                        CallState::Running if open => StepState::Running,
                        CallState::Failed | CallState::Interrupted => StepState::Failed,
                        // A call told only by its trail line is running while it is the newest.
                        CallState::Untold if open && i == last => StepState::Running,
                        _ => StepState::Done,
                    };
                    Some(said(&phrase, state))
                }
                Seen::Status(phrase) => {
                    let current = open && i == last && !running_card && !a.status.trim().is_empty();
                    Some(said(phrase, if current { StepState::Running } else { StepState::Done }))
                }
            })
            .collect()
    }
}

fn said(phrase: &Phrase, state: StepState) -> Step {
    let text = match state {
        StepState::Running | StepState::Waiting => phrase.running(),
        StepState::Done => phrase.done(),
        StepState::Failed => phrase.failed(),
    };
    Step { text, state, action: phrase.is_action() }
}

/// What else the current line has to say: whether the person is waited on, whether it looks
/// stuck, how long the turn has run, and whether the answer has started arriving.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Live {
    pub waiting: Option<String>,
    pub stuck: Option<String>,
    pub elapsed_secs: u64,
    pub answering: bool,
}

impl Live {
    /// From the shell's own reading of the agent (`progress::of`) and the open turn.
    pub fn of(a: &Agent, t: &Turn, now: u64, answering: bool) -> Live {
        let Some(p) = progress::of(a, now) else { return Live { answering, ..Live::default() } };
        let waiting = p.waiting_on_you.then(|| waiting_line(t, p.question.is_some()));
        let stuck = p.stuck_kind.as_ref().map(|kind| match kind {
            progress::Stuck::Quiet { secs } => format!("Nothing heard from {} for {}", a.meta.mind, progress::span(*secs)),
            progress::Stuck::Repeating { call, times } => {
                format!("{}, {times} times in a row", step_words::of_call(call, "", &serde_json::Value::Null, "").failed())
            }
        });
        Live { waiting, stuck, elapsed_secs: p.elapsed_secs, answering }
    }
}

/// "Waiting for you to allow: Delete an event from your calendar". The card itself is pinned above
/// the conversation; this line points at it.
fn waiting_line(t: &Turn, question: bool) -> String {
    let pending = t.items.iter().rev().find_map(|i| match i {
        Item::Approval(a) if a.outcome == ApprovalOutcome::Pending => Some(a.what.as_str()),
        _ => None,
    });
    match pending {
        Some(what) => format!("Waiting for you to allow: {}", step_words::of_call("os_act", what, &serde_json::Value::Null, "").asked()),
        None if question => "Waiting for your answer to its question".to_string(),
        None => "Waiting for you".to_string(),
    }
}

/// The lines the bubble draws: up to two finished steps, then the current one.
pub fn shown(steps: &[Step], live: &Live) -> Vec<Step> {
    let running = steps.iter().rposition(|s| s.state == StepState::Running);
    let current = if let Some(waiting) = &live.waiting {
        Step { text: waiting.clone(), state: StepState::Waiting, action: false }
    } else if let Some(stuck) = &live.stuck {
        Step { text: stuck.clone(), state: StepState::Running, action: false }
    } else {
        let text = match running {
            Some(i) => format!("{}\u{2026}", steps[i].text),
            None if live.answering => "Writing the answer\u{2026}".to_string(),
            None => "Thinking\u{2026}".to_string(),
        };
        let text = if live.elapsed_secs >= SHOW_ELAPSED_SECS {
            format!("{text} \u{00b7} {}", progress::span(live.elapsed_secs))
        } else {
            text
        };
        Step { text, state: StepState::Running, action: false }
    };
    let earlier: Vec<Step> = steps.iter().filter(|s| s.state != StepState::Running).cloned().collect();
    let mut out: Vec<Step> = earlier[earlier.len().saturating_sub(EARLIER)..].to_vec();
    out.push(current);
    out
}

/// The actions of a turn that has ended, for the reply's fold: nothing is still running, and the
/// thinking between them is left out.
pub fn folded(steps: Vec<Step>) -> Vec<Step> {
    steps
        .into_iter()
        .filter(|s| s.action)
        .map(|s| match s.state {
            StepState::Running | StepState::Waiting => Step { state: StepState::Done, ..s },
            _ => s,
        })
        .collect()
}

thread_local! {
    /// The Lens's one tracker. Only the UI thread reads or writes it: the tick and the pump.
    static TRACKER: RefCell<Tracker> = RefCell::new(Tracker::default());
}

/// The lines for the Lens's open turn, bound to `row` (the streaming bubble) the first time the
/// turn is seen. None when the active mind has no turn open.
pub fn live_lines(agent: &AgentId, row: Option<usize>, answering: impl Fn(usize) -> bool, now: u64) -> Option<(usize, Vec<Step>)> {
    crate::agents::store().read(|s| {
        let a = s.agent(agent)?;
        let t = a.open_turn()?;
        TRACKER.with(|tracker| {
            let mut tracker = tracker.borrow_mut();
            let steps = tracker.follow(a, t);
            if tracker.row.is_none() {
                tracker.row = row;
            }
            let row = tracker.row?;
            let lines = shown(&steps, &Live::of(a, t, now, answering(row)));
            tracker.said = lines.clone();
            Some((row, lines))
        })
    })
}

/// The steps of the turn whose bubble is `row`, now that its answer has ended: read once more from
/// the store (the pump can finish before the tick sees the last call) and folded. Empty when the
/// Lens was not following that bubble.
pub fn fold(row: usize) -> Vec<Step> {
    TRACKER.with(|tracker| {
        let mut tracker = tracker.borrow_mut();
        let Some((id, n)) = tracker.turn.clone().filter(|_| tracker.row == Some(row)) else {
            return Vec::new();
        };
        let steps = crate::agents::store()
            .read(|s| s.agent(&id).and_then(|a| a.turns.iter().find(|t| t.n == n).map(|t| tracker.follow(a, t))))
            .unwrap_or_default();
        tracker.said = folded(steps);
        tracker.said.clone()
    })
}

/// What `describe shell` says of the bubble at `row`: the lines it shows while it runs, or its
/// fold once it ended. None for any other row.
pub fn for_describe(row: usize) -> Option<serde_json::Value> {
    TRACKER.with(|tracker| {
        let tracker = tracker.borrow();
        (tracker.row == Some(row) && !tracker.said.is_empty()).then(|| {
            tracker
                .said
                .iter()
                .map(|s| serde_json::json!({ "text": s.text, "state": s.state.word() }))
                .collect()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::model::{AgentMeta, Approval, Provenance, State, Turn};

    fn agent() -> Agent {
        Agent {
            meta: AgentMeta::new(AgentId::new("yantrik-mind", AgentId::MAIN), "Yantrik Mind"),
            state: State::Thinking,
            since: 0,
            status: String::new(),
            turns: vec![Turn {
                n: 3,
                prompt: "What's on my calendar tomorrow?".into(),
                started: 100,
                ended: None,
                ok: None,
                lost: false,
                origin: Default::default(),
                items: Vec::new(),
                events: true,
                trail_seq: 0,
            }],
            usage: Default::default(),
            refused: 0,
            refusals: Vec::new(),
            refusals_shown: None,
            erasures: Vec::new(),
            approvals_asked: 0,
            approvals_answered: 0,
            pending_approvals: Vec::new(),
            job_waits: false,
            seq: 0,
            touched: 100,
            next_turn: 4,
        }
    }

    fn follow(tracker: &mut Tracker, a: &Agent) -> Vec<Step> {
        tracker.follow(a, &a.turns[0])
    }

    fn texts(steps: &[Step]) -> Vec<(&str, StepState)> {
        steps.iter().map(|s| (s.text.as_str(), s.state)).collect()
    }

    fn card(call: &str, name: &str, target: &str, state: CallState) -> Item {
        let mut c = Card::new(call, name, target, serde_json::json!({}), Provenance::Reported, 101);
        c.state = state;
        Item::Card(c)
    }

    #[test]
    fn a_status_only_turn_shows_its_steps_as_the_status_moves() {
        let mut a = agent();
        let mut tracker = Tracker::default();
        a.status = "Grounding from memory…".into();
        assert_eq!(texts(&follow(&mut tracker, &a)), [("Checking what I remember", StepState::Running)]);

        a.status = "Using web_search…".into();
        follow(&mut tracker, &a);
        a.status = "Using web_search… (30 s)".into();
        let steps = follow(&mut tracker, &a);
        assert_eq!(
            texts(&steps),
            [("Checked what I remember", StepState::Done), ("Searching the web", StepState::Running)],
            "a heartbeat repeating its line is not a new step"
        );

        let lines = shown(&steps, &Live { elapsed_secs: 24, ..Live::default() });
        assert_eq!(
            texts(&lines),
            [("Checked what I remember", StepState::Done), ("Searching the web\u{2026} \u{00b7} 24 s", StepState::Running)]
        );
    }

    #[test]
    fn calls_settle_in_place_and_a_failed_one_says_what_could_not_be_done() {
        let mut a = agent();
        let mut tracker = Tracker::default();
        a.turns[0].items.push(card("c1", "os_describe", "calendar", CallState::Running));
        assert_eq!(texts(&follow(&mut tracker, &a)), [("Looking at your calendar", StepState::Running)]);

        if let Item::Card(c) = &mut a.turns[0].items[0] {
            c.state = CallState::Ok;
        }
        a.turns[0].items.push(card("c2", "web_fetch", "", CallState::Failed));
        a.turns[0].items.push(card("c3", "os_act", "calendar.add_event", CallState::Running));
        let steps = follow(&mut tracker, &a);
        let lines = shown(&steps, &Live { elapsed_secs: 5, ..Live::default() });
        assert_eq!(
            texts(&lines),
            [
                ("Looked at your calendar", StepState::Done),
                ("Couldn\u{2019}t open the web page", StepState::Failed),
                ("Adding an event to your calendar\u{2026}", StepState::Running),
            ],
            "under 10 s the current line has no time on it"
        );
    }

    #[test]
    fn only_the_last_two_finished_steps_sit_above_the_current_one() {
        let steps: Vec<Step> = (0..5).map(|i| Step { text: format!("step {i}"), state: StepState::Done, action: true }).collect();
        let lines = shown(&steps, &Live { answering: true, ..Live::default() });
        assert_eq!(
            texts(&lines),
            [("step 3", StepState::Done), ("step 4", StepState::Done), ("Writing the answer\u{2026}", StepState::Running)]
        );
        assert_eq!(texts(&shown(&[], &Live::default())), [("Thinking\u{2026}", StepState::Running)], "a turn just opened says so at once");
    }

    #[test]
    fn a_wait_on_the_person_points_at_the_approval() {
        let mut a = agent();
        a.turns[0].items.push(Item::Approval(Approval {
            request: "appr-7".into(),
            what: "calendar.delete_event".into(),
            outcome: ApprovalOutcome::Pending,
            record: String::new(),
            asked: 101,
            settled: None,
        }));
        a.pending_approvals.push("appr-7".into());
        let live = Live::of(&a, &a.turns[0], 130, false);
        assert_eq!(live.waiting.as_deref(), Some("Waiting for you to allow: Delete an event from your calendar"));
        let lines = shown(&[], &live);
        assert_eq!(lines.last().map(|s| s.state), Some(StepState::Waiting));
    }

    #[test]
    fn a_quiet_turn_says_it_looks_stuck_in_the_shells_words() {
        let mut a = agent();
        a.touched = 100;
        let live = Live::of(&a, &a.turns[0], 100 + progress::STUCK_QUIET_SECS + 30, false);
        assert_eq!(live.stuck.as_deref(), Some("Nothing heard from Yantrik Mind for 2 min"));
    }

    #[test]
    fn a_turn_that_ended_folds_its_actions_with_nothing_left_running() {
        let mut a = agent();
        let mut tracker = Tracker::default();
        a.status = "Thinking…".into();
        follow(&mut tracker, &a);
        a.status = "Using web_search…".into();
        follow(&mut tracker, &a);
        a.turns[0].items.push(card("c1", "os_describe", "calendar", CallState::Running));
        a.turns[0].ended = Some(140);
        let steps = follow(&mut tracker, &a);
        assert_eq!(steps[0].text, "Thought it through", "the live line above the current one reads as done");
        assert_eq!(
            texts(&folded(steps)),
            [("Searched the web", StepState::Done), ("Looked at your calendar", StepState::Done)],
            "the thinking between actions is not an action"
        );
    }

    #[test]
    fn a_new_turn_starts_over() {
        let mut a = agent();
        let mut tracker = Tracker::default();
        a.status = "Thinking…".into();
        follow(&mut tracker, &a);
        a.turns[0].n = 4;
        a.status = "Grounding from memory…".into();
        assert_eq!(texts(&follow(&mut tracker, &a)), [("Checking what I remember", StepState::Running)]);
    }
}
