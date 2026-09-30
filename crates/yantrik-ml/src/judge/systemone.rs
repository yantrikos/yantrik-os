//! A judge reached over HTTP at a `/v1/systemone` endpoint: TypeSafe's cloud (Jev), or a local
//! server speaking the same protocol — Kev (`kev.serve`), Laya (`laya-serve`), Jeff (`jeff-serve`).
//!
//! They share the request and reply shape and differ in small ways that matter, which the
//! [`Dialect`] carries rather than every caller learning them:
//!
//! - **Jev** needs a key.
//! - **Laya**'s `confidence` is its own formula, not Jev's, so it is recomputed from the
//!   probabilities (Unsloth's own migration note says to recalibrate on `probabilities`).
//! - **Jeff** serves one request at a time and answers 529 while busy, so a busy reply is waited
//!   out briefly instead of failing the decision.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};

use super::{Answer, Judge, JudgeInfo, Locality, Question};

/// Which server this is, for the ways they differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Jev,
    Kev,
    Laya,
    Jeff,
    /// Any other `/v1/systemone` server: taken at its word.
    Generic,
}

impl Dialect {
    pub fn named(name: &str) -> Dialect {
        match name.trim().to_ascii_lowercase().as_str() {
            "jev" => Dialect::Jev,
            "kev" => Dialect::Kev,
            "laya" => Dialect::Laya,
            "jeff" => Dialect::Jeff,
            _ => Dialect::Generic,
        }
    }

    /// Guessed from the model name when the configuration does not say.
    pub fn of_model(model: &str) -> Dialect {
        let m = model.to_ascii_lowercase();
        ["jev", "kev", "laya", "jeff"].iter().find(|d| m.starts_with(*d)).map_or(Dialect::Generic, |d| Dialect::named(d))
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Dialect::Jev => "jev",
            Dialect::Kev => "kev",
            Dialect::Laya => "laya",
            Dialect::Jeff => "jeff",
            Dialect::Generic => "systemone",
        }
    }
}

/// How many times a busy server (429/503/529) is asked again, and how long between.
const BUSY_RETRIES: u32 = 6;
const BUSY_PAUSE: Duration = Duration::from_millis(150);

/// Where the judge is and how to reach it. The key is never held here as configuration: `key_env`
/// names the environment variable it is read from, when the endpoint needs one.
#[derive(Debug, Clone)]
pub struct SystemOneJudge {
    endpoint: String,
    model: String,
    key_env: Option<String>,
    timeout: Duration,
    dialect: Dialect,
}

impl SystemOneJudge {
    /// `base_url` is the server (e.g. `https://api.typesafe.ai`, `http://127.0.0.1:8009`); the
    /// `/v1/systemone` path is added unless it is already there. The dialect is read from the
    /// model name; [`SystemOneJudge::with_dialect`] says it outright.
    pub fn new(base_url: &str, model: &str, key_env: Option<&str>, timeout: Duration) -> Self {
        let base = base_url.trim_end_matches('/');
        let endpoint = if base.ends_with("/v1/systemone") { base.to_string() } else { format!("{base}/v1/systemone") };
        Self {
            endpoint,
            model: model.to_string(),
            key_env: key_env.filter(|k| !k.is_empty()).map(str::to_string),
            timeout,
            dialect: Dialect::of_model(model),
        }
    }

    pub fn with_dialect(mut self, dialect: Dialect) -> Self {
        self.dialect = dialect;
        self
    }

    fn body(&self, state: &Value, questions: &[(&str, Question)]) -> Value {
        let mut qs = Map::new();
        for (id, q) in questions {
            let q = match q {
                Question::Choice { instructions, options } => {
                    let criteria: Map<String, Value> =
                        options.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect();
                    json!({"type": "choice", "instructions": instructions, "criteria": criteria})
                }
                Question::Noul { instructions } => json!({"type": "noul", "instructions": instructions}),
                Question::Score { instructions, levels } => {
                    let criteria: Map<String, Value> =
                        levels.iter().enumerate().map(|(i, l)| (i.to_string(), Value::String(l.clone()))).collect();
                    json!({"type": "score", "instructions": instructions, "criteria": criteria})
                }
            };
            qs.insert((*id).to_string(), q);
        }
        json!({"model": self.model, "state": state, "questions": qs})
    }
}

impl Judge for SystemOneJudge {
    fn name(&self) -> &str {
        &self.model
    }

    fn info(&self) -> JudgeInfo {
        JudgeInfo {
            adapter: "systemone",
            provider: self.dialect.as_str().to_string(),
            model: self.model.clone(),
            locality: Locality::of_endpoint(&self.endpoint),
            calibrated: true,
        }
    }

