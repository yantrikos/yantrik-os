//! The `redact` event: erasing the shell's copies of a conversation at the person's request.
//!
//! A mind that asked "Keep or erase?" and was answered *Erase* erases its own memory, then sends
//! `{"kind": "redact", "request_id", "needles": [{"sha256", "len"}]}` on the run that asked. The
//! host takes it here rather than as one of the turn's events: it may arrive after the turn
//! closed, and it is never handed to a reader, logged or kept — not even as digests.
//!
//! The run store decides whether it may happen at all (`run_store::erase` has the rule: a question
//! this run asked, answered with the offered `Erase`, from the harness and session that hold the
//! run, while the run is in flight or within five minutes of its end, once per question) and
//! erases its own copy. Only then is the shell's [`ShellRedactor`] asked to erase the agent's pane
//! transcript. The reply says how many places, and where: `{"redacted": n, "where":
//! ["transcript", "runs"]}`, or `{"refused": why}` with nothing changed.
//!
//! # In steps, so nothing is searched under a lock and the search is bounded
//!
//! 1. The rule is checked, and what searching would cost is worked out from the raw lengths of the
//!    texts alone ([`RunStore::redact_size`], [`ShellRedactor::size`]): over `redact::MAX_WORK`
//!    the whole `redact` is refused ("too much to search; …") before anything is copied, hashed or
//!    touched. The third such refusal uses the question up.
//! 2. The run store's texts are copied out ([`RunStore::prepare_redact`]) and the shell copies the
//!    agent's ([`ShellRedactor::prepare`]), each lock held only to copy; the exact cost of the
//!    copies is checked against the same limit.
//! 3. Both copies are searched, with no lock held.
//! 4. The run store applies what it found in one `IMMEDIATE` transaction, checking the rule again
//!    and claiming the question; then the shell applies what it found under its own lock. Each
//!    match is checked again against the text as it is by then, and skipped if it no longer
//!    hashes to its needle.
//!
//! [`RunStore::prepare_redact`]: crate::run_store::RunStore::prepare_redact
//! [`RunStore::redact_size`]: crate::run_store::RunStore::redact_size

use std::sync::Arc;

use super::{refused, Host};
use crate::event::{AgentId, Event};
use crate::protocol;
use crate::redact::{self, Needle, Search};
use crate::run_store::{now_ms, Erasure};

/// What the shell is asked to erase from its own copy of one agent's conversation.
#[derive(Debug, Clone)]
pub struct ShellErasure<'a> {
    /// The Keep/Erase question, which the person answered *Erase*: what the record is keyed by.
    pub request_id: &'a str,
    pub needles: &'a [Needle],
    /// How many places the run store already erased, so the shell's record can say the whole.
    pub places_in_runs: usize,
}

/// What the shell erased from its copy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShellErased {
    /// Places whose words were replaced with the marker.
    pub places: usize,
    /// Records that keep their words (a tool call, an approval) whose display now shows the
    /// marker instead.
    pub masked: usize,
}

/// The shell's half of an erasure, in the same steps as the run store's (see the module docs).
pub trait ShellRedactor: Send + Sync {
    /// How many raw bytes of text [`ShellRedactor::prepare`] would copy, without copying any.
    fn size(&self, agent: &AgentId) -> u64;
    /// Copy the agent's texts out of the shell's store, holding its lock only to copy.
    fn prepare(&self, agent: &AgentId) -> Result<Box<dyn ShellPlan>, String>;
}

/// The shell's copy of one agent's texts, to be measured, searched and then applied.
pub trait ShellPlan: Send {
    /// What searching it will cost (`redact::MAX_WORK`'s units).
    fn work(&self, search: &Search) -> u64;
    /// Search the copy. No lock is held.
    fn search(&mut self, search: &Search);
    /// Apply what was found to the agent's session under the shell's lock, checking each match
    /// again, and write the session to disk before returning.
    fn apply(self: Box<Self>, erasure: &ShellErasure<'_>) -> Result<ShellErased, String>;
}

/// The shell's half, as the host holds it.
pub type Redactor = Arc<dyn ShellRedactor>;

impl Host {
    /// The same host, asking `redactor` to erase the shell's own copy of a conversation when a
    /// `redact` is accepted. Without one, only the run store is erased, and the reply says so.
    pub fn with_redactor(mut self, redactor: impl ShellRedactor + 'static) -> Host {
        self.redactor = Some(Arc::new(redactor));
        self
    }

