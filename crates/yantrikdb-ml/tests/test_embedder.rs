//! Integration test for CandleEmbedder.
//!
//! Downloads the all-MiniLM-L6-v2 model from HuggingFace Hub on first run
//! (cached in ~/.cache/huggingface/hub/).

use yantrikdb_core::Embedder;
use yantrikdb_ml::CandleEmbedder;

#[test]
fn test_embedder_from_hub() {
    // Download and load model from HuggingFace Hub
    let embedder = CandleEmbedder::from_hub(
        "sentence-transformers/all-MiniLM-L6-v2",
        None,
    )
    .expect("failed to load model from hub");

    // Check dimension
    assert_eq!(embedder.dim(), 384);

    // Embed a simple text
    let emb = embedder.embed( "Hello world")
        .expect("embed failed");

    assert_eq!(emb.len(), 384);

    // Check L2 normalization: norm should be ~1.0
    let norm: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!(
        (norm - 1.0).abs() < 0.01,
        "embedding should be L2 normalized, got norm={norm}"
    );
}

#[test]
fn test_semantic_similarity() {
    let embedder = CandleEmbedder::from_hub(
        "sentence-transformers/all-MiniLM-L6-v2",
        None,
    )
    .expect("failed to load model");

    let embed = |text: &str| -> Vec<f32> {
        embedder.embed( text).expect("embed failed")
    };

    let a = embed("The cat sat on the mat");
    let b = embed("A feline rested on a rug");
    let c = embed("Quantum computing uses qubits");

    // Cosine similarity (vectors are already normalized)
    let sim_ab: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let sim_ac: f32 = a.iter().zip(c.iter()).map(|(x, y)| x * y).sum();

    // Similar sentences should have higher similarity than unrelated ones
    assert!(
        sim_ab > sim_ac,
        "cat/feline similarity ({sim_ab:.3}) should be > cat/quantum ({sim_ac:.3})"
    );

    // Sanity bounds
    assert!(sim_ab > 0.4, "similar sentences should have sim > 0.4, got {sim_ab:.3}");
    assert!(sim_ac < 0.4, "unrelated sentences should have sim < 0.4, got {sim_ac:.3}");
}

#[test]
fn test_embed_batch() {
    let embedder = CandleEmbedder::from_hub(
        "sentence-transformers/all-MiniLM-L6-v2",
        None,
    )
    .expect("failed to load model");

    let texts = &["Hello", "World", "Test"];
    let results = embedder.embed_batch(texts)
        .expect("batch embed failed");

    assert_eq!(results.len(), 3);
    for emb in &results {
        assert_eq!(emb.len(), 384);
    }
}

#[test]
fn test_embedder_with_aidb() {
    // Test the full integration: CandleEmbedder plugged into YantrikDB
    let embedder = CandleEmbedder::from_hub(
        "sentence-transformers/all-MiniLM-L6-v2",
        None,
    )
    .expect("failed to load model");

    let mut db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("failed to create YantrikDB");
    db.set_embedder(Box::new(embedder));

    // Record with auto-embedding
    let rid = db
        .record_text(
            "I love playing chess on rainy days",
            "episodic",
            0.7,
            0.5,
            604800.0,
            &serde_json::json!({}),
            "default",
            0.9,
            "hobby",
            "user",
            Some("happy"),
        )
        .expect("record_text failed");

    assert!(!rid.is_empty());

    // Recall with auto-embedding
    let results = db.recall_text("What are my hobbies?", 5).expect("recall_text failed");

    assert!(!results.is_empty(), "should recall at least one memory");
    assert_eq!(results[0].rid, rid);
    assert!(
        results[0].text.contains("chess"),
        "recalled text should contain 'chess'"
    );
}
