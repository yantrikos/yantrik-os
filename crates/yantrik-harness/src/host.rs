//! The OS side: who is attached, who is answering, the agents they hold, and the turns in flight.
//!
//! Deliberately knows nothing about sockets. [`Host::handle`] takes a method name and a JSON
//! value and gives one back, so the shell can serve it on the bus it already has and every rule
//! in here can be tested without a compositor, a socket or a second process. The one fact a
//! socket has that a request does not — which process is on the other end — is handed in by the
//! caller through [`Host::handle_from`].
//!
//! # A harness exists because it is attached
//!
//! There is no registry file, no list of known harnesses, nothing to install. `mind` appears in
//! the picker when `mind` attaches and disappears when it stops polling — or at once, whatever
//! the grace, when the kernel says the process that attached is gone. This is the whole
//! correction: the OS was configuring endpoints and models that the harnesses already manage
//! themselves, and now it holds the one thing it actually owns — which mind the person is
//! talking to.
//!
//! # An agent is a conversation
//!
//! An agent is one conversation with one mind, [`AgentId`] `<harness>:<conversation>`. A harness
//! that attached with `conversations` gets a new one from [`Host::start_agent`], under an id this
//! host issued at random and never issues again, so an id from an earlier session cannot name a
//! live agent. A harness that did not has the one agent `<harness>:main`, and so does the Lens:
//! [`Host::send`] goes to the answering mind's `main`.
//!
//! Each agent has a token, 128 random bits, handed to the harness with every turn so the tools it
//! starts for that conversation can say which agent is asking. [`Host::agent_for_token`] reads it
//! back, with the pid of the process that attached, which the shell learned from the kernel.
//!
//! The desktop can also leave an agent a note for its next turn ([`Host::note_for`]) — a command
//! it ran that finished after the call that started it had returned. The note rides in that
//! turn's `context`, once.
//!
//! # What the host enforces
//!
//! - **One turn at a time per conversation**, first in first out. The next turn for an agent waits
//!   until the one in flight is completed or failed; different agents run at once. `/stop` alone
//!   does not wait, because it is how a person interrupts the one that is running.
//! - **Events belong to a turn in flight**, from the session that holds it, and a call's events
//!   come in order: `tool_start`, then its output, then one `tool_end`. Anything else is refused
//!   and counted ([`EventCounts`]), and the turn goes on. An event of a kind this build does not
//!   know is ignored — a newer harness must not break an older desktop.
//! - **`complete` and `fail` are the end.** Events after them are dropped and counted, and a call
//!   the harness left open is settled for the reader as *interrupted*, so no card is left
//!   spinning. The same happens when a harness detaches, restarts, goes quiet or is stopped.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use crate::event::{AgentId, Event};
use crate::protocol::{self, Assignment, Attach};
use crate::{Answer, Capabilities, Chunk, Harness, Health, Turn};

/// The summary a call gets when its turn ended before it did.
pub const INTERRUPTED: &str = "interrupted";

/// What the person who asked is told when an agent is stopped mid-turn.
pub const STOPPED: &str = "stopped before it finished";

/// How many finished turns each harness remembers, so an event that arrives after its turn was
/// closed is counted as late rather than mistaken for one it was never given.
const REMEMBERED_TURNS: usize = 256;

/// How long a turn the desktop stopped waiting for still holds its conversation.
///
/// Stopping an agent, or nobody listening any more, settles the turn for the reader at once. The
/// harness is still winding it down, though, and handing the same conversation its next turn
/// before it has would be answered with "still working on the previous request". So the turn
/// keeps the conversation until the harness closes it, or for this long if it never does.
const STOP_GRACE: Duration = Duration::from_secs(30);

/// The most tool calls one turn may open. A turn is a person's question, and a harness that
/// opens more than this in one is not answering it; past it, `tool_start` is refused so the
/// host's memory is bounded by something other than the harness's good behaviour.
const MAX_CALLS_PER_TURN: usize = 4096;

/// One call within a turn, as far as its events have gone.
struct Call {
    /// When it started among its turn's calls, so interrupted ones are settled in that order.
    order: usize,
    /// Its tool's name while it is open; `None` once its `tool_end` has come.
    open: Option<String>,
}

/// A turn handed to a harness and not yet closed by it.
struct Flight {
    conversation: String,
    /// Where its chunks go. `None` once the desktop has stopped waiting for it — the person
    /// stopped the agent, or nobody is listening — though the harness has not closed it yet.
    tx: Option<Sender<Chunk>>,
    /// When the desktop stopped waiting.
    abandoned: Option<Instant>,
    calls: HashMap<String, Call>,
}

impl Flight {
    fn new(conversation: String, tx: Sender<Chunk>) -> Flight {
        Flight { conversation, tx: Some(tx), abandoned: None, calls: HashMap::new() }
    }

    /// Whether the next turn in this conversation still has to wait for this one.
    fn holds_conversation(&self) -> bool {
        self.abandoned.map_or(true, |at| at.elapsed() < STOP_GRACE)
    }

    /// The newest call still open, which is the one a person would want named.
    fn running_tool(&self) -> Option<(String, String)> {
        self.calls
            .iter()
            .filter_map(|(id, c)| c.open.as_ref().map(|name| (c.order, id, name)))
            .max_by_key(|(order, _, _)| *order)
            .map(|(_, id, name)| (id.clone(), name.clone()))
    }

    /// Whether this event may follow the ones already accepted for its call.
    fn admit(&mut self, event: &Event) -> Result<(), String> {
        match event {
            Event::ToolStart { call, name, .. } => {
                if self.calls.contains_key(call) {
                    return Err(format!("call `{call}` has already started"));
                }
                if self.calls.len() >= MAX_CALLS_PER_TURN {
                    return Err(format!("one turn may open at most {MAX_CALLS_PER_TURN} calls"));
                }
                let order = self.calls.len();
                self.calls.insert(call.clone(), Call { order, open: Some(name.clone()) });
                Ok(())
            }
            Event::ToolOutput { call, .. } => match self.calls.get(call) {
                None => Err(format!("output for call `{call}` came before its tool_start")),
                Some(Call { open: None, .. }) => Err(format!("output for call `{call}` came after its tool_end")),
                Some(_) => Ok(()),
            },
            Event::ToolEnd { call, .. } => match self.calls.get_mut(call) {
                None => Err(format!("tool_end for call `{call}` came before its tool_start")),
                Some(Call { open: None, .. }) => Err(format!("call `{call}` has already ended")),
                Some(c) => {
                    c.open = None;
                    Ok(())
                }
            },
            Event::Thinking { .. } | Event::Status { .. } | Event::Usage { .. } => Ok(()),
        }
    }

    /// Finish the turn for whoever asked: every call still open ends *interrupted*, in the order
    /// the calls started, then the failure if there is one, then the channel closes. Once.
    fn settle(&mut self, failure: Option<String>) {
        let Some(tx) = self.tx.take() else { return };
        let mut open: Vec<(usize, String)> = self
            .calls
            .iter_mut()
            .filter_map(|(id, c)| c.open.take().map(|_| (c.order, id.clone())))
            .collect();
        open.sort();
        for (_, call) in open {
            let _ = tx.send(Chunk::Event(Event::ToolEnd {
                call,
                ok: false,
                summary: INTERRUPTED.to_string(),
                exit_code: None,
            }));
        }
        if let Some(why) = failure {
            let _ = tx.send(Chunk::Failed(why));
        }
        // Dropping the sender closes the channel, which is how the reader learns the turn is over.
    }

    /// Stop waiting for this turn: it is settled for the reader now and stays here, holding its
    /// conversation, until the harness closes it or [`STOP_GRACE`] passes.
    fn abandon(&mut self, failure: Option<String>) {
        self.settle(failure);
        if self.abandoned.is_none() {
            self.abandoned = Some(Instant::now());
        }
    }
}

/// A turn waiting to be collected by the next poll.
struct Waiting {
    assignment: Assignment,
    tx: Sender<Chunk>,
}

/// One live agent.
struct Agent {
    token: String,
    started: SystemTime,
    turns: u32,
    last: Option<TurnEnd>,
    /// What the desktop has to tell this agent at the start of its next turn, oldest first. See
    /// [`Host::note_for`].
    notes: VecDeque<String>,
    /// Notes pushed out by newer ones since the last turn took them, so that turn can say so.
    notes_dropped: usize,
}

impl Agent {
    fn new(token: String) -> Agent {
        Agent {
            token,
            started: SystemTime::now(),
            turns: 0,
            last: None,
            notes: VecDeque::new(),
            notes_dropped: 0,
        }
    }

    /// Every note it is owed, once: after this they are gone.
    fn take_notes(&mut self) -> Vec<String> {
        let mut notes = Vec::with_capacity(self.notes.len() + 1);
        if self.notes_dropped > 0 {
            notes.push(format!(
                "{} earlier note{} from the desktop {} dropped: only the latest {MAX_NOTES} are \
                 kept between turns.",
                self.notes_dropped,
                if self.notes_dropped == 1 { "" } else { "s" },
                if self.notes_dropped == 1 { "was" } else { "were" },
            ));
            self.notes_dropped = 0;
        }
        notes.extend(self.notes.drain(..));
        notes
    }
}

/// The most notes an agent holds for its next turn. Past it the oldest go, and the turn says how
/// many did, so a harness that never polls cannot make the desktop hold an unbounded pile of text.
pub const MAX_NOTES: usize = 8;

/// The longest one note may be, in bytes. A note is a sentence and a few lines of output, not a
/// log; a longer one is cut, and says so.
pub const MAX_NOTE_BYTES: usize = 1024;

/// One harness that has attached.
struct Attached {
    announced: Attach,
    session: String,
    last_seen: Instant,
    /// The process that attached, as the kernel reported it. `None` when the transport could not
    /// say (the TCP dev path) or the caller did not pass it.
    pid: Option<u32>,
    /// Turns handed out and not yet closed by the harness.
    in_flight: HashMap<u64, Flight>,
    /// Turns waiting to be collected, oldest first.
    queued: VecDeque<Waiting>,
    /// Its live agents, by conversation.
    agents: HashMap<String, Agent>,
    /// Turns it closed recently, oldest first.
    finished: VecDeque<u64>,
    /// Turns the desktop stopped waiting for, to tell the harness on its next poll.
    cancelled: Vec<u64>,
    /// Conversations the desktop ended, to tell the harness on its next poll.
    ended: Vec<String>,
}

impl Attached {
    /// Whether this session still stands for a mind that can poll.
    ///
    /// The grace of missed polls is for a harness that hiccuped — a slow turn or a blip must not
    /// drop it. It must not outlive the process itself, though: `systemctl stop` kills the
    /// harness, and the session used to sit out its whole grace anyway, long enough for the
    /// shell to keep offering a mind that is gone as the one answering (#67). What the
    /// liveness probe — the kernel, unless a test injected its own truth — says about the pid
    /// ends the grace at once.
    fn present(&self, alive: &(dyn Fn(u32) -> bool + Send + Sync)) -> bool {
        self.last_seen.elapsed() < Duration::from_secs(protocol::PRESENCE_TIMEOUT_SECS)
            && self.pid.map_or(true, alive)
    }

    fn remember_finished(&mut self, turn_id: u64) {
        self.finished.push_back(turn_id);
        while self.finished.len() > REMEMBERED_TURNS {
            self.finished.pop_front();
        }
    }

    /// Why a turn id is not in flight, in the words a harness author needs.
    fn not_in_flight(&self, turn_id: u64) -> String {
        if self.finished.contains(&turn_id) {
            format!("turn {turn_id} is already finished")
        } else {
            format!("turn {turn_id} is not one this harness was given")
        }
    }

