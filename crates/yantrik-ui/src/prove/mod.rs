//! The companion proves itself on request (#1291, first slice): the three proofs the first run
//! makes ("Can it answer?", "Can it remember?", "Can it reach the desktop?"), askable again at
//! any time, with the result kept.
//!
//! The proofs are the first run's own, by the same functions, in the same order: the answer proof
//! (`wire::onboarding::ask_the_mind`) and the `Proofs` trait's `remember` and `desktop`. Nothing
//! here asks a second way.
//!
//! What a report holds is decided here, once. A proof's `details` quote the model and describe the
//! desktop, so no report carries them: only the state, the first run's words for it, the time it
//! took and, for a failure, the reason, with anything a model said cut away (`reason_of`). The
//! kept file, the action's reply and `describe` are all built from the one [`Report`].

mod gate;
mod keep;

pub use gate::{Gate, ShellGate};
pub use keep::Prover;

use serde::{Deserialize, Serialize};

use crate::wire::onboarding::{CheckKind, CheckState, Proof, Proofs};

/// The longest reason kept: a sentence, not a log.
const REASON_CHARS: usize = 160;

/// One proof, as kept.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    pub id: String,
    /// The first run's label: "Can it answer?".
    pub label: String,
    /// `passed` or `attention`, as the first run says it.
    pub state: String,
    /// What was proven, or "Needs attention: <reason>".
    pub words: String,
    pub ms: u64,
}

/// What the three proofs found, and when.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    /// Unix seconds.
    pub at: i64,
    /// The installed build, from the machine's own record (`yantrik_version`).
    pub build: String,
    /// The mind that was proven: "Yantrik Companion".
    pub mind: String,
    pub lines: Vec<Line>,
}

/// What passing says for each proof: fixed words, never what the proof saw.
fn proven(kind: CheckKind) -> &'static str {
    match kind {
        CheckKind::Answer => "asked to use a tool; the answer was right",
        CheckKind::Remember => "wrote a note, read it back, removed it",
        CheckKind::Desktop => "described the desktop",
    }
}

/// The reason a proof gave, cut at the first `(`, `\u{201c}` or `"`, then capped. What follows one
/// of them is what something else said: the answer proof's reason quotes up to 80 characters of
/// the model ("it answered, but not the question (\u{201c}...\u{201d})"), the memory proof's puts
/// the store's error in brackets, a provider's refusal can be quoted. A report holds none of it,
/// only the sentence before.
fn reason_of(reason: &str) -> String {
    let said = reason.split(['(', '\u{201c}', '"']).next().unwrap_or_default();
    said.trim().chars().take(REASON_CHARS).collect()
}

/// Run the proofs for the first mind, in the order its checks are listed. `ask` is the answer
/// proof (`wire::onboarding::ask_the_mind`), passed in so a test can stand in for the model.
pub fn run(proofs: &dyn Proofs, ask: &dyn Fn(&str) -> Proof, build: &str, now: i64) -> Report {
    let (id, mind) = proofs.minds().into_iter().next().unwrap_or_default();
    let mut lines = Vec::new();
    for kind in proofs.checks(&id) {
        let started = std::time::Instant::now();
        let proof = match kind {
            CheckKind::Answer => ask(&id),
            CheckKind::Remember => proofs.remember(&id),
            CheckKind::Desktop => proofs.desktop(&id),
        };
        let ms = started.elapsed().as_millis() as u64;
        let kept = match proof {
            Proof::Passed { .. } => Proof::Passed { details: String::new() },
            Proof::Attention { reason, .. } => Proof::Attention { reason: reason_of(&reason), details: String::new() },
        };
        let state = CheckState::Done(kept);
        let words = if state.state() == "passed" { proven(kind).to_string() } else { state.words() };
        lines.push(Line { id: kind.id().into(), label: kind.label().into(), state: state.state().into(), words, ms });
    }
    Report { at: now, build: build.to_string(), mind, lines }
}

impl Report {
    pub fn passed(&self) -> usize {
        self.lines.iter().filter(|l| l.state == "passed").count()
    }

    /// "03:12", the local clock.
    fn clock(&self) -> String {
        chrono::DateTime::from_timestamp(self.at, 0)
            .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
            .unwrap_or_default()
    }

    /// The report as a person reads it.
    pub fn text(&self) -> String {
        let mut out = format!("What works on this computer right now        proven {}, build {}\n{}\n", self.clock(), self.build, self.mind);
        for l in &self.lines {
            out.push_str(&format!("  {:<10} {:<26} {:>6}   {}\n", l.state, l.label, format!("{:.1} s", l.ms as f64 / 1000.0), l.words));
        }
        out.push_str(&format!("{} of {} proven.", self.passed(), self.lines.len()));
        out
    }

    /// What `describe shell` shows under `proofs`: when, how old, how many, and the lines.
    pub fn describe(&self, now: i64) -> serde_json::Value {
        let lines: Vec<_> = self.lines.iter().map(|l| serde_json::json!({ "id": l.id, "label": l.label, "state": l.state, "words": l.words })).collect();
        serde_json::json!({
            "at": chrono::DateTime::from_timestamp(self.at, 0).map(|t| t.with_timezone(&chrono::Local).to_rfc3339()),
            "age_secs": (now - self.at).max(0),
            "build": self.build,
            "passed": self.passed(),
            "of": self.lines.len(),
            "lines": lines,
        })
    }
}

#[cfg(test)]
mod tests;
