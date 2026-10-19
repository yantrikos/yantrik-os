//! A reply's one timeline: what the mind did to answer, folded into the reply itself.
//!
//! On VM 520 one Lens turn showed its progress three times: steps inline in the reply, a work card
//! under it and a strip above the composer, each worded a little differently, and Hermes's "⏳
//! Working — 3 min" heartbeat leaked into the reply's text. The reply bubble is now the one place:
//! "Thought for 5 s", then each step on one line (a mark for how it ended, what it did in the
//! Lens's plain words, and its one-line result), a question the mind asked as a card where it was
//! asked, one live line while it runs, and the answer under all of it. A click on a step opens its
//! arguments and output. The Agents screen keeps its full cards; this is the Lens's account of the
//! same store, and nothing here asks the mind.
//!
//! The pieces come from the store's turn (`lens_steps::Tracker` remembers the order it saw them
//! in); [`of`] puts them in order, and [`blocks`] is how a bubble carries them, live and folded.

use std::cell::RefCell;

use slint::SharedString;

use crate::agents::model::{Agent, CallState, Card, Item, Question, Turn};
use crate::agents::{progress, AgentId};
use crate::{ContentBlock, ToolCallData};

use super::lens_steps::{self, Live, Seen, Step, StepState, Tracker};

/// The longest result a step's line carries after its words.
const RESULT_CHARS: usize = 60;
/// How much of a call's output a step holds for when it is opened.
const OUTPUT_BYTES: usize = 16 * 1024;
/// The longest answer a folded question names.
const ANSWER_CHARS: usize = 80;

/// What a step opens to: the call as the mind made it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Detail {
    pub name: String,
    pub target: String,
    /// The call on one line, name and arguments, as the Agents card heads it.
    pub summary: String,
    pub arguments: String,
    pub output: String,
}

/// One line of a reply's timeline.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    /// All of the turn's thinking, folded: "Thought for 5 s", the thinking itself on a click.
    Thinking { secs: u64, text: String },
    /// A step: a call (with what it opens to), or a step the mind's status line named.
    Step { step: Step, detail: Option<Detail> },
    /// A question the mind asked. Waiting, the reply draws its card here; answered or closed, it
    /// is this one line.
    Question { request: String, line: String, waiting: bool },
    /// The one live line while the reply runs. Never in a reply that has ended.
    Status(Step),
}

impl Entry {
    /// As the bubble's block and `describe shell` name it.
    pub fn kind(&self) -> &'static str {
        match self {
            Entry::Thinking { .. } => "thinking",
            Entry::Step { .. } => "step",
            Entry::Question { .. } => "question",
            Entry::Status(_) => "status",
        }
    }

    pub fn text(&self) -> String {
        match self {
            Entry::Thinking { secs, .. } => format!("Thought for {}", progress::span(*secs)),
            Entry::Step { step, .. } | Entry::Status(step) => step.text.clone(),
            Entry::Question { line, .. } => line.clone(),
        }
    }

    /// The step's state word; a question's is waiting, answered or closed.
    pub fn state(&self) -> &'static str {
        match self {
            Entry::Thinking { .. } => "done",
            Entry::Step { step, .. } | Entry::Status(step) => step.state.word(),
            Entry::Question { waiting: true, .. } => "waiting",
            Entry::Question { line, .. } if line.starts_with("Answered") => "answered",
            Entry::Question { .. } => "closed",
        }
    }
}