    /// Everything this harness owed, failed, so nobody is left waiting on an answer that is not
    /// coming: open calls interrupted, the turn in flight failed, the turns behind it failed.
    fn fail_everything(self, in_flight: &str, queued: &str) {
        for (_, mut flight) in self.in_flight {
            flight.settle(Some(in_flight.to_string()));
        }
        for waiting in self.queued {
            let _ = waiting.tx.send(Chunk::Failed(queued.to_string()));
        }
    }
}

/// Whether the process the kernel reported at attach still runs: the default liveness probe
/// every [`Host`] carries, replaceable per-host by [`Host::with_liveness`] for tests that
/// invent pids.
///
/// On Linux `/proc/<pid>` goes away with the process — the same fact `systemctl stop` leaves
/// behind, asked of the kernel rather than inferred from a unit file, so a harness started by
/// hand is as alive as one its unit runs. Anywhere else there is no `/proc` to ask and presence
/// stays the grace of missed polls alone; the crate builds and its other tests run unchanged.
#[cfg(target_os = "linux")]
fn pid_alive(pid: u32) -> bool {
    std::path::Path::new("/proc").join(pid.to_string()).exists()
}

#[cfg(not(target_os = "linux"))]
fn pid_alive(_pid: u32) -> bool {
    true
}

/// One row of the picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub detail: Option<String>,
    /// `true` for something compiled into the shell, `false` for an attached client.
    pub builtin: bool,
    pub active: bool,
    pub capabilities: Capabilities,
    /// The process that attached, as the kernel reported it at accept (`SO_PEERCRED`). `None`
    /// for a built-in, which never attaches, and for a harness that attached over a transport
    /// the kernel could not speak for (the TCP dev path). The shell matches an approval's
    /// caller against it: a caller descending from that process is that mind (#206).
    pub pid: Option<u32>,
}

/// What an agent is doing, as far as the host can see from the wire.
///
/// "Waiting for you" is not here: an approval is drawn by the shell, not reported by a harness,
/// and the Agents view knows it from there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentState {
    /// Nothing is asked of it.
    Idle,
    /// A turn is waiting — behind the one in flight, or for the harness's next poll.
    Queued,
    /// A turn is in flight and no tool call is open: the mind is thinking or writing.
    Thinking,
    /// A turn is in flight and this call, reported by the harness, is open.
    RunningTool { call: String, name: String },
}

/// How an agent's last turn ended, as the harness closed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnEnd {
    Completed,
    Failed(String),
}

/// One live agent, for the Agents view and `describe shell`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentEntry {
    pub id: AgentId,
    /// The harness it runs on, by id and by the name a person reads.
    pub harness: String,
    pub harness_name: String,
    /// Whether that harness holds more than one conversation. When it does not, this is its only
    /// agent, and the view says so ("Hermes holds one conversation at a time").
    pub conversations: bool,
    pub state: AgentState,
    pub started: SystemTime,
    /// Turns sent to it, including the one in flight and any waiting.
    pub turns: u32,
    /// Turns waiting behind the one it is on — the one in flight, or, when nothing is, the one
    /// its harness will collect next.
    pub queued: usize,
    pub last: Option<TurnEnd>,
}

/// What became of the events harnesses sent, since the shell started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EventCounts {
    /// Passed on to whoever asked.
    pub accepted: u64,
    /// For a turn already closed, or one the desktop had stopped waiting for.
    pub late: u64,
    /// A call's events out of order: output or an end before its start, a second start or end.
    pub out_of_order: u64,
    /// A kind this build knows that did not parse, or no kind at all.
    pub malformed: u64,
    /// Heavier than [`protocol::MAX_EVENT_BYTES`].
    pub oversized: u64,
    /// A kind this build does not know. Ignored, not an error.
    pub unknown: u64,
}

struct State {
    attached: HashMap<String, Attached>,
    active: String,
    next_turn: u64,
    next_session: u64,
    /// Every conversation id this host has issued, so none is issued twice.
    issued: HashSet<String>,
    events: EventCounts,
}

/// Everything the OS knows about the minds available to it.
#[derive(Clone)]
pub struct Host {
    builtins: Arc<Vec<Arc<dyn Harness>>>,
    state: Arc<Mutex<State>>,
    /// Asks whether the process that attached still runs — the kernel's `pid_alive` by default,
    /// unless a test injected its own with [`Host::with_liveness`].
    liveness: Arc<dyn Fn(u32) -> bool + Send + Sync>,
}

impl Host {
    /// `builtins` are compiled into the shell — the companion. They are always present and cannot
    /// be displaced by something attaching under the same id, so a misbehaving client cannot take
    /// over the conversation or leave the machine with no mind at all.
    pub fn new(builtins: Vec<Arc<dyn Harness>>) -> Host {
        let active = builtins.first().map(|h| h.id().to_string()).unwrap_or_default();
        Host {
            builtins: Arc::new(builtins),
            state: Arc::new(Mutex::new(State {
                attached: HashMap::new(),
                active,
                next_turn: 1,
                next_session: 1,
                issued: HashSet::new(),
                events: EventCounts::default(),
            })),
            liveness: Arc::new(pid_alive),
        }
    }