    fn ask(&self, state: &Value, questions: &[(&str, Question)]) -> Result<HashMap<String, Answer>> {
        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .timeout_global(Some(self.timeout))
                .http_status_as_error(false)
                .build(),
        );
        let key = match &self.key_env {
            Some(var) => Some(std::env::var(var).map_err(|_| anyhow!("the judge's key variable {var} is not set"))?),
            None => None,
        };
        let body = self.body(state, questions);
        let mut attempt = 0;
        loop {
            let mut request = agent.post(&self.endpoint).header("Content-Type", "application/json");
            if let Some(key) = &key {
                request = request.header("Authorization", &format!("Bearer {key}"));
            }
            let mut reply = request
                .send_json(&body)
                .with_context(|| format!("judge {} at {}", self.model, self.endpoint))?;
            let status = reply.status().as_u16();
            if matches!(status, 429 | 503 | 529) && attempt < BUSY_RETRIES {
                attempt += 1;
                std::thread::sleep(BUSY_PAUSE * attempt);
                continue;
            }
            if !(200..300).contains(&status) {
                bail!("judge {} at {} answered HTTP {status}", self.model, self.endpoint);
            }
            let reply: Value = reply.body_mut().read_json().context("judge reply is not JSON")?;
            return parse_answers(&reply, questions, self.dialect);
        }
    }
}

/// Every question asked must come back answered, in the shape its type promises; a partial or
/// malformed reply is an error rather than a guess.
fn parse_answers(reply: &Value, questions: &[(&str, Question)], dialect: Dialect) -> Result<HashMap<String, Answer>> {
    let answers = reply.get("answers").and_then(Value::as_object).ok_or_else(|| anyhow!("judge reply has no answers"))?;
    let mut out = HashMap::new();
    for (id, q) in questions {
        let a = answers.get(*id).ok_or_else(|| anyhow!("judge did not answer {id}"))?;
        let answer = match q {
            Question::Choice { options, .. } => {
                let choice = a.get("choice").and_then(Value::as_str).ok_or_else(|| anyhow!("{id}: no choice"))?;
                if !options.iter().any(|(k, _)| k == choice) {
                    bail!("{id}: the judge chose {choice:?}, which was not offered");
                }
                let probabilities: HashMap<String, f64> = a
                    .get("probabilities")
                    .and_then(Value::as_object)
                    .map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_f64()?))).collect())
                    .unwrap_or_default();
                let confidence = match dialect {
                    // Laya's confidence is its own formula: the concentration of the distribution
                    // is read off the probabilities instead, the same way for every server.
                    Dialect::Laya => concentration(&probabilities),
                    _ => a.get("confidence").and_then(Value::as_f64).unwrap_or_else(|| concentration(&probabilities)),
                };
                Answer::Choice { choice: choice.to_string(), probabilities, confidence }
            }
            Question::Noul { .. } => Answer::Noul(a.get("noul").and_then(Value::as_f64).ok_or_else(|| anyhow!("{id}: no noul"))?),
            Question::Score { levels, .. } => {
                let value = a.get("score").or_else(|| a.get("value")).and_then(Value::as_f64)
                    .ok_or_else(|| anyhow!("{id}: no score"))?;
                if value < 0.0 || value > (levels.len().saturating_sub(1)) as f64 {
                    bail!("{id}: score {value} is outside the {} levels asked for", levels.len());
                }
                let distribution = a
                    .get("probabilities")
                    .and_then(Value::as_object)
                    .map(|m| (0..levels.len()).map(|i| m.get(&i.to_string()).and_then(Value::as_f64).unwrap_or(0.0)).collect())
                    .unwrap_or_default();
                Answer::Score { value, distribution }
            }
        };
        out.insert((*id).to_string(), answer);
    }
    Ok(out)
}

