//! Questions in their wire form (`docs/decisions.md`), for callers in other processes and
//! languages: the `/v1/systemone` request's `questions` object.
//!
//! ```json
//! {"commit": {"type": "noul", "instructions": "Would pressing this spend money?"},
//!  "tool":   {"type": "choice", "instructions": "Which tool?", "criteria": {"a": "…", "b": "…"}},
//!  "urgency":{"type": "score", "instructions": "How urgent?", "criteria": {"0": "low", "1": "high"}}}
//! ```
//!
//! Bounded, because the caller may be anything that can reach the socket: a request cannot make
//! the decision model read a novel.

use serde_json::Value;

use super::Question;

pub const MAX_QUESTIONS: usize = 16;
pub const MAX_OPTIONS: usize = 64;
pub const MAX_TEXT: usize = 2000;
/// Longest question id or option name: a key for code, not a place for text.
pub const MAX_KEY: usize = 64;

/// The questions in a wire-form `questions` object, in id order; a sentence when it is not one.
pub fn questions_from_json(v: &Value) -> Result<Vec<(String, Question)>, String> {
    let obj = v.as_object().ok_or("`questions` is an object of question id to question")?;
    if obj.is_empty() {
        return Err("`questions` has no questions".into());
    }
    if obj.len() > MAX_QUESTIONS {
        return Err(format!("at most {MAX_QUESTIONS} questions at a time, not {}", obj.len()));
    }
    let mut out = Vec::with_capacity(obj.len());
    for (id, q) in obj {
        if id.is_empty() || id.chars().count() > MAX_KEY {
            return Err(format!("a question id is between 1 and {MAX_KEY} characters"));
        }
        let text = |k: &str| -> Result<String, String> {
            let t = q.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
            if t.chars().count() > MAX_TEXT {
                return Err(format!("{id}: `{k}` is longer than {MAX_TEXT} characters"));
            }
            Ok(t)
        };
        let instructions = text("instructions")?;
        if instructions.is_empty() {
            return Err(format!("{id}: a question needs `instructions`"));
        }
        let criteria = || -> Result<Vec<(String, String)>, String> {
            let c = q.get("criteria").and_then(Value::as_object).ok_or_else(|| format!("{id}: needs `criteria`"))?;
            if c.is_empty() || c.len() > MAX_OPTIONS {
                return Err(format!("{id}: between 1 and {MAX_OPTIONS} criteria"));
            }
            c.iter()
                .map(|(k, v)| {
                    if k.is_empty() || k.chars().count() > MAX_KEY {
                        return Err(format!("{id}: an option's name is between 1 and {MAX_KEY} characters"));
                    }
                    let v = v.as_str().ok_or_else(|| format!("{id}: criterion {k} is not text"))?;
                    if v.chars().count() > MAX_TEXT {
                        return Err(format!("{id}: criterion {k} is too long"));
                    }
                    Ok((k.clone(), v.to_string()))
                })
                .collect()
        };
        let question = match q.get("type").and_then(Value::as_str).unwrap_or("") {
            "noul" => Question::Noul { instructions },
            "choice" => Question::Choice { instructions, options: criteria()? },
            "score" => {
                let mut levels = criteria()?;
                // Levels are named 0, 1, 2 …; their order is the scale's.
                levels.sort_by_key(|(k, _)| k.parse::<usize>().unwrap_or(usize::MAX));
                if levels.iter().enumerate().any(|(i, (k, _))| k.parse::<usize>().ok() != Some(i)) {
                    return Err(format!("{id}: score criteria are named 0, 1, 2 … in order"));
                }
                Question::Score { instructions, levels: levels.into_iter().map(|(_, v)| v).collect() }
            }
            other => return Err(format!("{id}: `{other}` is not a question type (noul, choice, score)")),
        };
        out.push((id.clone(), question));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_three_kinds_are_read() {
        let qs = questions_from_json(&json!({
            "commit": {"type": "noul", "instructions": "Would it spend money?"},
            "tool": {"type": "choice", "instructions": "Which?", "criteria": {"a": "one", "b": "two"}},
            "u": {"type": "score", "instructions": "How?", "criteria": {"1": "high", "0": "low"}},
        })).unwrap();
        assert_eq!(qs.len(), 3);
        let score = qs.iter().find(|(id, _)| id == "u").unwrap();
        assert!(matches!(&score.1, Question::Score { levels, .. } if levels == &vec!["low".to_string(), "high".to_string()]));
    }

    #[test]
    fn what_is_not_a_question_is_refused_in_words() {
        for bad in [
            json!([]),
            json!({}),
            json!({"q": {"type": "essay", "instructions": "x"}}),
            json!({"q": {"type": "noul"}}),
            json!({"q": {"type": "choice", "instructions": "x"}}),
            json!({"q": {"type": "score", "instructions": "x", "criteria": {"0": "a", "2": "b"}}}),
            json!({"q": {"type": "noul", "instructions": "x".repeat(MAX_TEXT + 1)}}),
            json!({"q".repeat(MAX_KEY + 1): {"type": "noul", "instructions": "x"}}),
            json!({"q": {"type": "choice", "instructions": "x", "criteria": {"o".repeat(MAX_KEY + 1): "a"}}}),
        ] {
            assert!(questions_from_json(&bad).is_err(), "{bad}");
        }
        let many: serde_json::Map<String, Value> =
            (0..=MAX_QUESTIONS).map(|i| (format!("q{i}"), json!({"type": "noul", "instructions": "x"}))).collect();
        assert!(questions_from_json(&Value::Object(many)).is_err());
    }
}
