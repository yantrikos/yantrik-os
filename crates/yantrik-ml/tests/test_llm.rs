//! Integration tests for LLMEngine.
//!
//! Downloads Qwen2.5-0.5B-Instruct GGUF from HuggingFace Hub on first run
//! (cached in ~/.cache/huggingface/hub/).
//!
//! Uses Q4_K_M quantization (~491MB) — same as production deployment.

use yantrik_ml::{
    ChatMessage, GGUFFiles, GenerationConfig, LLMEngine, ModelFamily, ModelTier,
    template_for_family,
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

fn sample_tool_def() -> serde_json::Value {
    let mut desc = String::new();
    for _ in 0..20 {
        desc.push_str("This is a long tool description sentence. ");
    }
    let mut props = serde_json::Map::new();
    for i in 0..8 {
        let mut pdesc = String::new();
        for _ in 0..10 {
            pdesc.push_str("A long parameter description sentence. ");
        }
        props.insert(
            format!("param_{i}"),
            serde_json::json!({
                "type": "string",
                "description": pdesc,
                "enum": ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"],
            }),
        );
    }
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "sample_tool",
            "description": desc,
            "parameters": {
                "type": "object",
                "properties": props,
                "required": ["param_0", "param_1"],
                "additionalProperties": false,
            }
        }
    })
}

#[test]
fn small_tier_gets_compact_text() {
    let def = sample_tool_def();
    let tools = vec![def];
    let template = template_for_family(ModelFamily::Qwen);
    let text = template.format_tools_compact(&tools, &ModelTier::Small);

    assert!(!text.contains("additionalProperties"));
    assert!(!text.contains("$schema"));
    // The compact per-tool body must be materially shorter than the JSON form.
    let json_form = template.format_tools(&tools);
    assert!(text.len() < json_form.len(), "compact form should be shorter");
    // Per-tool body (between the <tools> wrapper) must stay under 600 chars.
    let body = text
        .split("<tools>\n")
        .nth(1)
        .and_then(|s| s.split("\n</tools>").next())
        .unwrap_or("");
    assert!(body.len() < 600, "compact body too long: {} chars", body.len());
}

#[test]
fn large_tier_keeps_the_full_schema() {
    let def = sample_tool_def();
    let tools = vec![def];
    let template = template_for_family(ModelFamily::Qwen);
    let text = template.format_tools_compact(&tools, &ModelTier::Large);

    assert!(text.contains("additionalProperties"));
    assert!(text.contains("This is a long tool description sentence"));
}

#[test]
fn name_and_required_params_survive() {
    let def = sample_tool_def();
    let tools = vec![def];
    let template = template_for_family(ModelFamily::Qwen);
    let text = template.format_tools_compact(&tools, &ModelTier::Tiny);

    assert!(text.contains("sample_tool"));
    assert!(text.contains("*param_0"));
    assert!(text.contains("*param_1"));
    for i in 0..8 {
        assert!(text.contains(&format!("param_{i}")), "missing param_{i}");
    }
}