/// How concentrated a distribution is: the largest probability, 1.0 when it is all on one option.
fn concentration(probabilities: &HashMap<String, f64>) -> f64 {
    probabilities.values().copied().fold(0.0, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_question() -> Vec<(&'static str, Question)> {
        vec![
            ("tool", Question::Choice {
                instructions: "Which tool?".into(),
                options: vec![("get_weather".into(), "weather".into()), ("none".into(), "no tool".into())],
            }),
            ("urgent", Question::Noul { instructions: "Is it urgent?".into() }),
        ]
    }

    #[test]
    fn endpoint_path_is_added_once() {
        let d = Duration::from_secs(1);
        assert_eq!(SystemOneJudge::new("http://127.0.0.1:8009/", "kev-latest", None, d).endpoint, "http://127.0.0.1:8009/v1/systemone");
        assert_eq!(SystemOneJudge::new("https://api.typesafe.ai/v1/systemone", "jev-latest", None, d).endpoint, "https://api.typesafe.ai/v1/systemone");
    }

    #[test]
    fn request_carries_model_state_and_typed_questions() {
        let j = SystemOneJudge::new("http://x", "kev-latest", None, Duration::from_secs(1));
        let mut qs = tool_question();
        qs.push(("urgency", Question::Score { instructions: "How urgent?".into(), levels: vec!["low".into(), "high".into()] }));
        let body = j.body(&json!({"request": "weather in Dallas"}), &qs);
        assert_eq!(body["model"], "kev-latest");
        assert_eq!(body["state"]["request"], "weather in Dallas");
        assert_eq!(body["questions"]["tool"]["type"], "choice");
        assert_eq!(body["questions"]["tool"]["criteria"]["get_weather"], "weather");
        assert_eq!(body["questions"]["urgent"]["type"], "noul");
        assert_eq!(body["questions"]["urgency"]["criteria"]["1"], "high");
    }

    #[test]
    fn a_whole_reply_parses() {
        let reply = json!({"answers": {
            "tool": {"type": "choice", "choice": "get_weather", "confidence": 0.9, "probabilities": {"get_weather": 0.95, "none": 0.05}},
            "urgent": {"type": "noul", "noul": 0.12}}});
        let a = parse_answers(&reply, &tool_question(), Dialect::Kev).unwrap();
        assert_eq!(a["tool"].picked_probability(), Some(0.95));
        assert_eq!(a["urgent"], Answer::Noul(0.12));
    }

    #[test]
    fn layas_confidence_is_read_off_its_probabilities() {
        let reply = json!({"answers": {
            "tool": {"choice": "get_weather", "confidence": 0.1, "probabilities": {"get_weather": 0.8, "none": 0.2}},
            "urgent": {"noul": 0.5}}});
        let Answer::Choice { confidence, .. } = &parse_answers(&reply, &tool_question(), Dialect::Laya).unwrap()["tool"] else { panic!() };
        assert_eq!(*confidence, 0.8);
    }

    #[test]
    fn a_score_outside_its_levels_is_an_error() {
        let qs = [("u", Question::Score { instructions: "?".into(), levels: vec!["a".into(), "b".into()] })];
        assert!(parse_answers(&json!({"answers": {"u": {"score": 1.4, "probabilities": {"0": 0.3, "1": 0.7}}}}), &qs, Dialect::Jev).is_err());
        let ok = parse_answers(&json!({"answers": {"u": {"score": 0.7, "probabilities": {"0": 0.3, "1": 0.7}}}}), &qs, Dialect::Jev).unwrap();
        assert_eq!(ok["u"], Answer::Score { value: 0.7, distribution: vec![0.3, 0.7] });
    }

    #[test]
    fn a_missing_or_unoffered_answer_is_an_error_not_a_guess() {
        let missing = json!({"answers": {"tool": {"choice": "none", "probabilities": {}}}});
        assert!(parse_answers(&missing, &tool_question(), Dialect::Generic).is_err());
        let unoffered = json!({"answers": {"tool": {"choice": "rm_rf"}, "urgent": {"noul": 0.1}}});
        assert!(parse_answers(&unoffered, &tool_question(), Dialect::Generic).unwrap_err().to_string().contains("not offered"));
    }

    #[test]
    fn a_key_variable_that_is_not_set_fails_before_any_request() {
        let j = SystemOneJudge::new("http://127.0.0.1:9", "jev-latest", Some("YANTRIK_TEST_UNSET_JUDGE_KEY"), Duration::from_millis(50));
        let err = j.ask(&json!({}), &tool_question()).unwrap_err().to_string();
        assert!(err.contains("YANTRIK_TEST_UNSET_JUDGE_KEY"), "{err}");
    }

    #[test]
    fn a_judge_that_cannot_be_reached_abstains_in_its_verdict() {
        let j = SystemOneJudge::new("http://127.0.0.1:9", "kev-latest", None, Duration::from_millis(200));
        let v = j.decide(&json!({}), &tool_question());
        assert!(v.abstained(), "{v:?}");
        assert_eq!(v.by.provider, "kev");
        assert_eq!(v.by.locality, Locality::ThisMachine);
    }

    #[test]
    fn the_dialect_follows_the_model_name() {
        assert_eq!(Dialect::of_model("jev-latest"), Dialect::Jev);
        assert_eq!(Dialect::of_model("laya"), Dialect::Laya);
        assert_eq!(Dialect::of_model("jeff-latest"), Dialect::Jeff);
        assert_eq!(Dialect::of_model("my-judge"), Dialect::Generic);
    }
}
