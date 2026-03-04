//! Notification Relay — converts D-Bus notifications into Whisper Card urges.
//!
//! Every NotificationReceived system event becomes an Urge, scored by the
//! D-Bus urgency hint × per-app learned score. Notifications from the same
//! app are grouped when 3+ arrive within 60 seconds.

use std::collections::HashMap;
use std::time::Instant;

use yantrik_os::SystemEvent;

use super::{FeatureContext, Outcome, ProactiveFeature, Urge, UrgeCategory};

/// Relays app notifications as Whisper Card urges, with learned per-app
/// urgency scores and batching of spammy apps.
pub struct NotificationRelay {
    /// Counter for unique urge IDs.
    counter: u64,
    /// Per-app urgency multiplier (default 1.0, range 0.1–1.5).
    app_scores: HashMap<String, f32>,
    /// Pending notifications per app for grouping.
    /// Key: app name (lowercase). Value: (summaries, first_seen).
    pending: HashMap<String, (Vec<String>, Instant)>,
}

/// How long to buffer notifications before grouping (seconds).
const GROUP_WINDOW_SECS: u64 = 60;
/// Minimum notifications from the same app to trigger grouping.
const GROUP_THRESHOLD: usize = 3;

/// Get the learned urgency multiplier for an app.
fn app_score(scores: &HashMap<String, f32>, app: &str) -> f32 {
    scores.get(&app.to_lowercase()).copied().unwrap_or(1.0)
}

/// Build a single urge from one notification.
fn make_single_urge(
    counter: &mut u64,
    scores: &HashMap<String, f32>,
    app: &str,
    summary: &str,
    body: &str,
    raw_urgency: f32,
    confidence: f32,
) -> Urge {
    *counter += 1;
    let app_key = app.to_lowercase().replace(' ', "-");

    let body_text = if body.is_empty() {
        summary.to_string()
    } else {
        format!("{}: {}", summary, body)
    };

    let adjusted = (raw_urgency * app_score(scores, app)).clamp(0.1, 0.95);

    Urge {
        id: format!("notif-{}-{}", app_key, counter),
        source: "NotificationRelay".into(),
        title: format!("{} — {}", app, summary),
        body: body_text,
        urgency: adjusted,
        confidence,
        category: UrgeCategory::Notification,
    }
}

/// Build a grouped urge for multiple notifications from the same app.
fn make_grouped_urge(
    counter: &mut u64,
    scores: &HashMap<String, f32>,
    app: &str,
    summaries: &[String],
    raw_urgency: f32,
    confidence: f32,
) -> Urge {
    *counter += 1;
    let app_key = app.to_lowercase().replace(' ', "-");
    let count = summaries.len();

    let body_text = if count <= 4 {
        summaries.join(" · ")
    } else {
        let preview: Vec<&str> = summaries.iter().take(3).map(|s| s.as_str()).collect();
        format!("{} (+{} more)", preview.join(" · "), count - 3)
    };

    let adjusted = (raw_urgency * app_score(scores, app)).clamp(0.1, 0.95);

    Urge {
        id: format!("notif-{}-batch-{}", app_key, counter),
        source: "NotificationRelay".into(),
        title: format!("{} notifications from {}", count, app),
        body: body_text,
        urgency: adjusted,
        confidence,
        category: UrgeCategory::Notification,
    }
}

impl NotificationRelay {
    pub fn new() -> Self {
        Self {
            counter: 0,
            app_scores: HashMap::new(),
            pending: HashMap::new(),
        }
    }
}

impl ProactiveFeature for NotificationRelay {
    fn name(&self) -> &str {
        "NotificationRelay"
    }

    fn on_event(&mut self, event: &SystemEvent, _ctx: &FeatureContext) -> Vec<Urge> {
        let SystemEvent::NotificationReceived {
            app,
            summary,
            body,
            urgency,
        } = event
        else {
            return Vec::new();
        };

        // Map D-Bus urgency: 0=low, 1=normal, 2=critical
        let (raw_urgency, confidence) = match urgency {
            0 => (0.35, 0.8),
            2 => (0.95, 1.0),
            _ => (0.65, 0.9),
        };

        // Critical notifications bypass grouping
        if *urgency == 2 {
            return vec![make_single_urge(
                &mut self.counter, &self.app_scores, app, summary, body, raw_urgency, confidence,
            )];
        }

        // Buffer for grouping
        let app_key = app.to_lowercase();
        let now = Instant::now();
        let mut urges = Vec::new();

        // Check if there's an existing entry with expired window
        let flush_batch = if let Some(entry) = self.pending.get(&app_key) {
            now.duration_since(entry.1).as_secs() >= GROUP_WINDOW_SECS
        } else {
            false
        };

        if flush_batch {
            if let Some((old_summaries, _)) = self.pending.remove(&app_key) {
                if old_summaries.len() >= GROUP_THRESHOLD {
                    urges.push(make_grouped_urge(
                        &mut self.counter, &self.app_scores, app, &old_summaries, raw_urgency, confidence,
                    ));
                } else {
                    for s in &old_summaries {
                        urges.push(make_single_urge(
                            &mut self.counter, &self.app_scores, app, s, "", raw_urgency, confidence,
                        ));
                    }
                }
            }
        }

        // Add to pending buffer
        let entry = self.pending.entry(app_key.clone()).or_insert_with(|| (Vec::new(), now));
        entry.0.push(summary.clone());

        // If we've hit the threshold, emit grouped immediately
        if entry.0.len() >= GROUP_THRESHOLD {
            let summaries = std::mem::take(&mut entry.0);
            self.pending.remove(&app_key);
            urges.push(make_grouped_urge(
                &mut self.counter, &self.app_scores, app, &summaries, raw_urgency, confidence,
            ));
        }

        urges
    }

    fn on_tick(&mut self, _ctx: &FeatureContext) -> Vec<Urge> {
        // Flush any pending groups that have exceeded the time window
        let now = Instant::now();
        let expired_keys: Vec<String> = self.pending.iter()
            .filter(|(_, (_, first_seen))| now.duration_since(*first_seen).as_secs() >= GROUP_WINDOW_SECS)
            .map(|(k, _)| k.clone())
            .collect();

        let mut urges = Vec::new();
        for key in expired_keys {
            if let Some((summaries, _)) = self.pending.remove(&key) {
                if summaries.len() >= GROUP_THRESHOLD {
                    urges.push(make_grouped_urge(
                        &mut self.counter, &self.app_scores, &key, &summaries, 0.65, 0.9,
                    ));
                } else {
                    for s in &summaries {
                        urges.push(make_single_urge(
                            &mut self.counter, &self.app_scores, &key, s, "", 0.65, 0.9,
                        ));
                    }
                }
            }
        }

        urges
    }

    fn on_feedback(&mut self, urge_id: &str, outcome: Outcome) {
        // Extract app name from urge ID: "notif-{app}-{counter}" or "notif-{app}-batch-{counter}"
        let app = urge_id
            .strip_prefix("notif-")
            .and_then(|rest| {
                if let Some(pos) = rest.rfind("-batch-") {
                    Some(&rest[..pos])
                } else {
                    rest.rfind('-').map(|pos| &rest[..pos])
                }
            });

        let Some(app) = app else { return };
        let app = app.to_string();

        let score = self.app_scores.entry(app).or_insert(1.0);
        match outcome {
            Outcome::Acted => {
                *score = (*score * 1.2).min(1.5);
            }
            Outcome::Dismissed => {
                *score = (*score * 0.85).max(0.1);
            }
            Outcome::Expired => {}
        }
    }
}