/// The timeline of `t` from what `tracker` has seen of it, oldest first. `live` while the reply is
/// still arriving: steps still running are left to the one status line at the end, which says
/// what is happening now. `None` once it has ended: what was running reads as done, the thinking
/// between steps is left out, and there is no status line.
pub fn of(tracker: &Tracker, a: &Agent, t: &Turn, live: Option<&Live>) -> Vec<Entry> {
    let steps = tracker.steps(a, t);
    let mut out = Vec::new();
    for (i, seen) in tracker.seen().iter().enumerate() {
        match seen {
            Seen::Thinking => out.push(Entry::Thinking { secs: tracker.thought_for(t), text: thinking_of(t) }),
            Seen::Question(request) => {
                if let Some(q) = t.items.iter().find_map(|item| match item {
                    Item::Question(q) if q.request == *request => Some(q),
                    _ => None,
                }) {
                    out.push(question_entry(q, live.is_some()));
                }
            }
            Seen::Card(_) | Seen::Status(_) => {
                let Some((_, step)) = steps.iter().find(|(at, _)| *at == i) else { continue };
                if !step.action || (live.is_some() && step.state == StepState::Running) {
                    continue;
                }
                let step = match step.state {
                    StepState::Running | StepState::Waiting => Step { state: StepState::Done, ..step.clone() },
                    _ => step.clone(),
                };
                let card = match seen {
                    Seen::Card(call) => t.cards().find(|c| c.call == *call),
                    _ => None,
                };
                out.push(match card {
                    Some(card) => Entry::Step { step: with_result(step, card), detail: Some(detail_of(card)) },
                    None => Entry::Step { step, detail: None },
                });
            }
        }
    }
    // While a question waits its card is what the reply is waiting on; a second line saying so
    // would only repeat it.
    let asking = out.iter().any(|e| matches!(e, Entry::Question { waiting: true, .. }));
    if let Some(live) = live.filter(|_| !asking) {
        let steps: Vec<Step> = steps.into_iter().map(|(_, step)| step).collect();
        out.push(Entry::Status(lens_steps::current_line(&steps, live)));
    }
    out
}

/// "Looked at your calendar — 3 events": the step's words and, once it has ended, its result.
fn with_result(step: Step, card: &Card) -> Step {
    let shown = card.shown();
    let result = progress::brief(shown.summary.trim(), RESULT_CHARS);
    if result.is_empty() || card.running() {
        return step;
    }
    Step { text: format!("{} \u{2014} {result}", step.text), ..step }
}

fn detail_of(card: &Card) -> Detail {
    // As shown: a call holding words the person erased opens with the marker in their place.
    let shown = card.shown();
    let call = shown.as_call();
    Detail {
        name: shown.name.clone(),
        target: shown.target.clone(),
        summary: call.summary(),
        arguments: call.detail(),
        output: shown.output.plain(OUTPUT_BYTES),
    }
}

