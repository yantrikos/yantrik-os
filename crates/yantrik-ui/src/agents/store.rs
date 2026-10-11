//! Every agent, its session and its cards, and the rules that change them.
//!
//! The store is fed, never polled: pieces 1 (the harness wire) and 2 (the agent terminal) push into
//! it through the input API below, and until they land the Lens text path does (`feed.rs`). It is
//! plain data behind one lock — no window, no socket — so every rule here is tested without either.
//!
//! # The lifecycle, enforced here too
//!
//! The host enforces the event lifecycle on the wire (design decision 2). The store does not rely
//! on that, because the store has more than one feeder and a view that trusted every feeder to be
//! right would draw whatever the most confused one sent:
//!
//! - a *reported* event is taken only while a turn is open; after the turn ends it is refused and
//!   counted, never silently lost;
//! - a call's events go in order: a second start for the same call is refused, output after its
//!   end is refused, a second end is refused;
//! - an end (or output) for a call that never started becomes a card of its own, marked as such,
//!   so the claim is visible and settles nothing else;
//! - when a turn ends, every reported call still open is settled *interrupted*. A *verified* call
//!   — a command the shell itself is running — keeps running: the shell owns that process, not
//!   the turn, and its card keeps streaming;
//! - one event's text is cut at [`EVENT_CAP`]; one call keeps at most [`CARD_CAP`] of output, its
//!   head and its tail with a marker between.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::model::*;
use crate::trail::ToolCall;

mod erase;
pub use erase::ErasurePlan;

/// How many turns an agent keeps in memory. The oldest go first.
pub const TURNS_KEPT: usize = 200;

/// How many of an agent's newest cards keep their whole output. Older ones keep [`OLD_OUTPUT`] of
/// it, so a session of a hundred commands is not two hundred megabytes.
pub const FULL_OUTPUTS: usize = 32;
pub const OLD_OUTPUT: usize = 64 * 1024;

/// How many of the newest refusals keep their line, what arrived and why it was refused (#212).
/// The count keeps all of them.
pub const REFUSED_LINES: usize = 6;

/// What is kept on disk: this many agents, the newest of each one's turns, and the newest part of
/// any one text or output.
pub const KEEP_AGENTS: usize = 24;
pub const PERSIST_TURNS: usize = 50;
pub const PERSIST_BYTES: usize = 64 * 1024;

/// What [`Store::transcript`] gives one reader: at most this much in all, the newest of any one
/// block of text, and the last lines of a card's output.
pub const TRANSCRIPT_BYTES: usize = 32 * 1024;
const TRANSCRIPT_TEXT: usize = 4 * 1024;
const TRANSCRIPT_OUTPUT_LINES: usize = 8;

pub struct Store {
    agents: Vec<Agent>,
    next_seq: u64,
    revision: u64,
    dirty: BTreeSet<AgentId>,
    removed: BTreeSet<AgentId>,
    /// Files a stale save would have deleted, to delete on the next one (`mark_all_dirty`).
    pending_deletes: Vec<PathBuf>,
    clock: Box<dyn Fn() -> u64 + Send>,
    /// How many agents it keeps before letting the oldest idle ones go: [`KEEP_AGENTS`].
    keep: usize,
}

impl Default for Store {
    fn default() -> Self {
        Store::new()
    }
}

impl Store {
    pub fn new() -> Store {
        Store::with_clock(Box::new(now))
    }

    /// A store that reads the time from `clock` — for tests, which need "two minutes later".
    pub fn with_clock(clock: Box<dyn Fn() -> u64 + Send>) -> Store {
        Store {
            agents: Vec::new(),
            next_seq: 1,
            revision: 0,
            dirty: BTreeSet::new(),
            removed: BTreeSet::new(),
            pending_deletes: Vec::new(),
            clock,
            keep: KEEP_AGENTS,
        }
    }

    /// Keep `n` agents rather than [`KEEP_AGENTS`]. For the one store a test binary shares: every
    /// test in it adds agents to it at once, and one test's agents let go to make room for
    /// another's made both flaky. The bound itself is tested on a store of its own.
    pub fn keeping(mut self, n: usize) -> Store {
        self.keep = n;
        self
    }

    fn now(&self) -> u64 {
        (self.clock)()
    }

    /// Goes up on every change, so a view can tell whether it has anything to redraw.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn agents(&self) -> &[Agent] {
        &self.agents
    }

    pub fn agent(&self, id: &AgentId) -> Option<&Agent> {
        self.agents.iter().find(|a| &a.meta.id == id)
    }

    fn index(&self, id: &AgentId) -> Option<usize> {
        self.agents.iter().position(|a| &a.meta.id == id)
    }

    fn mark(&mut self, i: usize) {
        let now = self.now();
        let agent = &mut self.agents[i];
        agent.touched = now;
        self.dirty.insert(agent.meta.id.clone());
        self.revision += 1;
    }

    // ── The input API: what pieces 1 and 2 call ───────────────────────

    /// Make an agent known, or bring what is known about it up to date. Its session is kept.
    pub fn upsert_agent(&mut self, meta: AgentMeta) {
        let now = self.now();
        match self.index(&meta.id) {
            Some(i) => {
                let known = &mut self.agents[i].meta;
                if !meta.mind.is_empty() {
                    known.mind = meta.mind;
                }
                if !meta.model.is_empty() {
                    known.model = meta.model;
                }
                if known.title.is_empty() && !meta.title.is_empty() {
                    known.title = meta.title;
                }
                if meta.parent.is_some() {
                    known.parent = meta.parent;
                }
                // A role is set once, when the agent is started as one; nothing takes it away.
                if meta.role.is_some() {
                    known.role = meta.role;
                }
                // So is the recipe that started it.
                if meta.recipe.is_some() {
                    known.recipe = meta.recipe;
                }
                known.conversations = meta.conversations;
                self.mark(i);
            }
            None => {
                let started = if meta.started == 0 { now } else { meta.started };
                let seq = self.next_seq;
                self.next_seq += 1;
                self.agents.push(Agent {
                    meta: AgentMeta { started, ..meta },
                    state: State::Idle,
                    since: now,
                    status: String::new(),
                    turns: Vec::new(),
                    usage: Usage::default(),
                    refused: 0,
                    refusals: Vec::new(),
                    refusals_shown: None,
                    erasures: Vec::new(),
                    approvals_asked: 0,
                    approvals_answered: 0,
                    pending_approvals: Vec::new(),
                    job_waits: false,
                    seq,
                    touched: now,
                    next_turn: 1,
                });
                let last = self.agents.len() - 1;
                self.mark(last);
                self.let_old_ones_go();
            }
        }
    }

    /// Forget an agent, and its file with it.
    pub fn remove_agent(&mut self, id: &AgentId) -> bool {
        let Some(i) = self.index(id) else { return false };
        self.agents.remove(i);
        self.dirty.remove(id);
        self.removed.insert(id.clone());
        self.revision += 1;
        true
    }

    /// The person said something to a mind in the chat (the Lens): a turn begins that is chat,
    /// not agent work. It is listed in Agents as a run only if it goes on to do work.
    pub fn open_chat_turn(&mut self, id: &AgentId, prompt: &str) {
        self.open_turn(id, prompt);
        if let Some(i) = self.index(id) {
            if let Some(t) = self.agents[i].turns.last_mut() {
                t.origin = TurnOrigin::Chat;
            }
            self.mark(i);
        }
    }

    /// The person (or a parent agent) said something to this agent: a turn begins.
    ///
    /// An agent nobody has announced is made known here, named after its harness — a turn is
    /// evidence enough that it exists. A turn still open is ended first: one turn at a time.
    pub fn open_turn(&mut self, id: &AgentId, prompt: &str) {
        if self.index(id).is_none() {
            self.upsert_agent(AgentMeta::new(id.clone(), id.harness()));
        }
        let Some(i) = self.index(id) else { return };
        let now = self.now();
        let agent = &mut self.agents[i];
        if agent.open_turn().is_some() {
            settle_turn(agent, false, now, Some("Another turn began before this one finished."));
        }
        if agent.meta.title.is_empty() {
            agent.meta.title = title_of(prompt);
        }
        let n = agent.next_turn;
        agent.next_turn += 1;
        agent.turns.push(Turn {
            n,
            prompt: prompt.to_string(),
            started: now,
            ended: None,
            ok: None,
            lost: false,
            origin: TurnOrigin::Agent,
            items: Vec::new(),
            events: false,
            trail_seq: 0,
        });
        if agent.turns.len() > TURNS_KEPT {
            let excess = agent.turns.len() - TURNS_KEPT;
            agent.turns.drain(..excess);
        }
        agent.status.clear();
        set_state(agent, State::Thinking, now);
        self.mark(i);
    }

    /// More of the mind's answer.
    pub fn text(&mut self, id: &AgentId, delta: &str) {
        let Some(i) = self.index(id) else { return };
        let now = self.now();
        let agent = &mut self.agents[i];
        let Some(turn) = agent.turns.last_mut().filter(|t| t.open()) else {
            refuse(agent, "some of its text — no turn was open".to_string());
            self.mark(i);
            return;
        };
        append(&mut turn.items, cap(delta), false);
        if matches!(agent.state, State::Idle | State::Done | State::Failed) {
            set_state(agent, State::Thinking, now);
        }
        self.mark(i);
    }

    /// Something the agent did, from the harness (`Reported`) or from the shell itself
    /// (`Verified`). See the module doc for what is refused and why.
    pub fn event(&mut self, id: &AgentId, event: &Event, provenance: Provenance) {
        let Some(i) = self.index(id) else { return };
        let now = self.now();
        let agent = &mut self.agents[i];
        if let Err(line) = apply(agent, event, provenance, now) {
            refuse(agent, line);
        }
        self.mark(i);
    }

    /// The turn ended — completed (`ok`) or failed.
    pub fn close_turn(&mut self, id: &AgentId, ok: bool) {
        let Some(i) = self.index(id) else { return };
        let now = self.now();
        let agent = &mut self.agents[i];
        if agent.open_turn().is_none() {
            return;
        }
        settle_turn(agent, ok, now, None);
        agent.status.clear();
        let next = if agent.state == State::HarnessGone {
            State::HarnessGone
        } else if agent.cards().any(Card::running) {
            // A command the shell owns outlives the turn that asked for it.
            State::RunningTool
        } else if ok {
            State::Done
        } else {
            State::Failed
        };
        set_state(agent, next, now);
        compact(agent);
        self.mark(i);
    }

    /// Say what state an agent is in, when its feeder knows better than the events do: waiting for
    /// the person, or its harness gone.
    pub fn set_state(&mut self, id: &AgentId, state: State) {
        let Some(i) = self.index(id) else { return };
        let now = self.now();
        let agent = &mut self.agents[i];
        if state == State::HarnessGone {
            // Its open calls are settled and its turn is over: nothing it started will report back.
            if agent.open_turn().is_some() {
                settle_turn(agent, false, now, Some("Its harness stopped answering."));
            }
        }
        set_state(agent, state, now);
        self.mark(i);
    }

    /// Which agents have a terminal job sitting at its prompt, as the shell's tick sees it
    /// (#182). A job waiting on the person's keyboard waits on the person exactly like an
    /// approval card does, and the list sorts the two alike: an agent with any job waiting goes
    /// to `WaitingForYou`, and comes back when none waits by the same rule a settled card uses.
    /// A repeated unchanged report changes nothing — the tick runs four times a second, and a
    /// job still waiting is not news, so it neither re-stamps `since` nor redraws the row.
    pub fn jobs_waiting(&mut self, waiting: &[AgentId]) {
        for id in waiting {
            self.known(id);
        }
        let now = self.now();
        for i in 0..self.agents.len() {
            let waits = waiting.contains(&self.agents[i].meta.id);
            let agent = &mut self.agents[i];
            if waits {
                if agent.job_waits && agent.state == State::WaitingForYou {
                    continue;
                }
                agent.job_waits = true;
                set_state(agent, State::WaitingForYou, now);
            } else if agent.job_waits {
                agent.job_waits = false;
                // The card's own rule for coming back, with nothing left holding the row.
                if agent.state == State::WaitingForYou && agent.pending_approvals.is_empty() {
                    let next = if agent.cards().any(Card::running) {
                        State::RunningTool
                    } else if agent.open_turn().is_some() {
                        State::Thinking
                    } else {
                        State::Idle
                    };
                    set_state(agent, next, now);
                }
            } else {
                continue;
            }
            self.mark(i);
        }
    }

    // ── Beyond the five: what the text path and the approval card add ──

    /// A call read out of the mind's text — a `⚙️` trail line (#125). Reported, and claiming no
    /// outcome. Ignored once the same turn has had structured events: then the trail is the same
    /// calls told a second time.
    pub fn trail_call(&mut self, id: &AgentId, call: &ToolCall) {
        let Some(i) = self.index(id) else { return };
        let now = self.now();
        let agent = &mut self.agents[i];
        let Some(turn) = agent.turns.last_mut().filter(|t| t.open()) else {
            refuse(agent, format!("a call read from its text (`{}`) — no turn was open", call.name));
            self.mark(i);
            return;
        };
        if turn.events {
            return;
        }
        turn.trail_seq += 1;
        let mut card = Card::new(
            &format!("trail-{}-{}", turn.n, turn.trail_seq),
            &call.name,
            &call.target,
            call.arguments.clone(),
            Provenance::Reported,
            now,
        );
        card.preview = call.preview.clone();
        card.repeats = call.repeats;
        card.state = CallState::Untold;
        card.ended = Some(now);
        turn.items.push(Item::Card(card));
        self.mark(i);
    }

    /// A command the shell started for this agent (`agent_run`): a verified card, running.
    pub fn command_started(&mut self, id: &AgentId, job: &str, command: &str, cwd: &str) {
        let i = self.known(id);
        let now = self.now();
        let agent = &mut self.agents[i];
        let args = serde_json::json!({ "command": command, "cwd": cwd });
        match find_card(agent, job, Provenance::Verified) {
            // Its output — or, for a command quicker than the call that started it, its end — got
            // here first; the start fills in what it was. Not a second start: the shell is the
            // only feeder of its own cards, and job ids are never reused, so the same id is the
            // same command reported in the other order.
            Some(card) => card.args = args,
            None => {
                let start = Event::ToolStart { call: job.into(), name: "agent_run".into(), target: String::new(), args };
                // Cannot be refused: no card exists for this job yet, which is the branch we are in.
                let _ = apply(agent, &start, Provenance::Verified, now);
            }
        }
        self.mark(i);
    }

    /// Bytes from a command's terminal, as the shell read them off its PTY: piece 2's
    /// `Jobs::on_output(agent, job, bytes)`. Bytes, not text, because a read can end inside a
    /// character. A job the store has not heard of opens its card here, verified; output after
    /// its end is refused.
    pub fn command_output(&mut self, id: &AgentId, job: &str, bytes: &[u8]) {
        let i = self.known(id);
        let now = self.now();
        let agent = &mut self.agents[i];
        let bytes = &bytes[..bytes.len().min(EVENT_CAP)];
        match find_card(agent, job, Provenance::Verified) {
            Some(card) if card.running() => card.push_output(Stream::Terminal, bytes),
            Some(_) => refuse(agent, format!("terminal bytes for `{job}` — the command had already ended")),
            None => {
                let mut card = Card::new(job, "agent_run", "", serde_json::json!({}), Provenance::Verified, now);
                card.output.push(Stream::Terminal, bytes);
                turn_for_verified(agent, now).items.push(Item::Card(card));
                if !matches!(agent.state, State::WaitingForYou | State::HarnessGone) {
                    set_state(agent, State::RunningTool, now);
                }
            }
        }
        self.mark(i);
    }

    /// A command ended: piece 2's `Jobs::on_finish`. `exit_code` is `None` for a command ended by
    /// a signal; `killed` when Stop, a timeout or the shell ended it.
    pub fn command_finished(&mut self, id: &AgentId, job: &str, command: &str, exit_code: Option<i32>, killed: bool) {
        let i = self.known(id);
        let now = self.now();
        let agent = &mut self.agents[i];
        if let Some(card) = find_card(agent, job, Provenance::Verified) {
            if card.args.get("command").is_none() && !command.is_empty() {
                card.args = serde_json::json!({ "command": command });
            }
        } else if !command.is_empty() {
            let start = Event::ToolStart {
                call: job.into(),
                name: "agent_run".into(),
                target: String::new(),
                args: serde_json::json!({ "command": command }),
            };
            // Cannot be refused: no card exists for this job yet, which is the branch we are in.
            let _ = apply(agent, &start, Provenance::Verified, now);
        }
        let summary = match (killed, exit_code) {
            (true, _) => "stopped".to_string(),
            (false, Some(code)) => format!("exit {code}"),
            (false, None) => "ended by a signal".to_string(),
        };
        let end = Event::ToolEnd { call: job.into(), ok: !killed && exit_code == Some(0), summary, exit_code };
        if let Err(line) = apply(agent, &end, Provenance::Verified, now) {
            refuse(agent, line);
        }
        self.mark(i);
    }

    /// The index of an agent, made known first when it is not: a command the shell runs for it is
    /// evidence enough that it exists.
    fn known(&mut self, id: &AgentId) -> usize {
        if self.index(id).is_none() {
            self.upsert_agent(AgentMeta::new(id.clone(), id.harness()));
        }
        self.index(id).expect("just made known")
    }

    /// A line from the shell into the session: "Stop asked", "the turn failed: …".
    pub fn note(&mut self, id: &AgentId, text: &str) {
        let Some(i) = self.index(id) else { return };
        let now = self.now();
        let agent = &mut self.agents[i];
        turn_for_verified(agent, now).items.push(Item::Note(text.to_string()));
        self.mark(i);
    }

    /// Replace the text the mind is writing — the built-in companion's `__REPLACE__`.
    pub fn replace_text(&mut self, id: &AgentId, text: &str) {
        let Some(i) = self.index(id) else { return };
        let agent = &mut self.agents[i];
        let Some(turn) = agent.turns.last_mut().filter(|t| t.open()) else { return };
        if let Some(Item::Text(_)) = turn.items.last() {
            turn.items.pop();
        }
        append(&mut turn.items, cap(text), false);
        self.mark(i);
    }

    /// The shell drew an approval card for this agent. Verified by construction: only the shell
    /// calls this, with the agent its token named. The row moves to Needs you, and the session
    /// gets the card itself — the shell's, bound to `request` — where the agent's work is.
    ///
    /// An agent the store has not heard of is made known: a token the host issued, checked against
    /// the caller's process, is evidence enough that it exists.
    pub fn approval_asked(&mut self, id: &AgentId, request: &str, what: &str) {
        let i = self.known(id);
        let now = self.now();
        let agent = &mut self.agents[i];
        if agent.pending_approvals.iter().any(|r| r == request) {
            return;
        }
        agent.pending_approvals.push(request.to_string());
        agent.approvals_asked += 1;
        turn_for_verified(agent, now).items.push(Item::Approval(Approval {
            request: request.to_string(),
            what: what.to_string(),
            outcome: ApprovalOutcome::Pending,
            record: String::new(),
            asked: now,
            settled: None,
        }));
        set_state(agent, State::WaitingForYou, now);
        self.mark(i);
    }

    /// The person answered it.
    pub fn approval_answered(&mut self, id: &AgentId, request: &str, allowed: bool) {
        let outcome = if allowed { ApprovalOutcome::Allowed } else { ApprovalOutcome::Denied };
        self.approval_settled(id, request, outcome, "");
    }

    /// An approval asked for this agent is over: answered, run out, or withdrawn. `record` is the
    /// line it leaves — the approval store's own, when it had one — and empty says it plainly.
    /// Only an answer counts as answered; a request that ran out or was taken back was not.
    pub fn approval_settled(&mut self, id: &AgentId, request: &str, outcome: ApprovalOutcome, record: &str) {
        if outcome == ApprovalOutcome::Pending {
            return;
        }
        let Some(i) = self.index(id) else { return };
        let now = self.now();
        let agent = &mut self.agents[i];
        let Some(at) = agent.pending_approvals.iter().position(|r| r == request) else { return };
        agent.pending_approvals.remove(at);
        if matches!(outcome, ApprovalOutcome::Allowed | ApprovalOutcome::Denied) {
            agent.approvals_answered += 1;
        }
        let line = if record.trim().is_empty() { settled_line(outcome, now) } else { record.trim().to_string() };
        let card = agent
            .turns
            .iter_mut()
            .rev()
            .flat_map(|t| t.items.iter_mut().rev())
            .find_map(|item| match item {
                Item::Approval(a) if a.request == request => Some(a),
                _ => None,
            });
        match card {
            Some(card) => {
                card.outcome = outcome;
                card.record = line;
                card.settled = Some(now);
            }
            // Its turn was let go (a long session keeps the newest turns); the answer still
            // belongs in the session.
            None => turn_for_verified(agent, now).items.push(Item::Note(line)),
        }
        // Back to work only when nothing else holds the row: a terminal job still at its prompt
        // keeps it waiting on the person (#182), and `jobs_waiting` brings it back instead.
        back_to_work(agent, now);
        self.mark(i);
    }

    /// The person answered the agent's question `request` through its card. `false` when there is
    /// no such question still waiting (answered, closed, or never asked), so nothing changed.
    pub fn question_answered(&mut self, id: &AgentId, request: &str, answer: &str) -> bool {
        let Some(i) = self.index(id) else { return false };
        let now = self.now();
        let agent = &mut self.agents[i];
        let Some(question) = agent
            .turns
            .iter_mut()
            .rev()
            .flat_map(|t| t.items.iter_mut().rev())
            .find_map(|item| match item {
                Item::Question(q) if q.request == request && q.waiting() => Some(q),
                _ => None,
            })
        else {
            return false;
        };
        question.answer = answer.to_string();
        agent.status.clear();
        back_to_work(agent, now);
        self.mark(i);
        true
    }

    /// The answer could not be delivered (the run ended, its harness left): the question stays in
    /// the session, closed, with why.
    pub fn question_closed(&mut self, id: &AgentId, request: &str, why: &str) {
        let Some(i) = self.index(id) else { return };
        let now = self.now();
        let agent = &mut self.agents[i];
        for item in agent.turns.iter_mut().flat_map(|t| t.items.iter_mut()) {
            if let Item::Question(q) = item {
                if q.request == request && q.waiting() {
                    q.closed = why.to_string();
                }
            }
        }
        back_to_work(agent, now);
        self.mark(i);
    }
}