    /// One `redact` from `harness`'s session `session`, sent on run `run_id`. A refusal is an
    /// answer, never an error, and changes nothing.
    pub(super) fn redact(&self, run_id: u64, harness: &str, session: &str, raw: &serde_json::Value) -> serde_json::Value {
        let outcome = self.try_redact(run_id, harness, session, raw);
        let mut state = self.lock();
        match outcome {
            Ok(reply) => {
                state.events.redacted += 1;
                reply
            }
            Err(why) => {
                state.events.redact_refused += 1;
                // The reason only: never the needles, which are digests of what the person wants gone.
                tracing::info!(harness, run = run_id, why = %why, "redact refused; nothing changed");
                refused(why)
            }
        }
    }

    fn try_redact(&self, run_id: u64, harness: &str, session: &str, raw: &serde_json::Value) -> Result<serde_json::Value, String> {
        let size = serde_json::to_string(raw).map(|s| s.len()).unwrap_or(usize::MAX);
        if size > protocol::MAX_EVENT_BYTES {
            return Err(format!("this event is {size} bytes and one event may be at most {}", protocol::MAX_EVENT_BYTES));
        }
        let store = self.runs.as_ref().ok_or("this desktop keeps no runs, so it cannot tell what the person answered")?;
        // The parse error is not repeated: it could quote a digest back.
        let Ok(Event::Redact { request_id, needles }) = serde_json::from_value::<Event>(raw.clone()) else {
            return Err("a malformed `redact`: it needs `request_id` and `needles: [{sha256, len}]`".to_string());
        };
        if request_id.trim().is_empty() {
            return Err("a `redact` needs the `request_id` of the question the person answered".to_string());
        }
        redact::validate(&needles)?;
        let search = Search::new(&needles);

        // 1. The rule, and how much there is to search, from lengths alone: nothing is copied yet.
        let erasure = Erasure { run_id, request_id: &request_id, harness, owner: session, needles: &needles };
        let size = store.redact_size(&erasure, now_ms()).map_err(|r| r.to_string())?;
        let agent = AgentId::new(&size.harness, &size.conversation);
        let shell_bytes = self.redactor.as_ref().map_or(0, |redactor| redactor.size(&agent));
        let too_much = || match store.too_much(&erasure) {
            Ok(refusal) | Err(refusal) => refusal.to_string(),
        };
        if search.estimate(size.bytes.saturating_add(shell_bytes)) > redact::MAX_WORK {
            return Err(too_much());
        }
        // 2. Copies of the texts, each store's lock held only to copy.
        let mut runs = store.prepare_redact(&erasure, now_ms()).map_err(|r| r.to_string())?;
        let mut shell_failed = None;
        let mut shell = match &self.redactor {
            Some(redactor) => match redactor.prepare(&agent) {
                Ok(plan) => Some(plan),
                Err(why) => {
                    tracing::error!(agent = %agent, run = run_id, why = %why, "the shell could not read the transcript to erase");
                    shell_failed = Some(why);
                    None
                }
            },
            None => {
                shell_failed = Some("this desktop keeps no transcript to erase".to_string());
                None
            }
        };
        // ... and measured exactly before anything is hashed.
        let work = runs.work(&search).saturating_add(shell.as_ref().map_or(0, |plan| plan.work(&search)));
        if work > redact::MAX_WORK {
            return Err(too_much());
        }
        // 3. Searched with no lock held.
        runs.search(&search);
        if let Some(plan) = shell.as_mut() {
            plan.search(&search);
        }
        // 4. Applied: the run store first, its rule checked again; then the shell.
        let erased = store.apply_redact(&erasure, now_ms(), &runs).map_err(|r| r.to_string())?;
        if !erased.checkpointed {
            tracing::error!(run = run_id, "erased, but the run store's write-ahead log could not be emptied yet");
        }
        let mut places = erased.places;
        let mut masked = 0;
        let mut places_in: Vec<&str> = Vec::new();
        if let Some(plan) = shell {
            let asked = ShellErasure { request_id: &request_id, needles: &needles, places_in_runs: erased.places };
            match plan.apply(&asked) {
                Ok(done) => {
                    places += done.places;
                    masked = done.masked;
                    places_in.push("transcript");
                }
                Err(why) => {
                    tracing::error!(agent = %agent, run = run_id, why = %why, "the run store was erased and the transcript was not");
                    shell_failed = Some(why);
                }
            }
        }
        places_in.push("runs");
        if let Err(e) = store.set_redaction_places(run_id, &request_id, places as u64) {
            tracing::error!(run = run_id, error = %e, "the erasure's count was not recorded");
        }
        tracing::info!(agent = %agent, run = run_id, places, masked, "erased at the person's request");
        let mut reply = serde_json::json!({ "redacted": places, "where": places_in });
        if masked > 0 {
            reply["masked"] = serde_json::json!(masked);
        }
        if let Some(why) = shell_failed {
            reply["transcript"] = serde_json::json!(why);
        }
        if let Some(warning) = erased.warning {
            reply["warning"] = serde_json::json!(warning);
        }
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_store::{RunState, RunStore};
    use crate::{Chunk, Turn};
    use serde_json::json;
    use std::sync::Mutex;

    /// What the shell's redactor was asked: the agent, the request, how many needles.
    type Asked = Arc<Mutex<Vec<(AgentId, String, usize)>>>;

    /// A shell that says it erased two places and masked one, and keeps what it was asked. Its
    /// texts cost `work` to search.
    struct FakeShell {
        asked: Asked,
        work: u64,
    }

    struct FakePlan {
        agent: AgentId,
        asked: Asked,
        work: u64,
    }

    impl ShellRedactor for FakeShell {
        fn size(&self, _: &AgentId) -> u64 {
            self.work
        }

        fn prepare(&self, agent: &AgentId) -> Result<Box<dyn ShellPlan>, String> {
            Ok(Box::new(FakePlan { agent: agent.clone(), asked: self.asked.clone(), work: self.work }))
        }
    }

    impl ShellPlan for FakePlan {
        fn work(&self, _: &Search) -> u64 {
            self.work
        }

        fn search(&mut self, _: &Search) {}

        fn apply(self: Box<Self>, e: &ShellErasure<'_>) -> Result<ShellErased, String> {
            self.asked.lock().unwrap().push((self.agent.clone(), e.request_id.to_string(), e.needles.len()));
            Ok(ShellErased { places: 2, masked: 1 })
        }
    }

    fn host_with_shell() -> (Host, Arc<RunStore>, Asked) {
        host_with_shell_costing(0)
    }

    fn host_with_shell_costing(work: u64) -> (Host, Arc<RunStore>, Asked) {
        let store = Arc::new(RunStore::in_memory().unwrap());
        let asked: Asked = Arc::default();
        let host = Host::new(vec![]).with_runs(store.clone()).with_redactor(FakeShell { asked: asked.clone(), work });
        (host, store, asked)
    }

    fn call(host: &Host, method: &str, params: serde_json::Value) -> serde_json::Value {
        host.handle(method, &params).unwrap()
    }

    /// A pi agent whose run said "Priya" across two chunks, asked Keep/Erase as `forget`, and was
    /// answered `answer`: (session, agent, run, the reader's answer).
    fn answered(host: &Host, answer: &str) -> (String, AgentId, u64, crate::Answer) {
        answered_by(host, answer, true)
    }

    /// [`answered`], the answer pressed (`true`) or typed (`false`).
    fn answered_by(host: &Host, answer: &str, pressed: bool) -> (String, AgentId, u64, crate::Answer) {
        let session = call(host, protocol::ATTACH, json!({ "id": "pi", "name": "pi", "conversations": true }))["session"]
            .as_str()
            .unwrap()
            .to_string();
        let agent = host.start_agent("pi").unwrap();
        let reader = host.send_to(&agent, Turn::new("forget my sister's name")).unwrap();
        let run = call(host, protocol::POLL, json!({ "session": session }))["turn_id"].as_u64().unwrap();
        for delta in ["Your sister is Pri", "ya."] {
            call(host, protocol::CHUNK, json!({ "session": session, "turn_id": run, "delta": delta }));
        }
        let ask = json!({ "kind": "request", "request_id": "forget", "prompt": "Forget your sister's name, \u{201c}Priya\u{201d}?", "options": ["Keep", "Erase"] });
        assert_eq!(call(host, protocol::EVENT, json!({ "session": session, "turn_id": run, "event": ask })), json!({}));
        host.answer(run, "forget", &json!(answer), pressed).unwrap();
        (session, agent, run, reader)
    }

    fn redact(host: &Host, session: &str, run: u64, request_id: &str) -> serde_json::Value {
        let event = json!({ "kind": "redact", "request_id": request_id, "needles": [Needle::of("Priya")] });
        call(host, protocol::EVENT, json!({ "session": session, "turn_id": run, "event": event }))
    }

    fn reply_text(store: &RunStore, run: u64) -> String {
        store
            .events(run, 0, crate::run_store::PAGE_MAX)
            .unwrap()
            .events
            .into_iter()
            .filter(|e| e.kind == "text")
            .map(|e| e.payload["delta"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn after_an_erase_answer_the_run_and_the_transcript_are_erased_and_the_reply_says_where() {
        let (host, store, asked) = host_with_shell();
        let (session, agent, run, reader) = answered(&host, "Erase");
        let reply = redact(&host, &session, run, "forget");
        assert_eq!(reply, json!({ "redacted": 4, "where": ["transcript", "runs"], "masked": 1 }));
        assert_eq!(reply_text(&store, run), format!("Your sister is {}.", redact::MARKER));
        assert_eq!(*asked.lock().unwrap(), vec![(agent, "forget".to_string(), 1)]);
        assert_eq!(store.redactions("pi", agent_conversation(&store, run).as_str()).unwrap()[0].places, 4);
        // Never handed to the reader as an event, and never written to the run's log.
        assert!(!reader.try_iter().any(|c| matches!(c, Chunk::Event(Event::Redact { .. }))));
        assert!(store.events(run, 0, 500).unwrap().events.iter().all(|e| !e.payload.to_string().contains("sha256")));
        assert_eq!(host.event_counts().redacted, 1);
    }

    fn agent_conversation(store: &RunStore, run: u64) -> String {
        store.run(run).unwrap().unwrap().conversation
    }

    #[test]
    fn a_keep_an_unknown_question_and_a_second_use_are_refused_and_change_nothing() {
        let (host, store, asked) = host_with_shell();
        let (session, _, run, _reader) = answered(&host, "Keep");
        assert!(redact(&host, &session, run, "forget")["refused"].as_str().unwrap().contains("Erase"));
        assert!(redact(&host, &session, run, "never-asked")["refused"].as_str().unwrap().contains("never asked"));
        assert_eq!(reply_text(&store, run), "Your sister is Priya.");
        assert!(asked.lock().unwrap().is_empty(), "the shell is not asked unless the rule held");

        let (host, _, asked) = host_with_shell();
        let (session, _, run, _reader) = answered(&host, "Erase");
        assert!(redact(&host, &session, run, "forget").get("redacted").is_some());
        assert!(redact(&host, &session, run, "forget")["refused"].as_str().unwrap().contains("one redaction per question"));
        assert_eq!(asked.lock().unwrap().len(), 1);
        assert_eq!((host.event_counts().redacted, host.event_counts().redact_refused), (1, 1));
    }

    #[test]
    fn another_harness_cannot_erase_a_run_it_does_not_hold() {
        let (host, store, asked) = host_with_shell();
        let (_, _, run, _reader) = answered(&host, "Erase");
        let other = call(&host, protocol::ATTACH, json!({ "id": "hermes", "name": "hermes" }))["session"].as_str().unwrap().to_string();
        assert!(redact(&host, &other, run, "forget")["refused"].as_str().unwrap().contains("not held by this harness"));
        assert_eq!(reply_text(&store, run), "Your sister is Priya.");
        assert!(asked.lock().unwrap().is_empty());
    }

    #[test]
    fn a_turn_already_closed_can_still_be_erased_within_the_window() {
        let (host, store, _) = host_with_shell();
        let (session, _, run, _reader) = answered(&host, "Erase");
        call(&host, protocol::COMPLETE, json!({ "session": session, "turn_id": run }));
        assert_eq!(store.run(run).unwrap().unwrap().state, RunState::Done);
        assert_eq!(redact(&host, &session, run, "forget")["redacted"], 4);
    }

    #[test]
    fn a_malformed_redact_is_refused_without_quoting_it() {
        let (host, _, _) = host_with_shell();
        let (session, _, run, _reader) = answered(&host, "Erase");
        let digest = Needle::of("Priya").sha256;
        for event in [
            json!({ "kind": "redact", "request_id": "forget", "needles": [{ "sha256": digest.to_uppercase(), "len": 5 }] }),
            json!({ "kind": "redact", "request_id": "forget", "needles": [{ "sha256": digest, "len": 0 }] }),
            json!({ "kind": "redact", "request_id": "forget", "needles": [] }),
            json!({ "kind": "redact", "request_id": "forget", "needles": [{ "sha256": 7, "len": 5 }] }),
            json!({ "kind": "redact", "needles": [{ "sha256": digest, "len": 5 }] }),
        ] {
            let reply = call(&host, protocol::EVENT, json!({ "session": session, "turn_id": run, "event": event }));
            let why = reply["refused"].as_str().expect("refused");
            assert!(!why.to_lowercase().contains(&digest), "the refusal does not repeat a digest");
        }
    }

    #[test]
    fn without_a_run_store_a_redact_is_refused() {
        let host = Host::new(vec![]);
        let session = call(&host, protocol::ATTACH, json!({ "id": "pi", "name": "pi" }))["session"].as_str().unwrap().to_string();
        let agent = host.ensure_main("pi").unwrap();
        let _answer = host.send_to(&agent, Turn::new("hi")).unwrap();
        let run = call(&host, protocol::POLL, json!({ "session": session }))["turn_id"].as_u64().unwrap();
        assert!(redact(&host, &session, run, "forget")["refused"].as_str().unwrap().contains("keeps no runs"));
    }

    #[test]
    fn too_much_to_search_across_the_runs_and_the_transcript_is_refused_and_changes_nothing() {
        // The run store's few words are cheap; the shell's transcript takes it over the limit.
        let (host, store, asked) = host_with_shell_costing(redact::MAX_WORK);
        let (session, _, run, _reader) = answered(&host, "Erase");
        let reply = redact(&host, &session, run, "forget");
        assert_eq!(reply, json!({ "refused": "too much to search; ask again with fewer or shorter needles" }));
        assert_eq!(reply_text(&store, run), "Your sister is Priya.");
        assert!(asked.lock().unwrap().is_empty(), "the shell applied nothing");
        assert!(store.redactions("pi", agent_conversation(&store, run).as_str()).unwrap().is_empty(), "the question is not used up");
    }

    #[test]
    fn an_erasure_that_committed_but_could_not_put_secure_delete_back_says_so_and_is_not_refused() {
        let (host, store, _) = host_with_shell();
        let (session, _, run, _reader) = answered(&host, "Erase");
        crate::run_store::FAIL_RESTORE.with(|fail| fail.set(true));
        let reply = redact(&host, &session, run, "forget");
        crate::run_store::FAIL_RESTORE.with(|fail| fail.set(false));
        assert_eq!(
            reply,
            json!({ "redacted": 4, "where": ["transcript", "runs"], "masked": 1,
                    "warning": "secure_delete could not be restored on this connection" })
        );
        assert_eq!(reply_text(&store, run), format!("Your sister is {}.", redact::MARKER));
    }

    #[test]
    fn a_typed_erase_and_a_needle_the_question_did_not_quote_are_refused_over_the_wire() {
        let (host, store, asked) = host_with_shell();
        let (session, _, run, _reader) = answered_by(&host, "Erase", false);
        assert!(redact(&host, &session, run, "forget")["refused"].as_str().unwrap().contains("typed"));
        assert_eq!(reply_text(&store, run), "Your sister is Priya.");

        let (host, store, _) = host_with_shell();
        let (session, _, run, _reader) = answered(&host, "Erase");
        let event = json!({ "kind": "redact", "request_id": "forget", "needles": [Needle::of("Priya"), Needle::of("sister is")] });
        let reply = call(&host, protocol::EVENT, json!({ "session": session, "turn_id": run, "event": event }));
        assert_eq!(reply, json!({ "refused": "a needle is not in the question the person answered" }));
        assert_eq!(reply_text(&store, run), "Your sister is Priya.");
        assert!(asked.lock().unwrap().is_empty());
    }

    #[test]
    fn the_third_too_much_uses_the_question_up() {
        let (host, store, _) = host_with_shell_costing(redact::MAX_WORK);
        let (session, _, run, _reader) = answered(&host, "Erase");
        for _ in 0..2 {
            assert_eq!(redact(&host, &session, run, "forget")["refused"], redact::TOO_MUCH);
        }
        assert_eq!(redact(&host, &session, run, "forget")["refused"], crate::run_store::TOO_MUCH_USED_UP);
        assert!(redact(&host, &session, run, "forget")["refused"].as_str().unwrap().contains("one redaction per question"));
        assert_eq!(reply_text(&store, run), "Your sister is Priya.");
    }

    #[test]
    fn a_needle_shorter_than_four_is_refused_as_too_short() {
        let (host, store, asked) = host_with_shell();
        let (session, _, run, _reader) = answered(&host, "Erase");
        for short in ["e", "not"] {
            let event = json!({ "kind": "redact", "request_id": "forget", "needles": [Needle::of(short)] });
            let reply = call(&host, protocol::EVENT, json!({ "session": session, "turn_id": run, "event": event }));
            assert_eq!(reply, json!({ "refused": "a needle is too short to erase safely" }), "{short:?}");
        }
        assert_eq!(reply_text(&store, run), "Your sister is Priya.");
        assert!(asked.lock().unwrap().is_empty());
    }
}
