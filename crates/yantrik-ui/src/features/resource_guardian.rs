//! Resource Guardian — monitors battery, CPU, RAM, disk.
//!
//! Fires urges when resources drop below thresholds.
//! Also celebrates recovery ("You freed 3GB — nice!").

use std::collections::HashMap;
use std::time::Instant;

use yantrik_os::SystemEvent;

use super::{FeatureContext, Outcome, ProactiveFeature, Urge, UrgeCategory};

pub struct ResourceGuardian {
    /// Cooldown tracker: key → last fire time.
    cooldowns: HashMap<String, Instant>,
    /// Cooldown duration per key.
    cooldown_secs: u64,

    // Previous values for delta detection
    prev_battery: Option<u8>,
    prev_disk_available: Option<u64>,

    // Thresholds
    battery_warning: u8,
    battery_critical: u8,
    cpu_sustained_threshold: f32,
    cpu_sustained_ticks: u32,
    memory_warning_percent: f32,
    disk_warning_percent: f32,

    // Sustained CPU tracking
    cpu_high_count: u32,
}

impl ResourceGuardian {
    pub fn new() -> Self {
        Self {
            cooldowns: HashMap::new(),
            cooldown_secs: 300, // 5 minutes between same-type urges
            prev_battery: None,
            prev_disk_available: None,
            battery_warning: 20,
            battery_critical: 10,
            cpu_sustained_threshold: 90.0,
            cpu_sustained_ticks: 6, // 6 × 10s = 60s sustained
            memory_warning_percent: 85.0,
            disk_warning_percent: 10.0,
            cpu_high_count: 0,
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

impl ProactiveFeature for ResourceGuardian {
    fn name(&self) -> &str {
        "resource_guardian"
    }

    fn on_event(&mut self, event: &SystemEvent, ctx: &FeatureContext) -> Vec<Urge> {
        let mut urges = Vec::new();

        match event {
            SystemEvent::BatteryChanged { level, charging, time_to_empty_mins } => {
                if !charging {
                    if *level <= self.battery_critical && self.should_fire("battery_critical") {
                        let body = match time_to_empty_mins {
                            Some(mins) => format!(
                                "Battery at {}% \u{2014} about {} minutes left. Plug in soon.",
                                level, mins
                            ),
                            None => format!("Battery at {}%. Plug in now.", level),
                        };
                        urges.push(Urge {
                            id: format!("rg:battery_critical:{}", level),
                            source: "resource_guardian".into(),
                            title: "Battery critical".into(),
                            body,
                            urgency: 0.95,
                            confidence: 1.0,
                            category: UrgeCategory::Resource,
                        });
                    } else if *level <= self.battery_warning && self.should_fire("battery_warning") {
                        let body = match time_to_empty_mins {
                            Some(mins) => format!(
                                "Battery at {}% \u{2014} roughly {} minutes remaining.",
                                level, mins
                            ),
                            None => format!("Battery at {}%. Consider plugging in.", level),
                        };
                        urges.push(Urge {
                            id: format!("rg:battery_warning:{}", level),
                            source: "resource_guardian".into(),
                            title: "Battery getting low".into(),
                            body,
                            urgency: 0.7,
                            confidence: 1.0,
                            category: UrgeCategory::Resource,
                        });
                    }
                }

                // Celebrate: battery was critical, now charging
                if *charging {
                    if let Some(prev) = self.prev_battery {
                        if prev <= self.battery_critical && self.should_fire("battery_recovery") {
                            urges.push(Urge {
                                id: "rg:battery_recovery".into(),
                                source: "resource_guardian".into(),
                                title: "Charging".into(),
                                body: "Plugged in \u{2014} crisis averted.".into(),
                                urgency: 0.3,
                                confidence: 1.0,
                                category: UrgeCategory::Celebration,
                            });
                        }
                    }
                }

                self.prev_battery = Some(*level);
            }

            SystemEvent::CpuPressure { usage_percent } => {
                if *usage_percent >= self.cpu_sustained_threshold {
                    self.cpu_high_count += 1;
                    if self.cpu_high_count >= self.cpu_sustained_ticks
                        && self.should_fire("cpu_sustained")
                    {
                        // V15: Action-first — identify the top CPU consumer
                        let top = top_cpu_process(ctx);
                        let body = if let Some((name, cpu)) = top {
                            format!(
                                "CPU at {:.0}% for over a minute. Top consumer: {} ({:.0}%).",
                                usage_percent, name, cpu
                            )
                        } else {
                            format!(
                                "CPU at {:.0}% for over a minute. Something might be stuck.",
                                usage_percent
                            )
                        };
                        urges.push(Urge {
                            id: format!("rg:cpu_sustained:{:.0}", usage_percent),
                            source: "resource_guardian".into(),
                            title: "CPU running hot".into(),
                            body,
                            urgency: 0.6,
                            confidence: 0.8,
                            category: UrgeCategory::Resource,
                        });
                        self.cpu_high_count = 0;
                    }
                } else {
                    self.cpu_high_count = 0;
                }
            }

            SystemEvent::MemoryPressure { used_bytes, total_bytes } => {
                if *total_bytes > 0 {
                    let percent = *used_bytes as f32 / *total_bytes as f32 * 100.0;
                    if percent >= self.memory_warning_percent
                        && self.should_fire("memory_warning")
                    {
                        let used_mb = *used_bytes / 1_000_000;
                        let total_mb = *total_bytes / 1_000_000;
                        // V15: Action-first — identify top memory consumer
                        let top = top_cpu_process(ctx);
                        let top_note = top.map(|(name, _)| format!(" Top process: {}.", name)).unwrap_or_default();
                        urges.push(Urge {
                            id: format!("rg:memory:{:.0}", percent),
                            source: "resource_guardian".into(),
                            title: "Memory pressure".into(),
                            body: format!(
                                "RAM at {:.0}% ({} / {} MB).{}",
                                percent, used_mb, total_mb, top_note
                            ),
                            urgency: 0.65,
                            confidence: 0.9,
                            category: UrgeCategory::Resource,
                        });
                    }
                }
            }

            SystemEvent::DiskPressure { mount_point, available_bytes, total_bytes } => {
                if *total_bytes > 0 {
                    let avail_percent = *available_bytes as f32 / *total_bytes as f32 * 100.0;
                    if avail_percent <= self.disk_warning_percent
                        && self.should_fire("disk_warning")
                    {
                        let avail_gb = *available_bytes as f64 / 1_000_000_000.0;
                        urges.push(Urge {
                            id: format!("rg:disk:{}", mount_point),
                            source: "resource_guardian".into(),
                            title: "Disk almost full".into(),
                            body: format!(
                                "Only {:.1} GB free on {}. Consider cleaning up.",
                                avail_gb, mount_point
                            ),
                            urgency: 0.75,
                            confidence: 1.0,
                            category: UrgeCategory::Resource,
                        });
                    }

                    // Celebrate: disk space recovered
                    if let Some(prev_avail) = self.prev_disk_available {
                        let recovered = available_bytes.saturating_sub(prev_avail);
                        let recovered_gb = recovered as f64 / 1_000_000_000.0;
                        if recovered_gb >= 1.0 && self.should_fire("disk_recovery") {
                            urges.push(Urge {
                                id: "rg:disk_recovery".into(),
                                source: "resource_guardian".into(),
                                title: "Space freed".into(),
                                body: format!("You freed {:.1} GB \u{2014} nice!", recovered_gb),
                                urgency: 0.25,
                                confidence: 1.0,
                                category: UrgeCategory::Celebration,
                            });
                        }
                    }
                    self.prev_disk_available = Some(*available_bytes);
                }
            }

            _ => {}
        }

        urges
    }

    fn on_tick(&mut self, _ctx: &FeatureContext) -> Vec<Urge> {
        // Resource Guardian is event-driven, not tick-driven.
        Vec::new()
    }

    fn on_feedback(&mut self, _urge_id: &str, outcome: Outcome) {
        // If user keeps dismissing resource warnings, we could increase
        // the cooldown. For now, just log it.
        match outcome {
            Outcome::Dismissed => {
                tracing::debug!("Resource warning dismissed — could adjust thresholds");
            }
            _ => {}
        }
    }
}

/// Find the top CPU-consuming process from the system snapshot.
fn top_cpu_process(ctx: &FeatureContext) -> Option<(String, f32)> {
    ctx.system
        .running_processes
        .iter()
        .max_by(|a, b| a.cpu_percent.partial_cmp(&b.cpu_percent).unwrap_or(std::cmp::Ordering::Equal))
        .filter(|p| p.cpu_percent > 5.0) // Only report if it's actually consuming significant CPU
        .map(|p| (p.name.clone(), p.cpu_percent))
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_os::SystemSnapshot;

    fn test_ctx() -> FeatureContext<'static> {
        // Leak a snapshot for test lifetime — fine in tests
        let snapshot = Box::leak(Box::new(SystemSnapshot::default()));
        FeatureContext {
            system: snapshot,
            clock: std::time::SystemTime::now(),
            bond_level: 1,
        }
    }

    #[test]
    fn battery_below_warning_fires_urge() {
        let mut guardian = ResourceGuardian::new();
        let ctx = test_ctx();

        let event = SystemEvent::BatteryChanged {
            level: 15,
            charging: false,
            time_to_empty_mins: Some(45),
        };

        let urges = guardian.on_event(&event, &ctx);
        assert_eq!(urges.len(), 1);
        assert!(urges[0].urgency >= 0.7);
        assert!(urges[0].body.contains("15%"));
    }

    #[test]
    fn battery_critical_fires_high_urgency() {
        let mut guardian = ResourceGuardian::new();
        let ctx = test_ctx();

        let event = SystemEvent::BatteryChanged {
            level: 5,
            charging: false,
            time_to_empty_mins: Some(10),
        };

        let urges = guardian.on_event(&event, &ctx);
        assert_eq!(urges.len(), 1);
        assert!(urges[0].urgency >= 0.9);
    }

    #[test]
    fn battery_charging_no_urge() {
        let mut guardian = ResourceGuardian::new();
        let ctx = test_ctx();

        let event = SystemEvent::BatteryChanged {
            level: 15,
            charging: true,
            time_to_empty_mins: None,
        };

        let urges = guardian.on_event(&event, &ctx);
        assert!(urges.is_empty());
    }

    #[test]
    fn cooldown_prevents_duplicate_urges() {
        let mut guardian = ResourceGuardian::new();
        let ctx = test_ctx();

        let event = SystemEvent::BatteryChanged {
            level: 15,
            charging: false,
            time_to_empty_mins: None,
        };

        let urges1 = guardian.on_event(&event, &ctx);
        assert_eq!(urges1.len(), 1);

        // Second identical event within cooldown — should not fire
        let urges2 = guardian.on_event(&event, &ctx);
        assert!(urges2.is_empty());
    }

    #[test]
    fn memory_pressure_fires_urge() {
        let mut guardian = ResourceGuardian::new();
        let ctx = test_ctx();

        let event = SystemEvent::MemoryPressure {
            used_bytes: 900_000_000,
            total_bytes: 1_000_000_000, // 90% used
        };

        let urges = guardian.on_event(&event, &ctx);
        assert_eq!(urges.len(), 1);
        assert!(urges[0].body.contains("90%"));
    }

    #[test]
    fn disk_recovery_celebration() {
        let mut guardian = ResourceGuardian::new();
        let ctx = test_ctx();

        // First: low disk
        let event1 = SystemEvent::DiskPressure {
            mount_point: "/".into(),
            available_bytes: 500_000_000, // 500MB of 10GB = 5%
            total_bytes: 10_000_000_000,
        };
        let _ = guardian.on_event(&event1, &ctx);

        // Then: disk freed
        let event2 = SystemEvent::DiskPressure {
            mount_point: "/".into(),
            available_bytes: 3_000_000_000, // 3GB free now
            total_bytes: 10_000_000_000,
        };
        let urges = guardian.on_event(&event2, &ctx);

        // Should have a celebration urge (2.5GB freed)
        let celebration = urges.iter().find(|u| u.category == UrgeCategory::Celebration);
        assert!(celebration.is_some());
    }
}
