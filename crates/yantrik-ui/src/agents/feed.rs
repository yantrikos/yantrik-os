//! Feeding the store from what a turn streams back.
//!
//! Every turn the shell sends an attached mind — from the Lens, from New agent, from an agent's
//! own prompt box — is read here as it streams, into that agent: its text, and its
//! `Chunk::Event`s, which go to the store as *reported*. A harness that writes only text still
//! gets cards: every `⚙️` trail line becomes a reported card, read by #125's one reader of the
//! trail (`crate::trail`), claiming no outcome because the trail does not say one. A harness that
//! sends both writes the line just before the event (docs/harness.md); the store lets the event
//! take the line's place and ignores the trail for the rest of that turn, so no call is shown
//! twice.

use std::sync::mpsc;

use yantrik_harness::{Answer, Chunk};

use super::model::{AgentId, AgentMeta, Provenance};
use crate::trail::{self, ToolCall};

/// One piece of an answer, sorted.
#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    Text(String),
    Call(ToolCall),
}

/// Splits streamed text into prose and trail calls, as it arrives.
///
/// A trail line has to be whole before it can be read, and prose should not wait for anything. So
/// a line is held only while it could still be a call — until its first visible character, and if
/// that is the gear, until its end. Hermes' verbose form puts a call's arguments on the line after
/// it, so a call written as `name([...])` waits one more line for them. A gear inside a code fence
/// is code.
#[derive(Debug, Default)]
pub struct TrailLines {
    pending: String,
    /// The current line is known to be prose and is passed through as it comes.
    prose: bool,
    /// A verbose call waiting for its arguments on the next line.
    held: Option<String>,
    fenced: bool,
}

impl TrailLines {
    pub fn push(&mut self, delta: &str) -> Vec<Piece> {
        let mut out = Vec::new();
        for segment in delta.split_inclusive('\n') {
            let whole = segment.ends_with('\n');
            if self.prose {
                out.push(Piece::Text(segment.to_string()));
                if whole {
                    self.prose = false;
                    self.note_fence(segment);
                }
                continue;
            }
            self.pending.push_str(segment);
            if whole {
                let line = std::mem::take(&mut self.pending);
                self.line(line, &mut out);
            } else if self.held.is_none() {
                let visible = self.pending.trim_start();
                if !visible.is_empty() && (self.fenced || !visible.starts_with('\u{2699}')) {
                    // Not a call: say it now and keep saying it until the line ends.
                    out.push(Piece::Text(std::mem::take(&mut self.pending)));
                    self.prose = true;
                }
            }
        }
        out
    }

    /// The answer ended: whatever was held is what it is.
    pub fn finish(&mut self) -> Vec<Piece> {
        let mut out = Vec::new();
        if let Some(held) = self.held.take() {
            if let Some((call, _)) = trail::parse(&held, None) {
                out.push(Piece::Call(call));
            }
        }
        let rest = std::mem::take(&mut self.pending);
        if !rest.is_empty() {
            self.classify(rest, &mut out);
        }
        self.prose = false;
        out
    }

    fn line(&mut self, line: String, out: &mut Vec<Piece>) {
        if let Some(held) = self.held.take() {
            let next = line.trim_end_matches(['\n', '\r']);
            if let Some((call, took)) = trail::parse(&held, Some(next)) {
                out.push(Piece::Call(call));
                if took {
                    return;
                }
            }
        }
        self.classify(line, out);
    }

    fn classify(&mut self, line: String, out: &mut Vec<Piece>) {
        if !self.fenced && trail::is_trail(&line) {
            let bare = line.trim_end_matches(['\n', '\r']);
            if verbose(bare) && line.ends_with('\n') {
                self.held = Some(bare.to_string());
                return;
            }
            if let Some((call, _)) = trail::parse(bare, None) {
                out.push(Piece::Call(call));
                return;
            }
        }
        self.note_fence(&line);
        out.push(Piece::Text(line));
    }

