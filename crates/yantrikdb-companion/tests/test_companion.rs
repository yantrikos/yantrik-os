//! End-to-end companion test.
//!
//! Uses real YantrikDB (in-memory) + real LLM (Qwen2.5-0.5B Q4_K_M).
//! Downloads model from HuggingFace Hub on first run (~491MB).

use yantrikdb_companion::{CompanionConfig, CompanionService};
use yantrikdb_ml::{CandleEmbedder, GGUFFiles, LLMEngine};

const GGUF_REPO: &str = "Qwen/Qwen2.5-0.5B-Instruct-GGUF";
const GGUF_FILE: &str = "qwen2.5-0.5b-instruct-q4_k_m.gguf";
const TOKENIZER_REPO: &str = "Qwen/Qwen2.5-0.5B-Instruct";
const EMBEDDER_REPO: &str = "sentence-transformers/all-MiniLM-L6-v2";

fn build_companion() -> CompanionService {
    // Load embedder
    let embedder = CandleEmbedder::from_hub(EMBEDDER_REPO, None)
        .expect("failed to load embedder");

    // Load LLM
    let files = GGUFFiles::from_hub(GGUF_REPO, GGUF_FILE, TOKENIZER_REPO)
        .expect("failed to download LLM");
    let llm = LLMEngine::from_gguf(&files.gguf, &files.tokenizer)
        .expect("failed to load LLM");

    // Create YantrikDB with embedder
    let mut db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("failed to create YantrikDB");
    db.set_embedder(Box::new(embedder));

    // Config
    let config = CompanionConfig {
        user_name: "Pranab".to_string(),
        ..Default::default()
    };

    CompanionService::new(db, llm, config)
}

#[test]
fn test_handle_message_basic() {
    let mut companion = build_companion();

    let response = companion.handle_message("Hello, how are you?");

    assert!(!response.message.is_empty(), "response should not be empty");
    println!("Response: {}", response.message);
    println!(
        "Memories recalled: {}, Tool calls: {:?}",
        response.memories_recalled, response.tool_calls_made
    );
}

#[test]
fn test_memory_round_trip() {
    let mut companion = build_companion();

    // First message: tell the companion something
    let r1 = companion.handle_message("I love playing chess on rainy days.");
    println!("R1: {}", r1.message);

    // Second message: ask about it
    let r2 = companion.handle_message("What are my hobbies?");
    println!("R2: {}", r2.message);

    // The companion should recall the chess memory
    assert!(
        r2.memories_recalled > 0,
        "should recall memories about chess"
    );
}

#[test]
fn test_conversation_history() {
    let mut companion = build_companion();

    companion.handle_message("My name is Pranab.");
    companion.handle_message("I work at a tech company.");

    // History should have 4 entries (2 user + 2 assistant)
    assert_eq!(companion.history().len(), 4);
}

#[test]
fn test_urge_queue() {
    let mut companion = build_companion();

    // Push a test urge
    let spec = yantrikdb_companion::UrgeSpec::new("test", "Test urge reason", 0.5)
        .with_cooldown("test:1");
    companion.urge_queue.push(companion.db.conn(), &spec);

    // Verify pending count
    let count = companion.urge_queue.count_pending(companion.db.conn());
    assert_eq!(count, 1);

    // Pop and verify
    let urges = companion.urge_queue.pop_for_interaction(companion.db.conn(), 5);
    assert_eq!(urges.len(), 1);
    assert_eq!(urges[0].reason, "Test urge reason");

    // Should be empty now (delivered)
    let count = companion.urge_queue.count_pending(companion.db.conn());
    assert_eq!(count, 0);
}

#[test]
fn test_instinct_evaluation() {
    let companion = build_companion();
    let state = companion.build_state();

    // With fresh state, no instincts should fire (recently interacted)
    let urges = companion.evaluate_instincts(&state);
    // Check-in won't fire because we just created (last_interaction_ts is now)
    // Others won't fire because no triggers/patterns/conflicts
    assert!(
        urges.is_empty(),
        "no instincts should fire on fresh state, got {:?}",
        urges.iter().map(|u| &u.instinct_name).collect::<Vec<_>>()
    );
}
