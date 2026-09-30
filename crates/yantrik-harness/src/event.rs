//! What an agent is doing — the structured half of the harness wire.
//!
//! [`crate::protocol::CHUNK`] carries text, and until now text was all there was: a tool call, its
//! result and a command's output reached the shell only if the harness wrote them into its answer
//! as prose (#125 reads those lines back). An agent pane needs more than prose — a card per call
//! that opens when the call starts, fills with its output, and settles with its exit code — so a
//! harness that can say what it is doing sends it here, beside the text.
//!
//! Sent as [`EVENT`]: `{session, turn_id, event: {kind, ...}}`. Everything here is optional for a
//! harness: one that sends no events keeps working exactly as before. And it is forward-tolerant
//! for the shell: an event of a kind this build does not know is dropped by [`Event::parse`], not
//! turned into an error, because a newer harness must not break an older desktop.
//!
//! See `design/agents-workspace-2026-09-23.md`.

use serde::{Deserialize, Serialize};

/// The method a harness calls to report one event during a turn.
pub const EVENT: &str = "harness.event";

/// Which of the person's agents something belongs to: one conversation with one mind, written
/// `<harness id>:<conversation>` — `pi:c3`. A harness that holds a single conversation has the one
/// agent `<harness id>:main`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentId(pub String);

impl AgentId {
    /// The conversation a harness that holds only one is in.
    pub const MAIN: &'static str = "main";

    pub fn new(harness: &str, conversation: &str) -> Self {
        let conversation = if conversation.trim().is_empty() { Self::MAIN } else { conversation };
        AgentId(format!("{harness}:{conversation}"))
    }

    /// The harness this agent runs on.
    pub fn harness(&self) -> &str {
        self.0.split_once(':').map(|(h, _)| h).unwrap_or(&self.0)
    }

    /// The conversation within that harness.
    pub fn conversation(&self) -> &str {
        self.0.split_once(':').map(|(_, c)| c).unwrap_or(Self::MAIN)
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where a piece of a call's output came from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stream {
    /// A tool's result text, or a command's standard output when it has no terminal.
    #[default]
    Stdout,
    Stderr,
    /// The bytes of a command's terminal, escape sequences and all. The shell feeds these to a
    /// terminal emulator rather than printing them.
    Terminal,
}

/// One thing an agent did or is doing, during one turn.
///
/// `call` ties the three tool events of one call together; it is the harness's own id for the
/// call (pi's `toolCallId`, an OpenAI `tool_call.id`), unique within the turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// A tool call began: a card opens, running.
    ToolStart {
        call: String,
        /// The tool as the harness names it: `os_act`, `bash`, `read`.
        name: String,
        /// What it touched, when that can be said: `terminal.run`, `notes`, `~/Pictures`.
        #[serde(default)]
        target: String,
        /// The arguments, whole. The shell shows them on one line and in full on a click.
        #[serde(default)]
        args: serde_json::Value,
    },
    /// Output from a running call, as it arrives.
    ToolOutput {
        call: String,
        #[serde(default)]
        stream: Stream,
        delta: String,
    },
    /// A call ended: the card settles.
    ToolEnd {
        call: String,
        ok: bool,
        /// One line on how it went — "38 files moved", "exit 2: no such file".
        #[serde(default)]
        summary: String,
        /// For a command, how it exited.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
    },
    /// The mind's reasoning, when the harness shares it. Shown folded.
    Thinking { delta: String },
    /// What the agent is doing or waiting on, in a few words: "waiting for your approval".
    Status { text: String },
    /// What the turn cost, when the harness knows. Every field is optional; send what you have.
    Usage {
        #[serde(default)]
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
    },
    /// The agent asks the person something and waits (#25). The answer comes back on a later
    /// poll as `answers: [{turn_id, request_id, answer}]`, once, to this run only: a second answer,
    /// or one to a question this run never asked, is refused before it reaches the harness. Only
    /// the answer bound to `request_id` counts; prose in the chat is never taken as one.
    Request {
        /// The harness's own id for the question, unique within the run.
        request_id: String,
        /// What to ask, as the person should read it.
        prompt: String,
        /// The answers to offer as buttons, when there is a fixed set ("Allow", "Deny").
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        options: Vec<String>,
    },
}

