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
//! - **A session answers only whoever attached it**: the same account, and the same process or
//!   one it started, as the kernel says them. Its id (`s<n>-<random>`) is not a secret it rests
//!   on; a session attached where the kernel named nobody (the TCP dev path) is held to nothing.
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
//! - **A `redact` erases only what the person said to erase**: words from a question this run
//!   asked and the person answered *Erase*, from the session holding the run, within five minutes
//!   of its end, once — and never the record of what happened. See `host::erase`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use crate::event::{AgentId, Event};
use crate::protocol::{self, Assignment, Attach};
use crate::run_store::{RunError, RunState, RunStore};
use crate::{Answer, Capabilities, Chunk, Harness, Health, Turn};

mod erase;
pub use erase::{Redactor, ShellErased, ShellErasure, ShellPlan, ShellRedactor};

/// The summary a call gets when its turn ended before it did.
pub const INTERRUPTED: &str = "interrupted";

/// What the person who asked is told when an agent is stopped mid-turn.
pub const STOPPED: &str = "stopped before it finished";

/// Why a turn ended when the person interrupted it to say something (#234).
pub const INTERRUPTED_TO_TELL: &str = "interrupted: the person has something to tell it";

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
    /// Why the desktop stopped waiting, for the one log line a drop gets.
    gone: Gone,
    /// Whether that line has been written for this turn.
    drop_logged: bool,
    /// What the harness said after nobody was listening, kept (capped) so the answer is not lost
    /// silently: see [`Host::with_late_answer`]. Never logged.
    late: String,
    calls: HashMap<String, Call>,
}

/// Why a turn's chunks have nowhere to go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gone {
    /// The person stopped the agent.
    Stopped,
    /// The person said something to it, which ended the turn.
    Interrupted,
    /// The panel stopped listening: New chat, a mind switch, or the window closed.
    NoListener,
}

impl Gone {
    fn words(self) -> &'static str {
        match self {
            Gone::Stopped => "stopped",
            Gone::Interrupted => "interrupted to say something",
            Gone::NoListener => "no listener (new chat, mind switch or panel closed)",
        }
    }
}

/// How much of a late answer is kept for the person to find.
const MAX_LATE_BYTES: usize = 16 * 1024;

/// An answer that arrived after the person had left the chat it was for.
#[derive(Clone, Debug)]
pub struct LateAnswer {
    pub harness: String,
    pub turn_id: u64,
    pub conversation: String,
    /// What the harness said, in full up to a cap.
    pub text: String,
}

impl Flight {
    /// Keeps `delta` for a late answer, up to [`MAX_LATE_BYTES`] in all, cut on a char boundary.
    fn keep_late(&mut self, delta: &str) {
        if self.late.len() >= MAX_LATE_BYTES {
            return;
        }
        let room = MAX_LATE_BYTES - self.late.len();
        let cut = (0..=delta.len().min(room)).rev().find(|i| delta.is_char_boundary(*i)).unwrap_or(0);
        self.late.push_str(&delta[..cut]);
    }

    fn new(conversation: String, tx: Sender<Chunk>) -> Flight {
        Flight {
            conversation,
            tx: Some(tx),
            abandoned: None,
            gone: Gone::NoListener,
            drop_logged: false,
            late: String::new(),
            calls: HashMap::new(),
        }
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
            Event::Thinking { .. } | Event::Status { .. } | Event::Usage { .. } | Event::Request { .. } => Ok(()),
            // Taken before a turn's stream is looked at (`Host::redact`); never one of its events.
            Event::Redact { .. } => Err("a `redact` is not part of a turn's stream".to_string()),
        }
    }

    /// Say once per turn that what the harness sends is being dropped, and why. The log carries
    /// the harness, the turn and the reason, never what was said.
    fn log_drop(&mut self, harness: &str, turn_id: u64) {
        if !std::mem::replace(&mut self.drop_logged, true) {
            tracing::info!(harness, turn = turn_id, why = self.gone.words(), "the desktop is not waiting for this turn; what it sends is dropped");
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
        self.gone = match failure.as_deref() {
            Some(STOPPED) => Gone::Stopped,
            Some(INTERRUPTED_TO_TELL) => Gone::Interrupted,
            _ => Gone::NoListener,
        };
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
    /// Its credential for the person's memory (#447), minted when the desktop first hands one
    /// out and gone with the agent: a mind that detaches, or whose grants are revoked, holds
    /// nothing the memory server will honour.
    memory: Option<MemoryCredential>,
}

/// A memory credential as the host keeps it: the secret the agent is handed, and its digest,
/// computed once at mint. The memory server asks by digest, so a lookup compares digests
/// rather than hashing every live credential under the host's lock on every question.
struct MemoryCredential {
    secret: String,
    digest: String,
}

/// Who holds a memory credential (#447), as the memory server's question is answered from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryHolder {
    pub agent: AgentId,
    /// The attach it was minted under (`s<n>-<random>`): a harness that restarts attaches again,
    /// so this tells two lives of the same mind apart in an audit.
    pub session: String,
    /// The process that attached, and the account it ran as, both as the kernel said at accept
    /// (`SO_PEERCRED`). Read then and kept, never looked up again later: by the time a question
    /// arrives the pid may be another process's, and `/proc` would name that one's account.
    pub pid: Option<u32>,
    pub uid: Option<u32>,
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
            memory: None,
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
    /// The account that process ran as, from the same `SO_PEERCRED` read, recorded at attach
    /// for the same reason: what decides whether it is the person's own mind account (#447) is
    /// the kernel's word when it attached, not a later look at a pid that may have been reused.
    uid: Option<u32>,
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
    /// Answers the person gave to questions its runs asked, to hand over on its next poll:
    /// `{turn_id, request_id, answer}`, each already consumed in the run store, so exactly once.
    answers: Vec<serde_json::Value>,
}

/// Who is on the other end of a call, as the kernel said at accept (`SO_PEERCRED`). Both `None`
/// when the transport could not say (the TCP dev path) or the caller did not pass them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Peer {
    pid: Option<u32>,
    uid: Option<u32>,
}

