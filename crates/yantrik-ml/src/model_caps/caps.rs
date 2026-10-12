//! What a model can do, for the picker and the gateway: whether it thinks and how hard it can be
//! asked to, whether it sees images, how much it reads at once, and whether it calls tools.
//!
//! Model lists say almost none of this (OpenAI's `/v1/models` gives an id and an owner), so it is
//! worked out here from the provider and the model's family, once, and every surface reads the
//! answer from [`caps_for`]. Where a family is not known the answer says so — `context: 0` is "not
//! stated", and a model with no known thinking knob is offered no Effort rather than one that
//! might be refused.

use serde::{Deserialize, Serialize};

use super::effort::{Effort, Reasoning};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCaps {
    /// The knob for thinking, which the gateway turns an effort into.
    pub reasoning: Reasoning,
    /// The levels the picker offers; empty when it shows no Effort for this model.
    pub efforts: Vec<Effort>,
    pub vision: bool,
    /// Tokens it reads at once; 0 when not stated.
    pub context: u32,
    pub tools: bool,
}

impl ModelCaps {
    pub fn thinks(&self) -> bool {
        !self.efforts.is_empty()
    }
}

/// The model's family: its id lowercased, without a vendor prefix (`openai/gpt-oss-120b`,
/// `@cf/openai/gpt-oss-120b`) or a free-tier suffix (`:free`), keeping an Ollama tag's size.
pub fn family(model: &str) -> String {
    let lower = model.trim().to_ascii_lowercase();
    let last = lower.rsplit('/').next().unwrap_or(&lower);
    last.trim_end_matches(":free").to_string()
}

fn any(f: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| f.contains(n))
}

/// Families that think when asked, whatever host serves them.
fn thinking_family(f: &str) -> bool {
    any(f, &["gpt-oss", "qwen3", "deepseek-r1", "deepseek-reasoner", "magistral", "-thinking", "nemotron"])
        || f.starts_with("o1")
        || f.starts_with("o3")
        || f.starts_with("o4")
        || f.starts_with("gpt-5")
        || claude_thinks(f)
        || gemini_thinks(f)
}

/// Claude thinks from 3.7 on: every Claude but the 3, 3.5 and older.
fn claude_thinks(f: &str) -> bool {
    f.starts_with("claude") && !(f.starts_with("claude-3-") && !f.starts_with("claude-3-7")) && !f.starts_with("claude-2")
        && !f.starts_with("claude-instant")
}

/// Gemini thinks from 2.5 on.
fn gemini_thinks(f: &str) -> bool {
    f.starts_with("gemini-") && !any(f, &["gemini-1", "gemini-2.0", "gemini-pro", "gemini-ultra"])
}

fn vision_family(f: &str) -> bool {
    any(
        f,
        &[
            "vision", "-vl", "vl-", "gpt-4o", "gpt-4.1", "gpt-5", "claude", "gemini", "llava", "pixtral", "llama-4",
            "llama4", "gemma3", "gemma-3", "minicpm-v", "moondream", "kimi-k2.5", "glm-4.5v", "glm-4v",
        ],
    )
}

/// The context a family is known to have, when its list does not say.
fn known_context(f: &str) -> u32 {
    let table: &[(&str, u32)] = &[
        ("gpt-4.1", 1_047_576),
        ("gpt-5", 400_000),
        ("gpt-4o", 128_000),
        ("gpt-oss", 131_072),
        ("o1", 200_000),
        ("o3", 200_000),
        ("o4", 200_000),
        ("claude", 200_000),
        ("gemini", 1_048_576),
        ("deepseek", 128_000),
        ("llama-3.3", 131_072),
        ("llama-3.1", 131_072),
        ("mistral-large", 131_072),
        ("mistral-medium", 131_072),
        ("mistral-small", 131_072),
        ("kimi-k2", 262_144),
        ("glm-4.7", 200_000),
        ("glm-4.5", 128_000),
        ("grok", 256_000),
    ];
    table.iter().find(|(prefix, _)| f.starts_with(prefix)).map(|(_, n)| *n).unwrap_or(0)
}

/// The thinking knob a provider of this type offers this model.
pub fn reasoning_for(provider_type: &str, model: &str) -> Reasoning {
    let f = family(model);
    if !thinking_family(&f) {
        return Reasoning::None;
    }
    match provider_type {
        "ollama" | "ollama-cloud" => Reasoning::OllamaThink { levels: f.contains("gpt-oss") },
        "anthropic" => {
            if claude_thinks(&f) {
                Reasoning::AnthropicBudget
            } else {
                Reasoning::None
            }
        }
        "openrouter" | "kilo" => Reasoning::OpenRouter,
        // OpenAI's own reasoning models, Gemini's 2.5+ and the open gpt-oss take
        // `reasoning_effort` wherever they are served OpenAI-style.
        "openai" => {
            if f.starts_with('o') || f.starts_with("gpt-5") || f.contains("gpt-oss") {
                Reasoning::OpenAiEffort
            } else {
                Reasoning::None
            }
        }
        "gemini" => {
            if gemini_thinks(&f) {
                Reasoning::OpenAiEffort
            } else {
                Reasoning::None
            }
        }
        // DeepSeek's reasoner always thinks and has no knob; R1 elsewhere is the same.
        "deepseek" => Reasoning::None,
        // Any other OpenAI-compatible host: only gpt-oss is known to take the knob there (Groq,
        // Cerebras, OVH, Cloudflare, vLLM all serve it with `reasoning_effort`).
        _ => {
            if f.contains("gpt-oss") {
                Reasoning::OpenAiEffort
            } else {
                Reasoning::None
            }
        }
    }
}