    fn note_fence(&mut self, line: &str) {
        if line.trim_start().starts_with("```") {
            self.fenced = !self.fenced;
        }
    }
}

/// Hermes' verbose form, `⚙️ name(['app', 'action'])`: its arguments are on the next line.
fn verbose(line: &str) -> bool {
    let rest = line.trim_start().trim_start_matches('\u{2699}').trim_start_matches('\u{fe0f}').trim();
    match rest.split_once('(') {
        Some((name, keys)) => !name.trim().is_empty() && !name.contains(' ') && keys.trim_end().ends_with(')'),
        None => false,
    }
}

/// Reads one answer into one agent.
struct Reader {
    agent: AgentId,
    lines: TrailLines,
    /// The built-in companion speaks the chat pump's token protocol: `__DONE__` and `__REPLACE__`
    /// are instructions, not text. From anything else they are text.
    builtin: bool,
    replace_next: bool,
    failed: bool,
}

impl Reader {
    fn new(agent: AgentId, builtin: bool) -> Self {
        Reader { agent, lines: TrailLines::default(), builtin, replace_next: false, failed: false }
    }

    fn chunk(&mut self, chunk: &Chunk) {
        let store = super::store();
        #[allow(unreachable_patterns)]
        match chunk {
            Chunk::Text(text) => {
                if self.builtin {
                    match text.as_str() {
                        "__DONE__" => return,
                        "__REPLACE__" => {
                            self.replace_next = true;
                            return;
                        }
                        _ if self.replace_next => {
                            self.replace_next = false;
                            self.lines = TrailLines::default();
                            store.replace_text(&self.agent, text);
                            return;
                        }
                        _ => {}
                    }
                }
                for piece in self.lines.push(text) {
                    self.apply(piece);
                }
            }
            Chunk::Failed(why) => {
                for piece in self.lines.finish() {
                    self.apply(piece);
                }
                store.note(&self.agent, &format!("The turn failed: {why}"));
                self.failed = true;
            }
            // What the agent is doing, beside the text: a card opening, its output, its end. The
            // harness's own account, so reported — and once a turn has them, the store stops
            // making cards out of the trail lines the harness still writes for the chat.
            Chunk::Event(event) => store.event(&self.agent, event, Provenance::Reported),
            _ => {}
        }
    }

    fn apply(&self, piece: Piece) {
        match piece {
            Piece::Text(text) => super::store().text(&self.agent, &text),
            Piece::Call(call) => super::store().trail_call(&self.agent, &call),
        }
    }

    fn finish(mut self, note: Option<&str>) {
        for piece in self.lines.finish() {
            self.apply(piece);
        }
        if let Some(note) = note {
            super::store().note(&self.agent, note);
        }
        super::store().close_turn(&self.agent, !self.failed && note.is_none());
    }
}

/// The agent a mind's one conversation is, today: `<harness>:main`.
pub fn main_agent(harness: &str) -> AgentId {
    AgentId::new(harness, AgentId::MAIN)
}

/// The run the chat's latest turn with `agent` was, as a row id (`mind:main#n`), when that turn
/// did work; None for talk. What the chat's reply links to.
pub fn chat_run(agent: &AgentId) -> Option<String> {
    super::store().read(|s| {
        let a = s.agent(agent)?;
        let t = a.turns.last().filter(|t| t.did_work())?;
        Some(super::RowKey::run(agent, t.n).id())
    })
}

/// Who an agent is, from what the harness host knows of it: its mind's name, and whether that
/// mind holds more than one conversation.
pub fn meta_for(agent: &AgentId) -> AgentMeta {
    let host = crate::wire::harness::host();
    let live = host.and_then(|h| h.agents().into_iter().find(|a| &a.id == agent));
    let name = live
        .as_ref()
        .map(|a| a.harness_name.clone())
        .or_else(|| host.and_then(|h| h.list().into_iter().find(|e| e.id == agent.harness()).map(|e| e.name)))
        .unwrap_or_else(|| agent.harness().to_string());
    let mut meta = AgentMeta::new(agent.clone(), name);
    meta.conversations = live.is_some_and(|a| a.conversations);
    meta
}

