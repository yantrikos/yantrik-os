//! Quick test for the Cognitive Kernel ONNX integration.
//!
//! Run: cargo run --example test_kernel --features cognitive-kernel -- Z:\yantrik-models\cognitive-kernel\onnx

use std::time::Instant;
use yantrik_ml::CognitiveKernelLLM;
use yantrik_ml::LLMBackend;
use yantrik_ml::types::{ChatMessage, GenerationConfig};

fn main() {

    let model_dir = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: test_kernel <path-to-onnx-dir>");
        std::process::exit(1);
    });

    println!("Loading Cognitive Kernel from {}...", model_dir);
    let t0 = Instant::now();
    let kernel = CognitiveKernelLLM::load(&model_dir).expect("Failed to load kernel");
    println!("Loaded in {:.1}s\n", t0.elapsed().as_secs_f64());

    let config = GenerationConfig::default();

    let tests = vec![
        "Hello!",
        "What is 42 times 58?",
        "Convert 100 miles to kilometers",
        "What time is it?",
        "25% of 800?",
        "Thanks!",
        "Goodbye",
        "What is the capital of France?",
        "Tell me about Mars",
        "Show me the git status",
        "Write a fibonacci function",
        "What's the weather like?",
    ];

    println!("{:-<70}", "");
    println!("  COGNITIVE KERNEL TEST — {} queries", tests.len());
    println!("{:-<70}\n", "");

    for query in &tests {
        let messages = vec![ChatMessage {
            role: "user".into(),
            content: query.to_string(),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }];

        let t0 = Instant::now();
        match kernel.chat(&messages, &config, None) {
            Ok(resp) => {
                let ms = t0.elapsed().as_millis();
                println!("  You: {query}");
                println!("  Bot: {}", resp.text.chars().take(200).collect::<String>());
                println!("  [{ms}ms]\n");
            }
            Err(e) => {
                println!("  You: {query}");
                println!("  ERROR: {e}\n");
            }
        }
    }
}