fn thinking_of(t: &Turn) -> String {
    t.items
        .iter()
        .filter_map(|item| match item {
            Item::Thinking(text) => Some(text.text()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A question as its line says it: what it asks while it waits, then what was answered, or why it
/// can no longer be.
///
/// Its words are only ever drawn live, read from the store each time, where an erasure of them
/// reaches (docs/harness.md, `redact`); a reply that has ended keeps a copy of its blocks, so a
/// question still waiting there is said as no longer waited on, never in its own words.
fn question_entry(q: &Question, live: bool) -> Entry {
    let waiting = q.waiting() && live;
    let line = if waiting {
        q.prompt.trim().to_string()
    } else if !q.answer.is_empty() {
        format!("Answered: {}", progress::brief(q.answer.trim(), ANSWER_CHARS))
    } else if q.closed.is_empty() {
        format!("Not answered: {}", Question::STOPPED_ASKING)
    } else {
        format!("Not answered: {}", q.closed)
    };
    Entry::Question { request: q.request.clone(), line, waiting }
}

/// A call as one step of a reply, for a reply put back from the store (`lens_history`): its words,
/// its result, and what it opens to.
pub fn step_block(card: &Card) -> ContentBlock {
    let shown = card.shown();
    let phrase = super::step_words::of_call(&shown.name, &shown.target, &shown.args, &shown.preview);
    let step = match card.state {
        CallState::Failed | CallState::Interrupted => Step { text: phrase.failed(), state: StepState::Failed, action: true },
        _ => Step { text: phrase.done(), state: StepState::Done, action: true },
    };
    let entry = Entry::Step { step: with_result(step, card), detail: Some(detail_of(card)) };
    blocks(std::slice::from_ref(&entry)).remove(0)
}

/// The blocks a bubble carries for its timeline, in order, ahead of its text. The step's state
/// word and what a click opens ride in the block's call; a question's request id is the call's
/// name, so the bubble can find its card.
pub fn blocks(entries: &[Entry]) -> Vec<ContentBlock> {
    entries
        .iter()
        .map(|entry| {
            let mut call = ToolCallData { status: entry.state().into(), ..Default::default() };
            match entry {
                Entry::Thinking { text, .. } => call.output = text.as_str().into(),
                Entry::Step { detail: Some(d), .. } => {
                    call.name = d.name.as_str().into();
                    call.target = d.target.as_str().into();
                    call.summary = d.summary.as_str().into();
                    call.arguments = d.arguments.as_str().into();
                    call.output = d.output.as_str().into();
                }
                Entry::Question { request, .. } => call.name = request.as_str().into(),
                Entry::Step { detail: None, .. } | Entry::Status(_) => {}
            }
            ContentBlock { block_type: entry.kind().into(), text: SharedString::from(entry.text()), call }
        })
        .collect()
}

/// Whether a block is part of a timeline rather than of the reply's text.
pub fn is_timeline(block_type: &str) -> bool {
    matches!(block_type, "thinking" | "step" | "question" | "status")
}

// ── The Lens's one followed turn ─────────────────────────────────────────────────────────────

/// The turn the Lens follows, the bubble its reply streams into, and what was last said of it.
#[derive(Default)]
struct Following {
    tracker: Tracker,
    /// The conversation row of the bubble the timeline belongs to, bound when the turn is first
    /// seen and let go with it.
    row: Option<usize>,
    said: Vec<Entry>,
}

impl Following {
    fn look(&mut self, a: &Agent, t: &Turn, now: u64) {
        let turn = Some((a.meta.id.clone(), t.n));
        if self.tracker.turn() != turn.as_ref() {
            *self = Following::default();
        }
        self.tracker.observe(a, t, now);
    }
}

thread_local! {
    /// Only the UI thread reads or writes it: the agents tick and the reply pump.
    static FOLLOWING: RefCell<Following> = RefCell::new(Following::default());
}

/// The live timeline of the Lens's open turn, bound to `row` (the streaming bubble) the first time
/// the turn is seen. None when the active mind has no turn open.
pub fn live(agent: &AgentId, row: Option<usize>, answering: impl Fn(usize) -> bool, now: u64) -> Option<(usize, Vec<Entry>)> {
    crate::agents::store().read(|s| {
        let a = s.agent(agent)?;
        let t = a.open_turn()?;
        FOLLOWING.with(|following| {
            let mut f = following.borrow_mut();
            f.look(a, t, now);
            if f.row.is_none() {
                f.row = row;
            }
            let row = f.row?;
            let entries = of(&f.tracker, a, t, Some(&Live::of(a, t, now, answering(row))));
            f.said = entries.clone();
            Some((row, entries))
        })
    })
}

/// The timeline of the reply at `row`, now that it has ended: read once more from the store (the
/// pump can finish before the tick sees the last call) and folded. Empty when the Lens was not
/// following that bubble.
pub fn fold(row: usize) -> Vec<Entry> {
    FOLLOWING.with(|following| {
        let mut f = following.borrow_mut();
        let Some((id, n)) = f.tracker.turn().cloned().filter(|_| f.row == Some(row)) else {
            return Vec::new();
        };
        let now = crate::agents::model::now();
        let entries = crate::agents::store()
            .read(|s| {
                let a = s.agent(&id)?;
                let t = a.turns.iter().find(|t| t.n == n)?;
                f.look(a, t, now);
                Some(of(&f.tracker, a, t, None))
            })
            .unwrap_or_default();
        f.said = entries.clone();
        entries
    })
}

/// Whether `agent`'s question `request` is drawn in the reply the Lens is following, so the panel
/// does not pin a second card for it.
pub fn in_reply(agent: &AgentId, request: &str) -> bool {
    FOLLOWING.with(|following| {
        let f = following.borrow();
        f.row.is_some()
            && f.tracker.turn().is_some_and(|(id, _)| id == agent)
            && f.tracker.seen().contains(&Seen::Question(request.to_string()))
    })
}

/// What `describe shell` says of the bubble at `row`: its timeline while it runs, or folded once it
/// ended. None for any other row.
pub fn for_describe(row: usize) -> Option<serde_json::Value> {
    FOLLOWING.with(|following| {
        let f = following.borrow();
        (f.row == Some(row) && !f.said.is_empty()).then(|| {
            f.said
                .iter()
                .map(|e| serde_json::json!({ "kind": e.kind(), "text": e.text(), "state": e.state() }))
                .collect()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::model::{AgentMeta, Capped, Provenance, State};

    fn agent(items: Vec<Item>, ended: Option<u64>) -> Agent {
        Agent {
            meta: AgentMeta::new(AgentId::new("hermes", AgentId::MAIN), "Hermes Agent"),
            state: State::Thinking,
            since: 0,
            status: String::new(),
            turns: vec![Turn {
                n: 1,
                prompt: "What will the weather be in Dallas this weekend?".into(),
                started: 100,
                ended,
                ok: ended.map(|_| true),
                lost: false,
                origin: Default::default(),
                items,
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
            next_turn: 2,
        }
    }

    fn card(call: &str, name: &str, args: serde_json::Value, state: CallState, summary: &str) -> Item {
        let mut c = Card::new(call, name, "", args, Provenance::Reported, 101);
        c.state = state;
        c.summary = summary.into();
        Item::Card(c)
    }

    fn thinking(words: &str) -> Item {
        let mut t = Capped::new(1024, 4096);
        t.push(words.as_bytes());
        Item::Thinking(t)
    }

    fn text(words: &str) -> Item {
        let mut t = Capped::new(1024, 4096);
        t.push(words.as_bytes());
        Item::Text(t)
    }

    fn question(answer: &str, closed: &str) -> Item {
        Item::Question(Question {
            request: "c1".into(),
            prompt: "Which days is this weekend for you?".into(),
            options: vec!["Sat 17 + Sun 18".into(), "Thu + Fri".into()],
            recommended: Some(0),
            answer: answer.into(),
            closed: closed.into(),
            asked: 103,
        })
    }

    fn lines(entries: &[Entry]) -> Vec<(&'static str, String, &'static str)> {
        entries.iter().map(|e| (e.kind(), e.text(), e.state())).collect()
    }

    /// The turn on VM 541, as the store has it once it ended: Hermes thought, searched, asked which
    /// weekend, was answered, fetched the forecast and wrote the reply.
    #[test]
    fn a_turns_items_become_one_timeline_in_the_order_they_happened() {
        let mut a = agent(vec![thinking("The person means this weekend; which days?")], None);
        let mut tracker = Tracker::default();
        tracker.observe(&a, &a.turns[0], 105);
        let items = &mut a.turns[0].items;
        items.push(card("t1", "web_search", serde_json::json!({"query": "dallas weather"}), CallState::Ok, "3 results"));
        items.push(question("Sat 17 + Sun 18", ""));
        items.push(card("t2", "web_fetch", serde_json::json!({"url": "https://weather.example/dallas"}), CallState::Ok, ""));
        items.push(text("Dallas, Sat 17 + Sun 18: sunny, 29°C."));
        a.turns[0].ended = Some(140);
        tracker.observe(&a, &a.turns[0], 140);

        let entries = of(&tracker, &a, &a.turns[0], None);
        assert_eq!(
            lines(&entries),
            [
                ("thinking", "Thought for 5 s".to_string(), "done"),
                ("step", "Searched the web for \u{201c}dallas weather\u{201d} \u{2014} 3 results".to_string(), "done"),
                ("question", "Answered: Sat 17 + Sun 18".to_string(), "answered"),
                ("step", "Opened the web page".to_string(), "done"),
            ],
            "thinking folded, steps in order, the question where it was asked, and no status line once it ended"
        );
        assert!(
            !entries.iter().any(|e| e.text().contains("Dallas, Sat")),
            "the reply's own words are its text, never a line of the timeline"
        );
        let Entry::Step { detail: Some(detail), .. } = &entries[1] else { panic!("a call opens to its detail") };
        assert_eq!(detail.name, "web_search");
        assert!(detail.arguments.contains("dallas weather"), "{}", detail.arguments);
    }

    #[test]
    fn while_it_runs_a_waiting_question_is_a_card_in_place_and_no_status_line_repeats_it() {
        let mut a = agent(vec![card("t1", "web_search", serde_json::json!({"query": "dallas weather"}), CallState::Ok, ""), question("", "")], None);
        a.status = "asks: Which days is this weekend for you?".into();
        a.state = State::WaitingForYou;
        let mut tracker = Tracker::default();
        tracker.observe(&a, &a.turns[0], 120);
        let entries = of(&tracker, &a, &a.turns[0], Some(&Live::of(&a, &a.turns[0], 120, false)));
        assert_eq!(
            lines(&entries),
            [
                ("step", "Searched the web for \u{201c}dallas weather\u{201d}".to_string(), "done"),
                ("question", "Which days is this weekend for you?".to_string(), "waiting"),
            ]
        );

        // Answered, the run goes on in the same reply: the card folds and the live line returns.
        a.turns[0].items[1] = question("Sat 17 + Sun 18", "");
        a.turns[0].items.push(card("t2", "web_fetch", serde_json::json!({}), CallState::Running, ""));
        a.status.clear();
        a.state = State::Thinking;
        tracker.observe(&a, &a.turns[0], 130);
        let entries = of(&tracker, &a, &a.turns[0], Some(&Live { elapsed_secs: 30, ..Live::default() }));
        assert_eq!(
            lines(&entries),
            [
                ("step", "Searched the web for \u{201c}dallas weather\u{201d}".to_string(), "done"),
                ("question", "Answered: Sat 17 + Sun 18".to_string(), "answered"),
                ("status", "Opening the web page\u{2026} \u{00b7} 30 s".to_string(), "running"),
            ],
            "a step still running is said once, by the live line, with how long the turn has run"
        );
    }

    #[test]
    fn a_question_closed_unanswered_says_why_in_one_line() {
        let a = agent(vec![question("", Question::STOPPED_BY_YOU)], Some(130));
        let mut tracker = Tracker::default();
        tracker.observe(&a, &a.turns[0], 130);
        assert_eq!(
            lines(&of(&tracker, &a, &a.turns[0], None)),
            [("question", "Not answered: you stopped it".to_string(), "closed")]
        );
    }

    #[test]
    fn hermess_heartbeat_is_never_a_step_and_never_in_the_text() {
        let mut a = agent(Vec::new(), None);
        let mut tracker = Tracker::default();
        for (minutes, now) in [(3, 280), (6, 460)] {
            a.status = format!("Working \u{2014} {minutes} min");
            tracker.observe(&a, &a.turns[0], now);
        }
        let entries = of(&tracker, &a, &a.turns[0], Some(&Live { elapsed_secs: 360, ..Live::default() }));
        assert_eq!(lines(&entries), [("status", "Working\u{2026} \u{00b7} 6 min".to_string(), "running")]);
        a.turns[0].ended = Some(500);
        assert!(of(&tracker, &a, &a.turns[0], None).is_empty(), "nothing to fold: working is not something done");
    }

    #[test]
    fn a_bubble_carries_its_timeline_as_blocks_a_question_by_its_request() {
        let entries = vec![
            Entry::Thinking { secs: 5, text: "which days?".into() },
            Entry::Question { request: "c1".into(), line: "Which days?".into(), waiting: true },
            Entry::Status(Step { text: "Thinking\u{2026}".into(), state: StepState::Running, action: false }),
        ];
        let blocks = blocks(&entries);
        let kinds: Vec<(String, String, String)> =
            blocks.iter().map(|b| (b.block_type.to_string(), b.text.to_string(), b.call.status.to_string())).collect();
        assert_eq!(
            kinds,
            [
                ("thinking".into(), "Thought for 5 s".into(), "done".into()),
                ("question".into(), "Which days?".into(), "waiting".into()),
                ("status".into(), "Thinking\u{2026}".into(), "running".into()),
            ]
        );
        assert_eq!(blocks[0].call.output, "which days?", "the thinking is what the line opens to");
        assert_eq!(blocks[1].call.name, "c1");
        assert!(blocks.iter().all(|b| is_timeline(&b.block_type)));
    }
}