/// One row of the Agents list: an agent, or one run of a mind's chat (`agent#n`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RowKey {
    pub agent: AgentId,
    pub run: Option<u64>,
}

impl RowKey {
    pub fn agent(id: &AgentId) -> RowKey {
        RowKey { agent: id.clone(), run: None }
    }

    pub fn run(id: &AgentId, n: u64) -> RowKey {
        RowKey { agent: id.clone(), run: Some(n) }
    }

    /// The row's id on screen: the agent's id, or `agent#n` for a run.
    pub fn id(&self) -> String {
        match self.run {
            Some(n) => format!("{}#{n}", self.agent.0),
            None => self.agent.0.clone(),
        }
    }

    /// A row id back into its key.
    pub fn parse(id: &str) -> RowKey {
        match id.rsplit_once('#').and_then(|(a, n)| n.parse().ok().map(|n| (a, n))) {
            Some((agent, n)) => RowKey { agent: AgentId(agent.to_string()), run: Some(n) },
            None => RowKey { agent: AgentId(id.to_string()), run: None },
        }
    }
}

/// Whether the person still owes this agent an answer: an approval, a job at its prompt (#182),
/// or a question it asked.
pub(super) fn waits_on_person(agent: &Agent) -> bool {
    !agent.pending_approvals.is_empty() || agent.job_waits || agent.waiting_questions().next().is_some()
}

/// Back to work once nothing holds the row on the person.
fn back_to_work(agent: &mut Agent, now: u64) {
    if agent.state == State::WaitingForYou && !waits_on_person(agent) {
        let next = if agent.cards().any(Card::running) {
            State::RunningTool
        } else if agent.open_turn().is_some() {
            State::Thinking
        } else {
            State::Idle
        };
        set_state(agent, next, now);
    }
}

impl Store {
    // ── Reading ─────────────────────────────────────────────────────

    /// How many agents each tab lists, in [`Tab::EVERY`] order.
    pub fn counts(&self) -> [usize; 4] {
        Tab::EVERY.map(|tab| self.rows(tab, None).len())
    }

    /// The Agents list's rows under `tab`, in the order to draw them.
    ///
    /// Chat and agents are different things (Pranab, 27 September). A mind's main conversation
    /// is the person's chat with it, so it is not one agent row: each of its requests that did
    /// work (a call, a question, an approval) is its own run row, "from chat", and plain
    /// conversation stays with the chat. An agent started as one (New agent, a recipe, a
    /// hand-off) is a row as before. Each row is judged by its own state: a run that finished is
    /// Complete even while its mind works on the next.
    ///
    /// Newest first; under Active, what needs the person first. While the pointer is over the list
    /// `hold` is the order on screen, kept, with rows that left the tab taken out and new ones
    /// added at the bottom (design decision 4).
    pub fn rows(&self, tab: Tab, hold: Option<&[RowKey]>) -> Vec<RowKey> {
        let mut all: Vec<(RowKey, u64, bool)> = Vec::new(); // (row, started, needs the person)
        for a in &self.agents {
            if a.is_plain_main() {
                for t in a.turns.iter().filter(|t| a.is_run(t)) {
                    let open = t.open();
                    let needs = open && (a.state == State::WaitingForYou || self.stuck(a));
                    let held = match tab {
                        Tab::Active => open,
                        Tab::NeedsYou => needs,
                        Tab::Complete => !open,
                        Tab::All | Tab::Tasks => true,
                    };
                    if held {
                        all.push((RowKey::run(&a.meta.id, t.n), t.started, needs));
                    }
                }
            } else if self.in_tab(tab, a) {
                let needs = a.state == State::WaitingForYou || self.stuck(a);
                all.push((RowKey::agent(&a.meta.id), a.meta.started, needs));
            }
        }
        all.sort_by(|x, y| y.1.cmp(&x.1));
        if tab == Tab::Active {
            all.sort_by_key(|(_, _, needs)| !needs);
        }
        let wanted: Vec<RowKey> = all.into_iter().map(|(k, _, _)| k).collect();
        match hold {
            None => wanted,
            Some(on_screen) => {
                let mut kept: Vec<RowKey> = on_screen.iter().filter(|k| wanted.contains(k)).cloned().collect();
                for k in wanted {
                    if !kept.contains(&k) {
                        kept.push(k);
                    }
                }
                kept
            }
        }
    }

    /// Whether `a` is listed under `tab`. Needs you holds what waits on the person *and* what is
    /// stuck (#234): a stuck task is still thinking or running a tool by its state, but it is the
    /// person it now needs, and it was found nowhere they look for that.
    fn in_tab(&self, tab: Tab, a: &Agent) -> bool {
        tab.holds(a.state) || (tab == Tab::NeedsYou && self.stuck(a))
    }

    /// Whether the shell judges `a` stuck now (`progress::of`, on this store's clock).
    fn stuck(&self, a: &Agent) -> bool {
        super::progress::of(a, self.now()).is_some_and(|p| p.stuck.is_some())
    }


    /// The Tasks tab's rows (#234): every request a person or an agent made, newest first, as
    /// `(agent, turn)`, at most `limit`. A turn with no prompt is the shell's own account of
    /// something outside any request, and is not a task.
    pub fn tasks(&self, limit: usize) -> Vec<(AgentId, u64)> {
        let mut all: Vec<(&Agent, &Turn)> = self
            .agents
            .iter()
            .flat_map(|a| a.turns.iter().filter(|t| !t.prompt.trim().is_empty() && a.is_run(t)).map(move |t| (a, t)))
            .collect();
        all.sort_by(|(a1, t1), (a2, t2)| t2.started.cmp(&t1.started).then(a2.seq.cmp(&a1.seq)).then(t2.n.cmp(&t1.n)));
        all.into_iter().take(limit).map(|(a, t)| (a.meta.id.clone(), t.n)).collect()
    }

