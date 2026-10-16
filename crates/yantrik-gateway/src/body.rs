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
            // A caller's own chat_template_kwargs keeps its other keys: only the switch is set.
            match (map.get_mut(&k), v) {
                (Some(Value::Object(theirs)), Value::Object(ours)) => theirs.extend(ours),
                (_, v) => {
                    map.insert(k, v);
                }
            }
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

/// Anthropic's rule while extended thinking is on: `temperature` only at 1, `top_p` only at 0.95
/// or above, and no `top_k`. The gateway is what turns thinking on (the effort mapping above), so it
/// is the gateway that makes the rest of the body fit: a caller that never asked for thinking cannot
/// know its sampling has become invalid. On 542 every Mind turn on Claude Sonnet 5.5 was refused with
/// "`top_p` must be greater than or equal to 0.95 or unset when thinking is enabled or in adaptive
/// mode" (16 Oct 2026). That is a range error, which `refused_params` rightly does not act on, so
/// it is fixed here, from the rule, before the first send. Nothing changes when thinking is off.
pub fn fit_sampling_to_thinking(map: &mut Map<String, Value>) -> Vec<&'static str> {
    let on = match map.get("thinking") {
        Some(Value::Object(t)) => !matches!(t.get("type").and_then(Value::as_str), Some("disabled")),
        _ => false,
    };
    let mut dropped = Vec::new();
    if !on {
        return dropped;
    }
    if map.get("temperature").is_some_and(|v| v.as_f64() != Some(1.0)) {
        map.remove("temperature");
        dropped.push("temperature");
    }
    if map.get("top_p").is_some_and(|v| v.as_f64().is_none_or(|p| p < 0.95)) {
        map.remove("top_p");
        dropped.push("top_p");
    }
    if map.remove("top_k").is_some() {
        dropped.push("top_k");
    }
    dropped
}

/// Take the gateway's own template switch back out of a body, leaving the rest of a caller's
/// `chat_template_kwargs` as it was: for a server that answered 400 to it.
pub fn drop_template_switch(body: &mut Value) {
    let Some(map) = body.as_object_mut() else { return };
    let empty = match map.get_mut("chat_template_kwargs") {
        Some(Value::Object(kw)) => {
            kw.remove("enable_thinking");
            kw.is_empty()
        }
        _ => false,
    };
    if empty {
        map.remove("chat_template_kwargs");
    }
}

/// Sampling parameters a provider may refuse outright for a model ("`temperature` is deprecated for
/// this model"). Every harness sends them, so the gateway takes them out rather than each harness
/// learning which model refuses which.
pub const DROPPABLE: [&str; 2] = ["temperature", "top_p"];

/// What the provider's own words call deprecated or unsupported. A range error ("temperature must
/// be at most 1") or a conflict ("`temperature` and `top_p` cannot both be specified") is not one of
/// these: the parameter is allowed, the value or the pair is wrong, and the caller is shown it.
const REFUSED: [&str; 7] =
    ["is deprecated", "deprecated for this model", "not supported", "unsupported", "does not support", "unrecognized request argument", "unknown parameter"];

/// The parameters of [`DROPPABLE`] that `sent` carries and an upstream's 400 body says the model
/// refuses. The message must name the parameter as a word of its own and say it is deprecated or
/// unsupported. Providers echo an unknown key's name in their 400, so when `sent` has another key
/// holding the parameter's name (`"x temperature"`), the message may be that echo, and the
/// parameter is not taken as refused.
pub fn refused_params(error_body: &[u8], sent: &Value) -> Vec<&'static str> {
    let Some(sent) = sent.as_object() else { return Vec::new() };
    refused_in(error_body)
        .into_iter()
        .filter(|p| sent.contains_key(*p) && !sent.keys().any(|k| k != p && k.to_ascii_lowercase().contains(p)))
        .collect()
}

fn refused_in(error_body: &[u8]) -> Vec<&'static str> {
    let Ok(v) = serde_json::from_slice::<Value>(error_body) else { return Vec::new() };
    let message = v
        .pointer("/error/message")
        .or_else(|| v.get("error").filter(|e| e.is_string()))
        .or_else(|| v.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    if !REFUSED.iter().any(|w| message.contains(w)) {
        return Vec::new();
    }
    // "`temperature` and `top_p` cannot both be specified, `top_p` is deprecated" names both, and
    // refuses one: a parameter counts only when it is named in the same sentence as the refusal.
    let mut found = Vec::new();
    for sentence in message.split(['.', ';', '\n']) {
        if !REFUSED.iter().any(|w| sentence.contains(w)) {
            continue;
        }
        for p in DROPPABLE {
            if names(sentence, p) && !found.contains(&p) {
                found.push(p);
            }
        }
    }
    found
}

