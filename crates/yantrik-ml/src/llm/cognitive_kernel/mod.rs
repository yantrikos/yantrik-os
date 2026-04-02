//! Cognitive Kernel — brain-inspired modular AI backend.
//!
//! 4 cortical columns (Perceiver, Comprehender, Reasoner, Speaker) + Router,
//! all running as ONNX models via `ort`. No external LLM needed.
//!
//! Architecture: SSA (Selective State Attention) with parallel scan recurrence.
//! Total ~268MB, runs on CPU in 50-500ms per query.

use anyhow::{Context, Result};
use ndarray::ArrayViewD;
use ort::session::Session;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;
use tokenizers::Tokenizer;

use crate::traits::LLMBackend;
use crate::types::{ChatMessage, GenerationConfig, LLMResponse};

// ── Column names ──────────────────────────────────────────────────────

const COLUMNS: &[&str] = &["perceiver", "comprehender", "reasoner", "speaker"];

// ── Special token IDs (populated at load time) ────────────────────────

struct SpecialTokens {
    input: u32,
    intent: u32,
    entities: u32,
    context: u32,
    query: u32,
    facts: u32,
    inferences: u32,
    tone: u32,
    response: u32,
    eos: u32,
}

// ── Tool registry for fast-path matching ──────────────────────────────

mod tools;
pub use tools::ToolRegistry;

// ── Cognitive Bus (shared state between columns) ──────────────────────

#[derive(Debug, Default)]
struct CognitiveBus {
    raw_input: String,
    intent: String,
    entities: Vec<String>,
    tool_match: String,
    tool_score: f32,
    tool_result: String,
    facts: Vec<String>,
    inferences: Vec<String>,
    response: String,
    path: &'static str, // "fast", "tool-bypass", "slow"
}

// ── Main backend struct ───────────────────────────────────────────────

pub struct CognitiveKernelLLM {
    perceiver: Mutex<Session>,
    comprehender: Mutex<Session>,
    reasoner: Mutex<Session>,
    speaker: Mutex<Session>,
    router: Mutex<Session>,
    tokenizer: Tokenizer,
    tokens: SpecialTokens,
    registry: ToolRegistry,
    model_dir: PathBuf,
}

impl CognitiveKernelLLM {
    /// Load all ONNX models + tokenizer from a directory.
    ///
    /// Expected layout:
    /// ```text
    /// dir/
    ///   perceiver.onnx + perceiver.onnx.data
    ///   comprehender.onnx + comprehender.onnx.data
    ///   reasoner.onnx + reasoner.onnx.data
    ///   speaker.onnx + speaker.onnx.data
    ///   router.onnx + router.onnx.data
    ///   tokenizer/tokenizer.json
    ///   kernel_config.json
    /// ```
    pub fn load(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        tracing::info!(path = %dir.display(), "Loading Cognitive Kernel");

        let load_session = |name: &str| -> Result<Session> {
            let path = dir.join(format!("{name}.onnx"));
            let session = Session::builder()
                .map_err(|e| anyhow::anyhow!("ORT session builder error: {e}"))?
                .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
                .map_err(|e| anyhow::anyhow!("ORT optimization error: {e}"))?
                .commit_from_file(&path)
                .map_err(|e| anyhow::anyhow!("Failed to load {}: {e}", path.display()))?;
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let data_size = std::fs::metadata(path.with_extension("onnx.data"))
                .map(|m| m.len())
                .unwrap_or(0);
            tracing::info!(
                model = name,
                size_mb = (size + data_size) as f64 / 1e6,
                "Loaded ONNX model"
            );
            Ok(session)
        };

        let perceiver = load_session("perceiver")?;
        let comprehender = load_session("comprehender")?;
        let reasoner = load_session("reasoner")?;
        let speaker = load_session("speaker")?;
        let router = load_session("router")?;

        // Load tokenizer
        let tok_path = dir.join("tokenizer").join("tokenizer.json");
        let tokenizer = Tokenizer::from_file(&tok_path)
            .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {e}"))?;

        // Resolve special token IDs
        let tok_id = |s: &str| -> u32 {
            tokenizer
                .token_to_id(s)
                .unwrap_or_else(|| {
                    tracing::warn!(token = s, "Special token not found in tokenizer");
                    0
                })
        };

        let tokens = SpecialTokens {
            input: tok_id("[Input]"),
            intent: tok_id("[Intent]"),
            entities: tok_id("[Entities]"),
            context: tok_id("[Context]"),
            query: tok_id("[Query]"),
            facts: tok_id("[Facts]"),
            inferences: tok_id("[Inferences]"),
            tone: tok_id("[Tone]"),
            response: tok_id("[Response]"),
            eos: tokenizer.token_to_id("<|endoftext|>").unwrap_or(50256),
        };

        let registry = ToolRegistry::new();

        tracing::info!("Cognitive Kernel loaded: 5 models, {} tools", registry.tool_count());

        Ok(Self {
            perceiver: Mutex::new(perceiver),
            comprehender: Mutex::new(comprehender),
            reasoner: Mutex::new(reasoner),
            speaker: Mutex::new(speaker),
            router: Mutex::new(router),
            tokenizer,
            tokens,
            registry,
            model_dir: dir.to_path_buf(),
        })
    }