    /// The rows of one tab, in the order to draw them.
    ///
    /// Newest first. Under Active, the rows waiting on the person come first — except while the
    /// pointer is over the list: then `hold` is the order on screen, and it is kept, with rows that
    /// left the tab taken out and new ones added at the bottom. A row must not move under a
    /// pointer that is about to click it (design decision 4).
    pub fn list(&self, tab: Tab, hold: Option<&[AgentId]>) -> Vec<AgentId> {
        let mut rows: Vec<&Agent> = self.agents.iter().filter(|a| self.in_tab(tab, a)).collect();
        rows.sort_by(|a, b| b.meta.started.cmp(&a.meta.started).then(b.seq.cmp(&a.seq)));
        if tab == Tab::Active {
            // What needs the person first: waiting on them, or stuck.
            rows.sort_by_key(|a| a.state != State::WaitingForYou && !self.stuck(a));
        }
        let wanted: Vec<AgentId> = rows.iter().map(|a| a.meta.id.clone()).collect();
        match hold {
            None => wanted,
            Some(on_screen) => {
                let mut kept: Vec<AgentId> = on_screen.iter().filter(|id| wanted.contains(id)).cloned().collect();
                for id in wanted {
                    if !kept.contains(&id) {
                        kept.push(id);
                    }
                }
                kept
            }
        }
    }

    /// How many agents are working right now, for the cap.
    pub fn working(&self) -> usize {
        self.agents.iter().filter(|a| a.busy()).count()
    }

    /// The details column for one agent. Commands, files and approvals are counted from what the
    /// shell itself saw; calls include what the harness reported.
    pub fn details(&self, id: &AgentId) -> Option<Details> {
        let agent = self.agent(id)?;
        let mut details = Details {
            turns: agent.turns.len(),
            approvals_asked: agent.approvals_asked,
            approvals_answered: agent.approvals_answered,
            usage: agent.usage.clone(),
            refused: agent.refused,
            // As shown: a line holding words the person had erased is drawn masked.
            refusals: agent.shown_refusals().to_vec(),
            ..Details::default()
        };
        for card in agent.cards() {
            let card = card.shown();
            details.calls += 1;
            if card.state == CallState::Failed {
                details.failed_calls += 1;
            }
            if card.provenance != Provenance::Verified {
                continue;
            }
            if card.is_command() {
                details.commands.push((card.command_line(), card.exit_code, card.state));
            }
            for path in card.paths() {
                if !details.files.contains(&path) {
                    details.files.push(path);
                }
            }
        }
        Some(details)
    }

    /// The agents `parent` started (`shell.new_agent`), oldest first.
    /// The agents a recipe run started (its Agent steps), oldest first.
    pub fn agents_of_recipe(&self, recipe_id: &str) -> Vec<AgentId> {
        let mut theirs: Vec<&Agent> =
            self.agents.iter().filter(|a| a.meta.recipe.as_ref().is_some_and(|r| r.id == recipe_id)).collect();
        theirs.sort_by_key(|a| (a.meta.started, a.seq));
        theirs.into_iter().map(|a| a.meta.id.clone()).collect()
    }

    pub fn children_of(&self, parent: &AgentId) -> Vec<AgentId> {
        let mut children: Vec<&Agent> =
            self.agents.iter().filter(|a| a.meta.parent.as_ref() == Some(parent)).collect();
        children.sort_by_key(|a| (a.meta.started, a.seq));
        children.into_iter().map(|a| a.meta.id.clone()).collect()
    }

    /// An agent's last `last` turns as plain text — what `read_agent` answers with: the prompts,
    /// the mind's text, each card on one line with how it went and the end of its output, each
    /// approval with how it came out, and the shell's notes. Bounded at [`TRANSCRIPT_BYTES`],
    /// oldest cut first, so what is kept is the newest.
    pub fn transcript(&self, id: &AgentId, last: usize) -> Option<String> {
        let agent = self.agent(id)?;
        let mut out: Vec<String> = Vec::new();
        let status = if agent.status.is_empty() { String::new() } else { format!(" — {}", agent.status) };
        let from = agent.turns.len().saturating_sub(last.max(1));
        out.push(format!(
            "{} · {} · \"{}\" · {}{status}. {} turn{} in all; the last {} here.",
            agent.meta.id,
            agent.meta.mind,
            agent.meta.title,
            agent.state.label(),
            agent.turns.len(),
            if agent.turns.len() == 1 { "" } else { "s" },
            agent.turns.len() - from,
        ));
        for turn in &agent.turns[from..] {
            out.push(String::new());
            if turn.prompt.is_empty() {
                out.push(format!("── turn {} ──", turn.n));
            } else {
                out.push(format!("── turn {} ── asked: {}", turn.n, clip_text(&turn.prompt, TRANSCRIPT_TEXT)));
            }
            for item in &turn.items {
                match item {
                    Item::Text(text) => {
                        let text = text.last(TRANSCRIPT_TEXT);
                        let text = text.trim();
                        if !text.is_empty() {
                            out.push(text.to_string());
                        }
                    }
                    // The mind's reasoning is folded on screen and left out here: it is not what
                    // the agent said, and a reader asking for the session wants the session.
                    Item::Thinking(_) => {}
                    Item::Note(note) => out.push(format!("(the desktop: {note})")),
                    Item::Question(q) => out.push(match (q.answer.as_str(), q.closed.as_str()) {
                        ("", "") => format!("[asked the person] {} — waiting for an answer", q.prompt),
                        ("", why) => format!("[asked the person] {} — not answered: {why}", q.prompt),
                        (answer, _) => format!("[asked the person] {} — answered: {answer}", q.prompt),
                    }),
                    Item::Approval(a) => out.push(match a.outcome {
                        ApprovalOutcome::Pending => format!("[asked the person] {} — waiting for an answer", a.what.as_str()),
                        _ => format!("[asked the person] {} — {}", a.what.as_str(), a.record.as_str()),
                    }),
                    Item::Card(card) => {
                        let card = card.shown();
                        let how = match (card.state, card.exit_code) {
                            (CallState::Running, _) => "running".to_string(),
                            (_, Some(code)) => format!("{} · exit {code}", card.state.key()),
                            (state, None) => state.key().to_string(),
                        };
                        out.push(format!(
                            "[{} call] {} — {how}",
                            card.provenance.key(),
                            clip_text(&card.as_call().summary(), TRANSCRIPT_TEXT)
                        ));
                        let tail = card.output.tail_lines(TRANSCRIPT_OUTPUT_LINES);
                        for line in tail.lines() {
                            out.push(format!("    {line}"));
                        }
                    }
                }
            }
            match (turn.ended, turn.ok) {
                (None, _) => out.push("(still working)".to_string()),
                (Some(_), Some(false)) => out.push("(this turn did not finish)".to_string()),
                _ => {}
            }
        }
        let mut text = out.join("\n");
        if text.len() > TRANSCRIPT_BYTES {
            let mut from = text.len() - TRANSCRIPT_BYTES;
            while !text.is_char_boundary(from) {
                from += 1;
            }
            text = format!("… (the start is cut; this is the newest {} of it)\n{}", bytes(TRANSCRIPT_BYTES as u64), &text[from..]);
        }
        Some(text)
    }

    // ── On disk ───────────────────────────────────────────────────────

    /// The agents whose files are out of date, as `(file, contents)`, and the files of agents that
    /// were removed. Clears both lists; the caller writes outside the lock.
    pub fn take_dirty(&mut self, dir: &Path) -> (Vec<(PathBuf, String)>, Vec<PathBuf>) {
        let dirty = std::mem::take(&mut self.dirty);
        let removed = std::mem::take(&mut self.removed);
        let writes = dirty
            .iter()
            .filter_map(|id| self.agent(id))
            .map(|agent| (file_for(dir, &agent.meta.id), serialize(agent)))
            .collect();
        let mut deletes: Vec<PathBuf> = std::mem::take(&mut self.pending_deletes);
        deletes.extend(removed.iter().map(|id| file_for(dir, id)));
        // A file an agent is held under again is written, not deleted.
        deletes.retain(|path| !self.agents.iter().any(|a| file_for(dir, &a.meta.id) == *path));
        (writes, deletes)
    }

    /// One agent's file and what it should hold now, for a write that cannot wait for the timer.
    pub fn file_of(&self, dir: &Path, id: &AgentId) -> Option<(PathBuf, String)> {
        self.agent(id).map(|agent| (file_for(dir, id), serialize(agent)))
    }

    /// Every agent is written again on the next save: what an older save had taken is stale.
    /// `removed` are the files that save would have deleted; each is deleted on the next save
    /// instead, unless an agent of that id is held again by then.
    pub fn mark_all_dirty(&mut self, removed: &[PathBuf], dir: &Path) {
        let ids: Vec<AgentId> = self.agents.iter().map(|a| a.meta.id.clone()).collect();
        self.dirty.extend(ids);
        self.pending_deletes.extend(removed.iter().filter(|path| !self.agents.iter().any(|a| file_for(dir, &a.meta.id) == **path)).cloned());
    }

    /// Write every change now. For tests and for shutdown; the shell's timer uses `take_dirty`.
    pub fn save(&mut self, dir: &Path) -> std::io::Result<()> {
        let (writes, deletes) = self.take_dirty(dir);
        write_all(dir, &writes, &deletes)
    }

    /// The agents kept in `dir`, the newest [`KEEP_AGENTS`] of them. Older files are deleted, and a
    /// file that cannot be read is left alone and logged. A turn that was open when the shell
    /// stopped is closed, and its open calls interrupted: nothing is still running them.
    pub fn load(dir: &Path, clock: Box<dyn Fn() -> u64 + Send>) -> Store {
        let mut store = Store::with_clock(clock);
        let now = store.now();
        let Ok(entries) = std::fs::read_dir(dir) else { return store };
        let mut found: Vec<(PathBuf, Agent)> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            match std::fs::read_to_string(&path).ok().and_then(|text| parse(&text, now)) {
                Some(agent) => found.push((path, agent)),
                None => tracing::warn!(path = %path.display(), "An agent's saved session could not be read; left as it is"),
            }
        }
        found.sort_by(|(_, a), (_, b)| b.touched.cmp(&a.touched));
        for (path, _) in found.iter().skip(KEEP_AGENTS) {
            let _ = std::fs::remove_file(path);
        }
        found.truncate(KEEP_AGENTS);
        // Oldest first, so creation order survives the round trip.
        found.reverse();
        for (_, mut agent) in found {
            agent.seq = store.next_seq;
            store.next_seq += 1;
            store.agents.push(agent);
        }
        store.revision = 1;
        store
    }

    /// Keep the list bounded: past [`KEEP_AGENTS`], the agents untouched longest and doing nothing
    /// are let go, files and all.
    fn let_old_ones_go(&mut self) {
        while self.agents.len() > self.keep {
            let oldest = self
                .agents
                .iter()
                .enumerate()
                .filter(|(_, a)| !a.busy())
                .min_by_key(|(_, a)| a.touched)
                .map(|(i, _)| i);
            let Some(i) = oldest else { break };
            let id = self.agents[i].meta.id.clone();
            self.remove_agent(&id);
        }
    }
}

// ── The rules, on one agent ─────────────────────────────────────────

fn set_state(agent: &mut Agent, state: State, now: u64) {
    if agent.state != state {
        agent.state = state;
        agent.since = now;
    }
}

/// The line a settled approval leaves when the approval store had none to give.
fn settled_line(outcome: ApprovalOutcome, at: u64) -> String {
    let when = hhmm(at);
    match outcome {
        ApprovalOutcome::Pending => String::new(),
        ApprovalOutcome::Allowed => format!("Allowed — {when}"),
        ApprovalOutcome::Denied => format!("Denied — {when}"),
        ApprovalOutcome::Expired => format!("Not answered in time — {when}"),
        ApprovalOutcome::Withdrawn => format!("Withdrawn by the desktop — {when}"),
    }
}

/// Local `HH:MM` for a unix time.
fn hhmm(unix: u64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(unix as i64, 0)
        .single()
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_default()
}

/// `text` trimmed, and at most its first `max` bytes, cut on a character and marked when cut.
fn clip_text(text: &str, max: usize) -> String {
    let flat = text.trim();
    if flat.len() <= max {
        return flat.to_string();
    }
    let mut end = max;
    while !flat.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &flat[..end])
}

/// At most the last `max` bytes of `text`, cut on a character.
fn tail_of(text: &str, max: usize) -> String {
    let mut from = text.len().saturating_sub(max);
    while !text.is_char_boundary(from) {
        from += 1;
    }
    text[from..].to_string()
}

/// One event's text, cut at [`EVENT_CAP`] on a character.
fn cap(text: &str) -> &str {
    if text.len() <= EVENT_CAP {
        return text;
    }
    let mut end = EVENT_CAP;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Add text or thinking to the end of a turn: onto the block already there, or as a new one.
/// Whitespace alone does not begin a block — the blank line after a trail line is not something
/// the mind said.
fn append(items: &mut Vec<Item>, delta: &str, thinking: bool) {
    match items.last_mut() {
        Some(Item::Text(buffer)) if !thinking => buffer.push(delta.as_bytes()),
        Some(Item::Thinking(buffer)) if thinking => buffer.push(delta.as_bytes()),
        _ if delta.trim().is_empty() => {}
        _ => {
            let mut buffer = Capped::new(TEXT_HEAD, TEXT_CAP);
            buffer.push(delta.as_bytes());
            items.push(if thinking { Item::Thinking(buffer) } else { Item::Text(buffer) });
        }
    }
}

/// End the open turn: reported calls still open are interrupted; the shell's own keep running.
/// Its questions still waiting are closed: the host expires a run's requests when it ends, so an
/// answer has nowhere to go. Left open, the card kept its buttons and a click was refused (VM 520,
/// 4 October: two identical Erase cards, one of them dead, and no telling which).
fn settle_turn(agent: &mut Agent, ok: bool, now: u64, note: Option<&str>) {
    let Some(turn) = agent.turns.last_mut().filter(|t| t.open()) else { return };
    for card in turn.cards_mut() {
        if card.running() && card.provenance == Provenance::Reported {
            card.state = CallState::Interrupted;
            card.ended = Some(now);
            card.output.settle();
        }
    }
    for item in &mut turn.items {
        if let Item::Question(q) = item {
            if q.waiting() {
                q.closed = Question::STOPPED_ASKING.to_string();
            }
        }
    }
    if let Some(note) = note {
        turn.items.push(Item::Note(note.to_string()));
    }
    turn.ended = Some(now);
    turn.ok = Some(ok);
}

/// Where a verified event, or a note, goes: the open turn; else a turn of its own with no prompt,
/// already ended — the shell can have something to say outside any turn. Not the last closed turn:
/// filed there, an approval or a command that came after a run had ended was listed among that
/// run's changes (VM 520, 4 October: the notes task showed arena saves and old approval cards).
/// One such turn is kept at the end and reused, so a quiet stretch does not grow the history.
fn turn_for_verified(agent: &mut Agent, now: u64) -> &mut Turn {
    let outside = |t: &Turn| !t.open() && t.prompt.is_empty();
    let fits = agent.turns.last().is_some_and(|t| t.open() || outside(t));
    if !fits {
        let n = agent.next_turn;
        agent.next_turn += 1;
        agent.turns.push(Turn {
            n,
            prompt: String::new(),
            started: now,
            ended: Some(now),
            ok: None,
            lost: false,
            origin: TurnOrigin::Agent,
            items: Vec::new(),
            events: false,
            trail_seq: 0,
        });
    }
    agent.turns.last_mut().expect("a turn was just made")
}

/// The card an event names: in the open turn, or — for the shell's own commands, which outlive
/// their turn — anywhere in the session. Never across provenance: a harness cannot write into a
/// card the shell owns by guessing its id, and the other way round.
fn find_card<'a>(agent: &'a mut Agent, call: &str, provenance: Provenance) -> Option<&'a mut Card> {
    let open = agent.turns.last().is_some_and(Turn::open);
    let turns: Vec<usize> = match (provenance, open) {
        (Provenance::Reported, true) => vec![agent.turns.len() - 1],
        (Provenance::Reported, false) => Vec::new(),
        (Provenance::Verified, _) => (0..agent.turns.len()).rev().collect(),
    };
    let (t, at) = turns.into_iter().find_map(|t| {
        agent.turns[t]
            .items
            .iter()
            .rposition(|i| matches!(i, Item::Card(c) if c.call == call && c.provenance == provenance))
            .map(|at| (t, at))
    })?;
    match &mut agent.turns[t].items[at] {
        Item::Card(card) => Some(card),
        _ => None,
    }
}