/// What this model can do, served by a provider of this type. `listed_context` and
/// `listed_tools` are what the provider or a curated list said, when it said; they win over the
/// family's defaults.
pub fn caps_for(provider_type: &str, model: &str, listed_context: Option<u32>, listed_tools: Option<bool>) -> ModelCaps {
    let f = family(model);
    let reasoning = reasoning_for(provider_type, model);
    let context = listed_context.filter(|n| *n > 0).unwrap_or_else(|| known_context(&f));
    // Embedding and speech models are not chat models: no tools, no thinking.
    let not_chat = any(&f, &["embed", "whisper", "tts", "rerank", "moderation"]);
    ModelCaps {
        efforts: if not_chat { Vec::new() } else { reasoning.efforts() },
        reasoning: if not_chat { Reasoning::None } else { reasoning },
        vision: !not_chat && vision_family(&f),
        context,
        tools: !not_chat && listed_tools.unwrap_or(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_family_drops_the_vendor_and_the_free_suffix() {
        assert_eq!(family("openai/gpt-oss-120b"), "gpt-oss-120b");
        assert_eq!(family("@cf/openai/gpt-oss-120b"), "gpt-oss-120b");
        assert_eq!(family("qwen/qwen3.8-27b:free"), "qwen3.8-27b");
        assert_eq!(family("qwen3:8b"), "qwen3:8b");
    }

    #[test]
    fn ollama_models_think_with_levels_or_a_switch() {
        let gpt = caps_for("ollama", "gpt-oss:20b", None, None);
        assert_eq!(gpt.reasoning, Reasoning::OllamaThink { levels: true });
        assert_eq!(gpt.efforts, vec![Effort::Easy, Effort::Medium, Effort::High]);
        let qwen = caps_for("ollama", "qwen3:8b", None, None);
        assert_eq!(qwen.reasoning, Reasoning::OllamaThink { levels: false });
        let llama = caps_for("ollama", "llama3.2:3b", None, None);
        assert!(!llama.thinks(), "a model with no knob is offered no Effort");
    }

    #[test]
    fn openai_reasoning_models_take_effort_and_chat_models_do_not() {
        assert_eq!(caps_for("openai", "o4-mini", None, None).reasoning, Reasoning::OpenAiEffort);
        assert_eq!(caps_for("openai", "gpt-5-mini", None, None).reasoning, Reasoning::OpenAiEffort);
        assert!(!caps_for("openai", "gpt-4o-mini", None, None).thinks());
        assert!(caps_for("openai", "gpt-4o-mini", None, None).vision);
        assert_eq!(caps_for("openai", "gpt-4.1", None, None).context, 1_047_576);
    }

    #[test]
    fn claude_thinks_from_three_point_seven_on_with_all_four_levels() {
        let sonnet = caps_for("anthropic", "claude-sonnet-5-5", None, None);
        assert_eq!(sonnet.reasoning, Reasoning::AnthropicBudget);
        assert_eq!(sonnet.efforts, Effort::ALL.to_vec());
        assert!(sonnet.vision && sonnet.context == 200_000);
        assert!(caps_for("anthropic", "claude-3-7-sonnet-latest", None, None).thinks());
        assert!(!caps_for("anthropic", "claude-3-5-haiku-latest", None, None).thinks());
    }

    #[test]
    fn hosts_name_the_knob_their_own_way() {
        assert_eq!(caps_for("openrouter", "openai/gpt-oss-120b:free", None, None).reasoning, Reasoning::OpenRouter);
        assert_eq!(caps_for("groq", "openai/gpt-oss-120b", None, None).reasoning, Reasoning::OpenAiEffort);
        assert_eq!(caps_for("gemini", "gemini-3.8-flash", None, None).reasoning, Reasoning::OpenAiEffort);
        assert_eq!(caps_for("gemini", "gemini-2.0-flash", None, None).reasoning, Reasoning::None);
        assert_eq!(caps_for("deepseek", "deepseek-reasoner", None, None).reasoning, Reasoning::None);
        assert_eq!(caps_for("custom", "llama-3.3-70b", None, None).reasoning, Reasoning::None);
    }

    #[test]
    fn what_the_list_said_wins_and_unknown_is_said_as_unknown() {
        assert_eq!(caps_for("groq", "qwen/qwen3.8-27b", Some(131_072), Some(false)).context, 131_072);
        assert!(!caps_for("groq", "qwen/qwen3.8-27b", Some(131_072), Some(false)).tools);
        assert_eq!(caps_for("custom", "my-finetune", None, None).context, 0);
        let embed = caps_for("openai", "text-embedding-3-small", None, None);
        assert!(!embed.tools && !embed.vision && !embed.thinks());
    }
}
