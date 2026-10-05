//! Which mind is answering.
//!
//! The shell used to have exactly one: `yantrik-companion`, compiled in, reached through
//! `CompanionCommand::SendMessage` on a channel. That is a fine companion and a poor assumption —
//! `yantrik-mind`, OpenClaw and hermes-agent all exist, all want to drive this desktop, and none
//! of them is the companion. This crate is the seam between the shell and whichever one is
//! answering.
//!
//! # What the OS owns, and what it does not
//!
//! A harness already knows how to be itself. `yantrik-mind` has its own models, config and
//! deployment; hermes-agent has its own; OpenClaw has its own. **None of that is the OS's
//! business**, and an earlier draft of this crate got it exactly backwards — it held endpoints,
//! model names and API keys in YAML files, which is the OS reimplementing, badly, the setup each
//! harness already does properly.
//!
//! So the OS owns one thing: **which mind the person is talking to**. Everything else belongs to
//! the harness, and the interface between them is [`protocol`] — seven methods, spoken by the
//! harness, over the socket bus this OS already has.
//!
//! # Adding a harness
//!
//! Write the poll loop. In any language with a JSON-RPC client, it is about forty lines:
//! announce yourself, ask for a turn, stream the answer back. There is nothing to register,
//! nothing to install, no endpoint for the OS to store, and no restart — a harness appears in the
//! picker when it attaches and is gone when it stops polling.
//!
//! `examples/echo-harness.rs` is a complete one, and it is short on purpose.
//!
//! Driving the desktop is separate and already exists: an attached harness reads and steers the
//! OS through `app.describe` / `app.act` (or `yos`), which is permission-graded and works the same
//! for a harness as for anything else.
//!
//! # Streaming is the contract
//!
//! [`Harness::send`] hands back a [`Receiver`] immediately and the chunks arrive as they are
//! produced. This is not a preference: the shell already renders a reply token by token from
//! `CompanionCommand::SendMessage`'s channel, and a trait that returned a finished `String` would
//! make every harness feel slower than the one it replaced. A backend with no streaming endpoint
//! sends one [`Chunk::Text`] and closes, which is honest and still works.
//!
//! # Health is asked, never assumed
//!
//! A picker that lists a harness the machine cannot reach is a picker that produces silence when
//! you choose it. [`Harness::health`] exists so the UI can say *why* before a person commits a
//! question to it — unreachable, or reachable but not configured, with the reason attached.

pub mod event;
pub mod host;
pub mod protocol;
pub mod redact;
pub mod run_store;

pub use event::{AgentId, Event};
pub use host::{random_hex, AgentEntry, AgentState, Entry, EventCounts, Host, LateAnswer, Resumed, TurnEnd};

use std::sync::mpsc::Receiver;

/// One thing said to a harness.
///
/// Deliberately not a transcript. Conversation history lives in the shell, which owns the panel
/// and the memory; a harness that keeps its own history (the companion does) uses the text and
/// ignores the rest, and a stateless HTTP endpoint gets `context` to prepend. Putting the whole
/// history in this struct would mean every adapter had to agree on how to serialise a
/// conversation, which is exactly the coupling this crate exists to remove.
#[derive(Clone, Debug, Default)]
pub struct Turn {
    /// What the person typed.
    pub text: String,
    /// Optional system framing — who the harness is, what it is looking at.
    pub context: Option<String>,
    /// Where the turn came from: the Lens at the desk, or a channel on the person's phone.
    /// `None` for a turn the desktop does not say (a harness that predates it sees no difference).
    pub origin: Option<protocol::Origin>,
}

impl Turn {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), context: None, origin: None }
    }

    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }

    pub fn with_origin(mut self, origin: protocol::Origin) -> Self {
        self.origin = Some(origin);
        self
    }

    /// Whether the person asked this from away from the machine.
    pub fn is_remote(&self) -> bool {
        self.origin.as_ref().is_some_and(|o| o.remote)
    }
}

/// One piece of an answer, as it arrives.
///
/// Not `Eq`: an [`Event`] can carry a cost in dollars and a tool's arguments, and neither has a
/// total equality.
#[derive(Clone, Debug, PartialEq)]
pub enum Chunk {
    /// Text to append to the reply.
    Text(String),
    /// The turn failed. Carries what to show a person — a stream that simply stopped would be
    /// indistinguishable from a harness that had nothing more to say.
    Failed(String),
    /// What the agent is doing, beside the text: a tool call opening, its output, its end, its
    /// thinking, what it cost. Already checked by the host — in order, for a turn in flight, from
    /// the harness that holds it — but still the harness's own account; see `crate::event`. A
    /// reader that only wants the answer skips these, as [`collect`] does.
    Event(Event),
}