/// Count a refusal and keep its line: what arrived, and why the lifecycle would not take it
/// (#212). The count never lies; the newest [`REFUSED_LINES`] lines are what the details column
/// can open into.
fn refuse(agent: &mut Agent, line: String) {
    agent.refused += 1;
    if let Some(shown) = agent.refusals_shown.as_mut() {
        shown.push(line.clone());
        if shown.len() > REFUSED_LINES {
            shown.remove(0);
        }
    }
    agent.refusals.push(line);
    if agent.refusals.len() > REFUSED_LINES {
        agent.refusals.remove(0);
    }
}

/// What an event is, in a few words, for a refusal line.
fn what_event(event: &Event) -> String {
    match event {
        Event::ToolStart { call, name, .. } => format!("a start for `{name}` (`{call}`)"),
        Event::ToolOutput { call, .. } => format!("output for `{call}`"),
        Event::ToolEnd { call, .. } => format!("an end for `{call}`"),
        Event::Thinking { .. } => "some thinking".to_string(),
        Event::Status { text } => format!("a status line ({})", clip_text(text, 40)),
        Event::Usage { .. } => "a usage report".to_string(),
        Event::Request { prompt, .. } => format!("a question ({})", clip_text(prompt, 40)),
        Event::Redact { .. } => "a redaction".to_string(),
    }
}

/// Apply one event. `Err` is the refusal line: what arrived, and why the lifecycle would not
/// take it.
fn apply(agent: &mut Agent, event: &Event, provenance: Provenance, now: u64) -> Result<(), String> {
    // A harness speaks only inside a turn.
    if provenance == Provenance::Reported {
        match agent.turns.last_mut().filter(|t| t.open()) {
            Some(turn) => turn.events = true,
            None => return Err(format!("{} — no turn was open", what_event(event))),
        }
    }
    match event {
        Event::ToolStart { call, name, target, args } => {
            if find_card(agent, call, provenance).is_some_and(|c| c.running() || provenance == Provenance::Reported) {
                return Err(format!("a second start for `{call}`"));
            }
            let card = Card::new(call, name, target, args.clone(), provenance, now);
            let turn = turn_for(agent, provenance, now);
            // `turn.tool_start` writes the call's trail line into the text just before the event
            // (docs/harness.md). That line has already become a card; the event is the same call,
            // told properly, and takes its place.
            if provenance == Provenance::Reported {
                if let Some(Item::Card(said)) = turn.items.last() {
                    if said.state == CallState::Untold && said.call.starts_with("trail-") && said.name == *name {
                        turn.items.pop();
                    }
                }
            }
            turn.items.push(Item::Card(card));
            // A status line is what the mind says between calls; the call now running says more.
            // A question it asked keeps its line, because that is what the person must answer.
            if agent.state != State::WaitingForYou {
                agent.status.clear();
            }
            if !matches!(agent.state, State::WaitingForYou | State::HarnessGone) {
                set_state(agent, State::RunningTool, now);
            }
        }
        Event::ToolOutput { call, stream, delta } => {
            let delta = cap(delta);
            match find_card(agent, call, provenance) {
                Some(card) if card.running() => card.push_output(*stream, delta.as_bytes()),
                Some(_) => return Err(format!("output for `{call}` — its call had already ended")),
                None => {
                    let mut card = Card::new(call, "(unknown call)", call, serde_json::Value::Null, provenance, now);
                    card.mark = Some(Mark::OutputWithoutStart);
                    card.output.push(*stream, delta.as_bytes());
                    turn_for(agent, provenance, now).items.push(Item::Card(card));
                }
            }
        }
        Event::ToolEnd { call, ok, summary, exit_code } => {
            let settle = |card: &mut Card| {
                card.state = if *ok { CallState::Ok } else { CallState::Failed };
                card.summary = summary.clone();
                card.exit_code = *exit_code;
                card.ended = Some(now);
                card.output.settle();
            };
            match find_card(agent, call, provenance) {
                Some(card) if card.running() => settle(card),
                Some(_) => return Err(format!("a second end for `{call}`")),
                None => {
                    let mut card = Card::new(call, "(unknown call)", call, serde_json::Value::Null, provenance, now);
                    card.mark = Some(Mark::EndWithoutStart);
                    settle(&mut card);
                    turn_for(agent, provenance, now).items.push(Item::Card(card));
                }
            }
            if agent.state == State::RunningTool && !agent.cards().any(Card::running) {
                let next = match agent.open_turn() {
                    Some(_) => State::Thinking,
                    None if agent.turns.last().and_then(|t| t.ok) == Some(false) => State::Failed,
                    None => State::Done,
                };
                set_state(agent, next, now);
            }
            compact(agent);
        }
        Event::Thinking { delta } => {
            let Some(turn) = agent.turns.last_mut().filter(|t| t.open()) else {
                return Err("some thinking — no turn was open".to_string());
            };
            append(&mut turn.items, cap(delta), true);
        }
        Event::Status { text } => agent.status = cap(text).to_string(),
        // The agent asked the person something (#25): it waits, and says what it asked. The
        // question itself is kept by the host's run store, which alone takes the answer.
        Event::Request { request_id, prompt, options } => {
            let turn = turn_for(agent, provenance, now);
            let known = turn.items.iter().any(|i| matches!(i, Item::Question(q) if q.request == *request_id));
            if !known {
                turn.items.push(Item::Question(Question {
                    request: request_id.clone(),
                    prompt: prompt.clone(),
                    options: options.clone(),
                    answer: String::new(),
                    closed: String::new(),
                    asked: now,
                }));
            }
            agent.status = cap(&format!("asks: {prompt}")).to_string();
            set_state(agent, State::WaitingForYou, now);
        }
        // The host applies an erasure itself, through `Agents::redact`, and only once its rule
        // held; one arriving here, from any feeder, is not one.
        Event::Redact { .. } => return Err("a redaction — only the host applies one".to_string()),
        Event::Usage { model, input_tokens, output_tokens, cost_usd } => {
            let usage = &mut agent.usage;
            usage.reported = true;
            usage.input_tokens += input_tokens.unwrap_or(0);
            usage.output_tokens += output_tokens.unwrap_or(0);
            usage.cost_usd += cost_usd.unwrap_or(0.0);
            if !model.is_empty() {
                usage.model = model.clone();
                if agent.meta.model.is_empty() {
                    agent.meta.model = model.clone();
                }
            }
        }
    }
    Ok(())
}

/// The turn an event's card goes in.
fn turn_for(agent: &mut Agent, provenance: Provenance, now: u64) -> &mut Turn {
    match provenance {
        // `apply` has already checked that a turn is open.
        Provenance::Reported => agent.turns.last_mut().expect("a reported event has an open turn"),
        Provenance::Verified => turn_for_verified(agent, now),
    }
}

/// Keep the whole output of the newest [`FULL_OUTPUTS`] ended cards; trim older ones to a tail.
fn compact(agent: &mut Agent) {
    let mut seen = 0;
    for turn in agent.turns.iter_mut().rev() {
        for card in turn.cards_mut().collect::<Vec<_>>().into_iter().rev() {
            if card.running() {
                continue;
            }
            seen += 1;
            if seen > FULL_OUTPUTS {
                card.output.trim_to(OLD_OUTPUT);
            }
        }
    }
}

// ── The file format ─────────────────────────────────────────────────
//
// One file per agent, `<id>.jsonl`: a line for the agent, then a line per turn. Rewritten whole
// when it changes — a turn is small once its outputs are cut to [`PERSIST_BYTES`].

/// Where one agent's session is kept. The id is `<harness>:<conversation>`; anything in it that
/// could leave the directory is replaced.
pub fn file_for(dir: &Path, id: &AgentId) -> PathBuf {
    let safe: String = id
        .0
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_' | '.') { c } else { '_' })
        .collect();
    let safe = safe.trim_start_matches('.').to_string();
    dir.join(format!("{safe}.jsonl"))
}

