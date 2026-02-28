//! Notification Relay — converts D-Bus notifications into Whisper Card urges.
//!
//! Every NotificationReceived system event becomes an Urge, scored by the
//! D-Bus urgency hint: low→Queue, normal→Whisper, critical→Interrupt.

use yantrik_os::SystemEvent;

use super::{FeatureContext, Outcome, ProactiveFeature, Urge, UrgeCategory};

/// Relays app notifications as Whisper Card urges.
pub struct NotificationRelay {
    /// Counter for unique urge IDs.
    counter: u64,
}

impl NotificationRelay {
    pub fn new() -> Self {
        Self { counter: 0 }
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

        self.counter += 1;

        // Map D-Bus urgency: 0=low, 1=normal, 2=critical
        let (raw_urgency, confidence) = match urgency {
            0 => (0.35, 0.8),
            2 => (0.95, 1.0),
            _ => (0.65, 0.9), // normal
        };

        let body_text = if body.is_empty() {
            summary.clone()
        } else {
            format!("{}: {}", summary, body)
        };

        vec![Urge {
            id: format!("notif-{}-{}", app.to_lowercase().replace(' ', "-"), self.counter),
            source: "NotificationRelay".into(),
            title: format!("{} — {}", app, summary),
            body: body_text,
            urgency: raw_urgency,
            confidence,
            category: UrgeCategory::Notification,
        }]
    }

    fn on_tick(&mut self, _ctx: &FeatureContext) -> Vec<Urge> {
        Vec::new()
    }

    fn on_feedback(&mut self, _urge_id: &str, _outcome: Outcome) {
        // Could learn which apps the user dismisses to lower urgency, but not yet.
    }
}