    /// Run the full cognitive pipeline on a user query.
    fn process(&self, query: &str) -> Result<(String, CognitiveBus)> {
        let mut bus = CognitiveBus {
            raw_input: query.to_string(),
            ..Default::default()
        };

        // ── TICK 1: Perceiver (intent + entities) ──
        let t0 = Instant::now();
        let p_prompt = format!("[Input] {query}\n[Intent]");
        let p_out = self.generate(&self.perceiver, &p_prompt, 40, 0.01)?;
        bus.intent = Self::parse_after(&p_out, "[Intent]")
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        let entities_raw = Self::parse_after(&p_out, "[Entities]");
        bus.entities = entities_raw
            .split(',')
            .map(|e| e.trim().to_string())
            .filter(|e| !e.is_empty())
            .collect();
        let perceiver_ms = t0.elapsed().as_millis();

        // ── Fast path: conversation (no tool needed) ──
        let q_lower = query.to_lowercase();
        let intent_lower = bus.intent.to_lowercase();

        let is_math = q_lower.contains("times")
            || q_lower.contains("plus")
            || q_lower.contains("minus")
            || q_lower.contains("divided")
            || q_lower.contains("percent");

        let is_conversation = !is_math
            && (intent_lower.starts_with("conversation")
                || intent_lower.starts_with("greeting")
                || intent_lower.starts_with("farewell")
                || ["hello", "hi ", "hey", "thanks", "thank you", "goodbye", "bye"]
                    .iter()
                    .any(|w| q_lower.contains(w)));

        if is_conversation {
            let tone = if ["hello", "hi", "hey", "morning"].iter().any(|w| q_lower.contains(w)) {
                "greeting"
            } else if ["bye", "goodbye", "thanks", "thank"].iter().any(|w| q_lower.contains(w)) {
                "farewell"
            } else {
                "neutral"
            };
            let s_prompt = format!("[Facts] none\n[Inferences] none\n[Tone] {tone}\n[Response]");
            bus.response = self.generate(&self.speaker, &s_prompt, 30, 0.3)?;
            bus.response = Self::parse_after(&bus.response, "[Response]");
            bus.tool_match = "respond".into();
            bus.tool_score = 1.0;
            bus.path = "fast";
            tracing::debug!(
                perceiver_ms,
                intent = %bus.intent,
                path = "fast",
                "Cognitive Kernel: conversation"
            );
            return Ok((bus.response.clone(), bus));
        }

        // ── Tool matching ──
        let (tool, score) = self.registry.best_match(query, &bus.intent);
        bus.tool_match = tool.clone();
        bus.tool_score = score;

        // ── TICK 2: Tool execution ──
        let tool_result = self.registry.execute(&tool, query);
        bus.tool_result = tool_result.clone();

        // ── Structured tool results bypass comprehender ──
        if matches!(tool.as_str(), "calculate" | "unit_convert" | "current_time") && !tool_result.is_empty() {
            let fact = match tool.as_str() {
                "calculate" => format!("calculation result is {tool_result}"),
                "unit_convert" => format!("conversion result: {tool_result}"),
                _ => tool_result.clone(),
            };
            bus.facts = vec![fact.clone()];
            bus.inferences = vec!["no reasoning needed".into()];

            let s_prompt = format!("[Facts] {fact}\n[Inferences] none\n[Tone] neutral\n[Response]");
            let s_out = self.generate(&self.speaker, &s_prompt, 40, 0.3)?;
            bus.response = Self::parse_after(&s_out, "[Response]");
            if bus.response.is_empty() || bus.response.len() < 3 {
                bus.response = tool_result;
            }
            bus.path = "tool-bypass";
            return Ok((bus.response.clone(), bus));
        }

        // ── TICK 2b: Comprehender (extract facts from context) ──
        let context = if tool_result.is_empty() || tool_result.starts_with("[Executed") {
            "No information available.".to_string()
        } else {
            tool_result
        };
        let c_prompt = format!("[Context] {}\n[Query] {query}\n[Facts]", &context[..context.len().min(400)]);
        let c_out = self.generate(&self.comprehender, &c_prompt, 80, 0.01)?;
        let facts_raw = Self::parse_after(&c_out, "[Facts]");
        bus.facts = facts_raw
            .split('|')
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty())
            .collect();

