//! What the learning pass keeps and what it leaves to the person (#1184).

use super::*;
use yantrik_ml::LLMResponse;

/// A model that answers every extraction with the same JSON.
struct Extracts(&'static str);

impl LLMBackend for Extracts {
    fn chat(&self, _: &[ChatMessage], _: &GenerationConfig, _: Option<&[serde_json::Value]>) -> anyhow::Result<LLMResponse> {
        Ok(LLMResponse { text: self.0.into(), prompt_tokens: 0, completion_tokens: 1, tool_calls: vec![], api_tool_calls: vec![], stop_reason: "stop".into() })
    }
    fn chat_streaming(&self, m: &[ChatMessage], c: &GenerationConfig, t: Option<&[serde_json::Value]>, _: &mut dyn FnMut(&str)) -> anyhow::Result<LLMResponse> {
        self.chat(m, c, t)
    }
    fn count_tokens(&self, text: &str) -> anyhow::Result<usize> {
        Ok(text.len())
    }
    fn backend_name(&self) -> &str {
        "extracts"
    }
}

fn kept(db: &YantrikDB) -> Vec<String> {
    let conn = db.conn();
    let Ok(mut stmt) = conn.prepare("SELECT text FROM memories ORDER BY rowid") else { return vec![] };
    stmt.query_map([], |r| r.get::<_, String>(0)).map(|rows| rows.flatten().collect()).unwrap_or_default()
}

fn learn(says: &'static str) -> Vec<String> {
    let db = YantrikDB::new(":memory:", 64).expect("in-memory database");
    extract_and_learn(&db, &Extracts(says), "Turn off getting things ready, and my sister lives in Austin.", "Noted.", &MemoryEvolutionConfig::default());
    kept(&db)
}

#[test]
fn a_memory_about_the_setting_is_not_learned_but_another_is() {
    let rows = learn(
        r#"{"should_remember":true,"memories":[
            {"text":"User prefers not to have things prepared for promises","domain":"preference","importance":0.8},
            {"text":"User does not want drafts made for commitments","domain":"preference","importance":0.8},
            {"text":"User wants the companion to stop drafting emails for their promises","domain":"preference","importance":0.8},
            {"text":"User's sister Anita lives in Austin","domain":"family","importance":0.8}]}"#,
    );
    assert_eq!(rows.len(), 1, "only the sister was kept: {rows:?}");
    assert!(rows[0].contains("Anita"), "{rows:?}");
}

#[test]
fn ordinary_life_that_shares_words_with_the_setting_is_learned() {
    let rows = learn(
        r#"{"should_remember":true,"memories":[
            {"text":"User is getting things ready for the party","domain":"general","importance":0.8},
            {"text":"User promised to prepare the Q4 deck for Maya","domain":"work","importance":0.8},
            {"text":"User is preparing things for the move to Austin","domain":"general","importance":0.8},
            {"text":"User prefers prepared meals and keeps promises","domain":"preference","importance":0.8}]}"#,
    );
    assert_eq!(rows.len(), 4, "{rows:?}");
}