impl Event {
    /// Every `kind` this build reads. Anything else is a newer harness talking to an older
    /// desktop, and is ignored; one of these that does not parse is malformed, and is counted.
    pub const KINDS: &'static [&'static str] =
        &["tool_start", "tool_output", "tool_end", "thinking", "status", "usage", "request"];

    /// Read one event from the wire. `None` for a kind this build does not know, or one that is
    /// malformed: the caller logs it and carries on, and the turn is not failed over it.
    pub fn parse(value: &serde_json::Value) -> Option<Event> {
        serde_json::from_value(value.clone()).ok()
    }

    /// The `kind` this event travels as.
    pub fn kind(&self) -> &'static str {
        match self {
            Event::ToolStart { .. } => "tool_start",
            Event::ToolOutput { .. } => "tool_output",
            Event::ToolEnd { .. } => "tool_end",
            Event::Thinking { .. } => "thinking",
            Event::Status { .. } => "status",
            Event::Usage { .. } => "usage",
            Event::Request { .. } => "request",
        }
    }

    /// The call this event belongs to, for the three tool events.
    pub fn call(&self) -> Option<&str> {
        match self {
            Event::ToolStart { call, .. } | Event::ToolOutput { call, .. } | Event::ToolEnd { call, .. } => {
                Some(call)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_tool_call_reads_off_the_wire_as_the_python_library_writes_it() {
        let start = Event::parse(&json!({
            "kind": "tool_start", "call": "t1", "name": "os_act",
            "target": "terminal.run", "args": {"command": "ls -la"}
        }))
        .expect("a well-formed tool_start is read");
        assert_eq!(start.call(), Some("t1"));
        assert!(matches!(&start, Event::ToolStart { target, .. } if target == "terminal.run"));

        let out = Event::parse(&json!({"kind": "tool_output", "call": "t1", "delta": "total 8\n"}))
            .expect("stream defaults to stdout when left out");
        assert!(matches!(out, Event::ToolOutput { stream: Stream::Stdout, .. }));

        let end = Event::parse(&json!({"kind": "tool_end", "call": "t1", "ok": false, "exit_code": 2}))
            .expect("summary defaults to empty");
        assert!(matches!(end, Event::ToolEnd { ok: false, exit_code: Some(2), .. }));
    }

    #[test]
    fn a_kind_this_build_does_not_know_is_dropped_not_fatal() {
        // A newer harness talking to an older desktop: the turn must go on.
        assert_eq!(Event::parse(&json!({"kind": "screenshot", "png": "…"})), None);
        assert_eq!(Event::parse(&json!({"no_kind": true})), None);
    }

    #[test]
    fn usage_takes_whatever_the_harness_has() {
        let usage = Event::parse(&json!({"kind": "usage", "model": "qwen3.8-27b", "output_tokens": 412}))
            .expect("partial usage is still usage");
        assert!(matches!(usage, Event::Usage { input_tokens: None, output_tokens: Some(412), .. }));
    }

    #[test]
    fn an_event_round_trips_through_its_own_serialisation() {
        let event = Event::ToolEnd { call: "c".into(), ok: true, summary: "done".into(), exit_code: Some(0) };
        let wire = serde_json::to_value(&event).unwrap();
        assert_eq!(wire["kind"], "tool_end");
        assert_eq!(Event::parse(&wire), Some(event));
    }

    #[test]
    fn every_kind_this_build_writes_is_one_it_reads() {
        let every = [
            Event::ToolStart { call: "c".into(), name: "n".into(), target: String::new(), args: json!({}) },
            Event::ToolOutput { call: "c".into(), stream: Stream::Stdout, delta: "d".into() },
            Event::ToolEnd { call: "c".into(), ok: true, summary: String::new(), exit_code: None },
            Event::Thinking { delta: "d".into() },
            Event::Status { text: "t".into() },
            Event::Usage { model: String::new(), input_tokens: None, output_tokens: None, cost_usd: None },
            Event::Request { request_id: "r".into(), prompt: "p".into(), options: vec![] },
        ];
        assert_eq!(every.len(), Event::KINDS.len());
        for event in every {
            let wire = serde_json::to_value(&event).unwrap();
            assert_eq!(wire["kind"], event.kind());
            assert!(Event::KINDS.contains(&event.kind()), "{} is written but not read", event.kind());
        }
    }

    #[test]
    fn an_agent_is_a_harness_and_a_conversation() {
        let agent = AgentId::new("pi", "c3");
        assert_eq!(agent.to_string(), "pi:c3");
        assert_eq!((agent.harness(), agent.conversation()), ("pi", "c3"));
        // A harness with one conversation has the one agent, named the same way every time.
        assert_eq!(AgentId::new("hermes", "").to_string(), "hermes:main");
        assert_eq!(serde_json::to_value(&agent).unwrap(), json!("pi:c3"));
    }
}