    /// The same host, asking `probe` whether a process that attached still runs instead of
    /// asking the kernel. For tests that invent pids — a fake process tree has no `/proc`
    /// behind it, and the invented harness must not be reaped for that (#67). Production
    /// takes the default and never comes through here.
    pub fn with_liveness(mut self, probe: impl Fn(u32) -> bool + Send + Sync + 'static) -> Host {
        self.liveness = Arc::new(probe);
        self
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn builtin(&self, id: &str) -> Option<&Arc<dyn Harness>> {
        self.builtins.iter().find(|h| h.id() == id)
    }

    /// Forget harnesses that stopped polling. Called before anything that reads the list.
    fn reap(&self, state: &mut State) {
        let gone: Vec<String> = state
            .attached
            .iter()
            .filter(|(_, a)| !a.present(&*self.liveness))
            .map(|(id, _)| id.clone())
            .collect();
        for id in gone {
            if let Some(lost) = state.attached.remove(&id) {
                // Anyone waiting on an answer from it is told, rather than left on a channel that
                // will never produce anything.
                lost.fail_everything(
                    &format!("{id} stopped responding"),
                    &format!("{id} left before it answered"),
                );
            }
        }
    }

    /// Everything selectable right now: built-ins, then whoever is attached.
    pub fn list(&self) -> Vec<Entry> {
        let mut state = self.lock();
        self.reap(&mut state);
        let active = state.active.clone();

        let mut rows: Vec<Entry> = self
            .builtins
            .iter()
            .map(|h| Entry {
                id: h.id().to_string(),
                name: h.name().to_string(),
                detail: None,
                builtin: true,
                active: h.id() == active,
                capabilities: h.capabilities(),
                pid: None,
            })
            .collect();

        let mut attached: Vec<&Attached> = state.attached.values().collect();
        attached.sort_by(|a, b| a.announced.id.cmp(&b.announced.id));
        for a in attached {
            rows.push(Entry {
                id: a.announced.id.clone(),
                name: a.announced.name.clone(),
                detail: a.announced.detail.clone(),
                builtin: false,
                active: a.announced.id == active,
                capabilities: Capabilities {
                    streaming: true,
                    tools: a.announced.tools,
                    memory: a.announced.memory,
                },
                pid: a.pid,
            });
        }
        rows
    }

    pub fn active_id(&self) -> String {
        self.lock().active.clone()
    }

    /// Choose which mind answers.
    pub fn set_active(&self, id: &str) -> Result<(), String> {
        let known: Vec<String> = self.list().into_iter().map(|e| e.id).collect();
        if !known.iter().any(|k| k == id) {
            return Err(format!(
                "no harness `{id}` is attached; this machine has: {}",
                known.join(", ")
            ));
        }
        self.lock().active = id.to_string();
        Ok(())
    }

    /// Put a turn to whichever harness is active — the Lens's question, to that mind's `main`.
    ///
    /// Returns immediately. A built-in answers on its own thread; an attached one is handed the
    /// turn by its next poll, once its `main` conversation has nothing else in flight.
    pub fn send(&self, turn: Turn) -> Answer {
        let active = self.active_id();

        if let Some(builtin) = self.builtin(&active) {
            return builtin.send(turn);
        }
        if active.is_empty() {
            return failed("no harness is attached to answer this".to_string());
        }
        // Said rather than left silent: a question typed into a panel whose harness has gone
        // should come back with that, not with nothing.
        self.send_to(&AgentId::new(&active, AgentId::MAIN), turn).unwrap_or_else(failed)
    }

    // ── Agents ──────────────────────────────────────────────────────

    /// Start a new agent on a harness: a new conversation, under an id nobody has had before.
    ///
    /// A harness that holds one conversation has the one agent `<id>:main`; it is started here if
    /// it is not already running, and asking for a second is refused with a sentence saying so
    /// rather than handing back the same conversation under a new name. Refused, too, when
    /// [`protocol::MAX_LIVE_AGENTS`] are already live, and for a built-in, which answers in the
    /// Lens and holds no agents.
    pub fn start_agent(&self, harness_id: &str) -> Result<AgentId, String> {
        if self.builtin(harness_id).is_some() {
            return Err(format!(
                "`{harness_id}` is built into the shell and answers in the Lens; it cannot be \
                 started as an agent"
            ));
        }
        let mut state = self.lock();
        self.reap(&mut state);
        let st = &mut *state;
        let live: usize = st.attached.values().map(|a| a.agents.len()).sum();
        let Some(harness) = st.attached.get_mut(harness_id) else {
            let mut known: Vec<&String> = st.attached.keys().collect();
            known.sort();
            return Err(if known.is_empty() {
                format!("no harness `{harness_id}` is attached, and none is")
            } else {
                format!(
                    "no harness `{harness_id}` is attached; attached: {}",
                    known.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(", ")
                )
            });
        };
        if !harness.announced.conversations && harness.agents.contains_key(AgentId::MAIN) {
            return Err(format!(
                "{} holds one conversation at a time, and it is already open as `{}`",
                harness.announced.name,
                AgentId::new(harness_id, AgentId::MAIN)
            ));
        }
        if live >= protocol::MAX_LIVE_AGENTS {
            return Err(format!(
                "{live} agents are already running, which is as many as this desktop runs at \
                 once (the most is {}); stop one to start another",
                protocol::MAX_LIVE_AGENTS
            ));
        }
        let conversation = if harness.announced.conversations {
            issue_conversation(&mut st.issued)?
        } else {
            AgentId::MAIN.to_string()
        };
        harness.agents.insert(conversation.clone(), Agent::new(mint_token()?));
        Ok(AgentId::new(harness_id, &conversation))
    }

    /// Put a turn to one agent. Returns immediately; chunks and events arrive on the channel.
    ///
    /// Refused for an agent that is not live — stopped, or started on a harness that has since
    /// restarted — because a conversation that no longer exists cannot be continued, and quietly
    /// starting a new one under the old name would be a pretence. `<harness>:main` is always
    /// there to be talked to while its harness is attached.
    pub fn send_to(&self, agent: &AgentId, turn: Turn) -> Result<Answer, String> {
        let harness_id = agent.harness();
        if self.builtin(harness_id).is_some() {
            return Err(format!(
                "`{harness_id}` is built into the shell and answers in the Lens; it holds no agents"
            ));
        }
        let conversation = agent.conversation().to_string();
        let mut state = self.lock();
        self.reap(&mut state);
        let st = &mut *state;
        let harness = st
            .attached
            .get_mut(harness_id)
            .ok_or_else(|| format!("`{harness_id}` is no longer attached, so `{agent}` cannot answer"))?;
        if !harness.agents.contains_key(&conversation) {
            if conversation != AgentId::MAIN {
                return Err(format!(
                    "there is no agent `{agent}` any more: it was stopped, or `{harness_id}` \
                     restarted since it began"
                ));
            }
            harness.agents.insert(conversation.clone(), Agent::new(mint_token()?));
        }
        let live = harness.agents.get_mut(&conversation).expect("present or just made");
        live.turns += 1;
        let agent_token = live.token.clone();

        let turn_id = st.next_turn;
        st.next_turn += 1;
        let (tx, rx) = mpsc::channel();
        harness.queued.push_back(Waiting {
            assignment: Assignment {
                turn_id,
                text: turn.text,
                context: turn.context,
                conversation,
                agent_token,
            },
            tx,
        });
        Ok(rx)
    }

    /// Every live agent, oldest first.
    pub fn agents(&self) -> Vec<AgentEntry> {
        let mut state = self.lock();
        self.reap(&mut state);
        let mut rows = Vec::new();
        for (harness_id, harness) in &state.attached {
            for (conversation, agent) in &harness.agents {
                let flights: Vec<&Flight> = harness
                    .in_flight
                    .values()
                    .filter(|f| f.conversation == *conversation && f.tx.is_some())
                    .collect();
                let queued = harness
                    .queued
                    .iter()
                    .filter(|w| w.assignment.conversation == *conversation)
                    .count();
                let running = flights.iter().find_map(|f| f.running_tool());
                let state = match (running, flights.is_empty(), queued) {
                    (Some((call, name)), _, _) => AgentState::RunningTool { call, name },
                    (None, false, _) => AgentState::Thinking,
                    (None, true, 0) => AgentState::Idle,
                    (None, true, _) => AgentState::Queued,
                };
                rows.push(AgentEntry {
                    id: AgentId::new(harness_id, conversation),
                    harness: harness_id.clone(),
                    harness_name: harness.announced.name.clone(),
                    conversations: harness.announced.conversations,
                    state,
                    started: agent.started,
                    turns: agent.turns,
                    queued: if flights.is_empty() { queued.saturating_sub(1) } else { queued },
                    last: agent.last.clone(),
                });
            }
        }
        rows.sort_by(|a, b| a.started.cmp(&b.started).then_with(|| a.id.cmp(&b.id)));
        rows
    }

    /// Which live agent a token names, and the pid of the harness process that holds it.
    ///
    /// The token comes from the caller and the pid from the kernel at attach; the shell checks
    /// that the caller descends from that pid before it believes the token.
    pub fn agent_for_token(&self, token: &str) -> Option<(AgentId, Option<u32>)> {
        let token = token.trim();
        if token.len() != TOKEN_HEX_LEN {
            return None;
        }
        let mut state = self.lock();
        self.reap(&mut state);
        for (harness_id, harness) in &state.attached {
            for (conversation, agent) in &harness.agents {
                if same_secret(&agent.token, token) {
                    return Some((AgentId::new(harness_id, conversation), harness.pid));
                }
            }
        }
        None
    }

    /// Whether an attached harness holds a conversation per agent — `None` when nothing by that id
    /// is attached. A role from the agent catalog is only ever started as a conversation of its
    /// own: a harness that holds one has only the person's own conversation to offer.
    pub fn holds_conversations(&self, harness_id: &str) -> Option<bool> {
        let mut state = self.lock();
        self.reap(&mut state);
        state.attached.get(harness_id).map(|h| h.announced.conversations)
    }

    /// Hand a live agent's token to `f`, for the one thing the shell derives from it: the one-way
    /// digest it publishes an agent's reach under (`yantrik_ipc_transport::reach`). The token
    /// itself goes no further than `f`. `None` for an agent that is not live.
    pub fn with_agent_token<R>(&self, agent: &AgentId, f: impl FnOnce(&str) -> R) -> Option<R> {
        let mut state = self.lock();
        self.reap(&mut state);
        let token = state
            .attached
            .get(agent.harness())
            .and_then(|harness| harness.agents.get(agent.conversation()))
            .map(|live| live.token.clone())?;
        drop(state);
        Some(f(&token))
    }

    /// Leave a note for an agent's next turn: something the desktop has to tell it that no call
    /// of its own carried — a command it ran that finished after the call that started it had
    /// returned.
    ///
    /// The note travels in the `context` of the next turn handed to that agent, under `notes`
    /// (see [`protocol::Assignment::context`]), and is delivered once. Not on `/stop` or `/new`,
    /// which a harness answers itself without asking the mind; the turn after them carries it.
    /// An agent holds at most [`MAX_NOTES`], each at most [`MAX_NOTE_BYTES`], and its notes end
    /// with it: stopped, its harness restarted or gone, they are dropped.
    ///
    /// Returns whether the note was kept — `false` for an agent that is not live, or a note with
    /// nothing in it.
    pub fn note_for(&self, agent: &AgentId, note: String) -> bool {
        let note = clip_note(note.trim());
        if note.is_empty() {
            return false;
        }
        let mut state = self.lock();
        self.reap(&mut state);
        let Some(live) = state
            .attached
            .get_mut(agent.harness())
            .and_then(|harness| harness.agents.get_mut(agent.conversation()))
        else {
            return false;
        };
        live.notes.push_back(note);
        while live.notes.len() > MAX_NOTES {
            live.notes.pop_front();
            live.notes_dropped += 1;
        }
        true
    }

    /// Stop an agent: the turns waiting for it are failed, the one in flight is settled for the
    /// reader — open calls interrupted, then [`STOPPED`] — and the harness is told on its next
    /// poll to stop working on it and, for a harness with conversations, to let the conversation
    /// go. The agent is no longer live, its token no longer names it, and its place under the
    /// cap is free. Returns whether there was anything to stop.
    pub fn stop_agent(&self, agent: &AgentId) -> bool {
        let mut state = self.lock();
        self.reap(&mut state);
        let Some(harness) = state.attached.get_mut(agent.harness()) else { return false };
        let conversation = agent.conversation();
        let existed = harness.agents.remove(conversation).is_some();
        let mut stopped = existed;

        let (theirs, others): (VecDeque<Waiting>, VecDeque<Waiting>) = std::mem::take(&mut harness.queued)
            .into_iter()
            .partition(|w| w.assignment.conversation == conversation);
        harness.queued = others;
        for waiting in theirs {
            let _ = waiting.tx.send(Chunk::Failed(STOPPED.to_string()));
            stopped = true;
        }
        for (turn_id, flight) in harness.in_flight.iter_mut() {
            if flight.conversation == conversation && flight.tx.is_some() {
                flight.abandon(Some(STOPPED.to_string()));
                harness.cancelled.push(*turn_id);
                stopped = true;
            }
        }
        if existed && harness.announced.conversations {
            harness.ended.push(conversation.to_string());
        }
        stopped
    }

    /// What became of the events harnesses sent.
    pub fn event_counts(&self) -> EventCounts {
        self.lock().events
    }

    /// Health of one harness, for the picker.
    pub fn health(&self, id: &str) -> Health {
        if let Some(builtin) = self.builtin(id) {
            return builtin.health();
        }
        let mut state = self.lock();
        self.reap(&mut state);
        match state.attached.get(id) {
            Some(_) => Health::Ready,
            None => Health::Unreachable("not attached".into()),
        }
    }

    // ── The wire ────────────────────────────────────────────────────

    /// Answer one protocol call. The shell wires this to the `harness` socket.
    pub fn handle(&self, method: &str, params: &serde_json::Value) -> Result<serde_json::Value, String> {
        self.handle_from(method, params, None)
    }

    /// The same, told which process is on the other end of the socket, as the kernel says it.
    ///
    /// Only `attach` uses it: the pid of the harness process is recorded then, so that a caller
    /// later presenting one of its agents' tokens can be checked against it.
    pub fn handle_from(
        &self,
        method: &str,
        params: &serde_json::Value,
        peer_pid: Option<u32>,
    ) -> Result<serde_json::Value, String> {
        match method {
            protocol::ATTACH => self.attach(params, peer_pid),
            protocol::POLL => self.poll(params),
            protocol::CHUNK => self.chunk(params),
            protocol::EVENT => self.event(params),
            protocol::COMPLETE => self.finish(params, None),
            protocol::FAIL => {
                let why = params["error"].as_str().unwrap_or("the harness reported a failure");
                self.finish(params, Some(why.to_string()))
            }
            protocol::DETACH => self.detach(params),
            other => Err(format!(
                "unknown method `{other}`; this service speaks: {}",
                protocol::METHODS.join(", ")
            )),
        }
    }

    fn attach(&self, params: &serde_json::Value, pid: Option<u32>) -> Result<serde_json::Value, String> {
        let announced: Attach = serde_json::from_value(params.clone())
            .map_err(|e| format!("`attach` needs at least an `id` and a `name`: {e}"))?;
        if announced.id.trim().is_empty() {
            return Err("`id` is what a person types to select this harness; it cannot be empty".into());
        }
        if announced.id.contains(char::is_whitespace) {
            return Err(format!(
                "`id` cannot contain spaces — it is what a person types to select this harness (`{}`)",
                announced.id
            ));
        }
        if announced.id.contains(':') {
            // An agent is `<harness>:<conversation>`; a colon in the harness would make that
            // ambiguous, and an id a person types has no use for one.
            return Err(format!("`id` cannot contain `:` (`{}`)", announced.id));
        }
        if self.builtin(&announced.id).is_some() {
            return Err(format!(
                "`{}` is a built-in harness; attach under a different id",
                announced.id
            ));
        }

        let mut state = self.lock();
        self.reap(&mut state);
        let session = format!("s{}", state.next_session);
        state.next_session += 1;

        // Re-attaching under an existing id replaces it, which is what a harness that restarted
        // should get. Anything the old one owed is failed rather than abandoned — the turn it was
        // answering and the turns waiting behind it — and its agents end with it: their
        // conversations lived in the process that is gone.
        if let Some(previous) = state.attached.remove(&announced.id) {
            previous.fail_everything(
                &format!("{} restarted mid-answer", announced.id),
                &format!("{} restarted before it answered", announced.id),
            );
        }

        let id = announced.id.clone();
        state.attached.insert(
            id.clone(),
            Attached {
                announced,
                session: session.clone(),
                last_seen: Instant::now(),
                pid,
                in_flight: HashMap::new(),
                queued: VecDeque::new(),
                agents: HashMap::new(),
                finished: VecDeque::new(),
                cancelled: Vec::new(),
                ended: Vec::new(),
            },
        );

        // The first mind to attach on a machine with no built-in becomes the one answering,
        // rather than leaving a desktop that has a harness and is not using it.
        if state.active.is_empty() {
            state.active = id;
        }
        Ok(serde_json::json!({ "session": session }))
    }

    /// Find the harness holding this session, refreshing its presence.
    fn touch<'a>(
        attached: &'a mut HashMap<String, Attached>,
        params: &serde_json::Value,
    ) -> Result<&'a mut Attached, String> {
        let session = params["session"].as_str().unwrap_or_default().to_string();
        if session.is_empty() {
            return Err("`session` is missing; call harness.attach first".into());
        }
        let harness = attached
            .values_mut()
            .find(|a| a.session == session)
            .ok_or_else(|| "this session is not attached any more; call harness.attach again".to_string())?;
        harness.last_seen = Instant::now();
        Ok(harness)
    }

    /// Hand over a turn if one is waiting, and say what the desktop has stopped waiting for.
    ///
    /// Does not block here: blocking is the caller's business, and holding the state lock would
    /// stop every other harness and the whole UI.
    ///
    /// The turn is the oldest one whose conversation has nothing in flight — first in, first out,
    /// one at a time per conversation. `/stop` alone skips the line.
    fn poll(&self, params: &serde_json::Value) -> Result<serde_json::Value, String> {
        let mut state = self.lock();
        self.reap(&mut state);
        let harness = Self::touch(&mut state.attached, params)?;

        let busy: HashSet<String> = harness
            .in_flight
            .values()
            .filter(|f| f.holds_conversation())
            .map(|f| f.conversation.clone())
            .collect();
        let next = harness
            .queued
            .iter()
            .position(|w| !busy.contains(&w.assignment.conversation) || interrupts(&w.assignment.text));

        let mut reply = match next.and_then(|i| harness.queued.remove(i)) {
            Some(Waiting { mut assignment, tx }) => {
                // What the desktop has to tell this agent rides on the turn that reaches its mind,
                // taken here rather than when the turn was queued, so a note that arrived while
                // the turn waited still goes with it.
                if !answered_by_the_harness(&assignment.text) {
                    if let Some(agent) = harness.agents.get_mut(&assignment.conversation) {
                        assignment.context = with_notes(assignment.context.take(), agent.take_notes());
                    }
                }
                harness
                    .in_flight
                    .insert(assignment.turn_id, Flight::new(assignment.conversation.clone(), tx));
                serde_json::to_value(&assignment).unwrap_or_else(|_| serde_json::json!({}))
            }
            // Nothing waiting is an ordinary answer, not an error: a harness polls far more often
            // than a person types.
            None => serde_json::json!({}),
        };
        if !harness.cancelled.is_empty() {
            reply["cancelled"] = serde_json::json!(std::mem::take(&mut harness.cancelled));
        }
        if !harness.ended.is_empty() {
            reply["ended"] = serde_json::json!(std::mem::take(&mut harness.ended));
        }
        Ok(reply)
    }

    fn chunk(&self, params: &serde_json::Value) -> Result<serde_json::Value, String> {
        let turn_id = params["turn_id"].as_u64().ok_or("`turn_id` must be a number")?;
        let delta = params["delta"].as_str().unwrap_or_default().to_string();
        let mut state = self.lock();
        let harness = Self::touch(&mut state.attached, params)?;
        let Some(flight) = harness.in_flight.get_mut(&turn_id) else {
            return Err(harness.not_in_flight(turn_id));
        };
        let Some(tx) = &flight.tx else {
            // Stopped, or nobody is listening: the harness should stop working on it. Its next
            // call on this turn — this one — is where it learns that.
            return Ok(serde_json::json!({ "dropped": true }));
        };
        // An empty delta is a heartbeat: the call has already refreshed this session's presence,
        // which is its whole purpose, and forwarding "" would only wake the panel for nothing.
        if delta.is_empty() {
            return Ok(serde_json::json!({}));
        }
        if tx.send(Chunk::Text(delta)).is_err() {
            // The panel stopped listening — the person closed it or asked something else.
            flight.abandon(None);
            return Ok(serde_json::json!({ "dropped": true }));
        }
        Ok(serde_json::json!({}))
    }

    /// One structured event for a turn in flight. See [`crate::event`] and the module docs for
    /// what is refused; a refusal is an answer (`{"refused": why}`), never an error, because an
    /// event that could not be shown is not a reason to lose the turn.
    fn event(&self, params: &serde_json::Value) -> Result<serde_json::Value, String> {
        let turn_id = params["turn_id"].as_u64().ok_or("`turn_id` must be a number")?;
        let raw = &params["event"];
        let mut state = self.lock();
        let st = &mut *state;
        let harness = Self::touch(&mut st.attached, params)?;
        let counts = &mut st.events;
        let who = harness.announced.id.clone();

        let Some(flight) = harness.in_flight.get_mut(&turn_id) else {
            if harness.finished.contains(&turn_id) {
                counts.late += 1;
                tracing::debug!(harness = %who, turn = turn_id, "event after the turn was closed; dropped");
                return Ok(refused(format!(
                    "turn {turn_id} is finished; events after complete or fail are dropped"
                )));
            }
            return Err(harness.not_in_flight(turn_id));
        };
        if flight.tx.is_none() {
            counts.late += 1;
            return Ok(serde_json::json!({ "dropped": true }));
        }

        let size = serde_json::to_string(raw).map(|s| s.len()).unwrap_or(usize::MAX);
        if size > protocol::MAX_EVENT_BYTES {
            counts.oversized += 1;
            tracing::warn!(harness = %who, turn = turn_id, bytes = size, "event over the size limit; refused");
            return Ok(refused(format!(
                "this event is {size} bytes and one event may be at most {}; send output as \
                 several tool_output events",
                protocol::MAX_EVENT_BYTES
            )));
        }
        let Some(kind) = raw.get("kind").and_then(|k| k.as_str()) else {
            counts.malformed += 1;
            tracing::warn!(harness = %who, turn = turn_id, "event with no `kind`; refused");
            return Ok(refused("an event needs a `kind`".to_string()));
        };
        if !Event::KINDS.contains(&kind) {
            // A newer harness talking to an older desktop. Not its fault, and not a reason to
            // stop listening to it.
            counts.unknown += 1;
            tracing::debug!(harness = %who, kind, "event of a kind this desktop does not know; ignored");
            return Ok(serde_json::json!({ "ignored": format!("`{kind}` is not a kind this desktop knows") }));
        }
        let event: Event = match serde_json::from_value(raw.clone()) {
            Ok(event) => event,
            Err(e) => {
                counts.malformed += 1;
                tracing::warn!(harness = %who, turn = turn_id, kind, error = %e, "malformed event; refused");
                return Ok(refused(format!("a malformed `{kind}` event: {e}")));
            }
        };
        if event.call().is_some_and(|call| call.trim().is_empty()) {
            counts.malformed += 1;
            tracing::warn!(harness = %who, turn = turn_id, kind, "tool event with an empty `call`; refused");
            return Ok(refused(format!("a `{kind}` event needs a `call` id")));
        }
        if let Err(why) = flight.admit(&event) {
            counts.out_of_order += 1;
            tracing::warn!(harness = %who, turn = turn_id, why = %why, "event out of order; refused");
            return Ok(refused(why));
        }
        let tx = flight.tx.as_ref().expect("checked above");
        if tx.send(Chunk::Event(event)).is_err() {
            flight.abandon(None);
            return Ok(serde_json::json!({ "dropped": true }));
        }
        counts.accepted += 1;
        Ok(serde_json::json!({}))
    }

    fn finish(&self, params: &serde_json::Value, failure: Option<String>) -> Result<serde_json::Value, String> {
        let turn_id = params["turn_id"].as_u64().ok_or("`turn_id` must be a number")?;
        let mut state = self.lock();
        let harness = Self::touch(&mut state.attached, params)?;
        let Some(mut flight) = harness.in_flight.remove(&turn_id) else {
            return Err(harness.not_in_flight(turn_id));
        };
        harness.remember_finished(turn_id);
        let abandoned = flight.tx.is_none();
        if !abandoned {
            if let Some(agent) = harness.agents.get_mut(&flight.conversation) {
                agent.last = Some(match &failure {
                    Some(why) => TurnEnd::Failed(why.clone()),
                    None => TurnEnd::Completed,
                });
            }
        }
        flight.settle(failure);
        Ok(if abandoned { serde_json::json!({ "dropped": true }) } else { serde_json::json!({}) })
    }

    fn detach(&self, params: &serde_json::Value) -> Result<serde_json::Value, String> {
        let mut state = self.lock();
        let id = {
            let harness = Self::touch(&mut state.attached, params)?;
            harness.announced.id.clone()
        };
        if let Some(gone) = state.attached.remove(&id) {
            gone.fail_everything(&format!("{id} detached mid-answer"), &format!("{id} detached before answering"));
        }
        Ok(serde_json::json!({}))
    }
}

