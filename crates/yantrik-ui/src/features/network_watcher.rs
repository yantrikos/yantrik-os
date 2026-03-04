//! Network Watcher — narrates connectivity changes.
//!
//! Emits urges when WiFi disconnects, reconnects, or signal drops.
//! Suggests `network_diagnose` tool for troubleshooting.

use std::collections::HashMap;
use std::time::Instant;

use yantrik_os::SystemEvent;

use super::{FeatureContext, Outcome, ProactiveFeature, Urge, UrgeCategory};

pub struct NetworkWatcher {
    /// Was connected last time we checked?
    was_connected: Option<bool>,
    /// Last known SSID.
    last_ssid: Option<String>,
    /// Cooldown tracker.
    cooldowns: HashMap<String, Instant>,
    cooldown_secs: u64,
    /// Tick counter for periodic signal checks.
    tick_count: u64,
}

impl NetworkWatcher {
    pub fn new() -> Self {
        Self {
            was_connected: None,
            last_ssid: None,
            cooldowns: HashMap::new(),
            cooldown_secs: 300,
            tick_count: 0,
        }
    }

    fn should_fire(&mut self, key: &str) -> bool {
        let now = Instant::now();
        if let Some(last) = self.cooldowns.get(key) {
            if now.duration_since(*last).as_secs() < self.cooldown_secs {
                return false;
            }
        }
        self.cooldowns.insert(key.to_string(), now);
        true
    }
}

impl ProactiveFeature for NetworkWatcher {
    fn name(&self) -> &str {
        "network_watcher"
    }

    fn on_event(&mut self, event: &SystemEvent, _ctx: &FeatureContext) -> Vec<Urge> {
        let SystemEvent::NetworkChanged { connected, ssid, signal } = event else {
            return Vec::new();
        };

        let mut urges = Vec::new();

        // Detect disconnect
        if !connected {
            if self.was_connected == Some(true) && self.should_fire("net_disconnect") {
                let ssid_name = self.last_ssid.as_deref().unwrap_or("network");
                urges.push(Urge {
                    id: "nw:disconnect".into(),
                    source: "network_watcher".into(),
                    title: "Connection lost".into(),
                    body: format!(
                        "Lost connection to '{}'. Ask me to run network_diagnose.",
                        ssid_name
                    ),
                    urgency: 0.7,
                    confidence: 1.0,
                    category: UrgeCategory::Resource,
                });
            }
        }

        // Detect reconnect
        if *connected && self.was_connected == Some(false) && self.should_fire("net_reconnect") {
            let ssid_name = ssid.as_deref().unwrap_or("network");
            urges.push(Urge {
                id: "nw:reconnect".into(),
                source: "network_watcher".into(),
                title: "Back online".into(),
                body: format!("Reconnected to '{}'.", ssid_name),
                urgency: 0.35,
                confidence: 1.0,
                category: UrgeCategory::Celebration,
            });
        }

        // Weak signal warning
        if *connected {
            if let Some(sig) = signal {
                if *sig < 30 && self.should_fire("net_weak_signal") {
                    let ssid_name = ssid.as_deref().unwrap_or("WiFi");
                    urges.push(Urge {
                        id: format!("nw:weak_signal:{}", sig),
                        source: "network_watcher".into(),
                        title: "Weak signal".into(),
                        body: format!(
                            "'{}' signal at {}%. Connection may be unstable.",
                            ssid_name, sig
                        ),
                        urgency: 0.45,
                        confidence: 0.8,
                        category: UrgeCategory::Resource,
                    });
                }
            }
        }

        self.was_connected = Some(*connected);
        if let Some(s) = ssid {
            self.last_ssid = Some(s.clone());
        }

        urges
    }

    fn on_tick(&mut self, ctx: &FeatureContext) -> Vec<Urge> {
        self.tick_count += 1;

        // Every ~30 ticks (5 min at 10s intervals), check signal from snapshot
        if self.tick_count % 30 != 0 {
            return Vec::new();
        }

        if !ctx.system.network_connected {
            return Vec::new();
        }

        if let Some(sig) = ctx.system.network_signal {
            if sig < 30 && self.should_fire("net_weak_signal_tick") {
                let ssid = ctx.system.network_ssid.as_deref().unwrap_or("WiFi");
                return vec![Urge {
                    id: format!("nw:weak_signal_tick:{}", sig),
                    source: "network_watcher".into(),
                    title: "Weak WiFi".into(),
                    body: format!("'{}' signal at {}%. Move closer to router?", ssid, sig),
                    urgency: 0.4,
                    confidence: 0.7,
                    category: UrgeCategory::Resource,
                }];
            }
        }

        Vec::new()
    }

    fn on_feedback(&mut self, _urge_id: &str, _outcome: Outcome) {
        // Could adjust signal threshold if user keeps dismissing weak signal warnings
    }
}
