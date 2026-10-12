//! Integration tests for LLMEngine.
//!
//! Downloads Qwen2.5-0.5B-Instruct GGUF from HuggingFace Hub on first run
//! (cached in ~/.cache/huggingface/hub/).
//!
//! Uses Q4_K_M quantization (~491MB) — same as production deployment.

use yantrik_ml::{
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
#[ignore = "downloads a model from the Hugging Face hub and runs it; `cargo test -- --ignored` on a machine that may"]
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
#[ignore = "downloads a model from the Hugging Face hub and runs it; `cargo test -- --ignored` on a machine that may"]
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
#[ignore = "downloads a model from the Hugging Face hub and runs it; `cargo test -- --ignored` on a machine that may"]
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
#[ignore = "downloads a model from the Hugging Face hub and runs it; `cargo test -- --ignored` on a machine that may"]
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
#[ignore = "downloads a model from the Hugging Face hub and runs it; `cargo test -- --ignored` on a machine that may"]
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

    let tool_text = yantrik_ml::format_tools(&tool_defs);

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

/// A tool definition the size of a real one: long description, JSON Schema
/// noise, and a required parameter the model cannot do without.
fn weather_tool() -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "get_weather",
            "description": "Get the current weather for a location. Results are in the user's units and reflect the live conditions at the nearest station.",
            "parameters": {
                "type": "object",
                "$schema": "http://json-schema.org/draft-07/schema#",
                "additionalProperties": false,
                "properties": {
                    "location": {
                        "type": "string",
                        "description": "City name, e.g. Tokyo"
                    },
                    "units": {
                        "type": "string",
                        "enum": ["metric", "imperial"],
                        "description": "Temperature units"
                    }
                },
                "required": ["location"]
            }
        }
    })
}

#[test]
fn test_small_tier_compacts_tools_and_large_keeps_json() {
    use yantrik_ml::capability::ModelTier;

    let defs = vec![weather_tool()];
    let full = yantrik_ml::format_tools(&defs);

    for tier in [ModelTier::Tiny, ModelTier::Small] {
        let compact = yantrik_ml::format_tools_compact(&defs, tier);
        assert!(compact.contains("get_weather"), "{tier:?}: tool name must survive");
        assert!(compact.contains("location*("), "{tier:?}: required param must be named and marked");
        assert!(!compact.contains("additionalProperties"), "{tier:?}: schema noise leaked");
        assert!(!compact.contains("$schema"), "{tier:?}: schema noise leaked");
        assert!(!compact.contains("live conditions"), "{tier:?}: description should be cut to one sentence");
        assert!(compact.len() < full.len(), "{tier:?}: {} chars vs full {}", compact.len(), full.len());
    }

    // Medium/Large keep the full schema, byte-identical to `format_tools`.
    for tier in [ModelTier::Medium, ModelTier::Large] {
        assert_eq!(
            yantrik_ml::format_tools_compact(&defs, tier),
            full,
            "{tier:?} must keep the full JSON schema"
        );
    }
}