/// An answer that is already over, with the reason.
fn failed(why: String) -> Answer {
    let (tx, rx) = mpsc::channel();
    let _ = tx.send(Chunk::Failed(why));
    rx
}

fn refused(why: String) -> serde_json::Value {
    serde_json::json!({ "refused": why })
}

/// Whether a turn is the one message that interrupts the turn in flight instead of waiting for it.
fn interrupts(text: &str) -> bool {
    text.split_whitespace().next().is_some_and(|word| word.eq_ignore_ascii_case("/stop"))
}

/// Whether a harness answers this turn itself rather than handing it to its mind: `/stop` and
/// `/new` (`harnesses/lib/yantrik_harness.py`, `_command`). A note put on one would never reach
/// the model, so notes wait for the turn after it.
fn answered_by_the_harness(text: &str) -> bool {
    text.split_whitespace()
        .next()
        .is_some_and(|word| word.eq_ignore_ascii_case("/stop") || word.eq_ignore_ascii_case("/new"))
}

/// A note, cut to [`MAX_NOTE_BYTES`] on a character boundary, saying so when it was.
fn clip_note(note: &str) -> String {
    if note.len() <= MAX_NOTE_BYTES {
        return note.to_string();
    }
    const MARK: &str = " … (cut)";
    let mut end = MAX_NOTE_BYTES - MARK.len();
    while !note.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{MARK}", &note[..end])
}

/// A turn's `context` with `notes` added to it.
///
/// The context is a JSON object in a string (`{"machine": …}`), so the notes go in as its
/// `notes` array, after any already there. A context that is not an object — something a caller
/// wrote as prose — is kept whole under `framing` rather than thrown away. No notes, no change.
fn with_notes(context: Option<String>, notes: Vec<String>) -> Option<String> {
    use serde_json::Value;
    if notes.is_empty() {
        return context;
    }
    let mut object = match context.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        None => serde_json::Map::new(),
        Some(text) => match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(map)) => map,
            _ => {
                let mut map = serde_json::Map::new();
                map.insert("framing".to_string(), Value::String(text.to_string()));
                map
            }
        },
    };
    let mut all: Vec<Value> = match object.remove("notes") {
        Some(Value::Array(earlier)) => earlier,
        Some(Value::Null) | None => Vec::new(),
        Some(other) => vec![other],
    };
    all.extend(notes.into_iter().map(Value::String));
    object.insert("notes".to_string(), Value::Array(all));
    Some(Value::Object(object).to_string())
}

/// An agent token: 128 random bits, as 32 lowercase hex digits.
const TOKEN_HEX_LEN: usize = 32;

fn mint_token() -> Result<String, String> {
    random_hex(TOKEN_HEX_LEN / 2)
}

/// A conversation id this host has never issued: `c-` and 24 random bits.
fn issue_conversation(issued: &mut HashSet<String>) -> Result<String, String> {
    for _ in 0..64 {
        let id = format!("c-{}", random_hex(3)?);
        if issued.insert(id.clone()) {
            return Ok(id);
        }
    }
    Err("could not find an unused conversation id".to_string())
}