/// Record a turn the Lens sent to an attached mind as that mind's `<harness>:main` agent, and
/// hand the answer on to the Lens unchanged.
///
/// The one hook in `wire/chat.rs`. The answer passes through a thread that copies each chunk into
/// the store before forwarding it, so the Lens sees exactly what it saw before. If the Lens stops
/// listening, this stops too and drops the harness's stream, which is what the harness saw before.
///
/// A word to a mind that is still at work is not a new turn yet. Opening one at once ended the
/// turn doing the work — `open_turn` settles an open turn as failed — so asking a busy Hermes
/// "what's the status?" recorded the town model it was building as failed and the agent as idle
/// (#234: "all 15 rows say done while Hermes is still building"). Such a word waits for the first
/// thing the mind says back. If the working turn has ended by then, as it has for a harness that
/// queues the word behind it, the word is the next turn. If it is still open, the mind answered
/// beside its work, and the exchange is kept as a note inside the working turn.
pub fn lens_turn(harness: &str, prompt: &str, answer: Answer) -> Answer {
    let agent = main_agent(harness);
    super::store().upsert_agent(meta_for(&agent));
    let working = working(&agent);
    if !working {
        super::store().open_chat_turn(&agent, prompt);
    }
    let prompt = prompt.to_string();
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new().name("agents-lens-turn".into()).spawn(move || {
        let mut reader = Reader::new(agent.clone(), false);
        // Some(prompt) until the word to a working agent is placed; then `aside` says how.
        let mut unplaced = working.then_some(prompt);
        let mut aside: Option<String> = None;
        let place = |unplaced: &mut Option<String>, aside: &mut Option<String>| {
            if let Some(prompt) = unplaced.take() {
                if self::working(&agent) {
                    super::store().note(&agent, &format!("While it worked, you asked: “{}”", brief(&prompt)));
                    *aside = Some(String::new());
                } else {
                    super::store().open_chat_turn(&agent, &prompt);
                }
            }
        };
        while let Ok(chunk) = answer.recv() {
            place(&mut unplaced, &mut aside);
            match (&mut aside, &chunk) {
                (Some(said), Chunk::Text(text)) => said.push_str(text),
                (Some(said), Chunk::Failed(why)) => said.push_str(&format!(" (failed: {why})")),
                (Some(_), _) => {}
                (None, _) => reader.chunk(&chunk),
            }
            if tx.send(chunk).is_err() {
                if aside.is_none() {
                    reader.finish(Some("The conversation panel stopped listening."));
                }
                return;
            }
        }
        place(&mut unplaced, &mut aside);
        match aside {
            Some(said) if !said.trim().is_empty() => {
                super::store().note(&agent, &format!("It answered beside its work: {}", brief(&said)))
            }
            Some(_) => {}
            None => reader.finish(None),
        }
    });
    if let Err(e) = spawned {
        // No thread, no copy: the Lens must still get its answer. It already has nothing to read
        // from, so say why instead.
        tracing::warn!(error = %e, "could not follow a Lens turn for the Agents screen");
    }
    rx
}

/// Whether `agent` has a turn open in the store: it is at work on something.
fn working(agent: &AgentId) -> bool {
    super::store().read(|s| s.agent(agent).is_some_and(|a| a.open_turn().is_some()))
}

/// One line of an exchange kept as a note: flat, and short enough to read in the session.
fn brief(text: &str) -> String {
    super::progress::brief(text, 300)
}

/// What the desktop notes on a turn a harness picked back up after the shell restarted (#246).
pub const PICKED_UP: &str = "Picked back up after the desktop restarted: what it says from here arrives as usual.";