/// A stream of answer chunks. It ends when the channel closes.
pub type Answer = Receiver<Chunk>;

/// Whether a harness can be used right now, and if not, why not.
///
/// The distinction matters to the person choosing: `Unreachable` is usually something to fix on
/// the network or the other machine, `NotConfigured` is something to fix here, in Settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Health {
    Ready,
    /// Configured, but nothing answered.
    Unreachable(String),
    /// Missing something it needs before it could be reached at all — an endpoint, a key.
    NotConfigured(String),
}

impl Health {
    pub fn is_ready(&self) -> bool {
        matches!(self, Health::Ready)
    }

    /// One line for the picker, beside the name.
    pub fn summary(&self) -> String {
        match self {
            Health::Ready => "ready".to_string(),
            Health::Unreachable(why) => format!("unreachable — {why}"),
            Health::NotConfigured(why) => format!("not configured — {why}"),
        }
    }
}

/// What a harness can do, so the UI offers only what is actually there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// Chunks arrive as they are produced rather than all at once at the end.
    pub streaming: bool,
    /// Can run the OS's tools.
    pub tools: bool,
    /// Has memory that outlives the turn.
    pub memory: bool,
}

/// A mind compiled into the shell.
///
/// This is for built-ins — today, the companion — and there is deliberately no reason for a new
/// harness to implement it. An external harness attaches over [`protocol`] instead, which needs
/// no Rust, no rebuild, and nothing from this OS but a socket.
pub trait Harness: Send + Sync {
    /// Stable id, as used in config and in `set_active`.
    fn id(&self) -> &str;

    /// What a person sees in the picker.
    fn name(&self) -> &str;

    fn capabilities(&self) -> Capabilities;

    /// Whether this could answer right now. May do IO; the UI calls it off the UI thread.
    fn health(&self) -> Health;

    /// Put one turn to this harness. Returns immediately; chunks arrive on the channel.
    fn send(&self, turn: Turn) -> Answer;
}

/// Collect a whole answer, for callers that cannot stream.
///
/// Returns `Err` on the first [`Chunk::Failed`], because half an answer followed by a failure is
/// not an answer, and a caller that concatenated both would show the error as if the harness had
/// said it.
pub fn collect(answer: Answer) -> Result<String, String> {
    let mut text = String::new();
    for chunk in answer {
        match chunk {
            Chunk::Text(part) => text.push_str(&part),
            Chunk::Failed(why) => return Err(why),
            // What the agent did is not what it said.
            Chunk::Event(_) => {}
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn answer_of(chunks: Vec<Chunk>) -> Answer {
        let (tx, rx) = mpsc::channel();
        for c in chunks {
            tx.send(c).unwrap();
        }
        rx
    }

    #[test]
    fn collecting_joins_the_chunks_in_order() {
        let answer = answer_of(vec![
            Chunk::Text("Hello".into()),
            Chunk::Text(", ".into()),
            Chunk::Text("world".into()),
        ]);
        assert_eq!(collect(answer).unwrap(), "Hello, world");
    }

    #[test]
    fn a_failure_partway_through_is_a_failure_not_a_partial_answer() {
        let answer = answer_of(vec![
            Chunk::Text("I was saying".into()),
            Chunk::Failed("the connection dropped".into()),
        ]);
        assert_eq!(collect(answer).unwrap_err(), "the connection dropped");
    }

    #[test]
    fn collecting_an_answer_skips_what_the_agent_did_along_the_way() {
        let answer = answer_of(vec![
            Chunk::Text("Looking. ".into()),
            Chunk::Event(Event::ToolStart {
                call: "t1".into(),
                name: "os_apps".into(),
                target: String::new(),
                args: serde_json::json!({}),
            }),
            Chunk::Event(Event::ToolEnd { call: "t1".into(), ok: true, summary: String::new(), exit_code: None }),
            Chunk::Text("Two windows.".into()),
        ]);
        assert_eq!(collect(answer).unwrap(), "Looking. Two windows.");
    }

    #[test]
    fn health_explains_itself_for_the_picker() {
        assert!(Health::Ready.is_ready());
        assert!(!Health::Unreachable("connection refused".into()).is_ready());
        assert_eq!(
            Health::NotConfigured("no endpoint".into()).summary(),
            "not configured — no endpoint"
        );
    }
}