        // ── TICK 3: Reasoner ──
        if !bus.facts.is_empty() {
            let facts_str = bus.facts.iter().take(5).cloned().collect::<Vec<_>>().join(" | ");
            let intent_str = bus.intent.lines().next().unwrap_or("question").trim();
            let r_prompt = format!("[Facts] {facts_str}\n[Intent] {intent_str}\n[Inferences]");
            let r_out = self.generate(&self.reasoner, &r_prompt, 40, 0.01)?;
            let inf_raw = Self::parse_after(&r_out, "[Inferences]");
            bus.inferences = inf_raw
                .split('|')
                .map(|i| i.trim().to_string())
                .filter(|i| !i.is_empty())
                .collect();
        }

        // ── TICK 3b: Speaker ──
        let facts_for_speak = if bus.facts.is_empty() {
            "none".to_string()
        } else {
            bus.facts.iter().take(5).cloned().collect::<Vec<_>>().join(" | ")
        };
        let inf_for_speak = if bus.inferences.is_empty() {
            "none".to_string()
        } else {
            bus.inferences.iter().take(3).cloned().collect::<Vec<_>>().join(" | ")
        };
        let s_prompt = format!(
            "[Facts] {facts_for_speak}\n[Inferences] {inf_for_speak}\n[Tone] neutral\n[Response]"
        );
        let s_out = self.generate(&self.speaker, &s_prompt, 80, 0.3)?;
        bus.response = Self::parse_after(&s_out, "[Response]");
        bus.path = "slow";