impl Attached {
    /// Whether a call on this session comes from whoever attached it.
    ///
    /// A session id is only a name the harness repeats. Believed on its own, any process of the
    /// person's could poll another harness's session (the Mind's, say) and be handed its turns,
    /// which are the person's messages, answer them in its name, and take the memory credential
    /// that rides on them. So the kernel's word at attach is held against the kernel's word now:
    /// the same account, and the same process or one it started (a harness may poll from a
    /// worker). What the attach did not record is not asked for, which keeps the TCP dev path,
    /// where the kernel names nobody, working as it did.
    fn admits(&self, peer: Peer, descends: &(dyn Fn(u32, u32) -> bool + Send + Sync)) -> Result<(), String> {
        if let Some(uid) = self.uid {
            if peer.uid != Some(uid) {
                return Err("this session was attached by another account; attach again".into());
            }
        }
        if let Some(pid) = self.pid {
            if !peer.pid.is_some_and(|caller| caller == pid || descends(caller, pid)) {
                return Err(
                    "this session was attached by another process, and answers only that process \
                     and the ones it started; attach again"
                        .into(),
                );
            }
        }
        Ok(())
    }

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
    /// Returns the turns that were in flight, whose runs are orphaned by the caller.
    fn fail_everything(self, in_flight: &str, queued: &str) -> Vec<u64> {
        let mut ids = Vec::with_capacity(self.in_flight.len());
        for (id, mut flight) in self.in_flight {
            flight.settle(Some(in_flight.to_string()));
            ids.push(id);
        }
        for waiting in self.queued {
            let _ = waiting.tx.send(Chunk::Failed(queued.to_string()));
        }
        ids
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

/// How far up the process tree [`pid_descends`] walks before it gives up. A harness's worker is a
/// few generations below it, never hundreds.
const ANCESTRY_BOUND: usize = 64;

/// Whether `pid` runs under `ancestor`: the default descent probe every [`Host`] carries,
/// replaceable per-host by [`Host::with_descent`] for tests that invent process trees.
///
/// Walks the parent chain in `/proc/<pid>/stat` up to `ancestor`, pid 1 or [`ANCESTRY_BOUND`]
/// steps, whichever comes first. A read that fails ends the walk with `false`: a process that
/// cannot be traced to the harness is not the harness's. Anywhere without `/proc` nothing
/// descends, and only the attaching process itself is admitted.
#[cfg(target_os = "linux")]
fn pid_descends(pid: u32, ancestor: u32) -> bool {
    let mut at = pid;
    for _ in 0..ANCESTRY_BOUND {
        if at == ancestor {
            return true;
        }
        if at <= 1 {
            return false;
        }
        let Some(parent) = std::fs::read_to_string(format!("/proc/{at}/stat")).ok().and_then(|s| parent_in_stat(&s))
        else {
            return false;
        };
        if parent == at {
            return false;
        }
        at = parent;
    }
    false
}

#[cfg(not(target_os = "linux"))]
fn pid_descends(_pid: u32, _ancestor: u32) -> bool {
    false
}

/// The parent pid in a `/proc/<pid>/stat` line: the second field after the command, which is in
/// parentheses and may itself hold spaces and parentheses, so it is found from the last `)`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parent_in_stat(stat: &str) -> Option<u32> {
    let after = &stat[stat.rfind(')')? + 1..];
    after.split_whitespace().nth(1)?.parse().ok()
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
    /// `redact` events the acceptance rule held for, and that erased.
    pub redacted: u64,
    /// `redact` events refused, with nothing changed.
    pub redact_refused: u64,
}

struct State {
    attached: HashMap<String, Attached>,
    active: String,
    next_turn: u64,
    next_session: u64,
    /// Every conversation id this host has issued, so none is issued twice.
    issued: HashSet<String>,
    events: EventCounts,
    /// Turns a re-attaching harness picked back up, waiting for the shell to take their
    /// answers ([`Host::take_resumed`]).
    resumed: Vec<Resumed>,
}

/// A turn a harness was still answering when it lost this desktop, taken back on its re-attach
/// (#246). Its answer is new: whatever the harness says from here on arrives on it.
pub struct Resumed {
    pub agent: AgentId,
    /// What the turn asked, as the harness remembered it.
    pub prompt: String,
    pub answer: Answer,
}

/// Everything the OS knows about the minds available to it.
#[derive(Clone)]
pub struct Host {
    builtins: Arc<Vec<Arc<dyn Harness>>>,
    state: Arc<Mutex<State>>,
    /// Asks whether the process that attached still runs — the kernel's `pid_alive` by default,
    /// unless a test injected its own with [`Host::with_liveness`].
    liveness: Arc<dyn Fn(u32) -> bool + Send + Sync>,
    /// Asks whether a caller's process runs under the one that attached a session:
    /// `pid_descends` by default, unless a test injected its own with [`Host::with_descent`].
    descends: Arc<dyn Fn(u32, u32) -> bool + Send + Sync>,
    /// Where each turn is kept as a run (#25), when the shell gave the host somewhere to keep
    /// them: see [`Host::with_runs`].
    runs: Option<Arc<RunStore>>,
    /// Whether a mind is handed a memory credential with its turns (#447), when the shell said
    /// how to decide: see [`Host::with_memory`].
    memory: Option<MemoryPolicy>,
    /// Why no turn goes to any mind right now, when something paused them all: the person's
    /// Private mode. See [`Host::pause`].
    paused: Arc<std::sync::RwLock<Option<String>>>,
    /// Told when an answer finishes for a chat the person had left. See [`Host::with_late_answer`].
    late_answer: Option<Arc<dyn Fn(LateAnswer) + Send + Sync>>,
    /// Erases the shell's own copy of a conversation (the agent's pane transcript) once a `redact`
    /// is accepted, when the shell said how: see [`Host::with_redactor`].
    redactor: Option<Redactor>,
}

/// How the host decides who carries a memory credential, and how it digests one. The decision
/// is the shell's (the person's grants); the host only asks, turn by turn.
#[derive(Clone)]
struct MemoryPolicy {
    /// `(harness id, uid it attached with)` -> whether that mind holds any memory grant.
    granted: Arc<dyn Fn(&str, Option<u32>) -> bool + Send + Sync>,
    hash: Arc<dyn Fn(&str) -> String + Send + Sync>,
    /// Where the person's memory server listens, as the harness should dial it, when the shell
    /// knows; see [`Host::with_memory_url`].
    url: Option<Arc<dyn Fn() -> Option<String> + Send + Sync>>,
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
                resumed: Vec::new(),
            })),
            liveness: Arc::new(pid_alive),
            descends: Arc::new(pid_descends),
            runs: None,
            memory: None,
            paused: Arc::new(std::sync::RwLock::new(None)),
            late_answer: None,
            redactor: None,
        }
    }

    /// The same host, telling `hook` once when a turn ends well after the person had left its chat
    /// (New chat, a mind switch, a closed panel), with what it said. The shell raises a
    /// notification, so an answer is never lost without a word. A turn the person stopped is not
    /// reported: they asked for it to end.
    pub fn with_late_answer(mut self, hook: impl Fn(LateAnswer) + Send + Sync + 'static) -> Host {
        self.late_answer = Some(Arc::new(hook));
        self
    }

    /// Stop every turn to every mind, built-in included, saying `why` to whoever asks (`Some`), or
    /// let them through again (`None`). A turn refused here is not queued for later: what the
    /// person or an agent tried to send while paused is never delivered.
    pub fn pause(&self, why: Option<String>) {
        if let Ok(mut paused) = self.paused.write() {
            *paused = why;
        }
    }

    /// Why turns are paused, if they are. A poisoned lock reads as paused.
    pub fn paused(&self) -> Option<String> {
        match self.paused.read() {
            Ok(p) => p.clone(),
            Err(_) => Some("turns are paused".to_string()),
        }
    }

    /// The same host, handing each turn of a mind that holds a memory grant its agent's memory
    /// credential (#447): `granted` is asked with the harness id and the uid it attached with,
    /// `hash` digests the credential as the memory server will. Without this, no turn carries one.
    pub fn with_memory(
        mut self,
        granted: impl Fn(&str, Option<u32>) -> bool + Send + Sync + 'static,
        hash: impl Fn(&str) -> String + Send + Sync + 'static,
    ) -> Host {
        self.memory = Some(MemoryPolicy { granted: Arc::new(granted), hash: Arc::new(hash), url: None });
        self
    }

    /// Where a turn that carries a memory credential tells the harness to present it (#447): the
    /// person's memory server as `url` answers at hand-over, `None` while there is none to dial. A
    /// credential with nowhere to go is still handed over, so a harness can say why it has no
    /// memory rather than finding nothing. Needs [`Host::with_memory`] first.
    pub fn with_memory_url(mut self, url: impl Fn() -> Option<String> + Send + Sync + 'static) -> Host {
        if let Some(policy) = self.memory.as_mut() {
            policy.url = Some(Arc::new(url));
        }
        self
    }

    /// The same host, keeping every turn a harness takes as a run in `store` (#25): its state and
    /// a sequenced log of what it streamed, each entry stored before the harness is answered.
    ///
    /// Whatever the previous host left unfinished is orphaned here, before anything attaches —
    /// readable, never resumed — and turn ids continue from the store's, so a run id never names
    /// two runs across restarts.
    pub fn with_runs(self, store: Arc<RunStore>) -> Host {
        match store.orphan_unfinished() {
            Ok(0) => {}
            Ok(n) => tracing::info!(runs = n, "runs left unfinished by the last start are orphaned"),
            Err(e) => tracing::error!(error = %e, "could not orphan unfinished runs"),
        }
        match store.next_run_id() {
            Ok(next) => {
                let mut st = self.lock();
                st.next_turn = st.next_turn.max(next);
            }
            Err(e) => tracing::error!(error = %e, "could not read the next run id; turn ids may repeat old runs"),
        }
        Host { runs: Some(store), ..self }
    }

    /// The run store, for reading runs and their logs, when there is one.
    pub fn run_store(&self) -> Option<Arc<RunStore>> {
        self.runs.clone()
    }

    /// Write to the run store, if there is one. A failed write is logged and the turn goes on:
    /// losing its record is better than losing the turn.
    fn record<T>(&self, what: &str, run_id: u64, f: impl FnOnce(&RunStore) -> Result<T, RunError>) {
        if let Some(store) = &self.runs {
            if let Err(e) = f(store) {
                tracing::error!(run = run_id, what, error = %e, "run not recorded");
            }
        }
    }

    /// End these runs in `state`: the harness that held them is gone, or stopped them.
    fn end_runs(&self, ids: &[u64], state: RunState) {
        for &id in ids {
            self.record("end", id, |s| s.transition(id, state));
        }
    }

    /// The person's answer to a question run `run_id` asked. It is consumed in the run store
    /// first, so it counts exactly once, and only then queued for the one harness answering that
    /// run, which gets it on its next poll. A connection that was replaced holds no runs, so it
    /// can never receive it. Errors say why nothing was delivered. `by_option`: the person pressed
    /// one of the question's offered answers, rather than typing one (the run store keeps which).
    pub fn answer(&self, run_id: u64, request_id: &str, answer: &serde_json::Value, by_option: bool) -> Result<(), String> {
        let store = self.runs.as_ref().ok_or("this desktop keeps no runs")?;
        let mut state = self.lock();
        self.reap(&mut state);
        let Some(harness) = state.attached.values_mut().find(|h| h.in_flight.contains_key(&run_id)) else {
            return Err(match store.run(run_id) {
                Ok(Some(r)) if r.state.is_final() => format!("run {run_id} has ended ({})", r.state.as_str()),
                Ok(Some(_)) => format!("no harness is answering run {run_id} now"),
                _ => format!("run {run_id} does not exist"),
            });
        };
        store.answer(run_id, request_id, answer, by_option).map_err(|e| e.to_string())?;
        harness.answers.push(serde_json::json!({ "turn_id": run_id, "request_id": request_id, "answer": answer }));
        Ok(())
    }

    /// [`Host::answer`] for a question `agent` asked, found by the agent and the request id,
    /// which is what the shell's agent view knows: the run is the one in flight for that agent
    /// that is still waiting on `request_id`.
    pub fn answer_for(&self, agent: &AgentId, request_id: &str, answer: &serde_json::Value, by_option: bool) -> Result<(), String> {
        let store = self.runs.as_ref().ok_or("this desktop keeps no runs")?;
        let runs: Vec<u64> = {
            let mut state = self.lock();
            self.reap(&mut state);
            state
                .attached
                .get(agent.harness())
                .map(|h| {
                    h.in_flight
                        .iter()
                        .filter(|(_, f)| f.conversation == agent.conversation())
                        .map(|(id, _)| *id)
                        .collect()
                })
                .unwrap_or_default()
        };
        let run = runs
            .into_iter()
            .find(|&run| store.pending_requests(run).is_ok_and(|p| p.iter().any(|(r, _)| r == request_id)))
            .ok_or_else(|| format!("`{agent}` is no longer waiting on that question"))?;
        self.answer(run, request_id, answer, by_option)
    }

    /// Cancel a run: the harness is told on its next poll, the person's listener is settled, and
    /// the run ends `cancelled`. `false` when no harness is answering it now.
    pub fn cancel_run(&self, run_id: u64) -> bool {
        let mut state = self.lock();
        self.reap(&mut state);
        let found = state.attached.values_mut().find_map(|h| {
            let flight = h.in_flight.get_mut(&run_id)?;
            if flight.tx.is_some() {
                flight.abandon(Some(STOPPED.to_string()));
            }
            h.cancelled.push(run_id);
            Some(())
        });
        drop(state);
        if found.is_some() {
            self.end_runs(&[run_id], RunState::Cancelled);
        }
        found.is_some()
    }

    /// The same host, asking `probe` whether a process that attached still runs instead of
    /// asking the kernel. For tests that invent pids — a fake process tree has no `/proc`
    /// behind it, and the invented harness must not be reaped for that (#67). Production
    /// takes the default and never comes through here.
    pub fn with_liveness(mut self, probe: impl Fn(u32) -> bool + Send + Sync + 'static) -> Host {
        self.liveness = Arc::new(probe);
        self
    }

    /// The same host, asking `probe(caller, attacher)` whether a caller's process runs under the
    /// one that attached a session, instead of walking `/proc`. For tests that invent process
    /// trees; production takes the default and never comes through here.
    pub fn with_descent(mut self, probe: impl Fn(u32, u32) -> bool + Send + Sync + 'static) -> Host {
        self.descends = Arc::new(probe);
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
                let orphaned = lost.fail_everything(
                    &format!("{id} stopped responding"),
                    &format!("{id} left before it answered"),
                );
                self.end_runs(&orphaned, RunState::Orphaned);
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
        if let Some(why) = self.paused() {
            return failed(why);
        }
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
        if let Some(why) = self.paused() {
            return Err(why);
        }
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
                // Decided when the harness takes the turn, not now: see `poll`.
                memory_credential: String::new(),
                memory_url: String::new(),
                origin: turn.origin,
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

    /// Whether a live agent holds a token whose digest is `digest`, digests computed by `hash` (the
    /// caller's, so this crate needs no hash of its own). What a door outside the shell asks,
    /// by digest, before it believes a mind account's call is an agent's (#411).
    pub fn knows_token_digest(&self, digest: &str, hash: impl Fn(&str) -> String) -> bool {
        let mut state = self.lock();
        self.reap(&mut state);
        state.attached.values().any(|harness| harness.agents.values().any(|agent| hash(&agent.token) == digest))
    }

    /// This agent's credential for the person's memory (#447), minted the first time it is asked
    /// for, with its digest computed then by `hash` (the caller's, as [`Host::knows_token_digest`]
    /// takes one, so this crate needs no hash of its own). Distinct from its agent token: the
    /// token lets it act on the desktop, this lets the memory server ask the desktop what it may
    /// remember, and either can be withdrawn alone. `None` when no such agent is alive.
    pub fn memory_credential(
        &self,
        agent: &AgentId,
        hash: impl Fn(&str) -> String,
    ) -> Option<Result<String, String>> {
        let mut state = self.lock();
        self.reap(&mut state);
        let live = state.attached.get_mut(agent.harness())?.agents.get_mut(agent.conversation())?;
        Some(ensure_memory_credential(live, hash))
    }

    /// Which live agent holds this memory credential, under which attach, and the process and
    /// account that attached it. What the shell answers the memory server from, before anything
    /// is recalled or kept.
    pub fn memory_credential_holder(&self, credential: &str) -> Option<MemoryHolder> {
        let credential = credential.trim();
        if !well_formed_memory_credential(credential) {
            return None;
        }
        self.memory_holder_where(|held| same_secret(&held.secret, credential))
    }

    /// The same, asked by the credential's digest: what the memory server sends, so the
    /// credential itself never crosses the shell's socket. Compared with the digest kept at mint.
    pub fn memory_credential_holder_by_digest(&self, digest: &str) -> Option<MemoryHolder> {
        if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let digest = digest.to_ascii_lowercase();
        self.memory_holder_where(|held| same_secret(&held.digest, &digest))
    }

    fn memory_holder_where(&self, matches: impl Fn(&MemoryCredential) -> bool) -> Option<MemoryHolder> {
        let mut state = self.lock();
        self.reap(&mut state);
        for (harness_id, harness) in &state.attached {
            for (conversation, agent) in &harness.agents {
                if agent.memory.as_ref().is_some_and(&matches) {
                    return Some(MemoryHolder {
                        agent: AgentId::new(harness_id, conversation),
                        session: harness.session.clone(),
                        pid: harness.pid,
                        uid: harness.uid,
                    });
                }
            }
        }
        None
    }

    /// Withdraw a mind's memory credentials: every agent of `harness_id` loses the one it holds,
    /// and the next it is handed is a new one. What a revoked grant does (#447). Answers the
    /// digests of what was withdrawn, which is all `memory.revoked` needs to tell the memory
    /// server: the secrets themselves never leave the host.
    pub fn revoke_memory_credentials(&self, harness_id: &str) -> Vec<String> {
        let mut state = self.lock();
        let Some(harness) = state.attached.get_mut(harness_id) else { return Vec::new() };
        harness.agents.values_mut().filter_map(|agent| agent.memory.take()).map(|held| held.digest).collect()
    }

    /// The account an attached harness ran as when it attached, as the kernel said at accept.
    /// `None` when nothing by that id is attached or the kernel gave none (the TCP dev path).
    pub fn attached_uid(&self, harness_id: &str) -> Option<u32> {
        let mut state = self.lock();
        self.reap(&mut state);
        state.attached.get(harness_id).and_then(|h| h.uid)
    }

    /// The main agent of an attached harness, made now with its token if it has not answered
    /// yet, so a caller can hold the agent before its first turn is queued.
    pub fn ensure_main(&self, harness_id: &str) -> Result<AgentId, String> {
        let mut state = self.lock();
        self.reap(&mut state);
        let harness = state
            .attached
            .get_mut(harness_id)
            .ok_or_else(|| format!("`{harness_id}` is not attached"))?;
        if !harness.agents.contains_key(AgentId::MAIN) {
            harness.agents.insert(AgentId::MAIN.to_string(), Agent::new(mint_token()?));
        }
        Ok(AgentId::new(harness_id, AgentId::MAIN))
    }

    /// Send a turn to a built-in harness by id, whichever mind is active: a caller that decided
    /// on the built-in is not redirected by a switch made in between. `None` when `harness_id` is
    /// no built-in.
    pub fn send_builtin(&self, harness_id: &str, turn: Turn) -> Option<Answer> {
        if let Some(why) = self.paused() {
            return Some(failed(why));
        }
        self.builtin(harness_id).map(|b| b.send(turn))
    }

    /// [`Host::send_to`], but only while the agent still holds the token `held` says it held:
    /// an agent stopped and remade between a hold and its turn would carry a token nothing holds.
    pub fn send_to_holding(&self, agent: &AgentId, turn: Turn, held: impl Fn(&str) -> bool) -> Result<Answer, String> {
        let still = self.with_agent_token(agent, |t| held(t)).unwrap_or(false);
        if !still {
            return Err(format!("`{agent}` changed before the turn was sent; nothing was sent"));
        }
        self.send_to(agent, turn)
    }

    /// Whether an attached harness holds a conversation per agent — `None` when nothing by that id
    /// is attached. A role from the agent catalog is only ever started as a conversation of its
    /// own: a harness that holds one has only the person's own conversation to offer.
    pub fn holds_conversations(&self, harness_id: &str) -> Option<bool> {
        let mut state = self.lock();
        self.reap(&mut state);
        state.attached.get(harness_id).map(|h| h.announced.conversations)
    }

    /// Whether an attached harness reads a hand-over from the turn's context (`context.handover`)
    /// rather than in front of the person's words. False when nothing by that id is attached.
    pub fn reads_handover(&self, harness_id: &str) -> bool {
        let mut state = self.lock();
        self.reap(&mut state);
        state.attached.get(harness_id).is_some_and(|h| h.announced.handover_context)
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
    /// Interrupt what `agent` is doing, without ending it (#234): the turn in flight in its
    /// conversation is cancelled (the harness hears on its next poll, the listener is settled, the
    /// run ends `cancelled`), but the agent, its conversation and its token stay, so the next turn
    /// (the person's word to a stuck task) goes to the same mind with its context. Unlike
    /// [`Host::stop_agent`], nothing is ended. `false` when it had nothing in flight.
    pub fn interrupt(&self, agent: &AgentId) -> bool {
        let mut state = self.lock();
        self.reap(&mut state);
        let Some(harness) = state.attached.get_mut(agent.harness()) else { return false };
        let conversation = agent.conversation();
        let mut cancelled = Vec::new();
        for (turn_id, flight) in harness.in_flight.iter_mut() {
            if flight.conversation == conversation && flight.tx.is_some() {
                flight.abandon(Some(INTERRUPTED_TO_TELL.to_string()));
                harness.cancelled.push(*turn_id);
                cancelled.push(*turn_id);
            }
        }
        drop(state);
        self.end_runs(&cancelled, RunState::Cancelled);
        !cancelled.is_empty()
    }

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
        let mut cancelled = Vec::new();
        for (turn_id, flight) in harness.in_flight.iter_mut() {
            if flight.conversation == conversation && flight.tx.is_some() {
                flight.abandon(Some(STOPPED.to_string()));
                harness.cancelled.push(*turn_id);
                cancelled.push(*turn_id);
                stopped = true;
            }
        }
        if existed && harness.announced.conversations {
            harness.ended.push(conversation.to_string());
        }
        self.end_runs(&cancelled, RunState::Cancelled);
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
        self.handle_from(method, params, None, None)
    }

    /// The same, told which process is on the other end of the socket and which account it runs
    /// as, as the kernel says them.
    ///
    /// `attach` records them: the pid of the harness process, so that a caller later presenting
    /// one of its agents' tokens can be checked against it, and the uid beside it, so that
    /// whether it is the person's own mind account is decided from what the kernel said at
    /// accept (#447). Every call on the session after that is held to them: only the account
    /// that attached, from the process that attached or one it started, is answered.
    pub fn handle_from(
        &self,
        method: &str,
        params: &serde_json::Value,
        peer_pid: Option<u32>,
        peer_uid: Option<u32>,
    ) -> Result<serde_json::Value, String> {
        let peer = Peer { pid: peer_pid, uid: peer_uid };
        match method {
            protocol::ATTACH => self.attach(params, peer_pid, peer_uid),
            protocol::POLL => self.poll(params, peer),
            protocol::CHUNK => self.chunk(params, peer),
            protocol::EVENT => self.event(params, peer),
            protocol::COMPLETE => self.finish(params, peer, None),
            protocol::FAIL => {
                let why = params["error"].as_str().unwrap_or("the harness reported a failure");
                self.finish(params, peer, Some(why.to_string()))
            }
            protocol::DETACH => self.detach(params, peer),
            other => Err(format!(
                "unknown method `{other}`; this service speaks: {}",
                protocol::METHODS.join(", ")
            )),
        }
    }

    fn attach(
        &self,
        params: &serde_json::Value,
        pid: Option<u32>,
        uid: Option<u32>,
    ) -> Result<serde_json::Value, String> {
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

        // The counter keeps ids ordered for a reader of the log; the random part is so that one
        // session's id says nothing about another's. A session answers only whoever attached it
        // (`Attached::admits`), so this is the second wall, not the only one.
        let salt = random_hex(SESSION_RANDOM_BYTES)?;
        let mut state = self.lock();
        self.reap(&mut state);
        let session = format!("s{}-{salt}", state.next_session);
        state.next_session += 1;

        // Re-attaching under an existing id replaces it, which is what a harness that restarted
        // should get. Anything the old one owed is failed rather than abandoned — the turn it was
        // answering and the turns waiting behind it — and its agents end with it: their
        // conversations lived in the process that is gone.
        //
        // Except what the harness says it is still answering (#246): those turns are kept, under
        // the same ids and the same listeners, so a harness that only lost its connection
        // carries on as though it had not.
        // Not across accounts. A live harness attached from one account is not replaced by a
        // process of another: that would hand the newcomer the turns it listed as resumed and
        // whoever asks this mind something next. A restart comes back from the same account.
        if let Some(previous) = state.attached.get(&announced.id) {
            if previous.uid.is_some() && previous.uid != uid {
                return Err(format!(
                    "`{}` is attached from another account; it can be replaced only from that one",
                    announced.id
                ));
            }
        }
        let mut kept: HashMap<u64, Flight> = HashMap::new();
        if let Some(mut previous) = state.attached.remove(&announced.id) {
            for turn in announced.resume.iter().filter_map(|r| r.turn_id) {
                if let Some(flight) = previous.in_flight.remove(&turn) {
                    kept.insert(turn, flight);
                }
            }
            let orphaned = previous.fail_everything(
                &format!("{} restarted mid-answer", announced.id),
                &format!("{} restarted before it answered", announced.id),
            );
            self.end_runs(&orphaned, RunState::Orphaned);
        }

        let id = announced.id.clone();
        state.attached.insert(
            id.clone(),
            Attached {
                announced,
                session: session.clone(),
                last_seen: Instant::now(),
                pid,
                uid,
                in_flight: HashMap::new(),
                queued: VecDeque::new(),
                agents: HashMap::new(),
                finished: VecDeque::new(),
                cancelled: Vec::new(),
                ended: Vec::new(),
                answers: Vec::new(),
            },
        );

        let (resumed, refused) = self.resume(&mut state, &id, kept);

        // The first mind to attach on a machine with no built-in becomes the one answering,
        // rather than leaving a desktop that has a harness and is not using it.
        if state.active.is_empty() {
            state.active = id;
        }
        let mut reply = serde_json::json!({ "session": session });
        if !resumed.is_empty() || !refused.is_empty() {
            reply["resumed"] = resumed.into();
            reply["refused"] = refused.into();
        }
        Ok(reply)
    }

    /// Take back what a re-attaching harness still holds (#246): each conversation under the
    /// token it presents, and each turn it is still answering. A turn this host still has is
    /// kept under its id and its listener. One a restarted shell never knew is re-opened under a
    /// new id, and its answer waits for [`Host::take_resumed`]. Returns the turns as
    /// `{"was", "turn_id"}` and what was not taken back as `{"conversation", "why"}`.
    fn resume(
        &self,
        state: &mut State,
        harness_id: &str,
        mut kept: HashMap<u64, Flight>,
    ) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
        let asked = state.attached.get(harness_id).map(|h| h.announced.resume.clone()).unwrap_or_default();
        let one_conversation = state.attached.get(harness_id).is_some_and(|h| !h.announced.conversations);
        let held_elsewhere: HashSet<String> = state
            .attached
            .iter()
            .filter(|(id, _)| id.as_str() != harness_id)
            .flat_map(|(_, h)| h.agents.values().map(|a| a.token.clone()))
            .collect();
        let (mut resumed, mut refused) = (Vec::new(), Vec::new());
        let mut taken: HashSet<String> = HashSet::new();
        for r in asked {
            let why = if r.conversation != AgentId::MAIN
                && (one_conversation || !well_formed_conversation(&r.conversation))
            {
                Some("not a conversation this desktop could have given it")
            } else if !r.agent_token.is_empty() && !well_formed_token(&r.agent_token) {
                Some("not a token this desktop could have minted")
            } else if !r.agent_token.is_empty()
                && (held_elsewhere.contains(&r.agent_token) || !taken.insert(r.agent_token.clone()))
            {
                Some("that token is held by another agent")
            } else {
                None
            };
            if let Some(why) = why {
                refused.push(serde_json::json!({ "conversation": r.conversation, "why": why }));
                continue;
            }
            // A harness that never used its token (Hermes) presents none: its conversation is
            // taken back under a new one, since nothing it runs was holding the old.
            let token = if r.agent_token.is_empty() {
                match mint_token() {
                    Ok(token) => token,
                    Err(why) => {
                        refused.push(serde_json::json!({ "conversation": r.conversation, "why": why }));
                        continue;
                    }
                }
            } else {
                r.agent_token.clone()
            };
            state.issued.insert(r.conversation.clone());
            let next = state.next_turn;
            let Some(harness) = state.attached.get_mut(harness_id) else { break };
            let mut agent = Agent::new(token);
            let Some(was) = r.turn_id else {
                harness.agents.insert(r.conversation.clone(), agent);
                continue;
            };
            agent.turns = 1;
            harness.agents.insert(r.conversation.clone(), agent);
            let session = harness.session.clone();
            if let Some(flight) = kept.remove(&was) {
                harness.in_flight.insert(was, flight);
                // The same run, carried on by the connection that came back.
                self.record("owner", was, |s| s.set_owner(was, &session));
                resumed.push(serde_json::json!({ "was": was, "turn_id": was }));
                continue;
            }
            let (tx, rx) = mpsc::channel();
            harness.in_flight.insert(next, Flight::new(r.conversation.clone(), tx));
            // A turn this host never knew (the shell restarted): a new run, saying what it continues.
            self.record("start", next, |s| {
                s.start(next, harness_id, &r.conversation, &session)?;
                s.append(next, "resumed", &serde_json::json!({ "was": was, "prompt": r.prompt }))
            });
            state.next_turn += 1;
            state.resumed.push(Resumed {
                agent: AgentId::new(harness_id, &r.conversation),
                prompt: r.prompt.clone(),
                answer: rx,
            });
            resumed.push(serde_json::json!({ "was": was, "turn_id": next }));
        }
        // What was kept for a conversation that was then refused has nobody left to answer it.
        for (id, mut flight) in kept {
            flight.settle(Some("the harness came back without it".to_string()));
            self.end_runs(&[id], RunState::Orphaned);
        }
        (resumed, refused)
    }

    /// The turns re-attaching harnesses picked back up since the last call, each with the answer
    /// the rest of it will arrive on. The shell takes them to put them back in front of the
    /// person (#246).
    pub fn take_resumed(&self) -> Vec<Resumed> {
        std::mem::take(&mut self.lock().resumed)
    }

    /// Find the harness holding this session, if `peer` is whoever attached it (see
    /// [`Attached::admits`]), refreshing its presence. A caller that is refused refreshes
    /// nothing: it cannot keep a session alive that it does not hold.
    fn touch<'a>(
        attached: &'a mut HashMap<String, Attached>,
        params: &serde_json::Value,
        peer: Peer,
        descends: &(dyn Fn(u32, u32) -> bool + Send + Sync),
    ) -> Result<&'a mut Attached, String> {
        let session = params["session"].as_str().unwrap_or_default().to_string();
        if session.is_empty() {
            return Err("`session` is missing; call harness.attach first".into());
        }
        let harness = attached
            .values_mut()
            .find(|a| same_secret(&a.session, &session))
            .ok_or_else(|| "this session is not attached any more; call harness.attach again".to_string())?;
        harness.admits(peer, descends)?;
        harness.last_seen = Instant::now();
        Ok(harness)
    }

    /// The turn the next poll of `harness` hands over, by its place in the queue: the oldest
    /// whose conversation has nothing in flight, or a `/stop`, which skips the line.
    fn next_waiting(harness: &Attached) -> Option<usize> {
        let busy: HashSet<&str> = harness
            .in_flight
            .values()
            .filter(|f| f.holds_conversation())
            .map(|f| f.conversation.as_str())
            .collect();
        harness
            .queued
            .iter()
            .position(|w| !busy.contains(w.assignment.conversation.as_str()) || interrupts(&w.assignment.text))
    }

    /// Hand over a turn if one is waiting, and say what the desktop has stopped waiting for.
    ///
    /// Does not block here: blocking is the caller's business, and holding the state lock would
    /// stop every other harness and the whole UI.
    ///
    /// The turn is the oldest one whose conversation has nothing in flight — first in, first out,
    /// one at a time per conversation. `/stop` alone skips the line.
    ///
    /// Whether the turn carries a memory credential (#447) is decided here too, when the harness
    /// takes it, not when it was queued: a grant given or taken away while the turn waited is
    /// the one that counts, and the turn goes only to a caller [`Attached::admits`].
    fn poll(&self, params: &serde_json::Value, peer: Peer) -> Result<serde_json::Value, String> {
        // The person's grants are the shell's to read, from a file, so they are asked outside the
        // host's lock: every harness and the whole UI wait on that lock. Asked only when a turn is
        // ready to go, and of the mind as it attached (its id and the account the kernel named).
        let carries_memory = match &self.memory {
            None => None,
            Some(policy) => {
                let asking = {
                    let mut state = self.lock();
                    self.reap(&mut state);
                    let harness = Self::touch(&mut state.attached, params, peer, &*self.descends)?;
                    Self::next_waiting(harness).map(|_| (harness.announced.id.clone(), harness.uid))
                };
                asking.map(|(id, uid)| (policy.granted)(&id, uid))
            }
        };

        let mut state = self.lock();
        self.reap(&mut state);
        // Found again by the same session: an attach in between would have minted a new one, so
        // this is the same harness, attached by the same account, that the grants were asked about.
        let harness = Self::touch(&mut state.attached, params, peer, &*self.descends)?;
        let next = match (&self.memory, carries_memory) {
            // A turn that became ready after the grants were asked waits for the next poll, a
            // fifth of a second, rather than going out with nothing decided about it.
            (Some(_), None) => None,
            _ => Self::next_waiting(harness),
        };

        // The credential is settled before the turn leaves the queue, so a failure to mint one
        // leaves the turn waiting rather than lost.
        let memory_credential = match (next, &self.memory, carries_memory) {
            (Some(i), Some(policy), Some(granted)) => {
                match harness.agents.get_mut(&harness.queued[i].assignment.conversation) {
                    Some(agent) if granted => ensure_memory_credential(agent, |s| (policy.hash)(s))?,
                    Some(agent) => {
                        // No grant now: whatever it was handed before is withdrawn, so the memory
                        // server finds nothing behind it.
                        agent.memory = None;
                        String::new()
                    }
                    None => String::new(),
                }
            }
            _ => String::new(),
        };

        let mut reply = match next.and_then(|i| harness.queued.remove(i)) {
            Some(Waiting { mut assignment, tx }) => {
                if !memory_credential.is_empty() {
                    assignment.memory_url = self
                        .memory
                        .as_ref()
                        .and_then(|p| p.url.as_ref())
                        .and_then(|url| url())
                        .unwrap_or_default();
                }
                assignment.memory_credential = memory_credential;
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
                // The turn becomes a run when a harness takes it, owned by the session that did.
                let (run, who, session) = (assignment.turn_id, harness.announced.id.clone(), harness.session.clone());
                self.record("start", run, |s| s.start(run, &who, &assignment.conversation, &session));
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
        if !harness.answers.is_empty() {
            reply["answers"] = serde_json::json!(std::mem::take(&mut harness.answers));
        }
        Ok(reply)
    }

    fn chunk(&self, params: &serde_json::Value, peer: Peer) -> Result<serde_json::Value, String> {
        let turn_id = params["turn_id"].as_u64().ok_or("`turn_id` must be a number")?;
        let delta = params["delta"].as_str().unwrap_or_default().to_string();
        let mut state = self.lock();
        let harness = Self::touch(&mut state.attached, params, peer, &*self.descends)?;
        let Some(flight) = harness.in_flight.get_mut(&turn_id) else {
            return Err(harness.not_in_flight(turn_id));
        };
        let Some(tx) = &flight.tx else {
            // Stopped, or nobody is listening: the harness should stop working on it. Its next
            // call on this turn — this one — is where it learns that.
            flight.log_drop(&harness.announced.id, turn_id);
            // An answer for a chat the person left is kept to be told of at the end, not lost.
            if flight.gone == Gone::NoListener {
                flight.keep_late(&delta);
            }
            return Ok(serde_json::json!({ "dropped": true }));
        };
        // An empty delta is a heartbeat: the call has already refreshed this session's presence,
        // which is its whole purpose, and forwarding "" would only wake the panel for nothing.
        if delta.is_empty() {
            return Ok(serde_json::json!({}));
        }
        self.record("text", turn_id, |s| s.append(turn_id, "text", &serde_json::json!({ "delta": delta })));
        if tx.send(Chunk::Text(delta.clone())).is_err() {
            // The panel stopped listening — the person closed it or asked something else. This
            // chunk is the first thing it missed.
            flight.abandon(None);
            flight.log_drop(&harness.announced.id, turn_id);
            flight.keep_late(&delta);
            return Ok(serde_json::json!({ "dropped": true }));
        }
        Ok(serde_json::json!({}))
    }

    /// One structured event for a turn in flight. See [`crate::event`] and the module docs for
    /// what is refused; a refusal is an answer (`{"refused": why}`), never an error, because an
    /// event that could not be shown is not a reason to lose the turn.
    fn event(&self, params: &serde_json::Value, peer: Peer) -> Result<serde_json::Value, String> {
        let turn_id = params["turn_id"].as_u64().ok_or("`turn_id` must be a number")?;
        let raw = &params["event"];
        let mut state = self.lock();
        let st = &mut *state;
        let harness = Self::touch(&mut st.attached, params, peer, &*self.descends)?;
        let who = harness.announced.id.clone();
        if raw.get("kind").and_then(|k| k.as_str()) == Some("redact") {
            // Not part of the turn's stream: it may come after the turn closed, and it is never
            // passed on to a reader. Its own rule, off the host's lock (`host::erase`).
            let session = harness.session.clone();
            drop(state);
            return Ok(self.redact(turn_id, &who, &session, raw));
        }
        let counts = &mut st.events;

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
            flight.log_drop(&who, turn_id);
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
        if let Event::Request { request_id, prompt, options } = &event {
            // A question is only asked if it can be answered exactly once, which needs the store.
            let Some(store) = &self.runs else {
                return Ok(refused("this desktop keeps no runs, so it cannot take a question".to_string()));
            };
            if request_id.trim().is_empty() {
                counts.malformed += 1;
                return Ok(refused("a `request` needs a `request_id`".to_string()));
            }
            if let Err(e) = store.ask(turn_id, request_id, &serde_json::json!({ "prompt": prompt, "options": options })) {
                return Ok(refused(e.to_string()));
            }
        } else {
            self.record("event", turn_id, |s| s.append(turn_id, "event", raw));
        }
        let tx = flight.tx.as_ref().expect("checked above");
        if tx.send(Chunk::Event(event)).is_err() {
            flight.abandon(None);
            flight.log_drop(&who, turn_id);
            return Ok(serde_json::json!({ "dropped": true }));
        }
        counts.accepted += 1;
        Ok(serde_json::json!({}))
    }

    fn finish(
        &self,
        params: &serde_json::Value,
        peer: Peer,
        failure: Option<String>,
    ) -> Result<serde_json::Value, String> {
        let turn_id = params["turn_id"].as_u64().ok_or("`turn_id` must be a number")?;
        let mut state = self.lock();
        let harness = Self::touch(&mut state.attached, params, peer, &*self.descends)?;
        let Some(mut flight) = harness.in_flight.remove(&turn_id) else {
            return Err(harness.not_in_flight(turn_id));
        };
        harness.remember_finished(turn_id);
        let abandoned = flight.tx.is_none();
        let harness_id = harness.announced.id.clone();
        if abandoned {
            // A harness that closes a turn nobody waited for is a drop even if it never sent text.
            flight.log_drop(&harness_id, turn_id);
        }
        // What it said to nobody, when the person simply left (not stopped it) and it ended well.
        let late = (abandoned && failure.is_none() && flight.gone == Gone::NoListener && !flight.late.trim().is_empty())
            .then(|| LateAnswer {
                harness: harness_id,
                turn_id,
                conversation: flight.conversation.clone(),
                text: std::mem::take(&mut flight.late),
            });
        if !abandoned {
            if let Some(agent) = harness.agents.get_mut(&flight.conversation) {
                agent.last = Some(match &failure {
                    Some(why) => TurnEnd::Failed(why.clone()),
                    None => TurnEnd::Completed,
                });
            }
        }
        // A run the person cancelled has already ended; the harness closing it changes nothing.
        let already_ended = self
            .runs
            .as_ref()
            .and_then(|s| s.run(turn_id).ok().flatten())
            .is_some_and(|r| r.state.is_final());
        if !already_ended {
            match &failure {
                Some(why) => self.record("finish", turn_id, |s| {
                    s.append(turn_id, "failure", &serde_json::json!({ "why": why }))?;
                    s.transition(turn_id, RunState::Failed)
                }),
                None => self.record("finish", turn_id, |s| s.transition(turn_id, RunState::Done)),
            }
        }
        flight.settle(failure);
        drop(state);
        if let (Some(late), Some(hook)) = (late, &self.late_answer) {
            hook(late);
        }
        Ok(if abandoned { serde_json::json!({ "dropped": true }) } else { serde_json::json!({}) })
    }

    fn detach(&self, params: &serde_json::Value, peer: Peer) -> Result<serde_json::Value, String> {
        let mut state = self.lock();
        let id = {
            let harness = Self::touch(&mut state.attached, params, peer, &*self.descends)?;
            harness.announced.id.clone()
        };
        if let Some(gone) = state.attached.remove(&id) {
            let orphaned = gone.fail_everything(&format!("{id} detached mid-answer"), &format!("{id} detached before answering"));
            self.end_runs(&orphaned, RunState::Orphaned);
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

/// The random part of a session id: 64 bits, as 16 hex digits after `s<n>-`.
const SESSION_RANDOM_BYTES: usize = 8;

fn mint_token() -> Result<String, String> {
    random_hex(TOKEN_HEX_LEN / 2)
}

/// This agent's memory credential, minted if it has none yet. Digested once, at mint.
fn ensure_memory_credential(live: &mut Agent, hash: impl Fn(&str) -> String) -> Result<String, String> {
    if let Some(existing) = &live.memory {
        return Ok(existing.secret.clone());
    }
    let secret = format!("{MEMORY_CREDENTIAL_PREFIX}{}", random_hex(MEMORY_CREDENTIAL_BYTES)?);
    let digest = hash(&secret).to_ascii_lowercase();
    live.memory = Some(MemoryCredential { secret: secret.clone(), digest });
    Ok(secret)
}

/// A memory credential (#447): `mem-` and 256 random bits as hex. Longer than an agent token and
/// told apart by its prefix, so neither is ever taken for the other.
const MEMORY_CREDENTIAL_PREFIX: &str = "mem-";
const MEMORY_CREDENTIAL_BYTES: usize = 32;

fn well_formed_memory_credential(credential: &str) -> bool {
    credential.strip_prefix(MEMORY_CREDENTIAL_PREFIX).is_some_and(|hex| {
        hex.len() == MEMORY_CREDENTIAL_BYTES * 2 && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// A token this host could have minted: [`TOKEN_HEX_LEN`] lowercase hex digits.
fn well_formed_token(token: &str) -> bool {
    token.len() == TOKEN_HEX_LEN && token.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A conversation id this host could have issued: `c-` and six lowercase hex digits.
fn well_formed_conversation(id: &str) -> bool {
    id.strip_prefix("c-").is_some_and(|hex| {
        hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
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
/// seeded generator: a token that could be guessed would name somebody else's agent. The one
/// source of secrets the desktop issues (agent tokens, job tickets, pairing codes).
pub fn random_hex(bytes: usize) -> Result<String, String> {
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

    #[test]
    fn a_paused_host_sends_no_turn_to_any_mind_and_keeps_none_for_later() {
        let host = Host::new(vec![Arc::new(Builtin) as Arc<dyn Harness>]);
        host.pause(Some("Private mode is on".into()));
        assert_eq!(host.send_to(&AgentId::new("pi", AgentId::MAIN), Turn::new("x")).err().as_deref(), Some("Private mode is on"));
        assert!(
            matches!(host.send(Turn::new("x")).recv(), Ok(Chunk::Failed(why)) if why == "Private mode is on"),
            "the built-in is paused too"
        );
        host.pause(None);
        assert!(host.paused().is_none());
    }

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

    /// A poll from the process and account the kernel names for the caller.
    fn poll_from(host: &Host, session: &str, pid: u32, uid: Option<u32>) -> Result<serde_json::Value, String> {
        host.handle_from(protocol::POLL, &json!({ "session": session }), Some(pid), uid)
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

    /// A shell restarted mid-answer (#246). The new one never gave the turn, so it used to refuse
    /// every chunk after the restart and the answer had nowhere to go. Now the harness says what
    /// it still holds on re-attach: the conversation comes back under the same token, the turn is
    /// re-opened under a new id, and what the harness says next reaches the shell.
    #[test]
    fn a_turn_open_across_a_shell_restart_is_picked_back_up_under_the_same_token() {
        let old = host();
        let (_, agent, was, _) = turn_in_flight(&old);
        let token = old.lock().attached["pi"].agents[agent.conversation()].token.clone();
        drop(old);

        let fresh = host(); // the shell after its restart
        let reply = fresh
            .handle(
                protocol::ATTACH,
                &json!({ "id": "pi", "name": "pi", "conversations": true, "resume": [
                    { "conversation": agent.conversation(), "agent_token": token, "turn_id": was,
                      "prompt": "tidy the photos" },
                ]}),
            )
            .unwrap();
        let session = reply["session"].as_str().unwrap().to_string();
        let now = reply["resumed"][0]["turn_id"].as_u64().expect("the turn is re-opened");
        assert_eq!(reply["resumed"][0]["was"], was);
        assert_eq!(reply["refused"], json!([]));
        assert_eq!(fresh.agent_for_token(&token).map(|(id, _)| id), Some(agent.clone()), "the same agent, the same token");

        let mut picked = fresh.take_resumed();
        assert_eq!(picked.len(), 1);
        let resumed = picked.remove(0);
        assert_eq!((resumed.agent.clone(), resumed.prompt.as_str()), (agent, "tidy the photos"));
        fresh.handle(protocol::CHUNK, &json!({ "session": session, "turn_id": now, "delta": "Done: 212 photos." })).unwrap();
        complete(&fresh, &session, now);
        let said: Vec<String> = resumed.answer.iter().filter_map(|c| match c { Chunk::Text(t) => Some(t), _ => None }).collect();
        assert_eq!(said, vec!["Done: 212 photos.".to_string()], "the rest of the answer arrived");
        assert!(fresh.take_resumed().is_empty(), "taken once");
    }

    /// A harness that lost only its connection, while the shell stayed up: the shell still holds
    /// the turn and still listens on it. Re-attaching keeps it, under the same id and listener,
    /// instead of failing it.
    #[test]
    fn a_harness_that_only_lost_its_connection_keeps_its_turn_and_listener() {
        let host = host();
        let (_, agent, was, answer) = turn_in_flight(&host);
        let token = host.lock().attached["pi"].agents[agent.conversation()].token.clone();
        let reply = host
            .handle(
                protocol::ATTACH,
                &json!({ "id": "pi", "name": "pi", "conversations": true, "resume": [
                    { "conversation": agent.conversation(), "agent_token": token, "turn_id": was },
                ]}),
            )
            .unwrap();
        assert_eq!(reply["resumed"][0], json!({ "was": was, "turn_id": was }));
        assert!(host.take_resumed().is_empty(), "the shell is still listening; nothing to hand it");
        let session = reply["session"].as_str().unwrap().to_string();
        host.handle(protocol::CHUNK, &json!({ "session": session, "turn_id": was, "delta": "still here" })).unwrap();
        complete(&host, &session, was);
        let said: Vec<Chunk> = answer.iter().collect();
        assert!(matches!(&said[..], [Chunk::Text(t)] if t == "still here"), "{said:?}");
    }

    /// Hermes carries no agent token: its one conversation is still taken back, under a new
    /// token, and its open turn re-opened.
    #[test]
    fn a_harness_that_never_used_its_token_still_gets_its_turn_back() {
        let host = host();
        let reply = host
            .handle(
                protocol::ATTACH,
                &json!({ "id": "hermes", "name": "Hermes", "resume": [
                    { "conversation": "main", "agent_token": "", "turn_id": 18, "prompt": "build the town" },
                ]}),
            )
            .unwrap();
        assert_eq!(reply["refused"], json!([]));
        assert_eq!(reply["resumed"][0]["was"], 18);
        let picked = host.take_resumed();
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].agent, AgentId::new("hermes", AgentId::MAIN));
    }

    /// What is not taken back: a token this desktop could not have minted, one another agent
    /// holds, and a conversation id a one-conversation harness never had.
    #[test]
    fn resume_takes_back_only_what_this_desktop_could_have_given() {
        let host = host();
        let (_, agent, _, _) = turn_in_flight(&host);
        let pis = host.lock().attached["pi"].agents[agent.conversation()].token.clone();
        let reply = host
            .handle(
                protocol::ATTACH,
                &json!({ "id": "deepseek", "name": "deepseek", "resume": [
                    { "conversation": "main", "agent_token": "not-a-token" },
                    { "conversation": "main", "agent_token": pis },
                    { "conversation": "c-abcdef", "agent_token": "a".repeat(32) },
                ]}),
            )
            .unwrap();
        let why: Vec<&str> = reply["refused"].as_array().unwrap().iter().map(|r| r["why"].as_str().unwrap()).collect();
        assert_eq!(
            why,
            ["not a token this desktop could have minted", "that token is held by another agent",
             "not a conversation this desktop could have given it"]
        );
        assert_eq!(host.agent_for_token(&pis).map(|(id, _)| id), Some(agent), "pi keeps its own agent");
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
            .handle_from(protocol::ATTACH, &json!({ "id": "pi", "name": "Pi", "conversations": true }), Some(4242), None)
            .unwrap()["session"]
            .as_str()
            .unwrap()
            .to_string();
        let first = host.start_agent("pi").unwrap();
        let second = host.start_agent("pi").unwrap();
        let _a = host.send_to(&first, Turn::new("one")).unwrap();
        let _b = host.send_to(&second, Turn::new("two")).unwrap();
        let token_one = poll_from(&host, &session, 4242, None).unwrap()["agent_token"].as_str().unwrap().to_string();
        let token_two = poll_from(&host, &session, 4242, None).unwrap()["agent_token"].as_str().unwrap().to_string();

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

    #[test]
    fn a_memory_credential_names_its_agent_until_it_is_withdrawn() {
        let host = host_with_nothing().with_liveness(|pid| pid == 4242);
        let session = host
            .handle_from(
                protocol::ATTACH,
                &json!({ "id": "pi", "name": "Pi", "conversations": true }),
                Some(4242),
                Some(1000),
            )
            .unwrap()["session"]
            .as_str()
            .unwrap()
            .to_string();
        let first = host.start_agent("pi").unwrap();
        let second = host.start_agent("pi").unwrap();
        // A stand-in hash that is 64 hex long, counting how often it runs.
        let hashed = std::sync::atomic::AtomicUsize::new(0);
        let hash = |s: &str| {
            hashed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            format!("{:0>64}", s.len().to_string() + &s[4..10])
        };
        let holder = |agent: &AgentId| MemoryHolder {
            agent: agent.clone(),
            session: session.clone(),
            pid: Some(4242),
            uid: Some(1000),
        };

        let one = host.memory_credential(&first, hash).unwrap().unwrap();
        assert!(one.starts_with("mem-") && one.len() == 4 + 64, "256 bits, marked: {one}");
        assert_eq!(host.memory_credential(&first, hash).unwrap().unwrap(), one, "the same one while it lives");
        let two = host.memory_credential(&second, hash).unwrap().unwrap();
        assert_ne!(one, two, "each agent its own");
        assert_eq!(hashed.load(std::sync::atomic::Ordering::Relaxed), 2, "hashed once each, at mint");
        // The pid and the account are the kernel's at attach, carried to whoever asks.
        assert_eq!(host.memory_credential_holder(&one), Some(holder(&first)));
        assert_eq!(host.memory_credential_holder(&two), Some(holder(&second)));

        // Asked by digest, as the memory server does: compared with what was kept at mint, and
        // nothing is hashed to answer.
        assert_eq!(host.memory_credential_holder_by_digest(&hash(&one)), Some(holder(&first)));
        assert_eq!(host.memory_credential_holder_by_digest(&hash(&one).to_ascii_uppercase()), Some(holder(&first)));
        let before = hashed.load(std::sync::atomic::Ordering::Relaxed);
        assert_eq!(host.memory_credential_holder_by_digest(&"0".repeat(64)), None);
        assert_eq!(host.memory_credential_holder_by_digest("not-a-digest"), None);
        assert_eq!(hashed.load(std::sync::atomic::Ordering::Relaxed), before);

        // Not a prefix, not an agent token, not a guess.
        assert_eq!(host.memory_credential_holder(&one[..60]), None);
        assert_eq!(host.memory_credential_holder(&format!("mem-{}", "0".repeat(64))), None);
        assert_eq!(host.memory_credential_holder(""), None);

        // Withdrawn: what comes back is the digests, never the credentials; the old one names
        // nothing, and the next is new.
        let mut withdrawn = host.revoke_memory_credentials("pi");
        withdrawn.sort();
        let mut expected = vec![hash(&one), hash(&two)];
        expected.sort();
        assert_eq!(withdrawn, expected);
        assert!(!withdrawn.contains(&one) && !withdrawn.contains(&two));
        assert_eq!(host.memory_credential_holder(&one), None);
        assert_eq!(host.memory_credential_holder_by_digest(&hash(&one)), None);
        assert_ne!(host.memory_credential(&first, hash).unwrap().unwrap(), one);

        // A stopped agent's credential names nothing, and nothing is minted for it.
        let three = host.memory_credential(&second, hash).unwrap().unwrap();
        host.stop_agent(&second);
        assert_eq!(host.memory_credential_holder(&three), None);
        assert!(host.memory_credential(&second, hash).is_none());
    }

    #[test]
    fn a_harness_attached_without_peer_credentials_holds_its_memory_credential_as_nobody_in_particular() {
        // The TCP dev path: no pid, no account. Nothing about the holder is made up to fill it.
        let host = host_with_nothing();
        host.handle(protocol::ATTACH, &json!({ "id": "pi", "name": "Pi", "conversations": true })).unwrap();
        let agent = host.start_agent("pi").unwrap();
        let credential = host.memory_credential(&agent, |s| format!("{:0>64}", s.len())).unwrap().unwrap();
        let held = host.memory_credential_holder(&credential).unwrap();
        assert_eq!((held.pid, held.uid), (None, None));
    }

    #[test]
    fn a_live_harness_is_not_replaced_from_another_account() {
        let host = host_with_nothing().with_liveness(|pid| pid == 700 || pid == 701);
        host.handle_from(protocol::ATTACH, &json!({ "id": "mind", "name": "Yantrik Mind" }), Some(700), Some(990))
            .unwrap();
        let refused = host
            .handle_from(protocol::ATTACH, &json!({ "id": "mind", "name": "Yantrik Mind" }), Some(701), Some(1000))
            .unwrap_err();
        assert!(refused.contains("another account"), "{refused}");
        assert_eq!(host.list().into_iter().find(|e| e.id == "mind").unwrap().pid, Some(700), "the original stands");
        // A restart from the same account replaces it, as it always has.
        host.handle_from(protocol::ATTACH, &json!({ "id": "mind", "name": "Yantrik Mind" }), Some(701), Some(990))
            .unwrap();
        assert_eq!(host.list().into_iter().find(|e| e.id == "mind").unwrap().pid, Some(701));
    }

    #[test]
    fn a_turn_carries_the_memory_credential_only_to_a_mind_the_person_granted() {
        let hash = |s: &str| format!("{:0>64}", s.len().to_string() + &s[4..10]);
        let host = host_with_nothing()
            .with_liveness(|pid| pid == 4242)
            .with_memory(|harness, _uid| harness == "hermes", hash)
            .with_memory_url(|| Some("unix:/run/yantrik-mind/1000/memory.sock".to_string()));
        let hermes = host
            .handle_from(protocol::ATTACH, &json!({ "id": "hermes", "name": "Hermes" }), Some(4242), Some(1000))
            .unwrap()["session"]
            .as_str()
            .unwrap()
            .to_string();
        let pi = host
            .handle_from(protocol::ATTACH, &json!({ "id": "pi", "name": "Pi" }), Some(4242), Some(1000))
            .unwrap()["session"]
            .as_str()
            .unwrap()
            .to_string();

        let _h1 = host.send_to(&AgentId::new("hermes", AgentId::MAIN), Turn::new("one")).unwrap();
        let first = poll_from(&host, &hermes, 4242, Some(1000)).unwrap();
        let credential = first["memory_credential"].as_str().expect("granted, so carried").to_string();
        assert!(credential.starts_with("mem-") && credential.len() == 68, "{credential}");
        assert_ne!(first["memory_credential"], first["agent_token"], "its own secret, not the token");
        assert_eq!(first["memory_url"], "unix:/run/yantrik-mind/1000/memory.sock", "where to present it");
        host.handle_from(protocol::COMPLETE, &json!({ "session": hermes, "turn_id": first["turn_id"] }), Some(4242), Some(1000))
            .unwrap();
        let _h2 = host.send_to(&AgentId::new("hermes", AgentId::MAIN), Turn::new("two")).unwrap();
        let second = poll_from(&host, &hermes, 4242, Some(1000)).unwrap();
        assert_eq!(second["memory_credential"], credential.as_str(), "the same while it lives");
        let held = host.memory_credential_holder(&credential).unwrap();
        assert_eq!(held.agent, AgentId::new("hermes", AgentId::MAIN));

        let _p = host.send_to(&AgentId::new("pi", AgentId::MAIN), Turn::new("three")).unwrap();
        let turn = poll_from(&host, &pi, 4242, Some(1000)).unwrap();
        assert!(turn.get("memory_credential").is_none(), "no grant, no credential: {turn}");
        assert!(turn.get("memory_url").is_none(), "and nowhere to take one: {turn}");

        // A host the shell gave no policy hands none to anyone.
        let bare = host_with_nothing().with_liveness(|pid| pid == 4242);
        let s = bare
            .handle_from(protocol::ATTACH, &json!({ "id": "hermes", "name": "Hermes" }), Some(4242), Some(1000))
            .unwrap()["session"]
            .as_str()
            .unwrap()
            .to_string();
        let _b = bare.send_to(&AgentId::new("hermes", AgentId::MAIN), Turn::new("four")).unwrap();
        assert!(poll_from(&bare, &s, 4242, Some(1000)).unwrap().get("memory_credential").is_none());
    }

    /// The grant that counts is the one standing when the harness takes the turn: one given while
    /// the turn waited is honoured, and one taken away withdraws the credential already held.
    #[test]
    fn the_memory_credential_is_decided_when_the_turn_is_handed_over_not_when_it_was_queued() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let granted = Arc::new(AtomicBool::new(false));
        let policy = granted.clone();
        let hash = |s: &str| format!("{:0>64}", s.len().to_string() + &s[4..10]);
        let host = host_with_nothing()
            .with_liveness(|pid| pid == 4242)
            .with_memory(move |_harness, _uid| policy.load(Ordering::SeqCst), hash);
        let session = host
            .handle_from(protocol::ATTACH, &json!({ "id": "hermes", "name": "Hermes" }), Some(4242), Some(1000))
            .unwrap()["session"]
            .as_str()
            .unwrap()
            .to_string();
        let agent = AgentId::new("hermes", AgentId::MAIN);

        // Queued with no grant; granted before the harness came for it.
        let _one = host.send_to(&agent, Turn::new("one")).unwrap();
        granted.store(true, Ordering::SeqCst);
        let first = poll_from(&host, &session, 4242, Some(1000)).unwrap();
        let credential = first["memory_credential"].as_str().expect("granted by the time it was taken").to_string();
        host.handle_from(protocol::COMPLETE, &json!({ "session": session, "turn_id": first["turn_id"] }), Some(4242), Some(1000))
            .unwrap();

        // Queued while granted; revoked before the harness came for it.
        let _two = host.send_to(&agent, Turn::new("two")).unwrap();
        granted.store(false, Ordering::SeqCst);
        let second = poll_from(&host, &session, 4242, Some(1000)).unwrap();
        assert_eq!(second["text"], "two");
        assert!(second.get("memory_credential").is_none(), "revoked while it waited: {second}");
        assert_eq!(host.memory_credential_holder(&credential), None, "and the one it held names nothing now");
    }

    /// The person's grants are read from a file by the shell's policy, which must never run with
    /// the host's lock held: every harness and the whole UI wait on that lock.
    #[test]
    fn the_memory_policy_is_asked_without_the_host_lock_held() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let slot: Arc<std::sync::OnceLock<Host>> = Arc::new(std::sync::OnceLock::new());
        let (asked, locked) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let (seen, held) = (slot.clone(), (asked.clone(), locked.clone()));
        let host = host_with_nothing().with_memory(
            move |_harness, _uid| {
                held.0.store(true, Ordering::SeqCst);
                let host = seen.get().expect("set before any poll");
                held.1.store(host.state.try_lock().is_err(), Ordering::SeqCst);
                true
            },
            |s: &str| format!("{:0>64}", s.len()),
        );
        let _ = slot.set(host.clone());
        let session = attach(&host, "hermes");
        let _a = host.send_to(&AgentId::new("hermes", AgentId::MAIN), Turn::new("one")).unwrap();
        assert!(poll(&host, &session)["memory_credential"].is_string());
        assert!(asked.load(Ordering::SeqCst), "the policy was asked");
        assert!(!locked.load(Ordering::SeqCst), "and not under the host's lock");
    }

    // ── A session answers whoever attached it (#449 review) ─────────

    /// The Mind attached from pid 700 as the mind account (uid 990), and a turn waiting for it.
    fn mind_with_a_turn_waiting(host: &Host) -> (String, Answer) {
        let session = host
            .handle_from(protocol::ATTACH, &json!({ "id": "mind", "name": "Yantrik Mind" }), Some(700), Some(990))
            .unwrap()["session"]
            .as_str()
            .unwrap()
            .to_string();
        let answer = host.send_to(&AgentId::new("mind", AgentId::MAIN), Turn::new("what did I say about Tuesday?")).unwrap();
        (session, answer)
    }

    #[test]
    fn another_account_polling_the_minds_session_is_refused_and_takes_nothing() {
        let host = host_with_nothing()
            .with_liveness(|pid| pid == 700)
            .with_memory(|_harness, uid| uid == Some(990), |s: &str| format!("{:0>64}", s.len()));
        let (session, _answer) = mind_with_a_turn_waiting(&host);

        // The person's own process, even from the Mind's own pid as the kernel might reuse it.
        for pid in [4242, 700] {
            let err = poll_from(&host, &session, pid, Some(1000)).unwrap_err();
            assert!(err.contains("another account") && err.contains("attach again"), "{err}");
        }
        // A caller the kernel could not name cannot show it is the account either.
        let err = host.handle(protocol::POLL, &json!({ "session": session })).unwrap_err();
        assert!(err.contains("another account"), "{err}");
        // Nor can it close, fail, stream into or detach the Mind's session.
        for (method, extra) in [
            (protocol::CHUNK, json!({ "turn_id": 1, "delta": "hi" })),
            (protocol::EVENT, json!({ "turn_id": 1, "event": { "kind": "status", "text": "x" } })),
            (protocol::COMPLETE, json!({ "turn_id": 1 })),
            (protocol::FAIL, json!({ "turn_id": 1, "error": "no" })),
            (protocol::DETACH, json!({})),
        ] {
            let mut params = extra;
            params["session"] = json!(session);
            let err = host.handle_from(method, &params, Some(4242), Some(1000)).unwrap_err();
            assert!(err.contains("another account"), "{method}: {err}");
        }

        // Nothing was delivered: the turn still waits for the real Mind, credential and all.
        let turn = poll_from(&host, &session, 700, Some(990)).unwrap();
        assert_eq!(turn["text"], "what did I say about Tuesday?");
        assert!(turn["memory_credential"].is_string(), "{turn}");
        assert!(host.list().iter().any(|e| e.id == "mind"), "and the Mind is still attached");
    }

    #[test]
    fn the_same_account_from_a_process_outside_the_harness_is_refused() {
        // 700 started 701, which started 702; 900 is a stranger of the same account.
        let tree = |caller: u32, ancestor: u32| matches!((caller, ancestor), (701, 700) | (702, 700) | (702, 701));
        let host = host_with_nothing().with_liveness(|pid| pid == 700).with_descent(tree);
        let (session, _answer) = mind_with_a_turn_waiting(&host);

        let err = poll_from(&host, &session, 900, Some(990)).unwrap_err();
        assert!(err.contains("another process") && err.contains("attach again"), "{err}");
        let err = host.handle_from(protocol::POLL, &json!({ "session": session }), None, Some(990)).unwrap_err();
        assert!(err.contains("another process"), "no pid from the kernel is not the harness's: {err}");

        // A worker the harness started polls for it; so does the harness itself.
        let turn = poll_from(&host, &session, 702, Some(990)).unwrap();
        assert_eq!(turn["text"], "what did I say about Tuesday?");
        assert!(poll_from(&host, &session, 700, Some(990)).is_ok());
    }

    #[test]
    fn a_session_attached_where_the_kernel_named_nobody_is_held_to_nothing() {
        // The TCP dev path, and every test above that attaches with `handle`: as before.
        let host = host_with_nothing();
        let session = attach(&host, "pi");
        let _a = host.send_to(&AgentId::new("pi", AgentId::MAIN), Turn::new("one")).unwrap();
        assert_eq!(poll_from(&host, &session, 4242, Some(1000)).unwrap()["text"], "one");
    }

    #[test]
    fn session_ids_cannot_be_guessed_from_one_another() {
        let host = host_with_nothing();
        let (one, two) = (attach(&host, "pi"), attach(&host, "hermes"));
        let random = |s: &str| s.split_once('-').map(|(_, r)| r.to_string()).unwrap_or_default();
        for s in [&one, &two] {
            let r = random(s);
            assert!(s.starts_with('s') && r.len() == 16 && r.bytes().all(|b| b.is_ascii_hexdigit()), "{s}");
        }
        assert_ne!(random(&one), random(&two), "more than a counter apart: {one} {two}");
        // A counter alone, the old shape, names nothing.
        assert!(host.handle(protocol::POLL, &json!({ "session": "s1" })).unwrap_err().contains("not attached any more"));
    }

    #[test]
    fn a_parent_pid_is_read_past_a_command_name_with_spaces_and_parentheses() {
        assert_eq!(parent_in_stat("4242 (pi) S 700 4242 4242 0 -1"), Some(700));
        assert_eq!(parent_in_stat("4243 (a (b) c) R 1 1 1 0 -1"), Some(1));
        assert_eq!(parent_in_stat("garbage"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_child_of_this_process_descends_from_it_and_pid_one_does_not() {
        let mut child = std::process::Command::new("sleep").arg("5").spawn().unwrap();
        assert!(pid_descends(child.id(), std::process::id()));
        assert!(pid_descends(std::process::id(), std::process::id()));
        assert!(!pid_descends(1, std::process::id()));
        assert!(!pid_descends(std::process::id(), child.id()), "a parent does not descend from its child");
        let _ = child.kill();
        let _ = child.wait();
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
        host.handle_from(protocol::ATTACH, &json!({ "id": "pi", "name": "Pi" }), Some(child.id()), None)
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
        host.handle_from(protocol::ATTACH, &json!({ "id": "pi", "name": "Pi" }), Some(std::process::id()), None)
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

    /// Counts the info lines of the drop and records their fields, never the text.
    struct DropLog(std::sync::Arc<std::sync::Mutex<Vec<String>>>);
    impl tracing::Subscriber for DropLog {
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            struct V(String);
            impl tracing::field::Visit for V {
                fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
                    self.0.push_str(&format!("{}={v:?} ", f.name()));
                }
            }
            let mut v = V(String::new());
            event.record(&mut v);
            if *event.metadata().level() == tracing::Level::INFO && v.0.contains("not waiting") {
                self.0.lock().unwrap().push(v.0);
            }
        }
    }

    #[test]
    fn a_turn_the_desktop_stopped_waiting_for_logs_its_drop_once_with_why_and_never_the_text() {
        let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let host = host_with_nothing();
        let (session, agent, turn_id, _answer) = turn_in_flight(&host);
        assert!(host.stop_agent(&agent));
        tracing::subscriber::with_default(DropLog(lines.clone()), || {
            for text in ["secret one", "secret two"] {
                let r = host
                    .handle(protocol::CHUNK, &json!({ "session": session, "turn_id": turn_id, "delta": text }))
                    .unwrap();
                assert_eq!(r, json!({ "dropped": true }));
            }
            assert_eq!(complete(&host, &session, turn_id), json!({ "dropped": true }));
        });
        let lines = lines.lock().unwrap();
        assert_eq!(lines.len(), 1, "one line per turn: {lines:?}");
        assert!(lines[0].contains("pi") && lines[0].contains(&format!("turn={turn_id}")), "{}", lines[0]);
        assert!(lines[0].contains("stopped"), "{}", lines[0]);
        assert!(!lines[0].contains("secret"), "the text is never logged: {}", lines[0]);
    }

    #[test]
    fn an_answer_that_arrives_after_the_chat_was_left_is_handed_on_not_lost() {
        let told = std::sync::Arc::new(Mutex::new(Vec::<LateAnswer>::new()));
        let sink = told.clone();
        let host = host_with_nothing().with_late_answer(move |late| sink.lock().unwrap().push(late));
        let (session, _agent, turn_id, answer) = turn_in_flight(&host);
        // New chat: the desktop drops its listener, and the harness keeps working.
        drop(answer);
        for part in ["It took ", "a while."] {
            host.handle(protocol::CHUNK, &json!({ "session": session, "turn_id": turn_id, "delta": part })).unwrap();
        }
        assert!(told.lock().unwrap().is_empty(), "nothing is said before the turn ends");
        complete(&host, &session, turn_id);
        let told = told.lock().unwrap();
        assert_eq!(told.len(), 1);
        assert_eq!((told[0].harness.as_str(), told[0].turn_id), ("pi", turn_id));
        assert!(told[0].text.ends_with("a while."), "{:?}", told[0].text);
    }

    #[test]
    fn the_late_buffer_is_capped_on_a_char_boundary_whichever_path_fills_it() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut flight = Flight::new("main".into(), tx);
        flight.keep_late(&"é".repeat(MAX_LATE_BYTES));
        assert!(flight.late.len() <= MAX_LATE_BYTES && flight.late.len() >= MAX_LATE_BYTES - 1);
        flight.keep_late("more");
        assert!(flight.late.len() <= MAX_LATE_BYTES);
    }

    #[test]
    fn a_turn_the_person_stopped_is_not_announced_as_a_late_answer() {
        let told = std::sync::Arc::new(Mutex::new(Vec::<LateAnswer>::new()));
        let sink = told.clone();
        let host = host_with_nothing().with_late_answer(move |late| sink.lock().unwrap().push(late));
        let (session, agent, turn_id, _answer) = turn_in_flight(&host);
        assert!(host.stop_agent(&agent));
        host.handle(protocol::CHUNK, &json!({ "session": session, "turn_id": turn_id, "delta": "too late" })).unwrap();
        complete(&host, &session, turn_id);
        assert!(told.lock().unwrap().is_empty());
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

    // ── Runs (#25): every turn a harness takes is kept, with its log ─────────────────────────

    fn host_with_runs() -> (Host, Arc<RunStore>) {
        let store = Arc::new(RunStore::in_memory().unwrap());
        (host().with_runs(store.clone()), store)
    }

    fn kinds(store: &RunStore, run: u64) -> Vec<String> {
        store.events(run, 0, crate::run_store::PAGE_MAX).unwrap().events.into_iter().map(|e| e.kind).collect()
    }

    #[test]
    fn a_turn_is_kept_as_a_run_with_everything_it_streamed_in_order() {
        let (host, store) = host_with_runs();
        let (session, agent, run, _answer) = turn_in_flight(&host);
        let long = "y".repeat(900);
        host.handle(protocol::CHUNK, &json!({ "session": session, "turn_id": run, "delta": long })).unwrap();
        event(&host, &session, run, start("c1")).unwrap();
        event(&host, &session, run, end("c1")).unwrap();
        complete(&host, &session, run);

        let info = store.run(run).unwrap().expect("the turn is a run");
        assert_eq!((info.harness.as_str(), info.conversation.as_str(), info.owner.as_str()), ("pi", agent.conversation(), session.as_str()));
        assert_eq!(info.state, RunState::Done);
        assert_eq!(kinds(&store, run), vec!["state", "text", "event", "event", "state"]);
        let text = &store.events(run, 1, 1).unwrap().events[0];
        assert_eq!(text.payload["delta"].as_str().unwrap().len(), 900, "a message longer than the 600-character summary is kept whole");
    }

    #[test]
    fn a_failed_turn_keeps_why() {
        let (host, store) = host_with_runs();
        let (session, _, run, _answer) = turn_in_flight(&host);
        host.handle(protocol::FAIL, &json!({ "session": session, "turn_id": run, "error": "model unreachable" })).unwrap();
        assert_eq!(store.run(run).unwrap().unwrap().state, RunState::Failed);
        let log = store.events(run, 0, 10).unwrap().events;
        assert!(log.iter().any(|e| e.kind == "failure" && e.payload["why"] == "model unreachable"), "{log:?}");
    }

    #[test]
    fn a_harness_that_goes_away_mid_answer_orphans_its_run_and_a_stopped_agent_cancels_it() {
        let (host, store) = host_with_runs();
        let (session, agent, run, _answer) = turn_in_flight(&host);
        host.handle(protocol::DETACH, &json!({ "session": session })).unwrap();
        assert_eq!(store.run(run).unwrap().unwrap().state, RunState::Orphaned);

        let (_, agent2, run2, _answer2) = turn_in_flight(&host);
        assert!(host.stop_agent(&agent2));
        assert_eq!(store.run(run2).unwrap().unwrap().state, RunState::Cancelled);
        assert_ne!(agent, agent2);
    }

    #[test]
    fn cancelling_a_run_tells_the_harness_and_its_late_close_changes_nothing() {
        let (host, store) = host_with_runs();
        let (session, _, run, answer) = turn_in_flight(&host);
        assert!(host.cancel_run(run));
        assert!(crate::collect(answer).unwrap_err().contains(STOPPED));
        assert_eq!(poll(&host, &session)["cancelled"], json!([run]), "the harness learns on its next poll");
        complete(&host, &session, run);
        assert_eq!(store.run(run).unwrap().unwrap().state, RunState::Cancelled, "a cancelled run stays cancelled");
        assert!(!host.cancel_run(999), "nobody is answering 999");
    }

    #[test]
    fn a_restarted_host_orphans_what_the_last_one_left_and_never_reuses_a_run_id() {
        let store = Arc::new(RunStore::in_memory().unwrap());
        let first = host().with_runs(store.clone());
        let (_, _, run, _answer) = turn_in_flight(&first);
        drop(first);

        let second = host().with_runs(store.clone());
        assert_eq!(store.run(run).unwrap().unwrap().state, RunState::Orphaned, "readable, not resumed");
        let (_, _, next, _answer) = turn_in_flight(&second);
        assert!(next > run, "run {next} would have named run {run} again");
    }

    #[test]
    fn a_turn_kept_across_a_lost_connection_is_the_same_run_under_the_new_connection() {
        let (host, store) = host_with_runs();
        let (_, agent, was, _answer) = turn_in_flight(&host);
        let token = host.lock().attached["pi"].agents[agent.conversation()].token.clone();
        let reply = host
            .handle(
                protocol::ATTACH,
                &json!({ "id": "pi", "name": "pi", "conversations": true, "resume": [
                    { "conversation": agent.conversation(), "agent_token": token, "turn_id": was },
                ]}),
            )
            .unwrap();
        let session = reply["session"].as_str().unwrap().to_string();
        assert_eq!(store.run(was).unwrap().unwrap().owner, session);
        complete(&host, &session, was);
        assert_eq!(store.run(was).unwrap().unwrap().state, RunState::Done);
    }

    #[test]
    fn without_a_store_nothing_changes() {
        let host = host();
        let (session, _, run, answer) = turn_in_flight(&host);
        host.handle(protocol::CHUNK, &json!({ "session": session, "turn_id": run, "delta": "hi" })).unwrap();
        complete(&host, &session, run);
        assert_eq!(crate::collect(answer).unwrap(), "hi");
        assert!(host.run_store().is_none());
    }

    // ── Questions and answers (#25): exactly once, to the run that asked ─────────────────────

    fn ask(host: &Host, session: &str, run: u64, request_id: &str) -> serde_json::Value {
        event(host, session, run, json!({ "kind": "request", "request_id": request_id, "prompt": "Delete 3 files?", "options": ["Allow", "Deny"] }))
            .unwrap()
    }

    #[test]
    fn answering_the_first_question_twice_lands_once_and_the_other_run_still_waits() {
        let (host, store) = host_with_runs();
        let (s1, _, r1, _a1) = turn_in_flight(&host);
        let (s2, _, r2, _a2) = {
            let agent = host.start_agent("pi").unwrap();
            let answer = host.send_to(&agent, Turn::new("second task")).unwrap();
            let run = poll(&host, &s1)["turn_id"].as_u64().unwrap();
            (s1.clone(), agent, run, answer)
        };
        assert_eq!(ask(&host, &s1, r1, "a"), json!({}));
        assert_eq!(ask(&host, &s2, r2, "a"), json!({}), "request ids are per run");
        assert_eq!(store.run(r1).unwrap().unwrap().state, RunState::WaitingOnPerson);

        host.answer(r1, "a", &json!("Allow"), true).unwrap();
        let again = host.answer(r1, "a", &json!("Allow"), true).unwrap_err();
        assert!(again.contains("already answered"), "{again}");

        let delivered = poll(&host, &s1);
        assert_eq!(delivered["answers"], json!([{ "turn_id": r1, "request_id": "a", "answer": "Allow" }]), "once, to the run that asked");
        assert!(poll(&host, &s1).get("answers").is_none(), "and never again");
        assert_eq!(store.run(r1).unwrap().unwrap().state, RunState::Running);
        assert_eq!(store.run(r2).unwrap().unwrap().state, RunState::WaitingOnPerson, "the other run still waits");
    }

    #[test]
    fn a_question_never_asked_a_repeated_one_and_one_without_an_id_are_refused() {
        let (host, _) = host_with_runs();
        let (session, _, run, _answer) = turn_in_flight(&host);
        assert!(host.answer(run, "never", &json!(1), true).unwrap_err().contains("never asked"));
        ask(&host, &session, run, "a");
        assert!(ask(&host, &session, run, "a")["refused"].as_str().unwrap().contains("already asked"));
        let blank = event(&host, &session, run, json!({ "kind": "request", "request_id": " ", "prompt": "?" })).unwrap();
        assert!(blank["refused"].as_str().unwrap().contains("request_id"));
    }

    #[test]
    fn an_answer_after_the_run_ended_or_its_harness_left_is_refused_with_why() {
        let (host, _) = host_with_runs();
        let (session, _, run, _answer) = turn_in_flight(&host);
        ask(&host, &session, run, "a");
        complete(&host, &session, run);
        assert!(host.answer(run, "a", &json!("Allow"), true).unwrap_err().contains("ended (done)"));

        let (session, _, run, _answer) = turn_in_flight(&host);
        ask(&host, &session, run, "b");
        host.handle(protocol::DETACH, &json!({ "session": session })).unwrap();
        assert!(host.answer(run, "b", &json!("Allow"), true).unwrap_err().contains("ended (orphaned)"));
        assert!(host.answer(777, "b", &json!("Allow"), true).unwrap_err().contains("does not exist"));
    }

    #[test]
    fn a_connection_that_was_replaced_never_receives_the_answer() {
        let (host, _) = host_with_runs();
        let (old, agent, run, _answer) = turn_in_flight(&host);
        ask(&host, &old, run, "a");
        let token = host.lock().attached["pi"].agents[agent.conversation()].token.clone();
        let reply = host
            .handle(protocol::ATTACH, &json!({ "id": "pi", "name": "pi", "conversations": true, "resume": [
                { "conversation": agent.conversation(), "agent_token": token, "turn_id": run } ]}))
            .unwrap();
        let new = reply["session"].as_str().unwrap().to_string();
        host.answer(run, "a", &json!("Allow"), true).unwrap();
        assert!(host.handle(protocol::POLL, &json!({ "session": old })).is_err(), "the old connection is gone");
        assert_eq!(poll(&host, &new)["answers"][0]["request_id"], "a", "the one answering the run gets it");
    }

    #[test]
    fn without_a_store_a_question_is_refused_rather_than_left_unanswerable() {
        let host = host();
        let (session, _, run, _answer) = turn_in_flight(&host);
        assert!(ask(&host, &session, run, "a")["refused"].as_str().unwrap().contains("keeps no runs"));
        assert!(host.answer(run, "a", &json!(1), true).is_err());
    }

    #[test]
    fn an_agents_question_is_answered_by_agent_and_request_id() {
        let (host, store) = host_with_runs();
        let (session, agent, run, _answer) = turn_in_flight(&host);
        ask(&host, &session, run, "r1");
        assert!(host.answer_for(&agent, "r9", &json!("Yes"), true).unwrap_err().contains("no longer waiting"));
        host.answer_for(&agent, "r1", &json!("Yes"), true).unwrap();
        assert_eq!(poll(&host, &session)["answers"][0], json!({ "turn_id": run, "request_id": "r1", "answer": "Yes" }));
        assert!(host.answer_for(&agent, "r1", &json!("Yes"), true).unwrap_err().contains("no longer waiting"), "once");
        assert_eq!(store.run(run).unwrap().unwrap().state, RunState::Running);
        let stranger = AgentId::new("pi", "c-someone-else");
        ask(&host, &session, run, "r2");
        assert!(host.answer_for(&stranger, "r2", &json!("Yes"), true).is_err(), "only the agent that asked");
    }

    #[test]
    fn interrupting_an_agent_cancels_its_turn_and_keeps_the_agent_its_token_and_its_run_log() {
        let (host, store) = host_with_runs();
        let (session, agent, run, answer) = turn_in_flight(&host);
        let token = host.lock().attached["pi"].agents[agent.conversation()].token.clone();
        assert!(host.interrupt(&agent));
        assert!(crate::collect(answer).unwrap_err().contains(INTERRUPTED_TO_TELL));
        assert_eq!(poll(&host, &session)["cancelled"], json!([run]), "the harness is told to stop that turn");
        assert_eq!(store.run(run).unwrap().unwrap().state, RunState::Cancelled);
        assert!(poll(&host, &session).get("ended").is_none(), "the conversation is not ended");
        assert_eq!(host.lock().attached["pi"].agents[agent.conversation()].token, token, "the same agent, the same token");
        // The person's word is the next turn, to the same conversation.
        let _next = host.send_to(&agent, Turn::new("the file does not exist yet: create it first")).unwrap();
        complete(&host, &session, run);
        let next = poll(&host, &session);
        assert_eq!(next["conversation"], agent.conversation());
        assert!(!host.interrupt(&AgentId::new("pi", "c-nobody")), "nothing in flight there");
    }
}