/// Whether `text` holds `word` with no identifier character either side (`top_p` is not `top_p2`).
fn names(text: &str, word: &str) -> bool {
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    text.match_indices(word).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + word.len()..].chars().next();
        !before.is_some_and(ident) && !after.is_some_and(ident)
    })
}

/// The parameters the gateway knows the target refuses before anything is sent: Claude 5 and later
/// on Anthropic's own endpoint takes no `temperature`. [`Reasoning::AnthropicBudget`] is the knob
/// only an `anthropic` account is given (`model_caps::reasoning_for`), so it names that endpoint;
/// the same model through a router keeps whatever the router accepts, and any other model is left
/// to the 400 and [`refused_params`].
pub fn known_refused(target: &Target) -> &'static [&'static str] {
    if target.reasoning != yantrik_ml::model_caps::Reasoning::AnthropicBudget {
        return &[];
    }
    let f = yantrik_ml::model_caps::family(&target.model);
    let parts: Vec<&str> = f.split('-').collect();
    // `claude-<name>-<major>…`: `claude-sonnet-5-5`, `claude-opus-5`, `claude-fable-5-1`. The older
    // `claude-3-5-sonnet` puts its version first and is not matched.
    let major = match parts.as_slice() {
        ["claude", name, version, ..] if name.chars().all(|c| c.is_ascii_alphabetic()) => {
            version.split('.').next().and_then(|v| v.parse::<u32>().ok())
        }
        _ => None,
    };
    if major.is_some_and(|m| m >= 5) {
        &["temperature"]
    } else {
        &[]
    }
}

