//! Integration tests for LLMEngine.
//!
//! Downloads Qwen2.5-0.5B-Instruct GGUF from HuggingFace Hub on first run
//! (cached in ~/.cache/huggingface/hub/).
//!
//! Uses Q4_K_M quantization (~491MB) — same as production deployment.

use yantrikdb_ml::{
    ChatMessage, GGUFFiles, GenerationConfig, LLMEngine,
};

const GGUF_REPO: &str = "Qwen/Qwen2.5-0.5B-Instruct-GGUF";
const GGUF_FILE: &str = "qwen2.5-0.5b-instruct-q4_k_m.gguf";
const TOKENIZER_REPO: &str = "Qwen/Qwen2.5-0.5B-Instruct";

fn load_engine() -> LLMEngine {
    let files = GGUFFiles::from_hub(GGUF_REPO, GGUF_FILE, TOKENIZER_REPO)
        .expect("failed to download model files");
    LLMEngine::from_gguf(&files.gguf, &files.tokenizer)
        .expect("failed to load LLM engine")
}

#[test]
fn test_llm_load_and_count_tokens() {
    let engine = load_engine();

    // Basic token counting
    let count = engine.count_tokens("Hello, world!").expect("count_tokens failed");
    assert!(count > 0 && count < 20, "expected reasonable token count, got {count}");

    // Longer text should have more tokens
    let count2 = engine
        .count_tokens("The quick brown fox jumps over the lazy dog. This is a longer sentence for testing.")
        .expect("count_tokens failed");
    assert!(count2 > count, "longer text should have more tokens");
}

#[test]
fn test_llm_generate_raw() {
    let engine = load_engine();

    let config = GenerationConfig {
        max_tokens: 32,
        temperature: 0.0, // greedy for determinism
        top_p: None,
        ..Default::default()
    };

    let response = engine.generate("Once upon a time", &config)
        .expect("generate failed");

    assert!(!response.text.is_empty(), "generated text should not be empty");
    assert!(response.prompt_tokens > 0, "should have prompt tokens");
    assert!(response.completion_tokens > 0, "should have generated tokens");
    assert!(response.completion_tokens <= 32, "should respect max_tokens");

    println!("Raw generation: {}", response.text);
    println!("Prompt tokens: {}, Completion tokens: {}", response.prompt_tokens, response.completion_tokens);
}

#[test]
fn test_llm_chat() {
    let engine = load_engine();

    let messages = vec![
        ChatMessage::system("You are a helpful assistant. Answer briefly."),
        ChatMessage::user("What is 2 + 2?"),
    ];

    let config = GenerationConfig {
        max_tokens: 64,
        temperature: 0.0,
        top_p: None,
        ..Default::default()
    };

    let response = engine.chat(&messages, &config)
        .expect("chat failed");

    assert!(!response.text.is_empty(), "chat response should not be empty");
    // The model should mention "4" somewhere in its response
    assert!(
        response.text.contains('4'),
        "response should contain '4', got: {}",
        response.text
    );

    println!("Chat response: {}", response.text);
    println!("Stop reason: {}", response.stop_reason);
}

#[test]
fn test_llm_stop_reason() {
    let engine = load_engine();

    // Very short max_tokens should hit length limit
    let config = GenerationConfig {
        max_tokens: 5,
        temperature: 0.0,
        top_p: None,
        ..Default::default()
    };

    let response = engine.generate("Tell me a long story about", &config)
        .expect("generate failed");

    assert!(
        response.completion_tokens <= 5,
        "should generate at most 5 tokens, got {}",
        response.completion_tokens
    );
    // Should be "length" or "eos" (if model happens to emit EOS in 5 tokens)
    assert!(
        response.stop_reason == "length" || response.stop_reason == "eos",
        "stop_reason should be length or eos, got: {}",
        response.stop_reason
    );
}

#[test]
fn test_llm_tool_calling() {
    let engine = load_engine();

    let tool_defs = vec![serde_json::json!({
        "type": "function",
        "function": {
            "name": "get_weather",
            "description": "Get the current weather for a location",
            "parameters": {
                "type": "object",
                "properties": {
                    "location": {
                        "type": "string",
                        "description": "City name"
                    }
                },
                "required": ["location"]
            }
        }
    })];

    let tool_text = yantrikdb_ml::format_tools(&tool_defs);

    let messages = vec![
        ChatMessage::system(format!("You are a helpful assistant.{tool_text}")),
        ChatMessage::user("What's the weather in Tokyo?"),
    ];

    let config = GenerationConfig {
        max_tokens: 128,
        temperature: 0.0,
        top_p: None,
        ..Default::default()
    };

    let response = engine.chat(&messages, &config)
        .expect("chat failed");

    println!("Tool calling response: {}", response.text);
    println!("Parsed tool calls: {:?}", response.tool_calls);

    // The model may or may not emit a tool call — 0.5B is small.
    // Just verify the response is non-empty and parsing doesn't crash.
    assert!(!response.text.is_empty());
}
