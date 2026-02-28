//! Process Sentinel — flags unknown/suspicious processes.
//!
//! Tracks which processes the user has seen before. First-seen processes
//! with high CPU get flagged. User dismissals build a trust list.

use std::collections::HashSet;
use std::time::Instant;

use yantrik_os::SystemEvent;

use super::{FeatureContext, Outcome, ProactiveFeature, Urge, UrgeCategory};

pub struct ProcessSentinel {
    /// Processes the user has seen and approved.
    trusted: HashSet<String>,
    /// Processes seen at least once (don't flag known system processes).
    seen: HashSet<String>,
    /// Recently flagged (process name → time), to prevent spam.
    flagged: std::collections::HashMap<String, Instant>,
    /// CPU threshold for flagging a new process.
    cpu_alert_threshold: f32,
    /// Cooldown before re-flagging the same process name.
    cooldown_secs: u64,
}

impl ProcessSentinel {
    pub fn new() -> Self {
        let mut seen = HashSet::new();
        // Pre-trust common system processes
        for name in &[
            "init", "syslogd", "crond", "sshd", "dbus-daemon",
            "seatd", "labwc", "pipewire", "wireplumber",
            "yantrik-ui", "Xwayland", "foot", "bash", "sh",
        ] {
            seen.insert(name.to_string());
        }

        Self {
            trusted: HashSet::new(),
            seen,
            flagged: std::collections::HashMap::new(),
            cpu_alert_threshold: 50.0,
            cooldown_secs: 600, // 10 minutes
        }
    }
}

impl ProactiveFeature for ProcessSentinel {
    fn name(&self) -> &str {
        "process_sentinel"
    }

    fn on_event(&mut self, event: &SystemEvent, _ctx: &FeatureContext) -> Vec<Urge> {
        let mut urges = Vec::new();

        if let SystemEvent::ProcessStarted { name, pid, cpu_percent } = event {
            // Skip if trusted
            if self.trusted.contains(name) {
                return urges;
            }

            // First time seeing this process?
            let is_new = !self.seen.contains(name);
            self.seen.insert(name.clone());

            if is_new && *cpu_percent >= self.cpu_alert_threshold {
                // Check cooldown
                let now = Instant::now();
                if let Some(last) = self.flagged.get(name) {
                    if now.duration_since(*last).as_secs() < self.cooldown_secs {
                        return urges;
                    }
                }
                self.flagged.insert(name.clone(), now);

                urges.push(Urge {
                    id: format!("ps:new_process:{}:{}", name, pid),
                    source: "process_sentinel".into(),
                    title: "New process detected".into(),
                    body: format!(
                        "\"{}\" (PID {}) started using {:.0}% CPU. First time seeing this.",
                        name, pid, cpu_percent
                    ),
                    urgency: 0.6,
                    confidence: 0.7,
                    category: UrgeCategory::Security,
                });
            } else if is_new {
                // New but low CPU — just note it, no urge
                tracing::debug!(name, pid, cpu = cpu_percent, "New process seen (low CPU, no alert)");
            }
        }

        urges
    }

    fn on_tick(&mut self, _ctx: &FeatureContext) -> Vec<Urge> {
        Vec::new()
    }

    fn on_feedback(&mut self, _urge_id: &str, outcome: Outcome) {
        // If user dismissed, trust this process in the future
        if let Outcome::Dismissed = outcome {
            // Extract process name from urge_id: "ps:new_process:NAME:PID"
            let parts: Vec<&str> = _urge_id.split(':').collect();
            if parts.len() >= 3 {
                let name = parts[2].to_string();
                tracing::info!(process = %name, "Process trusted by user");
                self.trusted.insert(name);
            }
        }
    }
}
