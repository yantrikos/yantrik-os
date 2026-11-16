//! The table `commitment_prep` keeps its items in is made where the companion starts (#1184), not
//! only by a test's own set-up: on 520 nothing created it, so every proposal was lost unseen.

mod common;

use common::*;
use yantrik_companion::anticipation::{AnticipationEngine, PreparationTrigger, PreparedItem, PreparedKind};
use yantrik_companion::commitment_prep::Outcome;
use yantrik_companion::config::CompanionConfig;
use yantrik_companion::CompanionService;
use yantrik_ml::{ChatMessage, GenerationConfig, LLMBackend, LLMResponse};

struct Silent;

impl LLMBackend for Silent {
    fn chat(&self, _: &[ChatMessage], _: &GenerationConfig, _: Option<&[serde_json::Value]>) -> anyhow::Result<LLMResponse> {
        Ok(LLMResponse { text: "Noted.".into(), prompt_tokens: 0, completion_tokens: 1, tool_calls: vec![], api_tool_calls: vec![], stop_reason: "stop".into() })
    }
    fn chat_streaming(&self, m: &[ChatMessage], c: &GenerationConfig, t: Option<&[serde_json::Value]>, _: &mut dyn FnMut(&str)) -> anyhow::Result<LLMResponse> {
        self.chat(m, c, t)
    }
    fn count_tokens(&self, text: &str) -> anyhow::Result<usize> {
        Ok(text.len())
    }
    fn backend_name(&self) -> &str {
        "silent"
    }
}

fn item(id: &str) -> PreparedItem {
    PreparedItem {
        id: id.into(),
        trigger: PreparationTrigger::CommitmentDeadline { commitment_id: "1".into() },
        kind: PreparedKind::CommitmentPrep,
        description: "x".into(),
        data: serde_json::json!({}),
        relevant_at: 0.0,
        expires_at: 1e12,
        surfaced: false,
        outcome: None,
        created_at: 0.0,
    }
}

#[test]
fn a_database_opened_by_the_companions_start_can_store_an_item() {
    let path = std::env::temp_dir().join(format!("yantrik-prep-start-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let db = yantrikdb_core::YantrikDB::new(path.to_str().unwrap(), 384).expect("a fresh database");
    let mut config = CompanionConfig::default();
    config.tools.enabled = false;
    let companion = CompanionService::new(db, std::sync::Arc::new(Silent), config);

    assert!(AnticipationEngine::store(&companion.db.conn(), &item("commitment-prep-1")), "start left no table to store in");
    assert!(AnticipationEngine::for_commitment(&companion.db.conn(), "1").is_some());
    drop(companion);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_proposal_that_could_not_be_stored_is_not_reported() {
    let rig = Rig::asked(&[DECK, SHEET]);
    rig.db.conn().execute_batch("DROP TABLE prepared_items").unwrap();
    rig.say(SAID, MONDAY);
    let outcomes = rig.cycle(window());
    assert!(matches!(outcomes.as_slice(), [Outcome::Nothing(_)]), "{outcomes:?}");
    assert!(!AnticipationEngine::store(&rig.db.conn(), &item("commitment-prep-1")));
}
