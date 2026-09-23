//! End-to-end companion test.
//!
//! Uses real YantrikDB (in-memory) + real LLM (Qwen2.5-0.5B Q4_K_M).
//! Downloads model from HuggingFace Hub on first run (~491MB).

use yantrik_companion::{CompanionConfig, CompanionService};
use yantrik_ml::{CandleEmbedder, GGUFFiles, LLMEngine};

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
    // Through the bridge, as the companion itself does. The engine's Embedder trait and
    // yantrik-ml's are two traits with one name; handing the ML embedder straight to the
    // engine stopped compiling when the bridge was introduced, and nobody saw, because CI had
    // never got far enough to compile a test.
    db.set_embedder(Box::new(yantrik_companion::embedder_bridge::EmbedderBridge::new(embedder)));

    // Config
    let config = CompanionConfig {
        user_name: "Pranab".to_string(),
        ..Default::default()
    };

    CompanionService::new(db, std::sync::Arc::new(llm), config)
}

#[test]
#[ignore = "downloads a model from the Hugging Face hub and runs it; `cargo test -- --ignored` on a machine that may"]
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
#[ignore = "downloads a model from the Hugging Face hub and runs it; `cargo test -- --ignored` on a machine that may"]
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
#[ignore = "downloads a model from the Hugging Face hub and runs it; `cargo test -- --ignored` on a machine that may"]
fn test_conversation_history() {
    let mut companion = build_companion();

    companion.handle_message("My name is Pranab.");
    companion.handle_message("I work at a tech company.");

    // History should have 4 entries (2 user + 2 assistant)
    assert_eq!(companion.history().len(), 4);
}

#[test]
#[ignore = "downloads a model from the Hugging Face hub and runs it; `cargo test -- --ignored` on a machine that may"]
fn test_urge_queue() {
    let mut companion = build_companion();

    // Push a test urge
    let spec = yantrik_companion::UrgeSpec::new("test", "Test urge reason", 0.5)
        .with_cooldown("test:1");
    companion.urge_queue.push(&companion.db.conn(), &spec);

    // Verify pending count
    let count = companion.urge_queue.count_pending(&companion.db.conn());
    assert_eq!(count, 1);

    // Pop and verify
    let urges = companion.urge_queue.pop_for_interaction(&companion.db.conn(), 5);
    assert_eq!(urges.len(), 1);
    assert_eq!(urges[0].reason, "Test urge reason");

    // Should be empty now (delivered)
    let count = companion.urge_queue.count_pending(&companion.db.conn());
    assert_eq!(count, 0);
}

#[test]
#[ignore = "downloads a model from the Hugging Face hub and runs it; `cargo test -- --ignored` on a machine that may"]
fn test_instinct_evaluation() {
    let companion = build_companion();
    let state = companion.build_state();

    // A fresh store holds no interaction events, so the companion has never heard the
    // person and the absence clock stays unset (#156 — it used to be stamped with the
    // boot time). No interaction yet means no absence to measure: you cannot be away
    // from someone the companion never met. The absence-driven instincts (check-in,
    // the weaver and curiosity idle gates, idle maintenance) therefore stay silent,
    // and the rest have no triggers, patterns or memories to work on.
    let urges = companion.evaluate_instincts(&state);
    assert!(
        urges.is_empty(),
        "no instincts should fire on fresh state, got {:?}",
        urges.iter().map(|u| &u.instinct_name).collect::<Vec<_>>()
    );
}