pub fn write_all(dir: &Path, writes: &[(PathBuf, String)], deletes: &[PathBuf]) -> std::io::Result<()> {
    if !writes.is_empty() {
        std::fs::create_dir_all(dir)?;
    }
    for (path, contents) in writes {
        // Written beside and renamed over, so a crash mid-write leaves the old session, not half
        // of a new one.
        let partial = path.with_extension("jsonl.partial");
        std::fs::write(&partial, contents)?;
        std::fs::rename(&partial, path)?;
    }
    for path in deletes {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

/// Rewrite one session so that, once this returns, the disk holds the new contents and not the
/// old: written beside, flushed, renamed over, and the directory flushed so the rename is kept.
pub fn write_durably(dir: &Path, path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    let partial = path.with_extension("jsonl.partial");
    {
        let mut file = std::fs::File::create(&partial)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&partial, path)?;
    std::fs::File::open(dir)?.sync_all()
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Line {
    Agent(AgentRecord),
    Turn(TurnRecord),
}

#[derive(Serialize, Deserialize)]
struct AgentRecord {
    id: AgentId,
    mind: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    parent: Option<AgentId>,
    started: u64,
    #[serde(default)]
    conversations: bool,
    #[serde(default)]
    role: Option<RoleMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recipe: Option<RecipeOrigin>,
    state: State,
    since: u64,
    #[serde(default)]
    status: String,
    #[serde(default)]
    usage: UsageRecord,
    #[serde(default)]
    refused: u32,
    #[serde(default)]
    refusals: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refusals_shown: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    erasures: Vec<Erasure>,
    #[serde(default)]
    approvals_asked: u32,
    #[serde(default)]
    approvals_answered: u32,
    touched: u64,
    next_turn: u64,
}

#[derive(Serialize, Deserialize, Default)]
struct UsageRecord {
    #[serde(default)]
    model: String,
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cost_usd: f64,
    #[serde(default)]
    reported: bool,
}

#[derive(Serialize, Deserialize)]
struct TurnRecord {
    n: u64,
    prompt: String,
    started: u64,
    ended: Option<u64>,
    ok: Option<bool>,
    #[serde(default)]
    events: bool,
    #[serde(default)]
    lost: bool,
    #[serde(default)]
    origin: TurnOrigin,
    items: Vec<ItemRecord>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ItemRecord {
    Text(String),
    Thinking(String),
    Note(String),
    Card(CardRecord),
    Approval(ApprovalRecord),
    Question(QuestionRecord),
}

#[derive(Serialize, Deserialize)]
struct QuestionRecord {
    request: String,
    prompt: String,
    #[serde(default)]
    options: Vec<String>,
    #[serde(default)]
    answer: String,
    #[serde(default)]
    closed: String,
    asked: u64,
}

#[derive(Serialize, Deserialize)]
struct ApprovalRecord {
    request: String,
    what: String,
    outcome: ApprovalOutcome,
    #[serde(default)]
    record: String,
    asked: u64,
    settled: Option<u64>,
}

#[derive(Serialize, Deserialize)]
struct CardMaskRecord {
    request: String,
    #[serde(default)]
    target: String,
    #[serde(default)]
    args: serde_json::Value,
    #[serde(default)]
    preview: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    output: String,
}

#[derive(Serialize, Deserialize)]
struct CardRecord {
    call: String,
    name: String,
    #[serde(default)]
    target: String,
    #[serde(default)]
    args: serde_json::Value,
    #[serde(default)]
    preview: String,
    #[serde(default)]
    repeats: u32,
    state: CallState,
    #[serde(default)]
    summary: String,
    exit_code: Option<i32>,
    provenance: Provenance,
    mark: Option<Mark>,
    #[serde(default)]
    output_kind: OutputKind,
    #[serde(default)]
    output: String,
    #[serde(default)]
    output_total: u64,
    #[serde(default)]
    output_lines: u64,
    started: u64,
    ended: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mask: Option<CardMaskRecord>,
}

fn serialize(agent: &Agent) -> String {
    let meta = &agent.meta;
    let head = Line::Agent(AgentRecord {
        id: meta.id.clone(),
        mind: meta.mind.clone(),
        model: meta.model.clone(),
        title: meta.title.clone(),
        parent: meta.parent.clone(),
        started: meta.started,
        conversations: meta.conversations,
        role: meta.role.clone(),
        recipe: meta.recipe.clone(),
        state: agent.state,
        since: agent.since,
        status: agent.status.clone(),
        usage: UsageRecord {
            model: agent.usage.model.clone(),
            input_tokens: agent.usage.input_tokens,
            output_tokens: agent.usage.output_tokens,
            cost_usd: agent.usage.cost_usd,
            reported: agent.usage.reported,
        },
        refused: agent.refused,
        refusals: agent.refusals.clone(),
        refusals_shown: agent.refusals_shown.clone(),
        erasures: agent.erasures.clone(),
        approvals_asked: agent.approvals_asked,
        approvals_answered: agent.approvals_answered,
        touched: agent.touched,
        next_turn: agent.next_turn,
    });
    let mut lines = vec![serde_json::to_string(&head).unwrap_or_default()];
    let from = agent.turns.len().saturating_sub(PERSIST_TURNS);
    for turn in &agent.turns[from..] {
        let items = turn
            .items
            .iter()
            .map(|item| match item {
                Item::Text(t) => ItemRecord::Text(t.last(PERSIST_BYTES)),
                Item::Thinking(t) => ItemRecord::Thinking(t.last(PERSIST_BYTES)),
                Item::Note(n) => ItemRecord::Note(n.clone()),
                Item::Question(q) => ItemRecord::Question(QuestionRecord {
                    request: q.request.clone(),
                    prompt: q.prompt.clone(),
                    options: q.options.clone(),
                    answer: q.answer.clone(),
                    closed: q.closed.clone(),
                    asked: q.asked,
                }),
                Item::Approval(a) => ItemRecord::Approval(ApprovalRecord {
                    request: a.request.clone(),
                    what: a.what.clone(),
                    outcome: a.outcome,
                    record: a.record.clone(),
                    asked: a.asked,
                    settled: a.settled,
                }),
                Item::Card(c) => ItemRecord::Card(CardRecord {
                    call: c.call.clone(),
                    name: c.name.clone(),
                    target: c.target.clone(),
                    args: c.args.clone(),
                    preview: c.preview.clone(),
                    repeats: c.repeats,
                    state: c.state,
                    summary: c.summary.clone(),
                    exit_code: c.exit_code,
                    provenance: c.provenance,
                    mark: c.mark,
                    output_kind: c.output.kind,
                    output: c.output.bytes.last(PERSIST_BYTES),
                    output_total: c.output.bytes.total(),
                    output_lines: c.output.lines(),
                    started: c.started,
                    ended: c.ended,
                    mask: c.mask.as_ref().map(|m| CardMaskRecord {
                        request: m.request.clone(),
                        target: m.target.clone(),
                        args: m.args.clone(),
                        preview: m.preview.clone(),
                        summary: m.summary.clone(),
                        output: tail_of(&m.output, PERSIST_BYTES),
                    }),
                }),
            })
            .collect();
        let line = Line::Turn(TurnRecord {
            n: turn.n,
            prompt: turn.prompt.clone(),
            started: turn.started,
            ended: turn.ended,
            ok: turn.ok,
            lost: turn.lost,
            origin: turn.origin,
            events: turn.events,
            items,
        });
        lines.push(serde_json::to_string(&line).unwrap_or_default());
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Read one agent back. `None` when the first line is not an agent: the file is not ours.
fn parse(text: &str, now: u64) -> Option<Agent> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let Line::Agent(record) = serde_json::from_str::<Line>(lines.next()?).ok()? else { return None };
    let mut agent = Agent {
        meta: AgentMeta {
            id: record.id,
            mind: record.mind,
            model: record.model,
            title: record.title,
            parent: record.parent,
            started: record.started,
            conversations: record.conversations,
            role: record.role,
            recipe: record.recipe,
        },
        state: record.state,
        since: record.since,
        status: record.status,
        turns: Vec::new(),
        usage: Usage {
            model: record.usage.model,
            input_tokens: record.usage.input_tokens,
            output_tokens: record.usage.output_tokens,
            cost_usd: record.usage.cost_usd,
            reported: record.usage.reported,
        },
        refused: record.refused,
        // Only a list that still pairs with the lines it masks.
        refusals_shown: record.refusals_shown.filter(|shown| shown.len() == record.refusals.len()),
        refusals: record.refusals,
        erasures: record.erasures,
        approvals_asked: record.approvals_asked,
        approvals_answered: record.approvals_answered,
        pending_approvals: Vec::new(),
        // A job that waited before a restart is not waiting now: nothing survived to hold the
        // prompt. The shell's next tick says otherwise if one somehow is.
        job_waits: false,
        seq: 0,
        touched: record.touched,
        next_turn: record.next_turn,
    };
    for line in lines {
        // One unreadable turn costs that turn, not the agent.
        let Ok(Line::Turn(turn)) = serde_json::from_str::<Line>(line) else { continue };
        let items = turn
            .items
            .into_iter()
            .map(|item| match item {
                ItemRecord::Text(t) => Item::Text(Capped::restore(TEXT_CAP, &t, t.len() as u64, 0)),
                ItemRecord::Thinking(t) => Item::Thinking(Capped::restore(TEXT_CAP, &t, t.len() as u64, 0)),
                ItemRecord::Note(n) => Item::Note(n),
                // Its run was orphaned when the desktop stopped, so nobody can answer it now.
                ItemRecord::Question(q) => Item::Question(Question {
                    closed: if q.answer.is_empty() && q.closed.is_empty() {
                        Question::RESTARTED.to_string()
                    } else {
                        q.closed
                    },
                    request: q.request,
                    prompt: q.prompt,
                    options: q.options,
                    answer: q.answer,
                    asked: q.asked,
                }),
                // A request still waiting when the shell stopped is gone with the shell: requests
                // are held in memory, so nobody can answer it now, and it is never drawn with
                // buttons again.
                ItemRecord::Approval(a) => Item::Approval(Approval {
                    outcome: if a.outcome == ApprovalOutcome::Pending { ApprovalOutcome::Withdrawn } else { a.outcome },
                    record: if a.outcome == ApprovalOutcome::Pending {
                        "Withdrawn — the desktop restarted while it waited".to_string()
                    } else {
                        a.record
                    },
                    settled: if a.outcome == ApprovalOutcome::Pending { Some(now) } else { a.settled },
                    request: a.request,
                    what: a.what,
                    asked: a.asked,
                }),
                ItemRecord::Card(c) => Item::Card(Card {
                    call: c.call,
                    name: c.name,
                    target: c.target,
                    args: c.args,
                    preview: c.preview,
                    repeats: c.repeats,
                    state: c.state,
                    summary: c.summary,
                    exit_code: c.exit_code,
                    provenance: c.provenance,
                    mark: c.mark,
                    output: Output::restore(c.output_kind, &c.output, c.output_total, c.output_lines),
                    started: c.started,
                    ended: c.ended,
                    mask: c.mask.map(|m| {
                        Box::new(CardMask {
                            request: m.request,
                            target: m.target,
                            args: m.args,
                            preview: m.preview,
                            summary: m.summary,
                            output: m.output,
                        })
                    }),
                }),
            })
            .collect();
        agent.turns.push(Turn {
            n: turn.n,
            prompt: turn.prompt,
            started: turn.started,
            ended: turn.ended,
            ok: turn.ok,
            lost: turn.lost,
            origin: turn.origin,
            items,
            events: turn.events,
            trail_seq: 0,
        });
    }
    // Whatever was running when the shell stopped is not running now: its process, its turn and
    // its harness's stream all went with the shell.
    let was_working = agent.state.working();
    for turn in &mut agent.turns {
        for card in turn.cards_mut() {
            if card.running() {
                card.state = CallState::Interrupted;
                card.ended = Some(now);
            }
        }
        if turn.open() {
            turn.items.push(Item::Note("The shell stopped while this turn was running.".into()));
            turn.ended = Some(now);
            turn.ok = Some(false);
            turn.lost = true;
        }
    }
    if was_working {
        agent.state = State::Idle;
        agent.since = now;
        agent.status = "interrupted when the shell stopped".into();
    }
    Some(agent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    /// A store whose clock the test moves.
    fn store() -> (Store, Arc<AtomicU64>) {
        let clock = Arc::new(AtomicU64::new(1_790_000_000));
        let reading = clock.clone();
        (Store::with_clock(Box::new(move || reading.load(Ordering::SeqCst))), clock)
    }

    fn id(s: &str) -> AgentId {
        AgentId(s.to_string())
    }

    fn start(call: &str, name: &str, args: serde_json::Value) -> Event {
        Event::ToolStart { call: call.into(), name: name.into(), target: String::new(), args }
    }

    fn output(call: &str, delta: &str) -> Event {
        Event::ToolOutput { call: call.into(), stream: Stream::Stdout, delta: delta.into() }
    }

    fn end(call: &str, ok: bool, exit: Option<i32>) -> Event {
        Event::ToolEnd { call: call.into(), ok, summary: String::new(), exit_code: exit }
    }

    fn cards(store: &Store, agent: &AgentId) -> Vec<(String, CallState, Option<Mark>, Provenance)> {
        store.agent(agent).unwrap().cards().map(|c| (c.call.clone(), c.state, c.mark, c.provenance)).collect()
    }

    #[test]
    fn a_turn_opens_names_the_agent_and_closes() {
        let (mut s, _) = store();
        let pi = id("pi:main");
        s.open_turn(&pi, "tidy the photos folder, dupes into Trash");
        let agent = s.agent(&pi).expect("a turn makes the agent known");
        assert_eq!(agent.meta.title, "tidy the photos folder, dupes into Trash");
        assert_eq!(agent.meta.mind, "pi", "named after its harness until someone says better");
        assert_eq!(agent.state, State::Thinking);
        s.text(&pi, "I'll find duplicates ");
        s.text(&pi, "by hash first.");
        s.close_turn(&pi, true);
        let agent = s.agent(&pi).unwrap();
        assert_eq!(agent.state, State::Done);
        assert!(matches!(&agent.turns[0].items[0], Item::Text(t) if t.text() == "I'll find duplicates by hash first."));
        // A second prompt continues the same agent and keeps its title.
        s.open_turn(&pi, "and the videos");
        assert_eq!(s.agent(&pi).unwrap().meta.title, "tidy the photos folder, dupes into Trash");
        assert_eq!(s.agent(&pi).unwrap().turns.len(), 2);
    }

    #[test]
    fn a_question_from_the_agent_makes_it_wait_and_says_what_it_asked() {
        let (mut s, _) = store();
        let pi = id("pi:main");
        s.open_turn(&pi, "clean up Downloads");
        let ask = Event::Request { request_id: "r1".into(), prompt: "Delete 3 old installers?".into(), options: vec!["Allow".into(), "Deny".into()] };
        s.event(&pi, &ask, Provenance::Reported);
        let agent = s.agent(&pi).unwrap();
        assert_eq!(agent.state, State::WaitingForYou);
        assert_eq!(agent.status, "asks: Delete 3 old installers?");
        assert!(agent.pending_approvals.is_empty(), "a question is not an OS approval and never goes to the gate");
    }

    #[test]
    fn a_status_replaces_the_last_one_and_a_tool_start_overrides_it() {
        let (mut s, _) = store();
        let pi = id("pi:main");
        s.open_turn(&pi, "look into it");
        let status = |t: &str| Event::Status { text: t.into() };
        s.event(&pi, &status("Grounding from memory…"), Provenance::Reported);
        assert_eq!(s.agent(&pi).unwrap().status, "Grounding from memory…");
        s.event(&pi, &status("Thinking… (30 s)"), Provenance::Reported);
        assert_eq!(s.agent(&pi).unwrap().status, "Thinking… (30 s)", "replaced, never appended");
        let start = Event::ToolStart { call: "c1".into(), name: "os_act".into(), target: String::new(), args: serde_json::Value::Null };
        s.event(&pi, &start, Provenance::Reported);
        assert_eq!(s.agent(&pi).unwrap().status, "", "a call in progress is the activity now");
    }

    #[test]
    fn a_call_opens_fills_and_settles_as_a_card() {
        let (mut s, _) = store();
        let pi = id("pi:main");
        s.open_turn(&pi, "list it");
        s.event(&pi, &start("t1", "bash", json!({"command": "ls"})), Provenance::Reported);
        assert_eq!(s.agent(&pi).unwrap().state, State::RunningTool);
        s.event(&pi, &output("t1", "a.jpg\nb.jpg\n"), Provenance::Reported);
        s.event(&pi, &end("t1", true, Some(0)), Provenance::Reported);
        let agent = s.agent(&pi).unwrap();
        assert_eq!(agent.state, State::Thinking, "the turn is still open");
        let card = agent.cards().next().unwrap();
        assert_eq!((card.state, card.exit_code), (CallState::Ok, Some(0)));
        assert_eq!(card.output.all(), "a.jpg\nb.jpg\n");
        assert_eq!(agent.refused, 0);
    }

    #[test]
    fn the_lifecycle_is_enforced_even_when_the_wire_is_not() {
        let (mut s, _) = store();
        let pi = id("pi:main");

        // Before any turn: a harness has nothing to report into.
        s.upsert_agent(AgentMeta::new(pi.clone(), "pi"));
        s.event(&pi, &start("early", "bash", json!({})), Provenance::Reported);
        assert!(cards(&s, &pi).is_empty());
        assert_eq!(s.agent(&pi).unwrap().refused, 1);

        s.open_turn(&pi, "go");
        // An end without a start is a card of its own, marked, and settles nothing else.
        s.event(&pi, &start("a", "bash", json!({})), Provenance::Reported);
        s.event(&pi, &end("ghost", false, Some(2)), Provenance::Reported);
        let now = cards(&s, &pi);
        assert_eq!(now[0], ("a".into(), CallState::Running, None, Provenance::Reported));
        assert_eq!(now[1], ("ghost".into(), CallState::Failed, Some(Mark::EndWithoutStart), Provenance::Reported));

        // A second start for a call is refused; so is a second end, and output after the end.
        s.event(&pi, &start("a", "bash", json!({})), Provenance::Reported);
        s.event(&pi, &end("a", true, Some(0)), Provenance::Reported);
        s.event(&pi, &end("a", false, Some(1)), Provenance::Reported);
        s.event(&pi, &output("a", "late"), Provenance::Reported);
        let card = s.agent(&pi).unwrap().cards().next().unwrap();
        assert_eq!((card.state, card.exit_code), (CallState::Ok, Some(0)), "the first end stands");
        assert_eq!(card.output.all(), "", "output after the end is not the call's");
        assert_eq!(s.agent(&pi).unwrap().refused, 4);

        // Output for a call that never started is kept, marked.
        s.event(&pi, &output("stray", "hello"), Provenance::Reported);
        assert_eq!(cards(&s, &pi)[2].2, Some(Mark::OutputWithoutStart));

        // When the turn ends, a reported call still open is interrupted...
        s.event(&pi, &start("b", "bash", json!({})), Provenance::Reported);
        s.close_turn(&pi, true);
        let b = s.agent(&pi).unwrap().cards().find(|c| c.call == "b").unwrap();
        assert_eq!(b.state, CallState::Interrupted);
        // ...and anything the harness says about that turn afterwards is refused and counted.
        let before = s.agent(&pi).unwrap().refused;
        s.event(&pi, &output("b", "too late"), Provenance::Reported);
        s.text(&pi, "too late");
        assert_eq!(s.agent(&pi).unwrap().refused, before + 2);
    }

    /// #212: the details column said "Refused 121 events" and nothing else — no way to see what
    /// was refused, or why. Every refusal now keeps a line saying both; the newest few are kept,
    /// the count stays the whole truth, and the lines survive a restart like the count does.
    #[test]
    fn a_refusal_keeps_a_line_that_says_what_arrived_and_why_it_was_refused() {
        let (mut s, _) = store();
        let pi = id("pi:main");
        s.upsert_agent(AgentMeta::new(pi.clone(), "pi"));
        s.event(&pi, &start("early", "bash", json!({})), Provenance::Reported);
        assert_eq!(s.agent(&pi).unwrap().refusals, ["a start for `bash` (`early`) — no turn was open"]);

        s.open_turn(&pi, "go");
        s.event(&pi, &start("a", "bash", json!({})), Provenance::Reported);
        s.event(&pi, &start("a", "bash", json!({})), Provenance::Reported);
        s.event(&pi, &end("a", true, Some(0)), Provenance::Reported);
        s.event(&pi, &end("a", false, Some(1)), Provenance::Reported);
        s.event(&pi, &output("a", "late"), Provenance::Reported);
        s.close_turn(&pi, true);
        s.text(&pi, "after the end");
        assert_eq!(
            s.details(&pi).unwrap().refusals,
            [
                "a start for `bash` (`early`) — no turn was open",
                "a second start for `a`",
                "a second end for `a`",
                "output for `a` — its call had already ended",
                "some of its text — no turn was open",
            ],
            "every refusal says what arrived and why it was refused"
        );
        assert_eq!(s.agent(&pi).unwrap().refused, 5);

        // A command's bytes after its exit get their own line.
        s.command_started(&pi, "job-1", "make", "/home/pranab/src");
        s.command_finished(&pi, "job-1", "make", Some(0), false);
        s.command_output(&pi, "job-1", b"late bytes");
        assert_eq!(
            s.details(&pi).unwrap().refusals.last().unwrap(),
            "terminal bytes for `job-1` — the command had already ended"
        );

        // The newest REFUSED_LINES are kept; the count keeps all of them.
        for _ in 0..REFUSED_LINES {
            s.text(&pi, "still talking");
        }
        let agent = s.agent(&pi).unwrap();
        assert_eq!(agent.refusals.len(), REFUSED_LINES, "the lines are bounded");
        assert_eq!(agent.refused, 12, "the count is not");
        assert!(agent.refusals.iter().all(|l| l == "some of its text — no turn was open"), "{:?}", agent.refusals);

        let dir = scratch_dir("refusals");
        s.save(&dir).unwrap();
        let back = Store::load(&dir, Box::new(|| 1_790_999_999));
        let agent = back.agent(&pi).unwrap();
        assert_eq!(agent.refused, 12);
        assert_eq!(agent.refusals.len(), REFUSED_LINES);
        assert_eq!(agent.refusals[0], "some of its text — no turn was open");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_command_the_shell_runs_outlives_the_turn_that_asked_for_it() {
        let (mut s, _) = store();
        let pi = id("pi:main");
        s.open_turn(&pi, "build it");
        s.event(&pi, &start("job-1", "agent_run", json!({"command": "make"})), Provenance::Verified);
        s.close_turn(&pi, true);
        let agent = s.agent(&pi).unwrap();
        assert!(agent.cards().next().unwrap().running(), "the shell still owns the process");
        assert_eq!(agent.state, State::RunningTool);
        // It keeps streaming after the turn, and settles when the command exits.
        s.event(&pi, &output("job-1", "done\n"), Provenance::Verified);
        s.event(&pi, &end("job-1", true, Some(0)), Provenance::Verified);
        let agent = s.agent(&pi).unwrap();
        assert_eq!(agent.cards().next().unwrap().output.all(), "done\n");
        assert_eq!(agent.state, State::Done);
        // A harness cannot write into the shell's card by knowing its id.
        s.open_turn(&pi, "again");
        s.event(&pi, &output("job-1", "forged"), Provenance::Reported);
        assert_eq!(s.agent(&pi).unwrap().cards().next().unwrap().output.all(), "done\n");
    }

    #[test]
    fn output_is_capped_at_the_head_and_the_tail_with_a_marker() {
        let (mut s, _) = store();
        let pi = id("pi:main");
        s.open_turn(&pi, "flood");
        s.event(&pi, &start("f", "bash", json!({})), Provenance::Reported);
        // One oversized event is cut at the event cap.
        s.event(&pi, &output("f", &"x".repeat(EVENT_CAP + 5000)), Provenance::Reported);
        let card = s.agent(&pi).unwrap().cards().next().unwrap();
        assert_eq!(card.output.bytes.total(), EVENT_CAP as u64);
        // Three megabytes through a card keep two, and say how much went.
        let line = format!("{}\n", "y".repeat(1023));
        for _ in 0..(3 * 1024) {
            s.event(&pi, &output("f", &line), Provenance::Reported);
        }
        let card = s.agent(&pi).unwrap().cards().next().unwrap();
        let kept = card.output.bytes.text();
        assert!(kept.len() <= CARD_CAP + 64, "kept {} bytes", kept.len());
        assert!(kept.starts_with("xxxx"), "the head is how it started");
        assert!(kept.contains("not kept …"), "the marker says what fell between");
        assert!(kept.ends_with("yyy\n"), "the tail is how it ended");
        assert_eq!(card.output.lines(), 3 * 1024, "every line is still counted");
    }

    #[test]
    fn the_details_count_only_what_the_shell_saw_for_commands_files_and_approvals() {
        let (mut s, _) = store();
        let ds = id("deepseek:main");
        s.open_turn(&ds, "release notes");
        // What the harness says it did: a line of its text, then an event.
        s.trail_call(&ds, &crate::trail::parse(r#"⚙️ os_act files.move {"args":{"from":"/a","to":"/b"}}"#, None).unwrap().0);
        s.event(&ds, &start("r1", "bash", json!({"command": "rm -rf ~/x", "path": "/home/p/x"})), Provenance::Reported);
        s.event(&ds, &end("r1", true, Some(0)), Provenance::Reported);
        // What the shell ran.
        s.event(&ds, &start("j1", "agent_run", json!({"command": "git log", "cwd": "/repo"})), Provenance::Verified);
        s.event(&ds, &end("j1", false, Some(128)), Provenance::Verified);
        s.event(&ds, &start("j2", "os_act", json!({"app": "files", "args": {"paths": ["/home/p/a.md", "~/b.md"]}})), Provenance::Verified);
        s.event(&ds, &end("j2", true, None), Provenance::Verified);
        s.approval_asked(&ds, "req-1", "files.move 38 files → Trash");
        assert_eq!(s.agent(&ds).unwrap().state, State::WaitingForYou);
        s.approval_answered(&ds, "req-1", true);
        s.approval_answered(&ds, "req-1", true); // answered once, counted once

        let d = s.details(&ds).unwrap();
        assert_eq!(d.calls, 4, "every call is a call");
        assert_eq!(d.failed_calls, 1);
        assert_eq!(d.commands, vec![("git log".to_string(), Some(128), CallState::Failed)], "the reported `rm` is not a command the shell saw");
        assert_eq!(d.files, vec!["/home/p/a.md".to_string(), "~/b.md".to_string()], "only paths a verified call names");
        assert_eq!((d.approvals_asked, d.approvals_answered), (1, 1));
        assert_eq!(s.agent(&ds).unwrap().state, State::Thinking);
    }

    /// An approval asked after a run ended is not that run's: it goes in a turn of its own, outside
    /// any run, and a second one shares it. On VM 520 the notes task listed arena saves and old
    /// approval cards among its changes, because late events were filed in the last closed turn.
    #[test]
    fn what_comes_after_a_run_ended_is_not_filed_in_that_run() {
        let (mut s, _) = store();
        let ds = id("deepseek:main");
        s.open_turn(&ds, "write the week plan");
        s.close_turn(&ds, true);
        s.approval_asked(&ds, "late-1", "files.move");
        s.approval_asked(&ds, "late-2", "files.copy");
        let a = s.agent(&ds).unwrap();
        let run = a.turns.iter().find(|t| t.prompt == "write the week plan").unwrap();
        assert!(!run.items.iter().any(|i| matches!(i, Item::Approval(_))), "the closed run keeps only its own");
        assert_eq!(a.turns.len(), 2, "one turn outside any run, shared by both late approvals");
        let outside = a.turns.last().unwrap();
        assert!(outside.prompt.is_empty() && !outside.open());
        assert_eq!(outside.items.iter().filter(|i| matches!(i, Item::Approval(_))).count(), 2);
        // The next run starts clean, and an approval during it is its own.
        s.open_turn(&ds, "tidy notes");
        s.approval_asked(&ds, "in-run", "notes.trash");
        let a = s.agent(&ds).unwrap();
        let open = a.turns.last().unwrap();
        assert_eq!(open.prompt, "tidy notes");
        assert_eq!(open.items.iter().filter(|i| matches!(i, Item::Approval(_))).count(), 1);
    }

    #[test]
    fn a_trail_call_claims_no_outcome_and_is_not_told_twice() {
        let (mut s, _) = store();
        let h = id("hermes:main");
        s.open_turn(&h, "make a picture");
        let (call, _) = crate::trail::parse(r#"⚙️ os_act studio.generate {"args":{"prompt":"a red kite"}}"#, None).unwrap();
        s.trail_call(&h, &call);
        let card = s.agent(&h).unwrap().cards().next().unwrap();
        assert_eq!(card.state, CallState::Untold);
        assert_eq!(card.state.status(), "", "the card says nothing about how it went");
        assert_eq!(card.provenance, Provenance::Reported);
        assert_eq!(card.as_call().summary(), r#"os_act studio.generate prompt="a red kite""#);
        // Once the harness sends structured events, its trail lines are the same calls again.
        s.event(&h, &start("c1", "bash", json!({})), Provenance::Reported);
        s.trail_call(&h, &call);
        assert_eq!(s.agent(&h).unwrap().cards().count(), 2);
    }

    #[test]
    fn the_trail_line_written_beside_an_event_is_not_a_second_card() {
        let (mut s, _) = store();
        let pi = id("pi:c1");
        s.open_turn(&pi, "list it");
        // `turn.tool_start` writes the line, then sends the event.
        s.trail_call(&pi, &crate::trail::parse(r#"⚙️ bash {"command":"ls"}"#, None).unwrap().0);
        s.event(&pi, &start("t1", "bash", json!({"command": "ls"})), Provenance::Reported);
        s.trail_call(&pi, &crate::trail::parse(r#"⚙️ bash {"command":"pwd"}"#, None).unwrap().0);
        s.event(&pi, &start("t2", "bash", json!({"command": "pwd"})), Provenance::Reported);
        let calls: Vec<String> = s.agent(&pi).unwrap().cards().map(|c| c.call.clone()).collect();
        assert_eq!(calls, vec!["t1", "t2"], "one card per call, the event's");
    }

    #[test]
    fn a_command_the_shell_runs_is_fed_as_bytes_and_counted_as_verified() {
        let (mut s, _) = store();
        let pi = id("pi:c1");
        s.open_turn(&pi, "build");
        // Piece 2 hands over bytes as they come off the PTY, and a read may split a character.
        let word = "caf\u{e9}\r\n".as_bytes();
        s.command_output(&pi, "job-7", &word[..4]);
        s.command_output(&pi, "job-7", &word[4..]);
        let card = s.agent(&pi).unwrap().cards().next().unwrap();
        assert_eq!((card.provenance, card.state, card.mark), (Provenance::Verified, CallState::Running, None));
        assert_eq!(card.output.runs()[0].text, "caf\u{e9}", "the character is whole on the screen");
        s.command_finished(&pi, "job-7", "make", Some(2), false);
        s.command_output(&pi, "job-7", b"late");
        let agent = s.agent(&pi).unwrap();
        let card = agent.cards().next().unwrap();
        assert_eq!((card.state, card.exit_code, card.summary.as_str()), (CallState::Failed, Some(2), "exit 2"));
        assert_eq!(agent.refused, 1, "output after the end is refused");
        assert_eq!(s.details(&pi).unwrap().commands, vec![("make".to_string(), Some(2), CallState::Failed)]);
        // A command stopped from the pane says so.
        s.command_started(&pi, "job-8", "sleep 60", "/home/p");
        s.command_finished(&pi, "job-8", "sleep 60", None, true);
        let card = s.agent(&pi).unwrap().cards().nth(1).unwrap();
        assert_eq!((card.state, card.summary.as_str()), (CallState::Failed, "stopped"));
        // An agent the store had not heard of is made known by its first command.
        s.command_output(&id("deepseek:c2"), "job-9", b"x");
        assert!(s.agent(&id("deepseek:c2")).is_some());
    }

    /// A command that ends before the call that started it gets to say so is still one card: its
    /// output, its end, then its start filling in what it was — and no refusal counted for it.
    #[test]
    fn a_command_quicker_than_its_own_start_is_one_card_and_no_refusal() {
        let (mut s, _) = store();
        let pi = id("pi:c1");
        s.open_turn(&pi, "go");
        s.command_output(&pi, "job-q", b"done\r\n");
        s.command_finished(&pi, "job-q", "true && echo done", Some(0), false);
        s.command_started(&pi, "job-q", "true && echo done", "/home/p");
        let agent = s.agent(&pi).unwrap();
        let cards: Vec<&Card> = agent.cards().collect();
        assert_eq!(cards.len(), 1, "one command, one card");
        assert_eq!(cards[0].args, json!({"command": "true && echo done", "cwd": "/home/p"}));
        assert_eq!((cards[0].state, cards[0].exit_code), (CallState::Ok, Some(0)));
        assert_eq!(agent.refused, 0);
    }

    /// Design decision 4, in the store: the approval is an item of the session — the request id,
    /// what was asked, how it came out — settled once; an answer counts as answered and an expiry
    /// #234: a task the desktop's stopping cut off is lost, not failed, and stays so across later
    /// restarts; a task that ended on its own is not.
    #[test]
    fn a_turn_the_shell_stopped_during_is_lost_and_stays_lost() {
        let dir = scratch_dir("lost");
        let (mut s, _) = store();
        let pi = id("pi:c-lost1");
        s.open_turn(&pi, "finished before");
        s.close_turn(&pi, false);
        s.open_turn(&pi, "build the town model");
        s.save(&dir).unwrap();

        let back = Store::load(&dir, Box::new(|| 1_800_000_000));
        let turns = &back.agent(&pi).unwrap().turns;
        assert_eq!((turns[0].lost, turns[0].ok), (false, Some(false)), "a failure of its own is still a failure");
        assert_eq!((turns[1].lost, turns[1].ok), (true, Some(false)), "cut off by the stop: lost");

        let mut back = back;
        back.save(&dir).unwrap();
        let again = Store::load(&dir, Box::new(|| 1_800_000_100));
        assert!(again.agent(&pi).unwrap().turns[1].lost, "still lost after the next restart");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #234: a stuck task is listed where the person looks for what needs them, counted there,
    /// and first under Active; once it is heard from again it leaves Needs you.
    #[test]
    fn a_stuck_task_is_in_needs_you_and_first_in_active() {
        let (mut s, clock) = store();
        let calm = id("pi:c-calm");
        let quiet = id("deepseek:c-quiet");
        s.open_turn(&quiet, "build the game");
        s.open_turn(&calm, "tidy Downloads");
        assert_eq!(s.list(Tab::NeedsYou, None), Vec::<AgentId>::new());

        clock.fetch_add(crate::agents::progress::STUCK_QUIET_SECS + 10, Ordering::SeqCst);
        s.text(&calm, "still going");
        assert_eq!(s.list(Tab::NeedsYou, None), vec![quiet.clone()], "quiet past the limit: stuck, and it needs you");
        let at = Tab::EVERY.iter().position(|t| *t == Tab::NeedsYou).unwrap();
        assert_eq!(s.counts()[at], 1);
        assert_eq!(s.list(Tab::Active, None)[0], quiet, "what needs the person comes first");

        s.text(&quiet, "found it, carrying on");
        assert!(s.list(Tab::NeedsYou, None).is_empty(), "heard from again: no longer stuck");
    }

    fn questions(s: &Store, agent: &AgentId) -> Vec<Question> {
        s.agent(agent).unwrap().turns.iter().flat_map(|t| t.items.iter()).filter_map(|i| match i {
            Item::Question(q) => Some(q.clone()),
            _ => None,
        }).collect()
    }

    fn asks(request: &str, prompt: &str) -> Event {
        Event::Request { request_id: request.into(), prompt: prompt.into(), options: vec!["Yes".into(), "No".into()] }
    }

    /// A question waits in the session as one card, is answered once, and an approval answered
    /// meanwhile does not take the agent out of waiting while the question still is.
    #[test]
    fn a_question_is_one_card_answered_once_and_holds_the_agent_waiting() {
        let (mut s, _) = store();
        let pi = id("pi:c-q1");
        s.open_turn(&pi, "tidy Downloads");
        s.event(&pi, &asks("r1", "Delete 3 installers?"), Provenance::Reported);
        s.event(&pi, &asks("r1", "Delete 3 installers?"), Provenance::Reported);
        assert_eq!(questions(&s, &pi).len(), 1, "the same question twice is one card");

        s.approval_asked(&pi, "appr-9", "files.move");
        s.approval_answered(&pi, "appr-9", true);
        assert_eq!(s.agent(&pi).unwrap().state, State::WaitingForYou, "the question still holds it");

        assert!(s.question_answered(&pi, "r1", "Yes"));
        assert!(!s.question_answered(&pi, "r1", "No"), "answered once");
        assert_eq!(questions(&s, &pi)[0].answer, "Yes");
        assert_eq!(s.agent(&pi).unwrap().state, State::Thinking, "back to work");
        assert!(s.transcript(&pi, 5).unwrap().contains("[asked the person] Delete 3 installers? — answered: Yes"));
    }

    /// A question past the quiet limit is still the person's to answer: it needs them as waiting,
    /// never as stuck. Answered and then quiet, it is stuck as any other task.
    #[test]
    fn a_question_long_unanswered_needs_you_as_waiting_not_stuck() {
        let (mut s, clock) = store();
        let mind = id("yantrik:c-erase");
        s.open_turn(&mind, "forget the old address");
        s.event(&mind, &asks("r1", "Keep or Erase?"), Provenance::Reported);
        clock.fetch_add(120, Ordering::SeqCst);

        let a = s.agent(&mind).unwrap();
        assert_eq!(a.state, State::WaitingForYou);
        assert!(!s.stuck(a), "waiting on the person is not stuck");
        assert_eq!(s.list(Tab::NeedsYou, None), vec![mind.clone()], "it needs the person");

        assert!(s.question_answered(&mind, "r1", "Keep"));
        clock.fetch_add(120, Ordering::SeqCst);
        assert!(s.stuck(s.agent(&mind).unwrap()), "answered, then silent: stuck as before");
    }

    #[test]
    fn a_question_whose_answer_could_not_be_delivered_closes_with_why() {
        let (mut s, _) = store();
        let pi = id("pi:c-q2");
        s.open_turn(&pi, "tidy Downloads");
        s.event(&pi, &asks("r1", "Delete them?"), Provenance::Reported);
        s.question_closed(&pi, "r1", "run 4 has ended (orphaned)");
        let q = &questions(&s, &pi)[0];
        assert_eq!((q.answer.as_str(), q.closed.as_str()), ("", "run 4 has ended (orphaned)"));
        assert!(!q.waiting());
        assert_eq!(s.agent(&pi).unwrap().state, State::Thinking);
    }

    #[test]
    fn a_question_still_waiting_when_the_shell_stops_comes_back_closed() {
        let dir = scratch_dir("questions");
        let (mut s, _) = store();
        let pi = id("pi:c-q3");
        s.open_turn(&pi, "tidy Downloads");
        s.event(&pi, &asks("r1", "Delete them?"), Provenance::Reported);
        s.event(&pi, &asks("r2", "And the videos?"), Provenance::Reported);
        s.question_answered(&pi, "r2", "No");
        s.save(&dir).unwrap();
        let back = Store::load(&dir, Box::new(|| 1_800_000_000));
        let kept = questions(&back, &pi);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].closed, "the desktop restarted while it waited", "never drawn with buttons again");
        assert_eq!((kept[1].answer.as_str(), kept[1].closed.as_str()), ("No", ""), "an answered one stays answered");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every way a turn ends in the store closes the questions it asked and nobody answered: the
    /// host has expired them, so a card left with buttons is one a click is refused on (VM 520,
    /// 4 October: two identical Erase cards, one dead). Closed, it leaves `waiting_questions`, the
    /// Needs-you tab and the transcript's "waiting" line.
    #[test]
    fn a_question_closes_when_the_turn_that_asked_ends_unanswered() {
        let ends: [(&str, fn(&mut Store, &AgentId)); 5] = [
            ("completed", |s, a| s.close_turn(a, true)),
            ("failed", |s, a| s.close_turn(a, false)),
            ("cancelled", |s, a| {
                s.note(a, "Stopped.");
                s.close_turn(a, false)
            }),
            ("orphaned", |s, a| s.set_state(a, State::HarnessGone)),
            ("interrupted", |s, a| s.open_turn(a, "something else")),
        ];
        for (how, end) in ends {
            let (mut s, _) = store();
            let mind = id("yantrik:c-erase");
            s.open_turn(&mind, "forget the old address");
            s.event(&mind, &asks("r1", "Keep or Erase?"), Provenance::Reported);
            assert_eq!(s.list(Tab::NeedsYou, None), vec![mind.clone()], "{how}: it waits on the person");
            end(&mut s, &mind);

            let q = &questions(&s, &mind)[0];
            assert_eq!((q.answer.as_str(), q.closed.as_str()), ("", Question::STOPPED_ASKING), "{how}");
            assert_eq!(s.agent(&mind).unwrap().waiting_questions().count(), 0, "{how}: nothing still asked");
            assert!(s.list(Tab::NeedsYou, None).is_empty(), "{how}: it no longer needs the person");
            let said = s.transcript(&mind, 5).unwrap();
            assert!(said.contains("Keep or Erase? — not answered: the agent stopped asking"), "{how}: {said}");
        }
    }

    /// Closing is for what still waits: an answer the person gave stands when the turn ends, and a
    /// question the next turn asks is open while the one before it is closed.
    #[test]
    fn an_ended_turn_closes_only_its_own_unanswered_questions() {
        let (mut s, _) = store();
        let mind = id("yantrik:c-erase2");
        s.open_turn(&mind, "forget the old address");
        s.event(&mind, &asks("r1", "Keep or Erase?"), Provenance::Reported);
        s.event(&mind, &asks("r2", "And the old phone number?"), Provenance::Reported);
        assert!(s.question_answered(&mind, "r2", "Keep"));
        s.close_turn(&mind, true);
        s.open_turn(&mind, "try again");
        s.event(&mind, &asks("r3", "Keep or Erase?"), Provenance::Reported);

        let q = questions(&s, &mind);
        assert_eq!(q[0].closed, Question::STOPPED_ASKING, "turn 1's unanswered question is closed");
        assert_eq!((q[1].answer.as_str(), q[1].closed.as_str()), ("Keep", ""), "an answered one is untouched");
        assert!(q[2].waiting(), "turn 2's question is open");
        let waiting: Vec<_> = s.agent(&mind).unwrap().waiting_questions().map(|q| q.request.clone()).collect();
        assert_eq!(waiting, vec!["r3".to_string()], "only the live one is waiting");
        assert_eq!(s.list(Tab::NeedsYou, None), vec![mind.clone()]);
    }

    /// A restart still closes what was waiting as before, and a question already closed when the
    /// agent stopped asking keeps that reason through the save.
    #[test]
    fn a_restart_keeps_a_closed_questions_reason_and_closes_the_rest_as_before() {
        let dir = scratch_dir("questions-closed");
        let (mut s, _) = store();
        let mind = id("yantrik:c-erase3");
        s.open_turn(&mind, "forget the old address");
        s.event(&mind, &asks("r1", "Keep or Erase?"), Provenance::Reported);
        s.close_turn(&mind, true);
        s.open_turn(&mind, "try again");
        s.event(&mind, &asks("r2", "Keep or Erase?"), Provenance::Reported);
        s.save(&dir).unwrap();
        let back = Store::load(&dir, Box::new(|| 1_800_000_000));
        let kept = questions(&back, &mind);
        assert_eq!(kept[0].closed, Question::STOPPED_ASKING);
        assert_eq!(kept[1].closed, Question::RESTARTED);
        assert_eq!(back.agent(&mind).unwrap().waiting_questions().count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// does not; and one still waiting when the shell stops comes back withdrawn, never waiting.
    #[test]
    fn an_approval_waits_in_the_session_settles_once_and_a_restart_withdraws_it() {
        let dir = scratch_dir("approvals");
        let (mut s, _) = store();
        let ds = id("deepseek:c-02be44");
        s.open_turn(&ds, "release notes");
        s.approval_asked(&ds, "appr-1", "files.move");
        s.approval_asked(&ds, "appr-1", "files.move"); // the same card asked twice is one card
        let approvals = |s: &Store| -> Vec<Approval> {
            s.agent(&ds).unwrap().turns.iter().flat_map(|t| t.items.iter()).filter_map(|i| match i {
                Item::Approval(a) => Some(a.clone()),
                _ => None,
            }).collect()
        };
        assert_eq!(approvals(&s).len(), 1);
        assert_eq!(approvals(&s)[0].outcome, ApprovalOutcome::Pending);
        assert_eq!(s.agent(&ds).unwrap().state, State::WaitingForYou);

        s.approval_settled(&ds, "appr-1", ApprovalOutcome::Expired, "");
        s.approval_settled(&ds, "appr-1", ApprovalOutcome::Allowed, "late"); // settled once
        let settled = &approvals(&s)[0];
        assert_eq!(settled.outcome, ApprovalOutcome::Expired);
        assert!(settled.record.starts_with("Not answered in time"), "{}", settled.record);
        let d = s.details(&ds).unwrap();
        assert_eq!((d.approvals_asked, d.approvals_answered), (1, 0), "nobody answered it");
        assert_eq!(s.agent(&ds).unwrap().state, State::Thinking, "no longer waiting on the person");

        // One more, still waiting when the shell stops.
        s.approval_asked(&ds, "appr-2", "shell.agent_run");
        s.save(&dir).unwrap();
        let back = Store::load(&dir, Box::new(|| 1_800_000_000));
        let agent = back.agent(&ds).unwrap();
        let kept: Vec<&Approval> = agent.turns.iter().flat_map(|t| t.items.iter()).filter_map(|i| match i {
            Item::Approval(a) => Some(a),
            _ => None,
        }).collect();
        assert_eq!(kept.len(), 2);
        assert_eq!((kept[0].outcome, kept[1].outcome), (ApprovalOutcome::Expired, ApprovalOutcome::Withdrawn));
        assert!(kept[1].record.contains("restarted"), "{}", kept[1].record);
        assert!(agent.pending_approvals.is_empty(), "nothing is waiting after a restart");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_agents_children_and_its_transcript_are_read_from_the_session() {
        let (mut s, clock) = store();
        let parent = id("pi:c-par");
        s.open_turn(&parent, "plan the release");
        for n in 0..2 {
            clock.fetch_add(1, Ordering::SeqCst);
            let mut meta = AgentMeta::new(id(&format!("pi:c-kid{n}")), "pi");
            meta.parent = Some(parent.clone());
            s.upsert_agent(meta);
        }
        assert_eq!(s.children_of(&parent), vec![id("pi:c-kid0"), id("pi:c-kid1")]);
        assert!(s.children_of(&id("pi:c-kid0")).is_empty());

        s.text(&parent, "Two helpers are on it.");
        s.event(&parent, &Event::Thinking { delta: "private reasoning".into() }, Provenance::Reported);
        s.event(&parent, &start("r1", "bash", json!({"command": "ls"})), Provenance::Reported);
        s.event(&parent, &output("r1", "a\nb\n"), Provenance::Reported);
        s.event(&parent, &end("r1", false, Some(2)), Provenance::Reported);
        s.close_turn(&parent, true);
        let text = s.transcript(&parent, 3).unwrap();
        for said in ["pi:c-par", "plan the release", "Two helpers are on it.", "[reported call] bash command=\"ls\" — failed · exit 2", "    b"] {
            assert!(text.contains(said), "{said:?} missing:\n{text}");
        }
        assert!(!text.contains("private reasoning"), "thinking is not the session:\n{text}");
        // Bounded, newest kept.
        for n in 0..60 {
            s.open_turn(&parent, &format!("turn {n}"));
            s.text(&parent, &"x".repeat(4000));
            s.close_turn(&parent, true);
        }
        let text = s.transcript(&parent, 20).unwrap();
        assert!(text.len() <= TRANSCRIPT_BYTES + 200, "{}", text.len());
        assert!(text.contains("turn 59") && text.starts_with("… (the start is cut"), "{}", &text[..80]);
    }

    #[test]
    fn a_harness_that_goes_settles_its_calls_and_says_so() {
        let (mut s, _) = store();
        let pi = id("pi:main");
        s.open_turn(&pi, "long job");
        s.event(&pi, &start("t", "bash", json!({})), Provenance::Reported);
        s.set_state(&pi, State::HarnessGone);
        let agent = s.agent(&pi).unwrap();
        assert_eq!(agent.state, State::HarnessGone);
        assert_eq!(agent.cards().next().unwrap().state, CallState::Interrupted);
        assert!(agent.open_turn().is_none());
        assert!(Tab::Complete.holds(agent.state));
    }

    #[test]
    fn chat_talk_is_not_listed_chat_work_is_a_run_and_an_agent_is_one_row() {
        let (mut s, clock) = store();
        let hermes = id("hermes:main");
        // Talk in the chat: never an agent, never a run.
        s.open_chat_turn(&hermes, "thanks, that's all");
        s.text(&hermes, "You're welcome.");
        s.close_turn(&hermes, true);
        clock.fetch_add(1, Ordering::SeqCst);
        // Work asked in the chat: its own run, from chat.
        s.open_chat_turn(&hermes, "build a small game");
        s.event(&hermes, &Event::ToolStart { call: "c1".into(), name: "os_act".into(), target: "editor".into(), args: json!({}) }, Provenance::Reported);
        s.close_turn(&hermes, true);
        clock.fetch_add(1, Ordering::SeqCst);
        s.open_chat_turn(&hermes, "and a town model");
        s.event(&hermes, &Event::ToolStart { call: "c2".into(), name: "os_act".into(), target: "editor".into(), args: json!({}) }, Provenance::Reported);
        // An agent started as one: one row, as before.
        clock.fetch_add(1, Ordering::SeqCst);
        let pi = id("pi:c-9a8b7c");
        s.open_turn(&pi, "tidy Downloads");

        let all: Vec<String> = s.rows(Tab::All, None).iter().map(RowKey::id).collect();
        assert_eq!(all, ["pi:c-9a8b7c", "hermes:main#3", "hermes:main#2"], "no row for the chat, none for its talk");
        let active: Vec<String> = s.rows(Tab::Active, None).iter().map(RowKey::id).collect();
        assert_eq!(active, ["pi:c-9a8b7c", "hermes:main#3"], "each run by its own state: the finished one is not active");
        assert_eq!(s.rows(Tab::Complete, None), vec![RowKey::run(&hermes, 2)]);
        assert_eq!(s.tasks(10).len(), 3, "the Tasks tab lists work, not talk");
        assert_eq!(RowKey::parse("hermes:main#3"), RowKey::run(&hermes, 3));
        assert_eq!(RowKey::parse("pi:c-9a8b7c"), RowKey::agent(&pi));
    }

    #[test]
    fn a_session_saved_before_turns_said_where_they_came_from_reads_as_chat_on_a_mind_and_work_on_an_agent() {
        let (mut s, _) = store();
        let hermes = id("hermes:main");
        s.open_turn(&hermes, "hello");
        s.close_turn(&hermes, true);
        for t in &mut s.agents.iter_mut().find(|a| a.meta.id == hermes).unwrap().turns {
            t.origin = TurnOrigin::Unknown;
        }
        let pi = id("pi:c-1");
        s.open_turn(&pi, "old work");
        s.close_turn(&pi, true);
        let all: Vec<String> = s.rows(Tab::All, None).iter().map(RowKey::id).collect();
        assert_eq!(all, ["pi:c-1"], "the old chat turn with no work is talk; the agent is still an agent");
    }

    #[test]
    fn the_tabs_count_and_filter_the_one_list() {
        let (mut s, clock) = store();
        for (name, state) in [
            ("a:c-a", State::Thinking),
            ("b:c-b", State::RunningTool),
            ("c:c-c", State::WaitingForYou),
            ("d:c-d", State::Idle),
            ("e:c-e", State::Done),
            ("f:c-f", State::Failed),
            ("g:c-g", State::HarnessGone),
        ] {
            clock.fetch_add(1, Ordering::SeqCst);
            s.upsert_agent(AgentMeta::new(id(name), name));
            s.set_state(&id(name), state);
        }
        assert_eq!(s.counts(), [4, 1, 3, 7], "Active, Needs you, Complete, All");
        let names = |tab| s.list(tab, None).into_iter().map(|i| i.0).collect::<Vec<_>>();
        assert_eq!(names(Tab::NeedsYou), vec!["c:c-c"]);
        assert_eq!(names(Tab::Complete), vec!["g:c-g", "f:c-f", "e:c-e"], "newest first");
        assert_eq!(names(Tab::All).len(), 7);
    }

    #[test]
    fn a_row_that_needs_you_rises_but_never_under_the_pointer() {
        let (mut s, clock) = store();
        for name in ["old:main", "mid:main", "new:main"] {
            clock.fetch_add(10, Ordering::SeqCst);
            s.open_turn(&id(name), "work");
        }
        let names = |ids: Vec<AgentId>| ids.into_iter().map(|i| i.0).collect::<Vec<_>>();
        let on_screen = s.list(Tab::Active, None);
        assert_eq!(names(on_screen.clone()), vec!["new:main", "mid:main", "old:main"]);

        s.approval_asked(&id("old:main"), "r", "delete it");
        // The pointer is over the list: the rows stay where the person is about to click.
        assert_eq!(names(s.list(Tab::Active, Some(&on_screen))), vec!["new:main", "mid:main", "old:main"]);
        // It leaves: the row that needs the person goes to the top.
        assert_eq!(names(s.list(Tab::Active, None)), vec!["old:main", "new:main", "mid:main"]);

        // Held, a row that leaves the tab goes and a new one joins at the bottom.
        s.close_turn(&id("mid:main"), true);
        clock.fetch_add(10, Ordering::SeqCst);
        s.open_turn(&id("newest:main"), "work");
        assert_eq!(names(s.list(Tab::Active, Some(&on_screen))), vec!["new:main", "old:main", "newest:main"]);
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yantrik-agents-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_session_reads_the_same_after_a_restart() {
        let dir = scratch_dir("roundtrip");
        let (mut s, clock) = store();
        let pi = id("pi:c-7f3a91");
        let mut meta = AgentMeta::new(pi.clone(), "pi");
        meta.model = "qwen3.8-27b".into();
        let coder = RoleMeta {
            id: "coder".into(),
            name: "Coder".into(),
            reach: "shell.agent_* and editor · at most sensitive".into(),
            reach_words: "its own terminal and the Editor, and it may ask for sensitive acts".into(),
            turns: 12,
            minutes: 45,
        };
        meta.role = Some(coder.clone());
        s.upsert_agent(meta);
        // A later upsert that knows nothing of the role leaves it as it was.
        s.upsert_agent(AgentMeta::new(pi.clone(), "pi"));
        s.open_turn(&pi, "tidy the photos folder");
        s.text(&pi, "Looking for duplicates.");
        s.event(&pi, &Event::Thinking { delta: "hash them first".into() }, Provenance::Reported);
        s.event(&pi, &start("j", "agent_run", json!({"command": "fdupes -r ~/Pictures"})), Provenance::Verified);
        s.event(
            &pi,
            &Event::ToolOutput { call: "j".into(), stream: Stream::Terminal, delta: "\x1b[1m~/Pictures/a.jpg\x1b[0m\r\n".into() },
            Provenance::Verified,
        );
        s.event(&pi, &end("j", true, Some(0)), Provenance::Verified);
        s.event(&pi, &Event::Usage { model: String::new(), input_tokens: Some(40_000), output_tokens: Some(1_000), cost_usd: None }, Provenance::Reported);
        s.close_turn(&pi, true);
        // A second agent was mid-turn when the shell stopped.
        s.open_turn(&id("hermes:main"), "weather");
        s.event(&id("hermes:main"), &start("h1", "terminal", json!({})), Provenance::Reported);
        s.save(&dir).unwrap();
        assert!(dir.join("pi:c-7f3a91.jsonl").is_file(), "one file per agent, named by its id");

        clock.fetch_add(3600, Ordering::SeqCst);
        let later = clock.load(Ordering::SeqCst);
        let back = Store::load(&dir, Box::new(move || later));
        let agent = back.agent(&pi).expect("the agent is back");
        assert_eq!(agent.meta.title, "tidy the photos folder");
        assert_eq!(agent.meta.model, "qwen3.8-27b");
        assert_eq!(agent.meta.role, Some(coder), "the role it was started as comes back with it");
        assert_eq!(agent.state, State::Done);
        assert_eq!(agent.usage.input_tokens, 40_000);
        let items = &agent.turns[0].items;
        assert!(matches!(&items[0], Item::Text(t) if t.text() == "Looking for duplicates."));
        assert!(matches!(&items[1], Item::Thinking(t) if t.text() == "hash them first"));
        let card = agent.cards().next().unwrap();
        assert_eq!((card.provenance, card.exit_code, card.state), (Provenance::Verified, Some(0), CallState::Ok));
        assert_eq!(card.output.runs()[0].text, "~/Pictures/a.jpg", "the terminal is redrawn from its bytes");
        assert!(card.output.runs()[0].bold);
        assert_eq!(back.details(&pi).unwrap().commands.len(), 1);

        let hermes = back.agent(&id("hermes:main")).unwrap();
        assert_eq!(hermes.state, State::Idle, "nothing is running it now");
        assert!(hermes.open_turn().is_none());
        assert_eq!(hermes.cards().next().unwrap().state, CallState::Interrupted);

        // Removing an agent removes its file.
        let mut back = back;
        back.remove_agent(&pi);
        back.save(&dir).unwrap();
        assert!(!dir.join("pi:c-7f3a91.jsonl").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_newest_agents_are_kept_on_disk() {
        let dir = scratch_dir("keep");
        let (mut s, clock) = store();
        for n in 0..(KEEP_AGENTS + 3) {
            clock.fetch_add(1, Ordering::SeqCst);
            s.open_turn(&id(&format!("h{n}:main")), "x");
            s.close_turn(&id(&format!("h{n}:main")), true);
        }
        assert_eq!(s.agents().len(), KEEP_AGENTS, "the list itself stays bounded");
        s.save(&dir).unwrap();
        let back = Store::load(&dir, Box::new(|| 1_800_000_000));
        assert_eq!(back.agents().len(), KEEP_AGENTS);
        assert!(back.agent(&id("h0:main")).is_none(), "the oldest went first");
        assert!(back.agent(&id(&format!("h{}:main", KEEP_AGENTS + 2))).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_id_cannot_write_outside_the_agents_directory() {
        let dir = Path::new("/data/agents");
        assert_eq!(file_for(dir, &id("pi:c3")), dir.join("pi:c3.jsonl"));
        assert_eq!(file_for(dir, &id("../../etc/passwd")), dir.join("_.._etc_passwd.jsonl"));
    }

    /// A terminal job at its prompt is the person's to answer (#182): the row goes to
    /// WaitingForYou and the Active list sorts it top, exactly like an approval card's. While
    /// the job waits, a settled card does not send the row back to work; when the job moves on,
    /// it goes back by the card's own rule.
    #[test]
    fn a_terminal_job_at_its_prompt_waits_on_the_person_like_a_card() {
        let (mut s, clock) = store();
        let pi = id("pi:c-182");
        let ds = id("deepseek:c-182");
        s.open_turn(&pi, "deploy the notes service");
        s.open_turn(&ds, "tidy the photos folder");
        assert_eq!(s.list(Tab::Active, None)[0], ds, "newest first, until somebody waits");

        s.jobs_waiting(&[pi.clone()]);
        assert_eq!(s.agent(&pi).unwrap().state, State::WaitingForYou);
        assert_eq!(s.list(Tab::Active, None)[0], pi, "the row waiting on the person sorts top");
        assert!(s.list(Tab::NeedsYou, None).contains(&pi), "and Needs you holds it");

        // The tick reports the same job four times a second: still waiting is not news.
        let (rev, since) = (s.revision(), s.agent(&pi).unwrap().since);
        clock.fetch_add(1, Ordering::SeqCst);
        s.jobs_waiting(&[pi.clone()]);
        assert_eq!((s.revision(), s.agent(&pi).unwrap().since), (rev, since), "unchanged, so nothing redraws");

        // A card asked and settled while the job waits leaves the row where it is.
        s.approval_asked(&pi, "appr-182", "files.move");
        s.approval_settled(&pi, "appr-182", ApprovalOutcome::Allowed, "");
        assert_eq!(s.agent(&pi).unwrap().state, State::WaitingForYou, "the job is still at its prompt");

        // The job moves on: back to work by the card's own rule — its turn is still open.
        s.jobs_waiting(&[]);
        assert_eq!(s.agent(&pi).unwrap().state, State::Thinking);

        // With no job waiting, a settled card sends the row back as it always has.
        s.approval_asked(&pi, "appr-183", "files.move");
        s.approval_settled(&pi, "appr-183", ApprovalOutcome::Denied, "");
        assert_eq!(s.agent(&pi).unwrap().state, State::Thinking);

        // A waiting job for an agent the store has not heard of makes it known.
        let hermes = id("hermes:c-182");
        s.jobs_waiting(&[hermes.clone()]);
        assert_eq!(s.agent(&hermes).unwrap().state, State::WaitingForYou);
        s.jobs_waiting(&[]);
        assert_eq!(s.agent(&hermes).unwrap().state, State::Idle, "no turn, no card, nothing to go back to");
    }
}
