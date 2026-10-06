//! The request as the account receives it: the upstream model in place of `<account>/<model>`,
//! the effort in the provider's own words, and usage asked for so the log can count tokens.

use serde_json::{json, Map, Value};
use yantrik_ml::model_caps::{effort_params, Effort};

use crate::policy::{Refusal, Target};

/// The effort a request asked for: the gateway's own `effort`, or OpenAI's `reasoning_effort` in
/// any of its words. Both are taken out: the provider is sent its own knob, or nothing.
pub fn take_effort(body: &mut Map<String, Value>) -> Option<Effort> {
    let ours = body.remove("effort");
    let openai = body.remove("reasoning_effort");
    ours.or(openai).and_then(|v| v.as_str().and_then(Effort::parse))
}

/// The highest level the model is offered at that is not above `asked`, when its levels are known.
pub fn clamp(asked: Effort, offered: &[Effort]) -> Effort {
    if offered.is_empty() || offered.contains(&asked) {
        return asked;
    }
    offered.iter().copied().filter(|e| *e <= asked).max().or_else(|| offered.iter().copied().min()).unwrap_or(asked)
}

/// Rewrite a chat-completions body for `target`. Returns the effort applied, for the log.
pub fn rewrite(body: &mut Value, target: &Target) -> Result<Option<Effort>, Refusal> {
    let map = body.as_object_mut().ok_or_else(|| Refusal::new(400, "bad_request", "The request body must be a JSON object."))?;
    map.insert("model".into(), Value::String(target.model.clone()));
    let asked = take_effort(map);
    let applied = asked.map(|e| clamp(e, &target.efforts));
    if let Some(effort) = applied {
        let p = effort_params(target.reasoning, effort);
        for (k, v) in p.set {
            map.insert(k, v);
        }
        if let Some(least) = p.min_max_tokens {
            for field in ["max_tokens", "max_completion_tokens"] {
                if let Some(n) = map.get(field).and_then(Value::as_u64) {
                    if n < least as u64 {
                        map.insert(field.into(), json!(least));
                    }
                }
            }
            if !map.contains_key("max_tokens") && !map.contains_key("max_completion_tokens") {
                map.insert("max_tokens".into(), json!(least));
            }
        }
    }
    if map.get("stream").and_then(Value::as_bool) == Some(true) && !map.contains_key("stream_options") {
        map.insert("stream_options".into(), json!({ "include_usage": true }));
    }
    // A model with no knob was sent none, and the log says so.
    Ok(if target.reasoning == yantrik_ml::model_caps::Reasoning::None { None } else { applied })
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_ml::model_caps::Reasoning;

    fn target(reasoning: Reasoning) -> Target {
        Target {
            account: "a".into(),
            model: "up-model".into(),
            base_url: "https://x/v1".into(),
            key: None,
            efforts: reasoning.efforts(),
            reasoning,
            local: false,
            private_ok: false,
        }
    }

    #[test]
    fn the_upstream_model_replaces_the_gateway_id_and_effort_becomes_the_providers_knob() {
        let mut b = json!({"model": "a/up-model", "effort": "high", "messages": []});
        let applied = rewrite(&mut b, &target(Reasoning::OpenAiEffort)).unwrap();
        assert_eq!(b["model"], "up-model");
        assert_eq!(b["reasoning_effort"], "high");
        assert!(b.get("effort").is_none(), "our own field never goes upstream");
        assert_eq!(applied, Some(Effort::High));
    }

    #[test]
    fn each_provider_gets_its_own_parameter() {
        let mut ollama = json!({"model": "x", "effort": "medium"});
        rewrite(&mut ollama, &target(Reasoning::OllamaThink { levels: true })).unwrap();
        assert_eq!(ollama["think"], "medium");
        assert!(ollama.get("reasoning_effort").is_none());

        let mut claude = json!({"model": "x", "reasoning_effort": "xhigh", "max_tokens": 1000});
        rewrite(&mut claude, &target(Reasoning::AnthropicBudget)).unwrap();
        assert_eq!(claude["thinking"]["budget_tokens"], 32_768);
        assert!(claude["max_tokens"].as_u64().unwrap() > 32_768, "the budget fits under max_tokens");

        let mut none = json!({"model": "x", "effort": "high", "reasoning_effort": "low"});
        let applied = rewrite(&mut none, &target(Reasoning::None)).unwrap();
        assert!(none.get("reasoning_effort").is_none() && none.get("think").is_none() && none.get("thinking").is_none());
        assert_eq!(applied, None, "a model with no knob is sent none and logged with none");
    }

    #[test]
    fn a_level_the_model_does_not_have_becomes_the_nearest_below() {
        assert_eq!(clamp(Effort::XHigh, &[Effort::Easy, Effort::Medium, Effort::High]), Effort::High);
        assert_eq!(clamp(Effort::High, &[Effort::Easy, Effort::Medium]), Effort::Medium);
        assert_eq!(clamp(Effort::Easy, &[Effort::Medium]), Effort::Medium);
        assert_eq!(clamp(Effort::High, &[]), Effort::High);
    }

    #[test]
    fn a_stream_asks_for_usage_so_tokens_can_be_counted() {
        let mut b = json!({"model": "x", "stream": true});
        rewrite(&mut b, &target(Reasoning::None)).unwrap();
        assert_eq!(b["stream_options"]["include_usage"], true);
        let mut kept = json!({"model": "x", "stream": true, "stream_options": {"include_usage": false}});
        rewrite(&mut kept, &target(Reasoning::None)).unwrap();
        assert_eq!(kept["stream_options"]["include_usage"], false, "the harness's own choice stands");
        assert!(rewrite(&mut json!([1]), &target(Reasoning::None)).is_err());
    }
}