/// Take these parameters out of a body. Returns the ones that were there.
pub fn drop_params(body: &mut Value, params: &[&'static str]) -> Vec<&'static str> {
    let Some(map) = body.as_object_mut() else { return Vec::new() };
    params.iter().copied().filter(|p| map.remove(*p).is_some()).collect()
}

#[cfg(test)]
mod thinking_sampling_tests {
    use super::fit_sampling_to_thinking;
    use serde_json::{json, Value};

    fn fit(v: Value) -> (Value, Vec<&'static str>) {
        let mut v = v;
        let d = fit_sampling_to_thinking(v.as_object_mut().unwrap());
        (v, d)
    }

    #[test]
    fn with_thinking_on_a_low_top_p_a_temperature_and_top_k_are_taken_out() {
        let (b, d) = fit(json!({"model": "m", "thinking": {"type": "enabled", "budget_tokens": 1024},
                                 "temperature": 0.7, "top_p": 0.9, "top_k": 40}));
        assert_eq!(d, vec!["temperature", "top_p", "top_k"]);
        assert!(b.get("temperature").is_none() && b.get("top_p").is_none() && b.get("top_k").is_none());
        assert_eq!(b["thinking"]["budget_tokens"], 1024, "the thinking itself is untouched");
    }

    #[test]
    fn values_the_rule_allows_stay() {
        let (b, d) = fit(json!({"thinking": {"type": "adaptive"}, "temperature": 1.0, "top_p": 0.95}));
        assert!(d.is_empty());
        assert_eq!((b["temperature"].as_f64(), b["top_p"].as_f64()), (Some(1.0), Some(0.95)));
    }

    #[test]
    fn with_thinking_off_or_absent_nothing_changes() {
        for body in [json!({"temperature": 0.2, "top_p": 0.5, "top_k": 3}),
                     json!({"thinking": {"type": "disabled"}, "temperature": 0.2, "top_p": 0.5, "top_k": 3})] {
            let (b, d) = fit(body.clone());
            assert!(d.is_empty());
            assert_eq!(b, body);
        }
    }

    #[test]
    fn a_top_p_that_is_not_a_number_is_taken_out_under_thinking() {
        let (_, d) = fit(json!({"thinking": {"type": "enabled"}, "top_p": "0.9"}));
        assert_eq!(d, vec!["top_p"]);
    }
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
            sharing: crate::policy::Sharing::Off,
            provider: "A".into(),
        }
    }

    #[test]
    fn the_template_switch_joins_a_callers_own_kwargs_and_comes_back_out_alone() {
        let target = Target {
            account: "home".into(),
            model: "qwen3.5:27b".into(),
            base_url: "https://aig.example/v1".into(),
            key: None,
            efforts: Reasoning::ChatTemplateThinking.efforts(),
            reasoning: Reasoning::ChatTemplateThinking,
            local: false,
            sharing: crate::policy::Sharing::Share,
            provider: "Custom".into(),
        };
        let mut b = json!({"model": "home/qwen3.5:27b", "effort": "easy", "chat_template_kwargs": {"keep": 1}, "messages": []});
        rewrite(&mut b, &target).unwrap();
        assert_eq!(b["chat_template_kwargs"], json!({"keep": 1, "enable_thinking": false}), "merged, not replaced");
        drop_template_switch(&mut b);
        assert_eq!(b["chat_template_kwargs"], json!({"keep": 1}), "only the switch comes out");
        let mut alone = json!({"model": "home/qwen3.5:27b", "effort": "easy", "messages": []});
        rewrite(&mut alone, &target).unwrap();
        drop_template_switch(&mut alone);
        assert!(alone.get("chat_template_kwargs").is_none(), "an object that held only the switch goes: {alone}");
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

    /// What Anthropic's OpenAI-compatible endpoint answered for claude-sonnet-5-5 (VM 542, 16 Oct 2026).
    const ANTHROPIC_400: &str = r#"{"error":{"code":"invalid_request_error","message":"`temperature` is deprecated for this model."}}"#;

    #[test]
    fn the_providers_refusal_names_the_parameter_to_drop() {
        let cases: &[(&str, &[&str])] = &[
            (ANTHROPIC_400, &["temperature"]),
            (r#"{"error":{"message":"`top_p` is deprecated for this model."}}"#, &["top_p"]),
            (r#"{"error":{"message":"Unsupported parameter: 'temperature' is not supported with this model."}}"#, &["temperature"]),
            (r#"{"error":{"message":"Unsupported value: 'temperature' does not support 0.2 with this model. Only the default (1) value is supported."}}"#, &["temperature"]),
            (r#"{"error":"Unrecognized request argument supplied: top_p"}"#, &["top_p"]),
            (r#"{"message":"temperature and top_p are not supported for this model"}"#, &["temperature", "top_p"]),
            // Allowed, but the value or the pair is wrong: the caller is shown it.
            (r#"{"error":{"message":"temperature: range error, must be at most 1.0"}}"#, &[]),
            (r#"{"error":{"message":"`temperature` and `top_p` cannot both be specified for this model."}}"#, &[]),
            (r#"{"error":{"message":"`temperature` and `top_p` cannot both be specified. `top_p` is deprecated."}}"#, &["top_p"]),
            // Another parameter, or no parameter of ours.
            (r#"{"error":{"message":"`top_k` is deprecated for this model."}}"#, &[]),
            (r#"{"error":{"message":"`temperatures` is not supported"}}"#, &[]),
            (r#"{"error":{"message":"Unrecognized request argument supplied: chat_template_kwargs"}}"#, &[]),
            (r#"{"error":{"message":"context length exceeded"}}"#, &[]),
            ("temperature is deprecated", &[]),
            ("", &[]),
        ];
        let sent = json!({"model": "x", "messages": [], "temperature": 0.7, "top_p": 0.9});
        for (body, want) in cases {
            assert_eq!(refused_params(body.as_bytes(), &sent), *want, "{body}");
        }
    }

    #[test]
    fn a_refusal_that_may_echo_the_callers_own_key_or_names_what_was_not_sent_drops_nothing() {
        let echoed = |key: &str| json!({"error": {"message": format!("Unrecognized request argument supplied: {key}")}}).to_string();
        for key in ["x temperature", "x. temperature is not supported", "my_temperature"] {
            let sent = json!({"model": "x", "temperature": 0.7, key: 1});
            assert!(refused_params(echoed(key).as_bytes(), &sent).is_empty(), "{key}");
        }
        let without = json!({"model": "x", "messages": []});
        assert!(refused_params(ANTHROPIC_400.as_bytes(), &without).is_empty(), "nothing to drop");
    }

    #[test]
    fn claude_five_on_anthropics_endpoint_is_sent_no_temperature_from_the_start() {
        let on = |reasoning, model: &str| known_refused(&Target { model: model.into(), ..target(reasoning) });
        for model in ["claude-sonnet-5-5", "claude-opus-5-5", "claude-haiku-5-5", "claude-fable-5-1", "claude-opus-5", "claude-sonnet-5-5-20261001"] {
            assert_eq!(on(Reasoning::AnthropicBudget, model), &["temperature"], "{model}");
        }
        for model in ["claude-opus-4-1", "claude-sonnet-4-5", "claude-3-7-sonnet-latest", "claude-3-5-sonnet-latest"] {
            assert!(on(Reasoning::AnthropicBudget, model).is_empty(), "{model}");
        }
        // The same model through a router, or an endpoint the person typed, is left to the 400.
        assert!(on(Reasoning::OpenRouter, "anthropic/claude-sonnet-5-5").is_empty());
        assert!(on(Reasoning::None, "claude-sonnet-5-5").is_empty());
    }

    #[test]
    fn dropping_takes_out_only_what_is_there() {
        let mut b = json!({"model": "x", "temperature": 0.7, "messages": []});
        assert_eq!(drop_params(&mut b, &["temperature", "top_p"]), vec!["temperature"]);
        assert_eq!(b, json!({"model": "x", "messages": []}));
        assert!(drop_params(&mut b, &["temperature"]).is_empty());
    }
}