/// Bytes from the operating system's random source, as hex. Never a clock, a counter or a
/// seeded generator: a token that could be guessed would name somebody else's agent.
fn random_hex(bytes: usize) -> Result<String, String> {
    let mut buf = vec![0u8; bytes];
    getrandom::getrandom(&mut buf)
        .map_err(|e| format!("this machine would not give the desktop random bytes: {e}"))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// Compare two secrets without stopping at the first difference.
fn same_secret(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

/// Wait for a turn, for a harness client that wants one call rather than a loop.
///
/// Lives here so the polling interval is defined once, by the side that knows what the timeout
/// means, rather than guessed at by every harness author.
pub fn poll_interval() -> Duration {
    Duration::from_millis(protocol::POLL_INTERVAL_MS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::mpsc;

    struct Builtin;

    impl Harness for Builtin {
        fn id(&self) -> &str {
            "companion"
        }
        fn name(&self) -> &str {
            "Companion"
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities { streaming: true, tools: true, memory: true }
        }
        fn health(&self) -> Health {
            Health::Ready
        }
        fn send(&self, _turn: Turn) -> Answer {
            let (tx, rx) = mpsc::channel();
            tx.send(Chunk::Text("from the companion".into())).ok();
            rx
        }
    }

    fn host() -> Host {
        Host::new(vec![Arc::new(Builtin)])
    }

    fn attach(host: &Host, id: &str) -> String {
        let reply = host
            .handle(protocol::ATTACH, &serde_json::json!({ "id": id, "name": id }))
            .unwrap();
        reply["session"].as_str().unwrap().to_string()
    }

    /// A harness that holds many conversations, as pi does.
    fn attach_many(host: &Host, id: &str) -> String {
        let reply = host
            .handle(protocol::ATTACH, &json!({ "id": id, "name": id, "conversations": true }))
            .unwrap();
        reply["session"].as_str().unwrap().to_string()
    }

    fn poll(host: &Host, session: &str) -> serde_json::Value {
        host.handle(protocol::POLL, &json!({ "session": session })).unwrap()
    }

    fn event(host: &Host, session: &str, turn_id: u64, event: serde_json::Value) -> Result<serde_json::Value, String> {
        host.handle(protocol::EVENT, &json!({ "session": session, "turn_id": turn_id, "event": event }))
    }

    fn complete(host: &Host, session: &str, turn_id: u64) -> serde_json::Value {
        host.handle(protocol::COMPLETE, &json!({ "session": session, "turn_id": turn_id })).unwrap()
    }

    fn start(call: &str) -> serde_json::Value {
        json!({ "kind": "tool_start", "call": call, "name": "os_act", "target": "terminal.run", "args": {"command": "ls"} })
    }

    fn output(call: &str, delta: &str) -> serde_json::Value {
        json!({ "kind": "tool_output", "call": call, "delta": delta })
    }

    fn end(call: &str) -> serde_json::Value {
        json!({ "kind": "tool_end", "call": call, "ok": true, "summary": "done", "exit_code": 0 })
    }

    /// Everything the reader has been sent so far, without waiting.
    fn so_far(answer: &Answer) -> Vec<Chunk> {
        answer.try_iter().collect()
    }

    fn closed(answer: &Answer) -> bool {
        matches!(answer.try_recv(), Err(mpsc::TryRecvError::Disconnected))
    }

    fn interrupted(call: &str) -> Chunk {
        Chunk::Event(Event::ToolEnd { call: call.into(), ok: false, summary: INTERRUPTED.into(), exit_code: None })
    }

    /// A live turn in flight on a conversations harness: (session, agent, turn id, answer).
    fn turn_in_flight(host: &Host) -> (String, AgentId, u64, Answer) {
        let session = attach_many(host, "pi");
        let agent = host.start_agent("pi").unwrap();
        let answer = host.send_to(&agent, Turn::new("tidy the photos")).unwrap();
        let turn_id = poll(host, &session)["turn_id"].as_u64().unwrap();
        (session, agent, turn_id, answer)
    }

    #[test]
    fn a_harness_exists_because_it_attached_not_because_it_was_configured() {
        let host = host();
        assert_eq!(host.list().len(), 1, "only the built-in to begin with");
        attach(&host, "mind");
        let ids: Vec<String> = host.list().into_iter().map(|e| e.id).collect();
        assert_eq!(ids, vec!["companion", "mind"]);
    }

    #[test]
    fn a_whole_turn_goes_out_and_comes_back_in_pieces() {
        let host = host();
        let session = attach(&host, "mind");
        host.set_active("mind").unwrap();

        let answer = host.send(Turn::new("what is open?"));

        // The harness collects it.
        let assignment = host
            .handle(protocol::POLL, &serde_json::json!({ "session": session }))
            .unwrap();
        let turn_id = assignment["turn_id"].as_u64().unwrap();
        assert_eq!(assignment["text"], "what is open?");

        for delta in ["Two ", "windows."] {
            host.handle(
                protocol::CHUNK,
                &serde_json::json!({ "session": session, "turn_id": turn_id, "delta": delta }),
            )
            .unwrap();
        }
        host.handle(
            protocol::COMPLETE,
            &serde_json::json!({ "session": session, "turn_id": turn_id }),
        )
        .unwrap();

        assert_eq!(crate::collect(answer).unwrap(), "Two windows.");
    }

    #[test]
    fn nothing_waiting_is_an_answer_not_an_error() {
        // A harness polls far more often than a person types.
        let host = host();
        let session = attach(&host, "mind");
        let reply = host.handle(protocol::POLL, &serde_json::json!({ "session": session })).unwrap();
        assert_eq!(reply, serde_json::json!({}));
    }

    #[test]
    fn a_failure_from_the_harness_reaches_the_person_who_asked() {
        let host = host();
        let session = attach(&host, "mind");
        host.set_active("mind").unwrap();
        let answer = host.send(Turn::new("hi"));
        let assignment =
            host.handle(protocol::POLL, &serde_json::json!({ "session": session })).unwrap();
        host.handle(
            protocol::FAIL,
            &serde_json::json!({
                "session": session,
                "turn_id": assignment["turn_id"],
                "error": "my model is not loaded",
            }),
        )
        .unwrap();
        assert_eq!(crate::collect(answer).unwrap_err(), "my model is not loaded");
    }

    #[test]
    fn asking_a_harness_that_left_says_so_rather_than_hanging() {
        // The worst failure this design could have: a question typed into a panel whose harness
        // has gone, answered by silence for ever.
        let host = host();
        let session = attach(&host, "mind");
        host.set_active("mind").unwrap();
        host.handle(protocol::DETACH, &serde_json::json!({ "session": session })).unwrap();

        let answer = host.send(Turn::new("anyone there?"));
        assert!(crate::collect(answer).unwrap_err().contains("no longer attached"));
    }

    /// An empty chunk keeps a long turn's presence and adds nothing to the answer.
    #[test]
    fn an_empty_chunk_is_a_heartbeat_not_text() {
        let host = host();
        let session = attach(&host, "mind");
        host.set_active("mind").unwrap();
        let answer = host.send(Turn::new("hi"));
        let assignment =
            host.handle(protocol::POLL, &serde_json::json!({ "session": session })).unwrap();
        let turn_id = assignment["turn_id"].as_u64().unwrap();
        for delta in ["", "Hello", "", " there"] {
            host.handle(
                protocol::CHUNK,
                &serde_json::json!({ "session": session, "turn_id": turn_id, "delta": delta }),
            )
            .unwrap();
        }
        host.handle(protocol::COMPLETE, &serde_json::json!({ "session": session, "turn_id": turn_id }))
            .unwrap();
        assert_eq!(crate::collect(answer).unwrap(), "Hello there");
    }

    #[test]
    fn a_harness_that_detaches_mid_answer_does_not_leave_the_asker_waiting() {
        let host = host();
        let session = attach(&host, "mind");
        host.set_active("mind").unwrap();
        let answer = host.send(Turn::new("hi"));
        let assignment =
            host.handle(protocol::POLL, &serde_json::json!({ "session": session })).unwrap();
        host.handle(
            protocol::CHUNK,
            &serde_json::json!({ "session": session, "turn_id": assignment["turn_id"], "delta": "I was" }),
        )
        .unwrap();
        host.handle(protocol::DETACH, &serde_json::json!({ "session": session })).unwrap();
        assert!(crate::collect(answer).unwrap_err().contains("detached mid-answer"));
    }

    #[test]
    fn a_restarted_harness_replaces_itself_and_owes_nothing() {
        let host = host();
        let first = attach(&host, "mind");
        host.set_active("mind").unwrap();
        let answer = host.send(Turn::new("hi"));
        host.handle(protocol::POLL, &serde_json::json!({ "session": first })).unwrap();

        // It crashes and comes back. The old session is dead and the old turn is not left hanging.
        let second = attach(&host, "mind");
        assert_ne!(first, second);
        assert!(crate::collect(answer).unwrap_err().contains("restarted"));
        assert_eq!(host.list().iter().filter(|e| e.id == "mind").count(), 1);
    }

    #[test]
    fn a_stale_session_is_told_to_attach_again() {
        let host = host();
        let err = host
            .handle(protocol::POLL, &serde_json::json!({ "session": "s404" }))
            .unwrap_err();
        assert!(err.contains("attach"), "{err}");
    }

    #[test]
    fn nothing_can_attach_over_a_builtin() {
        let host = host();
        let err = host
            .handle(protocol::ATTACH, &serde_json::json!({ "id": "companion", "name": "Impostor" }))
            .unwrap_err();
        assert!(err.contains("built-in"), "{err}");
        assert_eq!(host.list().len(), 1);
    }

    #[test]
    fn an_id_a_person_could_not_type_is_refused() {
        let host = host();
        assert!(host
            .handle(protocol::ATTACH, &serde_json::json!({ "id": "my mind", "name": "X" }))
            .unwrap_err()
            .contains("spaces"));
        assert!(host
            .handle(protocol::ATTACH, &serde_json::json!({ "id": "", "name": "X" }))
            .unwrap_err()
            .contains("cannot be empty"));
        // An agent is `<harness>:<conversation>`; a colon in the harness would blur the two.
        assert!(host
            .handle(protocol::ATTACH, &serde_json::json!({ "id": "pi:c-1", "name": "X" }))
            .unwrap_err()
            .contains(':'));
    }

    #[test]
    fn selecting_something_not_attached_names_what_is() {
        let host = host();
        attach(&host, "mind");
        let err = host.set_active("hermes").unwrap_err();
        assert!(err.contains("companion"), "{err}");
        assert!(err.contains("mind"), "{err}");
    }

    #[test]
    fn a_builtin_still_answers_the_way_it_always_did() {
        let host = host();
        assert_eq!(host.active_id(), "companion");
        assert_eq!(crate::collect(host.send(Turn::new("hi"))).unwrap(), "from the companion");
    }

    #[test]
    fn a_chunk_for_a_turn_the_harness_was_never_given_is_refused() {
        let host = host();
        let session = attach(&host, "mind");
        let err = host
            .handle(
                protocol::CHUNK,
                &serde_json::json!({ "session": session, "turn_id": 999, "delta": "x" }),
            )
            .unwrap_err();
        assert!(err.contains("not one this harness was given"), "{err}");
    }

    #[test]
    fn an_unknown_method_lists_the_real_ones() {
        let host = host();
        let err = host.handle("harness.think", &serde_json::json!({})).unwrap_err();
        for method in protocol::METHODS {
            assert!(err.contains(method), "{err}");
        }
    }

    #[test]
    fn capabilities_are_what_the_harness_claimed_for_itself() {
        let host = host();
        host.handle(
            protocol::ATTACH,
            &serde_json::json!({ "id": "mind", "name": "Mind", "tools": true, "detail": "qwen2.5" }),
        )
        .unwrap();
        let row = host.list().into_iter().find(|e| e.id == "mind").unwrap();
        assert!(row.capabilities.tools);
        assert!(!row.capabilities.memory);
        assert_eq!(row.detail.as_deref(), Some("qwen2.5"));
        assert!(!row.builtin);
        assert_eq!(row.pid, None, "`handle` brings no peer for the kernel to name");
    }

    // ── Order: first in, first out, one at a time per conversation ─────
    //
    // These three are written against the API that existed before agents, so they can be run
    // against the old host and seen to fail there.

    #[test]
    fn turns_are_handed_out_in_the_order_they_were_asked() {
        // `poll` used `queued.pop()`, which hands out the NEWEST turn: three questions typed in a
        // row were answered third, second, first.
        let host = host();
        let session = attach(&host, "mind");
        host.set_active("mind").unwrap();
        let _answers: Vec<Answer> =
            ["first", "second", "third"].iter().map(|t| host.send(Turn::new(*t))).collect();

        let mut order = Vec::new();
        for _ in 0..3 {
            let assignment = host.handle(protocol::POLL, &json!({ "session": session })).unwrap();
            order.push(assignment["text"].as_str().unwrap_or("(nothing handed out)").to_string());
            host.handle(protocol::COMPLETE, &json!({ "session": session, "turn_id": assignment["turn_id"] }))
                .unwrap();
        }
        assert_eq!(order, ["first", "second", "third"]);
    }

    #[test]
    fn a_conversation_is_handed_one_turn_at_a_time() {
        let host = host();
        let session = attach(&host, "mind");
        host.set_active("mind").unwrap();
        let _first = host.send(Turn::new("first"));
        let _second = host.send(Turn::new("second"));

        let handed = host.handle(protocol::POLL, &json!({ "session": session })).unwrap();
        assert_eq!(handed["text"], "first");
        // The second waits for the first, rather than arriving mid-answer to be told "still
        // working on the previous request".
        let meanwhile = host.handle(protocol::POLL, &json!({ "session": session })).unwrap();
        assert!(meanwhile.get("turn_id").is_none(), "handed out mid-turn: {meanwhile}");

        host.handle(protocol::COMPLETE, &json!({ "session": session, "turn_id": handed["turn_id"] }))
            .unwrap();
        let next = host.handle(protocol::POLL, &json!({ "session": session })).unwrap();
        assert_eq!(next["text"], "second");
    }

    #[test]
    fn a_harness_that_restarts_fails_the_turns_still_waiting_too() {
        // It used to fail only the turn in flight. The ones queued behind it were dropped with
        // the old registration, which closed their channels — and a closed channel is how a
        // reader learns an answer is COMPLETE. They "succeeded" with no text.
        let host = host();
        let session = attach(&host, "mind");
        host.set_active("mind").unwrap();
        let answering = host.send(Turn::new("first"));
        let waiting = host.send(Turn::new("second"));
        host.handle(protocol::POLL, &json!({ "session": session })).unwrap();

        attach(&host, "mind");
        assert!(crate::collect(answering).unwrap_err().contains("restarted"));
        assert!(crate::collect(waiting).is_err(), "a turn that was never answered reads as answered");
    }

    #[test]
    fn harness_event_is_accepted_for_a_turn_in_flight_and_refused_once_it_is_closed() {
        let host = host();
        let session = attach(&host, "mind");
        host.set_active("mind").unwrap();
        let _answer = host.send(Turn::new("hi"));
        let handed = host.handle(protocol::POLL, &json!({ "session": session })).unwrap();
        let turn_id = handed["turn_id"].clone();
        let status = json!({ "kind": "status", "text": "looking" });

        let accepted = host
            .handle("harness.event", &json!({ "session": session, "turn_id": turn_id, "event": status }))
            .expect("the desktop takes an event for a turn in flight");
        assert_eq!(accepted, json!({}));

        host.handle(protocol::COMPLETE, &json!({ "session": session, "turn_id": turn_id })).unwrap();
        let late = host
            .handle("harness.event", &json!({ "session": session, "turn_id": turn_id, "event": status }))
            .expect("a late event is dropped, not an error");
        assert!(late["refused"].as_str().unwrap_or_default().contains("finished"), "{late}");
    }

    // ── Events ──────────────────────────────────────────────────────

    /// A host with no built-in, for the tests that only care about attached harnesses.
    fn host_with_nothing() -> Host {
        Host::new(vec![])
    }

    #[test]
    fn events_arrive_between_the_text_in_the_order_they_were_sent() {
        let host = host_with_nothing();
        let (session, _agent, turn_id, answer) = turn_in_flight(&host);
        let chunk = |delta: &str| {
            host.handle(protocol::CHUNK, &json!({ "session": session, "turn_id": turn_id, "delta": delta }))
                .unwrap()
        };

        chunk("Looking for duplicates.\n");
        assert_eq!(event(&host, &session, turn_id, start("t1")).unwrap(), json!({}));
        assert_eq!(event(&host, &session, turn_id, output("t1", "a.jpg\n")).unwrap(), json!({}));
        assert_eq!(event(&host, &session, turn_id, end("t1")).unwrap(), json!({}));
        chunk("Done.");
        complete(&host, &session, turn_id);

        let chunks: Vec<Chunk> = answer.iter().collect();
        assert_eq!(chunks.len(), 5, "{chunks:?}");
        assert_eq!(chunks[0], Chunk::Text("Looking for duplicates.\n".into()));
        assert!(matches!(&chunks[1], Chunk::Event(Event::ToolStart { call, target, .. }) if call == "t1" && target == "terminal.run"));
        assert!(matches!(&chunks[2], Chunk::Event(Event::ToolOutput { delta, .. }) if delta == "a.jpg\n"));
        assert!(matches!(&chunks[3], Chunk::Event(Event::ToolEnd { ok: true, exit_code: Some(0), .. })));
        assert_eq!(chunks[4], Chunk::Text("Done.".into()));
        assert_eq!(host.event_counts().accepted, 3);
    }

    #[test]
    fn an_event_for_a_turn_not_yet_handed_out_is_refused() {
        let host = host_with_nothing();
        let session = attach_many(&host, "pi");
        let agent = host.start_agent("pi").unwrap();
        let _answer = host.send_to(&agent, Turn::new("hi")).unwrap();
        // Queued, not polled: the harness does not hold it, so it cannot speak for it.
        let err = event(&host, &session, 1, start("t1")).unwrap_err();
        assert!(err.contains("not one this harness was given"), "{err}");
    }

    #[test]
    fn an_event_from_a_session_that_does_not_hold_the_turn_is_refused() {
        let host = host_with_nothing();
        let (_session, _agent, turn_id, answer) = turn_in_flight(&host);
        let other = attach(&host, "hermes");
        let err = event(&host, &other, turn_id, start("t1")).unwrap_err();
        assert!(err.contains("not one this harness was given"), "{err}");
        assert!(so_far(&answer).is_empty(), "another harness's event reached the asker");
    }

    #[test]
    fn a_calls_events_come_start_then_output_then_one_end() {
        let host = host_with_nothing();
        let (session, _agent, turn_id, answer) = turn_in_flight(&host);
        let refused_for = |e: serde_json::Value| {
            let reply = event(&host, &session, turn_id, e).unwrap();
            reply["refused"].as_str().map(str::to_string).unwrap_or_else(|| panic!("accepted: {reply}"))
        };

        assert!(refused_for(output("t1", "early")).contains("before its tool_start"));
        assert!(refused_for(end("t1")).contains("before its tool_start"));
        event(&host, &session, turn_id, start("t1")).unwrap();
        assert!(refused_for(start("t1")).contains("already started"));
        event(&host, &session, turn_id, end("t1")).unwrap();
        assert!(refused_for(end("t1")).contains("already ended"));
        assert!(refused_for(output("t1", "late")).contains("after its tool_end"));

        let counts = host.event_counts();
        assert_eq!((counts.out_of_order, counts.accepted), (5, 2));
        // The turn goes on: a refused event is not a reason to lose the answer.
        host.handle(protocol::CHUNK, &json!({ "session": session, "turn_id": turn_id, "delta": "still here" }))
            .unwrap();
        complete(&host, &session, turn_id);
        let chunks: Vec<Chunk> = answer.iter().collect();
        assert_eq!(chunks.len(), 3, "only the accepted start and end, and the text: {chunks:?}");
    }

    #[test]
    fn completing_with_a_call_still_open_settles_it_interrupted() {
        let host = host_with_nothing();
        let (session, _agent, turn_id, answer) = turn_in_flight(&host);
        event(&host, &session, turn_id, start("done")).unwrap();
        event(&host, &session, turn_id, end("done")).unwrap();
        event(&host, &session, turn_id, start("open-1")).unwrap();
        event(&host, &session, turn_id, start("open-2")).unwrap();
        complete(&host, &session, turn_id);

        let chunks: Vec<Chunk> = answer.iter().collect();
        // Only the two still open are settled, in the order they started, and the turn itself
        // completed: this is the harness's answer, with its loose ends tied.
        assert_eq!(&chunks[chunks.len() - 2..], &[interrupted("open-1"), interrupted("open-2")]);
        assert_eq!(chunks.iter().filter(|c| matches!(c, Chunk::Event(Event::ToolEnd { .. }))).count(), 3);
        assert!(!chunks.iter().any(|c| matches!(c, Chunk::Failed(_))));
    }

    #[test]
    fn failing_with_a_call_open_settles_the_call_before_saying_why() {
        let host = host_with_nothing();
        let (session, _agent, turn_id, answer) = turn_in_flight(&host);
        event(&host, &session, turn_id, start("t1")).unwrap();
        host.handle(protocol::FAIL, &json!({ "session": session, "turn_id": turn_id, "error": "model gone" }))
            .unwrap();
        let chunks: Vec<Chunk> = answer.iter().collect();
        assert_eq!(&chunks[1..], &[interrupted("t1"), Chunk::Failed("model gone".into())]);
    }

    #[test]
    fn a_harness_that_leaves_mid_call_settles_the_call() {
        let host = host_with_nothing();
        let (session, _agent, turn_id, answer) = turn_in_flight(&host);
        event(&host, &session, turn_id, start("t1")).unwrap();
        host.handle(protocol::DETACH, &json!({ "session": session })).unwrap();
        let chunks: Vec<Chunk> = answer.iter().collect();
        assert_eq!(chunks[1], interrupted("t1"));
        assert!(matches!(&chunks[2], Chunk::Failed(why) if why.contains("detached mid-answer")));
    }

    #[test]
    fn events_after_complete_or_fail_are_dropped_and_counted() {
        let host = host_with_nothing();
        let (session, _agent, turn_id, answer) = turn_in_flight(&host);
        complete(&host, &session, turn_id);
        for late in [start("t9"), json!({ "kind": "thinking", "delta": "hmm" })] {
            let reply = event(&host, &session, turn_id, late).unwrap();
            assert!(reply["refused"].as_str().unwrap().contains("finished"), "{reply}");
        }
        assert_eq!(host.event_counts().late, 2);
        assert!(so_far(&answer).is_empty(), "nothing reached the reader after the end");
        assert!(closed(&answer));
        // A chunk after the end is the harness closing a turn twice, and still an error.
        let err = host
            .handle(protocol::CHUNK, &json!({ "session": session, "turn_id": turn_id, "delta": "x" }))
            .unwrap_err();
        assert!(err.contains("already finished"), "{err}");
    }

    #[test]
    fn an_event_over_64_kib_is_refused_and_the_turn_goes_on() {
        let host = host_with_nothing();
        let (session, _agent, turn_id, answer) = turn_in_flight(&host);
        event(&host, &session, turn_id, start("t1")).unwrap();
        let heavy = output("t1", &"x".repeat(protocol::MAX_EVENT_BYTES));
        let reply = event(&host, &session, turn_id, heavy).unwrap();
        assert!(reply["refused"].as_str().unwrap().contains("at most 65536"), "{reply}");
        assert_eq!(host.event_counts().oversized, 1);

        // Just under the limit is fine.
        let fits = output("t1", &"x".repeat(protocol::MAX_EVENT_BYTES - 100));
        assert_eq!(event(&host, &session, turn_id, fits).unwrap(), json!({}));
        assert_eq!(so_far(&answer).len(), 2);
    }

    #[test]
    fn an_unknown_kind_is_ignored_and_a_malformed_known_kind_is_counted() {
        let host = host_with_nothing();
        let (session, _agent, turn_id, answer) = turn_in_flight(&host);

        // A newer harness: ignored, not refused, not counted against it.
        let newer = event(&host, &session, turn_id, json!({ "kind": "screenshot", "png": "…" })).unwrap();
        assert!(newer["ignored"].as_str().unwrap().contains("screenshot"), "{newer}");

        // A kind this build knows, missing what it needs: refused and counted.
        let broken = event(&host, &session, turn_id, json!({ "kind": "tool_end", "call": "t1" })).unwrap();
        assert!(broken["refused"].as_str().unwrap().contains("malformed `tool_end`"), "{broken}");
        let no_kind = event(&host, &session, turn_id, json!({ "call": "t1" })).unwrap();
        assert!(no_kind["refused"].is_string(), "{no_kind}");
        let no_call = event(&host, &session, turn_id, json!({ "kind": "tool_start", "call": " ", "name": "x" })).unwrap();
        assert!(no_call["refused"].as_str().unwrap().contains("`call`"), "{no_call}");

        let counts = host.event_counts();
        assert_eq!((counts.unknown, counts.malformed, counts.accepted), (1, 3, 0));
        assert!(so_far(&answer).is_empty());
        // The turn is intact.
        complete(&host, &session, turn_id);
        assert_eq!(crate::collect(answer).unwrap(), "");
    }

    #[test]
    fn a_turn_nobody_is_listening_to_keeps_its_conversation_until_the_harness_closes_it() {
        let host = host_with_nothing();
        let (session, agent, turn_id, answer) = turn_in_flight(&host);
        let _next = host.send_to(&agent, Turn::new("and then")).unwrap();
        drop(answer);

        // The event that finds nobody listening is how the host learns it; the ones after it are
        // late.
        let reply = event(&host, &session, turn_id, start("t1")).unwrap();
        assert_eq!(reply, json!({ "dropped": true }));
        assert_eq!(event(&host, &session, turn_id, end("t1")).unwrap(), json!({ "dropped": true }));
        assert_eq!(host.event_counts().late, 1);
        // The harness is still winding the first one down; the second must not land on top of it.
        assert!(poll(&host, &session).get("turn_id").is_none());
        assert_eq!(complete(&host, &session, turn_id), json!({ "dropped": true }));
        assert_eq!(poll(&host, &session)["text"], "and then");
    }

    // ── Agents ──────────────────────────────────────────────────────

    #[test]
    fn a_harness_with_conversations_gets_a_new_agent_each_time_and_they_run_at_once() {
        let host = host_with_nothing();
        let session = attach_many(&host, "pi");
        let first = host.start_agent("pi").unwrap();
        let second = host.start_agent("pi").unwrap();
        assert_ne!(first, second);
        for agent in [&first, &second] {
            assert_eq!(agent.harness(), "pi");
            let conversation = agent.conversation();
            assert!(conversation.starts_with("c-") && conversation.len() == 8, "{conversation}");
            assert!(conversation[2..].chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        }

        let _a = host.send_to(&first, Turn::new("tidy the photos")).unwrap();
        let _b = host.send_to(&second, Turn::new("write release notes")).unwrap();
        // Different conversations do not wait for each other.
        let one = poll(&host, &session);
        let two = poll(&host, &session);
        assert_eq!((one["conversation"].as_str(), two["conversation"].as_str()),
                   (Some(first.conversation()), Some(second.conversation())));
        assert_ne!(one["agent_token"], two["agent_token"]);
    }

    #[test]
    fn a_harness_without_conversations_has_the_one_agent_main_and_says_so() {
        let host = host_with_nothing();
        let session = attach(&host, "hermes");
        let agent = host.start_agent("hermes").unwrap();
        assert_eq!(agent.to_string(), "hermes:main");
        let err = host.start_agent("hermes").unwrap_err();
        assert!(err.contains("one conversation at a time") && err.contains("hermes:main"), "{err}");

        let _answer = host.send_to(&agent, Turn::new("hi")).unwrap();
        assert_eq!(poll(&host, &session)["conversation"], "main");
        let rows = host.agents();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].conversations);
    }

    #[test]
    fn the_lens_talks_to_the_answering_minds_main_agent() {
        let host = host();
        let session = attach_many(&host, "pi");
        host.set_active("pi").unwrap();
        let _answer = host.send(Turn::new("what is open?"));
        let handed = poll(&host, &session);
        assert_eq!(handed["conversation"], "main");
        assert_eq!(handed["agent_token"].as_str().unwrap().len(), 32);
        let ids: Vec<String> = host.agents().iter().map(|a| a.id.to_string()).collect();
        assert_eq!(ids, ["pi:main"]);
        // The same token for every turn of that conversation.
        complete(&host, &session, handed["turn_id"].as_u64().unwrap());
        let _again = host.send(Turn::new("and now?"));
        assert_eq!(poll(&host, &session)["agent_token"], handed["agent_token"]);
    }

    #[test]
    fn no_more_than_six_agents_run_at_once_and_the_refusal_says_so() {
        let host = host_with_nothing();
        attach_many(&host, "pi");
        attach_many(&host, "deepseek");
        let mut live = Vec::new();
        for n in 0..protocol::MAX_LIVE_AGENTS {
            live.push(host.start_agent(if n % 2 == 0 { "pi" } else { "deepseek" }).unwrap());
        }
        let err = host.start_agent("pi").unwrap_err();
        assert!(err.contains("6 agents are already running") && err.contains("stop one"), "{err}");
        assert_eq!(host.agents().len(), 6);

        assert!(host.stop_agent(&live[0]));
        assert!(host.start_agent("deepseek").is_ok(), "stopping one frees its place");
    }

    #[test]
    fn a_conversation_id_is_never_issued_twice() {
        let host = host_with_nothing();
        attach_many(&host, "pi");
        let mut seen = HashSet::new();
        for _ in 0..300 {
            let agent = host.start_agent("pi").unwrap();
            assert!(seen.insert(agent.to_string()), "{agent} issued twice");
            host.stop_agent(&agent);
        }
    }

    #[test]
    fn an_agent_token_names_its_agent_and_the_process_that_attached() {
        // A fabricated pid, held alive by an injected probe: this test is about tokens, not
        // presence, and the registry asks the kernel about real ones (#67).
        let host = host_with_nothing().with_liveness(|pid| pid == 4242);
        let session = host
            .handle_from(protocol::ATTACH, &json!({ "id": "pi", "name": "Pi", "conversations": true }), Some(4242))
            .unwrap()["session"]
            .as_str()
            .unwrap()
            .to_string();
        let first = host.start_agent("pi").unwrap();
        let second = host.start_agent("pi").unwrap();
        let _a = host.send_to(&first, Turn::new("one")).unwrap();
        let _b = host.send_to(&second, Turn::new("two")).unwrap();
        let token_one = poll(&host, &session)["agent_token"].as_str().unwrap().to_string();
        let token_two = poll(&host, &session)["agent_token"].as_str().unwrap().to_string();

        assert_eq!(token_one.len(), 32, "128 bits as hex");
        assert!(token_one.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(token_one, token_two);
        assert_eq!(host.agent_for_token(&token_one), Some((first.clone(), Some(4242))));
        assert_eq!(host.agent_for_token(&token_two), Some((second, Some(4242))));
        // The picker's row carries the same pid: the shell matches an approval's caller
        // against it, which is what told the real Pi apart from an impostor (#206).
        assert_eq!(host.list().into_iter().find(|e| e.id == "pi").unwrap().pid, Some(4242));

        // Not a prefix, not a guess, not empty.
        assert_eq!(host.agent_for_token(&token_one[..31]), None);
        assert_eq!(host.agent_for_token(&"0".repeat(32)), None);
        assert_eq!(host.agent_for_token(""), None);

        // A stopped agent's token names nothing.
        host.stop_agent(&first);
        assert_eq!(host.agent_for_token(&token_one), None);
    }

    /// Agents catalog: the shell asks whether a mind can give a role a conversation of its own, and
    /// derives the reach file's digest from an agent's token without the token being handed out.
    #[test]
    fn a_harness_says_whether_it_holds_conversations_and_a_live_agents_token_is_lent_not_given() {
        let host = host_with_nothing();
        let session = attach_many(&host, "pi");
        host.handle(protocol::ATTACH, &json!({ "id": "hermes", "name": "Hermes" })).unwrap();
        assert_eq!(host.holds_conversations("pi"), Some(true));
        assert_eq!(host.holds_conversations("hermes"), Some(false));
        assert_eq!(host.holds_conversations("openclaw"), None, "not attached");

        let agent = host.start_agent("pi").unwrap();
        let _a = host.send_to(&agent, Turn::new("one")).unwrap();
        let token = poll(&host, &session)["agent_token"].as_str().unwrap().to_string();
        assert_eq!(host.with_agent_token(&agent, |t| t == token), Some(true), "the same token the harness holds");
        host.stop_agent(&agent);
        assert_eq!(host.with_agent_token(&agent, |t| t.len()), None, "a stopped agent lends nothing");
    }

    #[test]
    fn a_harness_attached_without_peer_credentials_has_no_pid() {
        let host = host_with_nothing();
        let session = attach_many(&host, "pi");
        let agent = host.start_agent("pi").unwrap();
        let _a = host.send_to(&agent, Turn::new("one")).unwrap();
        let token = poll(&host, &session)["agent_token"].as_str().unwrap().to_string();
        assert_eq!(host.agent_for_token(&token), Some((agent, None)));
    }

    // ── Presence: the kernel's word about the process (#67) ─────────────

    /// A harness whose process died leaves the list at once rather than sitting out the grace of
    /// missed polls. `systemctl --user stop yantrik-pi.service` kills the attaching process, and
    /// the registry used to hold the session for up to 90 s — long enough for the Settings page
    /// to say "attached", offer *Use this*, and hand the next question to nobody.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_harness_whose_process_died_leaves_the_list_without_waiting_out_the_grace() {
        let host = host_with_nothing();
        let mut child = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .expect("the test machine can start a sleeper");
        host.handle_from(protocol::ATTACH, &json!({ "id": "pi", "name": "Pi" }), Some(child.id()))
            .unwrap();
        assert!(host.list().iter().any(|e| e.id == "pi"), "listed while its process lives");

        // Stopped the way systemctl stops it, and waited for the kernel to reap it. The wait is
        // on state, not on time: once `wait` returns, `/proc/<pid>` is gone.
        child.kill().expect("the sleeper can be killed");
        child.wait().expect("the sleeper can be waited for");

        let ids: Vec<String> = host.list().into_iter().map(|e| e.id).collect();
        assert!(!ids.contains(&"pi".to_string()), "still listed after its process died: {ids:?}");
        // And a question for it is refused with what is attached, not queued for a poll that
        // will never come.
        assert!(host.set_active("pi").is_err());
    }

    #[test]
    fn a_harness_attached_with_a_live_pid_stays_listed() {
        let host = host_with_nothing();
        // Its own pid, as the kernel reports for a harness started by hand from a terminal while
        // its unit file sits installed and stopped. Presence asks the process, never the unit.
        host.handle_from(protocol::ATTACH, &json!({ "id": "pi", "name": "Pi" }), Some(std::process::id()))
            .unwrap();
        assert!(host.list().iter().any(|e| e.id == "pi"));
        host.set_active("pi").unwrap();
        assert_eq!(host.active_id(), "pi");
    }

    #[test]
    fn a_restarted_harness_takes_its_agents_with_it() {
        let host = host_with_nothing();
        let session = attach_many(&host, "pi");
        let agent = host.start_agent("pi").unwrap();
        let _a = host.send_to(&agent, Turn::new("one")).unwrap();
        let token = poll(&host, &session)["agent_token"].as_str().unwrap().to_string();

        attach_many(&host, "pi");
        assert!(host.agents().is_empty());
        assert_eq!(host.agent_for_token(&token), None);
        let err = host.send_to(&agent, Turn::new("still there?")).unwrap_err();
        assert!(err.contains("no agent") && err.contains(&agent.to_string()), "{err}");
    }

    #[test]
    fn stopping_an_agent_fails_what_waits_settles_what_runs_and_tells_the_harness() {
        let host = host_with_nothing();
        let (session, agent, turn_id, answer) = turn_in_flight(&host);
        let bystander = host.start_agent("pi").unwrap();
        let waiting = host.send_to(&agent, Turn::new("and then")).unwrap();
        let _other = host.send_to(&bystander, Turn::new("unrelated")).unwrap();
        event(&host, &session, turn_id, start("t1")).unwrap();

        assert!(host.stop_agent(&agent));

        let chunks: Vec<Chunk> = answer.iter().collect();
        assert_eq!(chunks, vec![
            Chunk::Event(Event::ToolStart { call: "t1".into(), name: "os_act".into(), target: "terminal.run".into(), args: json!({"command": "ls"}) }),
            interrupted("t1"),
            Chunk::Failed(STOPPED.into()),
        ]);
        assert_eq!(crate::collect(waiting).unwrap_err(), STOPPED);

        // The harness learns on its next poll, alongside whatever else it is handed.
        let reply = poll(&host, &session);
        assert_eq!(reply["cancelled"], json!([turn_id]));
        assert_eq!(reply["ended"], json!([agent.conversation()]));
        assert_eq!(reply["text"], "unrelated", "the other agent is not held up");
        assert!(poll(&host, &session).get("cancelled").is_none(), "said once");

        // Whatever it still sends for the stopped turn is dropped, and closing it is accepted.
        let chunk = host
            .handle(protocol::CHUNK, &json!({ "session": session, "turn_id": turn_id, "delta": "" }))
            .unwrap();
        assert_eq!(chunk, json!({ "dropped": true }));
        assert_eq!(complete(&host, &session, turn_id), json!({ "dropped": true }));

        let ids: Vec<AgentId> = host.agents().into_iter().map(|a| a.id).collect();
        assert_eq!(ids, vec![bystander]);
        assert!(host.send_to(&agent, Turn::new("again")).is_err());
        assert!(!host.stop_agent(&agent), "nothing left to stop");
    }

    #[test]
    fn stop_interrupts_the_turn_in_flight_rather_than_waiting_behind_it() {
        let host = host();
        let session = attach(&host, "mind");
        host.set_active("mind").unwrap();
        let _long = host.send(Turn::new("a long job"));
        let _behind = host.send(Turn::new("another question"));
        let _stop = host.send(Turn::new("/stop"));
        assert_eq!(poll(&host, &session)["text"], "a long job");
        // /stop jumps the queue; the ordinary question still waits its turn.
        assert_eq!(poll(&host, &session)["text"], "/stop");
        assert!(poll(&host, &session).get("turn_id").is_none());
    }

    #[test]
    fn an_agent_says_what_it_is_doing() {
        let host = host_with_nothing();
        let session = attach_many(&host, "pi");
        let agent = host.start_agent("pi").unwrap();
        let state = || host.agents().into_iter().find(|a| a.id == agent).unwrap();
        assert_eq!(state().state, AgentState::Idle);

        let _answer = host.send_to(&agent, Turn::new("tidy")).unwrap();
        assert_eq!(state().state, AgentState::Queued);
        let turn_id = poll(&host, &session)["turn_id"].as_u64().unwrap();
        assert_eq!(state().state, AgentState::Thinking);
        event(&host, &session, turn_id, start("t1")).unwrap();
        assert_eq!(state().state, AgentState::RunningTool { call: "t1".into(), name: "os_act".into() });
        event(&host, &session, turn_id, end("t1")).unwrap();
        assert_eq!(state().state, AgentState::Thinking);
        complete(&host, &session, turn_id);
        let row = state();
        assert_eq!((row.state, row.last, row.turns, row.harness_name.as_str()),
                   (AgentState::Idle, Some(TurnEnd::Completed), 1, "pi"));
    }

    #[test]
    fn a_builtin_is_not_started_as_an_agent() {
        let host = host();
        let err = host.start_agent("companion").unwrap_err();
        assert!(err.contains("built into the shell"), "{err}");
        assert!(host.send_to(&AgentId::new("companion", "main"), Turn::new("hi")).is_err());
    }

    #[test]
    fn starting_an_agent_on_a_harness_that_is_not_attached_names_what_is() {
        let host = host_with_nothing();
        attach_many(&host, "pi");
        let err = host.start_agent("openclaw").unwrap_err();
        assert!(err.contains("openclaw") && err.contains("pi"), "{err}");
    }

    // ── Notes for an agent's next turn ──────────────────────────────

    /// The `notes` a handed-out turn carries in its context, if any.
    fn notes_in(assignment: &serde_json::Value) -> Vec<String> {
        let Some(context) = assignment["context"].as_str() else { return Vec::new() };
        let context: serde_json::Value = serde_json::from_str(context).expect("the context is JSON");
        context["notes"]
            .as_array()
            .map(|notes| notes.iter().map(|n| n.as_str().unwrap().to_string()).collect())
            .unwrap_or_default()
    }

    /// One turn for `agent`, handed out and closed: what the harness was given.
    fn next_turn(host: &Host, session: &str, agent: &AgentId, text: &str) -> serde_json::Value {
        let _answer = host.send_to(agent, Turn::new(text)).unwrap();
        let handed = poll(host, session);
        assert_eq!(handed["text"], text, "{handed}");
        complete(host, session, handed["turn_id"].as_u64().unwrap());
        handed
    }

    #[test]
    fn a_note_reaches_the_agent_in_its_next_turn_once_beside_what_the_context_already_said() {
        let host = host_with_nothing();
        let session = attach_many(&host, "pi");
        let agent = host.start_agent("pi").unwrap();
        let bystander = host.start_agent("pi").unwrap();

        assert!(host.note_for(&agent, "Your command `make` finished: exit code 0.".into()));
        // The machine facts the Lens sends stay; the note goes in beside them.
        let machine = json!({ "machine": { "timezone": "Asia/Kolkata" } }).to_string();
        let _answer = host.send_to(&agent, Turn::new("and now?").with_context(machine)).unwrap();
        let handed = poll(&host, &session);
        let context: serde_json::Value = serde_json::from_str(handed["context"].as_str().unwrap()).unwrap();
        assert_eq!(context["machine"]["timezone"], "Asia/Kolkata", "{context}");
        assert_eq!(notes_in(&handed), ["Your command `make` finished: exit code 0."]);
        complete(&host, &session, handed["turn_id"].as_u64().unwrap());

        // Once: the turn after it carries nothing, and a turn with no note has no `notes` at all.
        let again = next_turn(&host, &session, &agent, "anything else?");
        assert!(again["context"].is_null(), "{again}");
        // Another agent's turn never carries this agent's note.
        assert!(host.note_for(&agent, "second".into()));
        let theirs = next_turn(&host, &session, &bystander, "unrelated");
        assert!(notes_in(&theirs).is_empty(), "{theirs}");
        assert_eq!(notes_in(&next_turn(&host, &session, &agent, "mine")), ["second"]);
    }

    #[test]
    fn a_note_left_while_the_turn_waits_goes_with_that_turn() {
        let host = host_with_nothing();
        let (session, agent, turn_id, _answer) = turn_in_flight(&host);
        // Queued behind the turn in flight, then the command finishes.
        let _waiting = host.send_to(&agent, Turn::new("next question")).unwrap();
        assert!(host.note_for(&agent, "the build finished".into()));
        complete(&host, &session, turn_id);
        let handed = poll(&host, &session);
        assert_eq!(handed["text"], "next question");
        assert_eq!(notes_in(&handed), ["the build finished"]);
    }

    #[test]
    fn stop_and_new_do_not_take_the_notes_the_harness_would_never_show_its_mind() {
        let host = host_with_nothing();
        let (session, agent, turn_id, _answer) = turn_in_flight(&host);
        assert!(host.note_for(&agent, "late finish".into()));
        let _stop = host.send_to(&agent, Turn::new("/stop")).unwrap();
        let stop = poll(&host, &session);
        assert_eq!(stop["text"], "/stop");
        assert!(notes_in(&stop).is_empty(), "{stop}");
        complete(&host, &session, stop["turn_id"].as_u64().unwrap());
        complete(&host, &session, turn_id);
        let new = next_turn(&host, &session, &agent, "/new");
        assert!(notes_in(&new).is_empty(), "{new}");
        assert_eq!(notes_in(&next_turn(&host, &session, &agent, "go on")), ["late finish"]);
    }

    #[test]
    fn notes_are_capped_in_number_and_length_and_the_turn_says_what_was_dropped() {
        let host = host_with_nothing();
        let session = attach_many(&host, "pi");
        let agent = host.start_agent("pi").unwrap();
        for n in 0..MAX_NOTES + 3 {
            assert!(host.note_for(&agent, format!("note {n}")));
        }
        let long = "é".repeat(MAX_NOTE_BYTES);
        assert!(host.note_for(&agent, long));
        // Nothing to say is not a note.
        assert!(!host.note_for(&agent, "   ".into()));

        let notes = notes_in(&next_turn(&host, &session, &agent, "hi"));
        assert_eq!(notes.len(), MAX_NOTES + 1, "{notes:?}");
        assert!(notes[0].starts_with("4 earlier notes from the desktop were dropped"), "{notes:?}");
        // The oldest went; the newest stayed, in order.
        assert_eq!(notes[1], "note 4");
        assert_eq!(notes[MAX_NOTES - 1], format!("note {}", MAX_NOTES + 2));
        let cut = notes.last().unwrap();
        assert!(cut.len() <= MAX_NOTE_BYTES && cut.ends_with("(cut)"), "{} bytes", cut.len());
        assert!(cut.starts_with('é'));
    }

    #[test]
    fn notes_end_with_their_agent() {
        let host = host_with_nothing();
        let session = attach_many(&host, "pi");
        let agent = host.start_agent("pi").unwrap();
        assert!(host.note_for(&agent, "for the stopped one".into()));
        host.stop_agent(&agent);
        assert!(!host.note_for(&agent, "too late".into()), "a stopped agent takes no notes");

        // A conversation that no longer exists cannot be given one either, and a new agent does
        // not inherit the old one's.
        let fresh = host.start_agent("pi").unwrap();
        assert!(notes_in(&next_turn(&host, &session, &fresh, "hello")).is_empty());

        // A harness that restarts takes its agents' notes with it.
        assert!(host.note_for(&fresh, "before the restart".into()));
        let session = attach_many(&host, "pi");
        assert!(!host.note_for(&fresh, "after".into()));
        let newest = host.start_agent("pi").unwrap();
        assert!(notes_in(&next_turn(&host, &session, &newest, "hi")).is_empty());

        // And nothing is noted for a harness that is not attached, or a built-in.
        assert!(!host.note_for(&AgentId::new("openclaw", "main"), "x".into()));
    }

    #[test]
    fn a_context_that_is_not_an_object_is_kept_under_framing() {
        let merged = with_notes(Some("you are on the desktop".into()), vec!["n".into()]).unwrap();
        let merged: serde_json::Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(merged, json!({ "framing": "you are on the desktop", "notes": ["n"] }));
        let merged = with_notes(Some(json!({ "notes": ["a"] }).to_string()), vec!["b".into()]).unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&merged).unwrap(), json!({ "notes": ["a", "b"] }));
        assert_eq!(with_notes(Some("{}".into()), vec![]), Some("{}".into()), "no notes, no change");
    }
}
