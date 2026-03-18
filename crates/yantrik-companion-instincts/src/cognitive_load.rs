//! Cognitive Load Monitor instinct — detects overwhelm from interaction
//! patterns (density, topic switching, negative valence, session length).

use std::sync::Mutex;
use std::collections::HashSet;

use yantrik_companion_core::bond::BondLevel;
use yantrik_companion_core::types::{CompanionState, UrgeSpec};
use crate::Instinct;

/// A single interaction sample for load analysis.
#[derive(Debug, Clone)]
struct LoadSample {
    timestamp: f64,
    topic_hash: u64,
    msg_length: usize,
    negative: bool,
}

pub struct CognitiveLoadInstinct {
    samples: Mutex<Vec<LoadSample>>,
    last_alert_ts: Mutex<f64>,
}

impl CognitiveLoadInstinct {
    pub fn new() -> Self {
        Self {
            samples: Mutex::new(Vec::new()),
            last_alert_ts: Mutex::new(0.0),
        }
    }

    fn compute_score(&self, state: &CompanionState) -> f64 {
        let samples = self.samples.lock().unwrap();
        if samples.len() < 3 {
            return 0.0;
        }

        let now = state.current_ts;
        let one_hour_ago = now - 3600.0;
        let recent: Vec<_> = samples.iter().filter(|s| s.timestamp > one_hour_ago).collect();

        if recent.is_empty() {
            return 0.0;
        }

        // Factor 1: Interaction density (>15/hour = max) — weight 0.3
        let density = (recent.len() as f64 / 15.0).min(1.0);

        // Factor 2: Negative valence keywords — weight 0.25
        let neg_count = recent.iter().filter(|s| s.negative).count();
        let neg_ratio = if recent.is_empty() {
            0.0
        } else {
            neg_count as f64 / recent.len() as f64
        };

        // Factor 3: Topic switching (>5 distinct in last 10) — weight 0.25
        let last_10: Vec<_> = recent.iter().rev().take(10).collect();
        let distinct_topics: HashSet<u64> = last_10.iter().map(|s| s.topic_hash).collect();
        let topic_switch = ((distinct_topics.len() as f64 - 1.0) / 4.0).clamp(0.0, 1.0);

        // Factor 4: Session length (>2h = max) — weight 0.2
        let session_hours = (now - state.last_interaction_ts).min(now - recent[0].timestamp) / 3600.0;
        // Use continuous session (first recent sample to now)
        let first_ts = recent.first().map(|s| s.timestamp).unwrap_or(now);
        let continuous = (now - first_ts) / 7200.0; // 2 hours = 1.0
        let session_factor = continuous.min(1.0);

        density * 0.3 + neg_ratio * 0.25 + topic_switch * 0.25 + session_factor * 0.2
    }
}

impl Instinct for CognitiveLoadInstinct {
    fn name(&self) -> &str {
        "cognitive_load"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Gate: Acquaintance+
        if state.bond_level < BondLevel::Acquaintance {
            return vec![];
        }

        // 30-min cooldown
        {
            let last = *self.last_alert_ts.lock().unwrap();
            if state.current_ts - last < 1800.0 {
                return vec![];
            }
        }

        let score = self.compute_score(state);
        if score < 0.5 {
            return vec![];
        }

        // Update last alert
        *self.last_alert_ts.lock().unwrap() = state.current_ts;

        let urgency = (score * 0.8).min(0.8);

        let msg = if score > 0.75 {
            "You've been going really hard. Take a break \u{2014} seriously."
        } else {
            "You've been going hard for a while. Maybe a short break?"
        };

        vec![
            UrgeSpec::new("cognitive_load", msg, urgency)
                .with_cooldown("cognitive_load:alert")
                .with_message(msg)
                .with_context(serde_json::json!({
                    "load_score": score,
                })),
        ]
    }

    fn on_interaction(&self, state: &CompanionState, user_text: &str) -> Vec<UrgeSpec> {
        // Record sample
        let topic_hash = simple_hash(user_text);
        let negative = has_negative_keywords(user_text);

        let sample = LoadSample {
            timestamp: state.current_ts,
            topic_hash,
            msg_length: user_text.len(),
            negative,
        };

        let mut samples = self.samples.lock().unwrap();
        samples.push(sample);
        // Keep max 20 samples
        while samples.len() > 20 {
            samples.remove(0);
        }

        vec![]
    }
}

/// Simple string hash for topic dedup.
fn simple_hash(text: &str) -> u64 {
    // Hash first 3 significant words as topic fingerprint
    let words: Vec<&str> = text
        .split_whitespace()
        .filter(|w| w.len() > 3)
        .take(3)
        .collect();
    let mut hash: u64 = 0;
    for w in words {
        for b in w.bytes() {
            hash = hash.wrapping_mul(31).wrapping_add(b as u64);
        }
    }
    hash
}

fn has_negative_keywords(text: &str) -> bool {
    let lower = text.to_lowercase();
    const KEYWORDS: &[&str] = &[
        "frustrated", "annoyed", "stuck", "broken", "failing", "error",
        "hate", "angry", "stressed", "overwhelmed", "confused", "lost",
        "ugh", "damn", "wtf", "shit", "fuck", "bug", "crash",
    ];
    KEYWORDS.iter().any(|kw| lower.contains(kw))
}
