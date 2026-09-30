//! Judges: small models that answer a typed question about some state with probabilities,
//! not text.
//!
//! A System One model (TypeSafe's Jev in the cloud, Kev, Laya or Jeff run locally, and whatever
//! speaks the same `/v1/systemone` protocol next) reads the state once and returns, for each
//! question, a distribution over its options. That is enough for the decisions a chat model is
//! otherwise asked to write out at length (which tool fits this request, whether a condition
//! holds), at a fraction of the tokens and in a fraction of the time. What to do with an answer,
//! and how sure it must be, stays with the caller.
//!
//! # One shape for every model
//!
//! Every adapter answers in the same [`Verdict`]: the answers keyed by question id, and who gave
//! them — which adapter, which model, and whether the state left this machine. A caller written
//! against a `Verdict` does not know or care whether Kev on the GPU box, Laya on this CPU, Jev in
//! the cloud or the chat model answered, which is what lets the person switch the decision model
//! in Settings without anything else changing. The same shape is the wire form other languages
//! read (`docs/decisions.md`).
//!
//! An answer may be [`Answer::Abstain`]: the adapter could not, or would not, decide. That is not
//! an error. It is what "Off" answers to everything, what an adapter says when the model is
//! unsure in a way it can tell, and what every caller must treat as "decide as you would have
//! without a judge".

mod adapters;
mod verdict;
mod wire;

#[cfg(feature = "api-llm")]
mod systemone;

pub use adapters::{ChatJudge, OffJudge};
pub use verdict::{JudgeInfo, Locality, Verdict};
pub use wire::questions_from_json;

#[cfg(feature = "api-llm")]
pub use systemone::{Dialect, SystemOneJudge};

use std::collections::HashMap;
use std::time::Instant;

use anyhow::Result;
use serde_json::Value;

/// One question put to a judge.
#[derive(Debug, Clone)]
pub enum Question {
    /// Pick one of `options` (name, description), in the order given.
    Choice { instructions: String, options: Vec<(String, String)> },
    /// Does the condition in `instructions` hold?
    Noul { instructions: String },
    /// Where on the scale `levels` (lowest first, each a description) does the state sit?
    Score { instructions: String, levels: Vec<String> },
}

/// A judge's answer to one question.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    /// The most likely option, the probability of each option, and how concentrated the
    /// distribution is (1.0 = all on one option).
    Choice { choice: String, probabilities: HashMap<String, f64>, confidence: f64 },
    /// The probability that the condition holds.
    Noul(f64),
    /// The expected level (0-based, fractional: 1.6 is between the second and third level) and
    /// the probability of each level, lowest first. A position on a scale, not a probability.
    Score { value: f64, distribution: Vec<f64> },
    /// No decision: the judge is off, unreachable, or said it could not tell. The caller decides
    /// as it would have without a judge.
    Abstain { reason: String },
}

impl Answer {
    /// The probability the judge gave the option it picked, for a choice answer.
    pub fn picked_probability(&self) -> Option<f64> {
        match self {
            Answer::Choice { choice, probabilities, .. } => probabilities.get(choice).copied(),
            _ => None,
        }
    }

    /// The probability of yes, for a noul answer; `None` for an abstention or another kind.
    pub fn yes(&self) -> Option<f64> {
        match self {
            Answer::Noul(p) => Some(*p),
            _ => None,
        }
    }

    pub fn is_abstain(&self) -> bool {
        matches!(self, Answer::Abstain { .. })
    }
}

/// Something that answers typed questions about a state. Questions asked together see the same
/// state and are answered independently.
pub trait Judge: Send + Sync {
    /// Model name, for logs.
    fn name(&self) -> &str;

    /// Who this judge is, for the verdict: adapter, provider, model and where it runs. The
    /// default claims nothing it cannot know: an unknown judge is taken to run in the cloud and
    /// to be uncalibrated, so no caller relaxes a privacy rule or a threshold on its word.
    fn info(&self) -> JudgeInfo {
        JudgeInfo {
            adapter: "custom",
            provider: self.name().to_string(),
            model: self.name().to_string(),
            locality: Locality::Cloud,
            calibrated: false,
        }
    }

    /// Ask `questions` (id, question) about `state`; answers are keyed by the same ids.
    fn ask(&self, state: &Value, questions: &[(&str, Question)]) -> Result<HashMap<String, Answer>>;

    /// Ask, and answer in the one shape every caller reads. A judge that fails answers every
    /// question with an abstention carrying the failure, so a caller never has to tell "the
    /// decision model is down" from "the decision model is off" to behave correctly.
    fn decide(&self, state: &Value, questions: &[(&str, Question)]) -> Verdict {
        let started = Instant::now();
        let answers = match self.ask(state, questions) {
            Ok(answers) => answers,
            Err(e) => {
                let reason = format!("{e:#}");
                questions.iter().map(|(id, _)| ((*id).to_string(), Answer::Abstain { reason: reason.clone() })).collect()
            }
        };
        Verdict { answers, by: self.info(), latency_ms: started.elapsed().as_millis() as u64 }
    }
}