/// A turn a re-attaching harness was still answering when the shell restarted (#246): open it
/// again in the store, say so, and read the rest of its answer into it. `forward` hands the
/// answer on unchanged, for the Lens to show, when the turn is the Lens's own conversation.
pub fn resumed(agent: AgentId, prompt: &str, answer: Answer, forward: bool) -> Option<Answer> {
    super::store().upsert_agent(meta_for(&agent));
    super::store().open_turn(&agent, prompt);
    super::store().note(&agent, PICKED_UP);
    if !forward {
        record(agent, answer, false);
        return None;
    }
    let (tx, rx) = mpsc::channel();
    let _ = std::thread::Builder::new().name("agents-resumed-turn".into()).spawn(move || {
        let mut reader = Reader::new(agent, false);
        while let Ok(chunk) = answer.recv() {
            reader.chunk(&chunk);
            if tx.send(chunk).is_err() {
                reader.finish(Some("The conversation panel stopped listening."));
                return;
            }
        }
        reader.finish(None);
    });
    Some(rx)
}

/// Record an answer that nothing else is reading — a turn from New agent or an agent's prompt box.
pub fn record(agent: AgentId, answer: Answer, builtin: bool) {
    let _ = std::thread::Builder::new().name("agents-turn".into()).spawn(move || {
        let mut reader = Reader::new(agent, builtin);
        while let Ok(chunk) = answer.recv() {
            let done = builtin && matches!(&chunk, Chunk::Text(t) if t == "__DONE__");
            reader.chunk(&chunk);
            if done {
                break;
            }
        }
        reader.finish(None);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(chunks: &[&str]) -> Vec<Piece> {
        let mut lines = TrailLines::default();
        let mut out: Vec<Piece> = chunks.iter().flat_map(|c| lines.push(c)).collect();
        out.extend(lines.finish());
        out
    }

    /// Prose as one string, calls by their one-line summary — the shape a reader cares about.
    fn shape(pieces: &[Piece]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for piece in pieces {
            match piece {
                Piece::Text(t) => match out.last_mut() {
                    Some(last) if !last.starts_with("CALL ") => last.push_str(t),
                    _ => out.push(t.clone()),
                },
                Piece::Call(c) => out.push(format!("CALL {}", c.summary())),
            }
        }
        out
    }

    #[test]
    fn a_trail_line_split_across_chunks_is_one_call_between_the_prose() {
        let pieces = feed(&[
            "Making it ",
            "now.\n⚙️ os_act studio.gen",
            "erate {\"args\":{\"prompt\":\"a red kite\"}}\n\nDone: one picture.",
        ]);
        assert_eq!(
            shape(&pieces),
            vec![
                "Making it now.\n".to_string(),
                "CALL os_act studio.generate prompt=\"a red kite\"".to_string(),
                "\nDone: one picture.".to_string(),
            ]
        );
    }

    #[test]
    fn prose_is_not_held_back_waiting_for_a_newline() {
        let mut lines = TrailLines::default();
        assert_eq!(lines.push("Hel"), vec![Piece::Text("Hel".into())], "said at once");
        assert_eq!(lines.push("lo"), vec![Piece::Text("lo".into())]);
        // A line that might be a call is held until it is whole.
        assert_eq!(lines.push("\n⚙️ os_apps"), vec![Piece::Text("\n".into())]);
        let rest = lines.finish();
        assert!(matches!(&rest[..], [Piece::Call(c)] if c.name == "os_apps"), "{rest:?}");
    }

    #[test]
    fn hermes_verbose_form_waits_one_line_for_its_arguments() {
        let pieces = feed(&[
            "⚙️ mcp_yantrik_os_os_act(['app', 'action', 'args'])\n",
            "{\"app\": \"studio\", \"action\": \"generate\", \"args\": {\"prompt\": \"a kite\"}}\n",
            "ok",
        ]);
        assert_eq!(
            shape(&pieces),
            vec!["CALL mcp_yantrik_os_os_act studio.generate prompt=\"a kite\"".to_string(), "ok".to_string()]
        );
    }

    #[test]
    fn a_gear_in_prose_or_in_a_code_fence_is_not_a_call() {
        let pieces = feed(&["The gear ⚙️ opens settings.\n```\n⚙️ os_apps\n```\n"]);
        assert!(pieces.iter().all(|p| matches!(p, Piece::Text(_))), "{pieces:?}");
    }

    #[test]
    fn hermes_default_line_is_a_name_and_nothing_more() {
        let pieces = feed(&["⚙️ mcp_yantrik_os_os_act...\n", "Done."]);
        assert_eq!(shape(&pieces), vec!["CALL mcp_yantrik_os_os_act".to_string(), "Done.".to_string()]);
    }

    /// Everything a Lens turn forwarded, read to the end: the turn's thread has then finished
    /// with the store.
    fn drain(answer: Answer) {
        while answer.recv().is_ok() {}
    }

    /// (turn count, last turn's prompt, whether it is open, whether it ended well, its notes).
    fn last_turn(agent: &AgentId) -> (usize, String, bool, Option<bool>, Vec<String>) {
        super::super::store().read(|s| {
            let a = s.agent(agent).expect("the agent is recorded");
            let t = a.turns.last().expect("it has a turn");
            let notes = t
                .items
                .iter()
                .filter_map(|i| match i {
                    super::super::model::Item::Note(n) => Some(n.clone()),
                    _ => None,
                })
                .collect();
            (a.turns.len(), t.prompt.clone(), t.open(), t.ok, notes)
        })
    }

    /// A mind that answers beside its work, as Hermes does ("still working"): the word asked
    /// while it worked is a note in the working turn, and the working turn stays open and is not
    /// failed. It used to be settled as failed the moment the word was sent (#234).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_word_to_a_mind_at_work_does_not_end_its_work() {
        let agent = main_agent("feed-aside");
        let (work_tx, work) = mpsc::channel();
        let work = lens_turn("feed-aside", "build the town model", work);
        work_tx.send(Chunk::Text("Starting on the roads.".into())).unwrap();

        let (status_tx, status) = mpsc::channel();
        let status = lens_turn("feed-aside", "what's the status?", status);
        status_tx.send(Chunk::Text("Hermes is still working.".into())).unwrap();
        drop(status_tx);
        drain(status);

        let (turns, prompt, open, ok, notes) = last_turn(&agent);
        assert_eq!((prompt.as_str(), open, ok), ("build the town model", true, None), "the work goes on");
        assert!(notes.iter().any(|n| n == "While it worked, you asked: “what's the status?”"), "{notes:?}");
        assert!(notes.iter().any(|n| n == "It answered beside its work: Hermes is still working."), "{notes:?}");

        drop(work_tx);
        drain(work);
        let (after, prompt, open, ok, _) = last_turn(&agent);
        assert_eq!((after, prompt.as_str(), open, ok), (turns, "build the town model", false, Some(true)));
    }

    /// A mind whose host queues the word behind its work (pi, DeepSeek): by the time the mind
    /// answers it, the work is done, and the word is the next turn of its own.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_word_queued_behind_the_work_becomes_the_next_turn() {
        let agent = main_agent("feed-queued");
        let (work_tx, work) = mpsc::channel();
        let work = lens_turn("feed-queued", "tidy the photos", work);
        let (next_tx, next) = mpsc::channel();
        let next = lens_turn("feed-queued", "and then the music", next);

        work_tx.send(Chunk::Text("Tidied.".into())).unwrap();
        drop(work_tx);
        drain(work);
        let (first, _, _, ok, _) = last_turn(&agent);
        assert_eq!(ok, Some(true), "the first turn ended as itself, not cut short by the second");

        next_tx.send(Chunk::Text("On it.".into())).unwrap();
        drop(next_tx);
        drain(next);
        let (turns, prompt, open, ok, _) = last_turn(&agent);
        assert_eq!((turns, prompt.as_str(), open, ok), (first + 1, "and then the music", false, Some(true)));
    }
}
