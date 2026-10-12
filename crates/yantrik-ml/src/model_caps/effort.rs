//! How hard a model is asked to think, in the person's four words, and what each provider calls it.
//!
//! The picker offers easy / medium / high / xhigh. Every provider has its own knob for the same
//! thing — Ollama's `think`, OpenAI's `reasoning_effort`, OpenRouter's `reasoning.effort`,
//! Anthropic's thinking budget — or none at all, and the gateway (crates/yantrik-gateway) turns the
//! word into that knob with [`params`]. A model that has no knob is sent none: the picker does not
//! offer Effort for it, and a harness that asks anyway is not refused, its word is dropped.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// The person's word for how hard to think.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Easy,
    Medium,
    High,
    #[serde(rename = "xhigh")]
    XHigh,
}

impl Effort {
    pub const ALL: [Effort; 4] = [Effort::Easy, Effort::Medium, Effort::High, Effort::XHigh];

    /// The word on the wire and in the picker: `easy`, `medium`, `high`, `xhigh`.
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Easy => "easy",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::XHigh => "xhigh",
        }
    }

    /// The person's words, and the words harnesses already send as OpenAI's `reasoning_effort`
    /// (`minimal`, `low`), so a harness that speaks OpenAI is understood too.
    pub fn parse(word: &str) -> Option<Effort> {
        match word.trim().to_ascii_lowercase().as_str() {
            "easy" | "low" | "minimal" => Some(Effort::Easy),
            "medium" => Some(Effort::Medium),
            "high" => Some(Effort::High),
            "xhigh" | "x-high" | "max" => Some(Effort::XHigh),
            _ => None,
        }
    }
}

/// Which knob a model's provider has for thinking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reasoning {
    /// No knob: the model thinks as it always does, or not at all.
    None,
    /// `reasoning_effort: low|medium|high` (OpenAI, Gemini's and Groq's OpenAI-compatible APIs).
    OpenAiEffort,
    /// `reasoning: {effort}` (OpenRouter, which passes it on to whichever host serves the model).
    OpenRouter,
    /// Ollama's `think`: `low|medium|high` for the models that take levels (gpt-oss), `true` or
    /// `false` for the ones that only switch thinking on and off (qwen3, deepseek-r1).
    OllamaThink { levels: bool },
    /// Anthropic's extended thinking, `thinking: {type: enabled, budget_tokens}`, through its
    /// OpenAI-compatible endpoint.
    AnthropicBudget,
}

impl Reasoning {
    /// The levels the picker offers for a model with this knob. Empty: Effort is not shown.
    pub fn efforts(self) -> Vec<Effort> {
        match self {
            Reasoning::None => Vec::new(),
            // OpenAI-style knobs stop at high; xhigh would be sent as high, so it is not offered.
            Reasoning::OpenAiEffort | Reasoning::OpenRouter => vec![Effort::Easy, Effort::Medium, Effort::High],
            Reasoning::OllamaThink { levels: true } => vec![Effort::Easy, Effort::Medium, Effort::High],
            // On or off: easy is off, medium is on. Nothing above it would differ.
            Reasoning::OllamaThink { levels: false } => vec![Effort::Easy, Effort::Medium],
            Reasoning::AnthropicBudget => Effort::ALL.to_vec(),
        }
    }
}

/// Anthropic's thinking budget for each word, in tokens.
pub fn anthropic_budget(effort: Effort) -> u32 {
    match effort {
        Effort::Easy => 1_024,
        Effort::Medium => 4_096,
        Effort::High => 16_384,
        Effort::XHigh => 32_768,
    }
}

/// What to put into a chat-completions body for `effort` on a model with this knob: the fields to
/// set, and the least `max_tokens` the request then needs (Anthropic refuses a budget that is not
/// below `max_tokens`). Nothing for a model with no knob.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EffortParams {
    pub set: Map<String, Value>,
    pub min_max_tokens: Option<u32>,
}

pub fn params(reasoning: Reasoning, effort: Effort) -> EffortParams {
    let mut set = Map::new();
    let mut min_max_tokens = None;
    // xhigh is never offered where it would not differ; a harness that asks for it anyway gets
    // the highest the provider has.
    let openai_word = match effort {
        Effort::Easy => "low",
        Effort::Medium => "medium",
        Effort::High | Effort::XHigh => "high",
    };
    match reasoning {
        Reasoning::None => {}
        Reasoning::OpenAiEffort => {
            set.insert("reasoning_effort".into(), json!(openai_word));
        }
        Reasoning::OpenRouter => {
            set.insert("reasoning".into(), json!({ "effort": openai_word }));
        }
        Reasoning::OllamaThink { levels: true } => {
            set.insert("think".into(), json!(openai_word));
        }
        Reasoning::OllamaThink { levels: false } => {
            set.insert("think".into(), json!(effort != Effort::Easy));
        }
        Reasoning::AnthropicBudget => {
            let budget = anthropic_budget(effort);
            set.insert("thinking".into(), json!({ "type": "enabled", "budget_tokens": budget }));
            min_max_tokens = Some(budget + 4_096);
        }
    }
    EffortParams { set, min_max_tokens }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_words_round_trip_and_openai_words_are_understood() {
        for e in Effort::ALL {
            assert_eq!(Effort::parse(e.as_str()), Some(e));
            assert_eq!(serde_json::to_value(e).unwrap(), json!(e.as_str()));
        }
        assert_eq!(Effort::parse("low"), Some(Effort::Easy));
        assert_eq!(Effort::parse(" HIGH "), Some(Effort::High));
        assert_eq!(Effort::parse("turbo"), None);
    }

    #[test]
    fn openai_style_providers_get_reasoning_effort() {
        assert_eq!(params(Reasoning::OpenAiEffort, Effort::Easy).set["reasoning_effort"], "low");
        assert_eq!(params(Reasoning::OpenAiEffort, Effort::Medium).set["reasoning_effort"], "medium");
        assert_eq!(params(Reasoning::OpenAiEffort, Effort::XHigh).set["reasoning_effort"], "high");
        assert_eq!(params(Reasoning::OpenRouter, Effort::High).set["reasoning"], json!({"effort": "high"}));
    }

    #[test]
    fn ollama_gets_think_as_a_level_or_a_switch() {
        assert_eq!(params(Reasoning::OllamaThink { levels: true }, Effort::Medium).set["think"], "medium");
        assert_eq!(params(Reasoning::OllamaThink { levels: false }, Effort::Easy).set["think"], false);
        assert_eq!(params(Reasoning::OllamaThink { levels: false }, Effort::High).set["think"], true);
    }

    #[test]
    fn anthropic_gets_a_thinking_budget_below_max_tokens() {
        let p = params(Reasoning::AnthropicBudget, Effort::XHigh);
        assert_eq!(p.set["thinking"], json!({"type": "enabled", "budget_tokens": 32_768}));
        assert!(p.min_max_tokens.unwrap() > 32_768);
        let budgets: Vec<u32> = Effort::ALL.iter().map(|e| anthropic_budget(*e)).collect();
        assert!(budgets.windows(2).all(|w| w[0] < w[1]), "each word thinks more: {budgets:?}");
    }

    #[test]
    fn a_model_with_no_knob_is_sent_nothing_and_offered_nothing() {
        assert_eq!(params(Reasoning::None, Effort::XHigh), EffortParams::default());
        assert!(Reasoning::None.efforts().is_empty());
        assert_eq!(Reasoning::AnthropicBudget.efforts().len(), 4);
        assert!(!Reasoning::OpenAiEffort.efforts().contains(&Effort::XHigh), "never offered where it would read as high");
    }
}
