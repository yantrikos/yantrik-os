//! On a tight free account the companion calls only when asked: what it starts by itself makes no
//! call there, and a call the gateway kept back anyway is not an outage.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use yantrik_ml::asked::{started_by_itself, tight_for_this_thread};
use yantrik_ml::{ChatMessage, GenerationConfig, LLMBackend, LLMResponse};
use yantrikdb_core::YantrikDB;

use crate::bond::BondLevel;
use crate::companion::CompanionService;
use crate::config::{CompanionConfig, MemoryEvolutionConfig};
use crate::narrative::Narrative;

/// Counts its calls, notes whether each wore the mark, and answers with a JSON that asks for
/// nothing to be stored (so a learning pass goes on to its reflection).
#[derive(Default)]
struct Counting {
    calls: AtomicUsize,
    marks: Mutex<Vec<bool>>,
}

impl LLMBackend for Counting {
    fn chat(&self, _: &[ChatMessage], _: &GenerationConfig, _: Option<&[serde_json::Value]>) -> anyhow::Result<LLMResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.marks.lock().unwrap().push(started_by_itself());
        Ok(LLMResponse {
            text: r#"{"should_remember": true, "memories": [], "has_reflection": false}"#.into(),
            prompt_tokens: 0,
            completion_tokens: 1,
            tool_calls: vec![],
            api_tool_calls: vec![],
            stop_reason: "stop".into(),
        })
    }
    fn chat_streaming(&self, m: &[ChatMessage], c: &GenerationConfig, t: Option<&[serde_json::Value]>, _: &mut dyn FnMut(&str)) -> anyhow::Result<LLMResponse> {
        self.chat(m, c, t)
    }
    fn count_tokens(&self, text: &str) -> anyhow::Result<usize> {
        Ok(text.len())
    }
    fn backend_name(&self) -> &str {
        "counting"
    }
}

const SAID: &str = "the person said something worth a long enough exchange to be reflected on, at some length";

fn learn(llm: &Counting) {
    let db = YantrikDB::new(":memory:", 64).expect("in-memory database");
    crate::learning::extract_and_learn(&db, llm, SAID, "and the assistant answered it", &MemoryEvolutionConfig::default());
}

#[test]
fn on_a_tight_account_a_finished_turn_starts_no_learning_call_and_no_reflection() {
    let _tight = tight_for_this_thread(true);
    let llm = Counting::default();
    learn(&llm);
    assert_eq!(llm.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn on_another_account_it_learns_and_reflects_as_before_and_each_call_wears_the_mark() {
    let _roomy = tight_for_this_thread(false);
    let llm = Counting::default();
    learn(&llm);
    assert_eq!(llm.calls.load(Ordering::SeqCst), 2, "the extraction and the reflection");
    assert_eq!(*llm.marks.lock().unwrap(), vec![true, true], "both are the companion's own, and say so to the gateway");
    assert!(!started_by_itself(), "the mark ended with the pass");
}

fn refresh_narrative(llm: &Counting) {
    let conn = Connection::open_in_memory().unwrap();
    Narrative::ensure_table(&conn);
    Narrative::update(&conn, llm, "Ann", BondLevel::Stranger, 0.1, &[], 64);
}

#[test]
fn the_narrative_refresh_is_kept_from_a_tight_account_and_marked_elsewhere() {
    let llm = Counting::default();
    {
        let _tight = tight_for_this_thread(true);
        refresh_narrative(&llm);
    }
    assert_eq!(llm.calls.load(Ordering::SeqCst), 0);
    let _roomy = tight_for_this_thread(false);
    refresh_narrative(&llm);
    assert_eq!(*llm.marks.lock().unwrap(), vec![true]);
}

#[test]
fn what_a_person_asked_is_never_skipped_whatever_the_account() {
    let _tight = tight_for_this_thread(true);
    let llm = Counting::default();
    // The person's own turn does not ask `skip_self_started` and wears no mark.
    let _ = llm.chat(&[ChatMessage::user("hi")], &GenerationConfig::default(), None);
    assert_eq!(*llm.marks.lock().unwrap(), vec![false]);
}

/// What the gateway answers a call it kept back, as the backend turns it into an error.
fn kept() -> anyhow::Error {
    let body = serde_json::json!({"error": {"message": "OVHcloud allows few requests, so this account is kept for what you ask.", "code": "kept_for_asked"}}).to_string();
    yantrik_ml::asked::kept(429, &body).expect("the gateway's refusal").into()
}

struct Kept;

impl LLMBackend for Kept {
    fn chat(&self, _: &[ChatMessage], _: &GenerationConfig, _: Option<&[serde_json::Value]>) -> anyhow::Result<LLMResponse> {
        Err(kept())
    }
    fn chat_streaming(&self, m: &[ChatMessage], c: &GenerationConfig, t: Option<&[serde_json::Value]>, _: &mut dyn FnMut(&str)) -> anyhow::Result<LLMResponse> {
        self.chat(m, c, t)
    }
    fn count_tokens(&self, text: &str) -> anyhow::Result<usize> {
        Ok(text.len())
    }
    fn backend_name(&self) -> &str {
        "kept"
    }
}

/// A refused marked call is a failed turn that reached the model: the shell sets `companion_online`
/// from `offline_mode` and raises the banner from that, so neither may be set. It names the
/// failure so the shell does not deliver the sentence as the companion's own message.
#[test]
fn a_call_the_gateway_kept_back_leaves_the_companion_online_and_raises_no_banner() {
    for streaming in [false, true] {
        let db = YantrikDB::new(":memory:", 384).expect("in-memory database");
        let mut config = CompanionConfig::default();
        config.tools.enabled = false;
        let mut companion = CompanionService::new(db, Arc::new(Kept), config);
        let _own = yantrik_ml::asked::self_started();
        let ask = "What's on my calendar?";
        let reply = if streaming { companion.handle_message_streaming(ask, |_| {}) } else { companion.handle_message(ask) };
        assert!(!reply.offline_mode, "streaming {streaming}: online stays as it was; the banner reads this");
        assert!(reply.message.contains("kept for what you ask"), "{}", reply.message);
        assert!(reply.model_failure.is_some(), "so the shell does not deliver it as the companion's own message");
        assert!(!reply.message.contains("I'm listening"), "not the offline responder: {}", reply.message);
    }
}