        Ok((bus.response.clone(), bus))
    }

    /// Autoregressive generation from an ONNX model.
    fn generate(
        &self,
        session: &Mutex<Session>,
        prompt: &str,
        max_tokens: usize,
        temperature: f32,
    ) -> Result<String> {
        let session = &mut *session.lock().map_err(|e| anyhow::anyhow!("Lock poisoned: {e}"))?;
        let encoding = self.tokenizer.encode(prompt, false)
            .map_err(|e| anyhow::anyhow!("Tokenizer encode error: {e}"))?;
        let mut ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
        let prompt_len = ids.len();
        let max_seq = 256; // match model's positional embedding limit

        for _ in 0..max_tokens {
            // Truncate to max sequence length
            let start = if ids.len() > max_seq { ids.len() - max_seq } else { 0 };
            let input_ids = &ids[start..];

            let seq_len = input_ids.len();

            // Create tensor via (shape, data) tuple
            let input_tensor = ort::value::Tensor::from_array(
                ([1usize, seq_len], input_ids.to_vec())
            ).map_err(|e| anyhow::anyhow!("ORT tensor error: {e}"))?;

            let outputs = session.run([input_tensor.into()])
                .map_err(|e| anyhow::anyhow!("ORT run error: {e}"))?;

            // Extract logits: shape = [1, seq_len, vocab_size]
            let logits_view = outputs[0].try_extract_tensor::<f32>()
                .map_err(|e| anyhow::anyhow!("ORT extract error: {e}"))?;
            let (shape, logits_data) = logits_view;
            let vocab_size = shape[2] as usize;
            let last_pos = shape[1] as usize - 1;

            // Get logits slice for last position
            let offset = last_pos * vocab_size;
            let logits_slice: Vec<f32> = logits_data[offset..offset + vocab_size].to_vec();

            // Repetition penalty on generated tokens
            let mut logits = logits_slice;
            if ids.len() > prompt_len {
                let penalty = 1.4f32;
                let recent = &ids[prompt_len.max(ids.len().saturating_sub(50))..];
                for &tid in recent {
                    if (tid as usize) < logits.len() {
                        logits[tid as usize] /= penalty;
                    }
                }
            }

            // Sample next token
            let next_id = if temperature <= 0.01 {
                // Greedy
                logits
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                    .map(|(i, _)| i as i64)
                    .unwrap_or(0)
            } else {
                // Temperature sampling
                let max_logit = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
                let probs: Vec<f32> = logits
                    .iter()
                    .map(|&l| ((l - max_logit) / temperature).exp())
                    .collect();
                let sum: f32 = probs.iter().sum();
                let probs: Vec<f32> = probs.iter().map(|p| p / sum).collect();

                // Multinomial sampling
                let r: f32 = rand::random();
                let mut cumsum = 0.0;
                let mut sampled = 0i64;
                for (i, &p) in probs.iter().enumerate() {
                    cumsum += p;
                    if cumsum >= r {
                        sampled = i as i64;
                        break;
                    }
                }
                sampled
            };

            ids.push(next_id);

            // Stop conditions
            if next_id as u32 == self.tokens.eos {
                break;
            }
            // Check for double newline or "User:" in recent output
            let tail_ids = &ids[ids.len().saturating_sub(10)..];
            let tail = self.tokenizer.decode(
                &tail_ids.iter().map(|&id| id as u32).collect::<Vec<_>>(),
                false,
            ).unwrap_or_default();
            if tail.contains("\n\n") || tail.contains("\nUser:") {
                break;
            }
        }

        let all_ids: Vec<u32> = ids.iter().map(|&id| id as u32).collect();
        let decoded = self.tokenizer.decode(&all_ids, false)
            .map_err(|e| anyhow::anyhow!("Tokenizer decode error: {e}"))?;
        Ok(decoded)
    }

    /// Extract text after a tag marker.
    fn parse_after(text: &str, tag: &str) -> String {
        if let Some(pos) = text.find(tag) {
            let after = text[pos + tag.len()..].trim();
            // Stop at double newline or next tag
            let end = after
                .find("\n\n")
                .or_else(|| after.find("\n["))
                .unwrap_or(after.len());
            after[..end].trim().to_string()
        } else {
            String::new()
        }
    }
}

// ── LLMBackend implementation ──────────────────────────────────────────

impl LLMBackend for CognitiveKernelLLM {
    fn chat(
        &self,
        messages: &[ChatMessage],
        _config: &GenerationConfig,
        _tools: Option<&[serde_json::Value]>,
    ) -> Result<LLMResponse> {
        // Extract the last user message as the query
        let query = messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .unwrap_or("");

        let t0 = Instant::now();
        let (response, bus) = self.process(query)?;
        let elapsed = t0.elapsed();

        tracing::info!(
            tool = %bus.tool_match,
            path = bus.path,
            ms = elapsed.as_millis(),
            "Cognitive Kernel response"
        );

        Ok(LLMResponse {
            text: response,
            prompt_tokens: 0,
            completion_tokens: 0,
            tool_calls: Vec::new(),
            api_tool_calls: Vec::new(),
            stop_reason: "stop".into(),
        })
    }

    fn chat_streaming(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<LLMResponse> {
        // Cognitive Kernel generates all at once — emit as single chunk
        let resp = self.chat(messages, config, tools)?;
        on_token(&resp.text);
        Ok(resp)
    }

    fn count_tokens(&self, text: &str) -> Result<usize> {
        let encoding = self.tokenizer.encode(text, false)
            .map_err(|e| anyhow::anyhow!("Tokenizer encode error: {e}"))?;
        Ok(encoding.get_ids().len())
    }

    fn backend_name(&self) -> &str {
        "cognitive-kernel"
    }

    fn model_id(&self) -> &str {
        "yantrik-cognitive-kernel-v2"
    }
}
