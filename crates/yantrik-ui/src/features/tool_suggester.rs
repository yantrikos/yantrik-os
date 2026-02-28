//! Tool Suggester — contextual tool suggestions via Whisper Cards.
//!
//! Monitors system state and suggests relevant tools:
//! - Low battery → dim screen
//! - WiFi disconnected → scan for networks
//! - Archive downloaded → extract it
//! - Idle with terminal → commit your changes
//! - High CPU → check processes
//! - High memory → free memory
//! - Low disk → find large files

use std::collections::HashMap;
use std::time::Instant;

use yantrik_os::SystemEvent;

use super::{FeatureContext, ProactiveFeature, Urge, UrgeCategory};

pub struct ToolSuggester {
    cooldowns: HashMap<String, Instant>,
    cooldown_secs: u64,
    counter: u64,
    /// Track previous network state to detect disconnects.
    was_connected: Option<bool>,
}

impl ToolSuggester {
    pub fn new() -> Self {
        Self {
            cooldowns: HashMap::new(),
            cooldown_secs: 600, // 10 min between same suggestions
            counter: 0,
            was_connected: None,
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

    fn next_id(&mut self, prefix: &str) -> String {
        self.counter += 1;
        format!("ts:{}-{}", prefix, self.counter)
    }
}

impl ProactiveFeature for ToolSuggester {
    fn name(&self) -> &str {
        "tool_suggester"
    }

    fn on_event(&mut self, event: &SystemEvent, _ctx: &FeatureContext) -> Vec<Urge> {
        let mut urges = Vec::new();

        match event {
            // Low battery (not charging) → suggest dimming screen
            SystemEvent::BatteryChanged { level, charging, .. } if !charging && *level <= 15 => {
                if self.should_fire("battery_dim") {
                    urges.push(Urge {
                        id: self.next_id("battery-dim"),
                        source: "tool_suggester".into(),
                        title: "Dim the screen?".into(),
                        body: format!(
                            "Battery at {}%. Dimming the display saves power. Ask me to lower brightness.",
                            level
                        ),
                        urgency: 0.55,
                        confidence: 0.85,
                        category: UrgeCategory::Resource,
                    });
                }
            }

            // Network just disconnected → suggest scanning
            SystemEvent::NetworkChanged { connected, .. } => {
                let was = self.was_connected.unwrap_or(true);
                self.was_connected = Some(*connected);

                if was && !connected && self.should_fire("wifi_scan") {
                    urges.push(Urge {
                        id: self.next_id("wifi-scan"),
                        source: "tool_suggester".into(),
                        title: "Scan for networks?".into(),
                        body: "WiFi disconnected. I can scan for available networks and reconnect.".into(),
                        urgency: 0.5,
                        confidence: 0.9,
                        category: UrgeCategory::Resource,
                    });
                }
            }

            // File created in Downloads → check if it's extractable or openable
            SystemEvent::FileChanged { path, kind: yantrik_os::FileChangeKind::Created } => {
                let lower = path.to_lowercase();
                if path.contains("ownload") || path.contains("tmp") {
                    let filename = path.rsplit('/').next().unwrap_or(path);
                    if is_archive(&lower) && self.should_fire("extract_archive") {
                        urges.push(Urge {
                            id: self.next_id("extract"),
                            source: "tool_suggester".into(),
                            title: format!("Extract {}?", filename),
                            body: format!("New archive: {}. Want me to extract it?", filename),
                            urgency: 0.4,
                            confidence: 0.85,
                            category: UrgeCategory::FileManagement,
                        });
                    } else if is_media(&lower) && self.should_fire("open_media") {
                        urges.push(Urge {
                            id: self.next_id("open-media"),
                            source: "tool_suggester".into(),
                            title: format!("Open {}?", filename),
                            body: format!("New file: {}. Want me to open it?", filename),
                            urgency: 0.3,
                            confidence: 0.7,
                            category: UrgeCategory::FileManagement,
                        });
                    }
                }
            }

            // High CPU → suggest checking processes
            SystemEvent::CpuPressure { usage_percent } if *usage_percent > 85.0 => {
                if self.should_fire("cpu_process") {
                    urges.push(Urge {
                        id: self.next_id("cpu-proc"),
                        source: "tool_suggester".into(),
                        title: "Check what's using CPU?".into(),
                        body: format!(
                            "CPU at {:.0}%. Want me to list the top processes?",
                            usage_percent
                        ),
                        urgency: 0.45,
                        confidence: 0.8,
                        category: UrgeCategory::Resource,
                    });
                }
            }

            // High memory → suggest freeing
            SystemEvent::MemoryPressure { used_bytes, total_bytes } if *total_bytes > 0 => {
                let percent = *used_bytes as f32 / *total_bytes as f32 * 100.0;
                if percent > 80.0 && self.should_fire("memory_cleanup") {
                    urges.push(Urge {
                        id: self.next_id("mem-cleanup"),
                        source: "tool_suggester".into(),
                        title: "Free up memory?".into(),
                        body: format!(
                            "RAM at {:.0}%. I can show which apps use the most memory.",
                            percent
                        ),
                        urgency: 0.45,
                        confidence: 0.8,
                        category: UrgeCategory::Resource,
                    });
                }
            }

            // Low disk → suggest finding large files
            SystemEvent::DiskPressure { available_bytes, total_bytes, mount_point } if *total_bytes > 0 => {
                let avail_percent = *available_bytes as f32 / *total_bytes as f32 * 100.0;
                if avail_percent < 15.0 && self.should_fire("disk_cleanup") {
                    let avail_gb = *available_bytes as f64 / 1_000_000_000.0;
                    urges.push(Urge {
                        id: self.next_id("disk-cleanup"),
                        source: "tool_suggester".into(),
                        title: "Find large files?".into(),
                        body: format!(
                            "Only {:.1} GB free on {}. I can find the largest files to clean up.",
                            avail_gb, mount_point
                        ),
                        urgency: 0.5,
                        confidence: 0.85,
                        category: UrgeCategory::FileManagement,
                    });
                }
            }

            _ => {}
        }

        urges
    }

    fn on_tick(&mut self, ctx: &FeatureContext) -> Vec<Urge> {
        let mut urges = Vec::new();

        // Suggest git commit if terminal is running and user has been mildly idle (2-5 min)
        if ctx.system.idle_seconds >= 120 && ctx.system.idle_seconds <= 300 {
            let has_terminal = ctx
                .system
                .running_processes
                .iter()
                .any(|p| is_terminal_process(&p.name));

            if has_terminal && self.should_fire("git_commit") {
                urges.push(Urge {
                    id: self.next_id("git-commit"),
                    source: "tool_suggester".into(),
                    title: "Commit your changes?".into(),
                    body: "You've been idle with a terminal open. Good time to commit?".into(),
                    urgency: 0.35,
                    confidence: 0.6,
                    category: UrgeCategory::Focus,
                });
            }
        }

        // Clean expired cooldowns
        let now = Instant::now();
        self.cooldowns
            .retain(|_, t| now.duration_since(*t).as_secs() < self.cooldown_secs * 2);

        urges
    }
}

/// Check if a file path looks like an extractable archive.
fn is_archive(path: &str) -> bool {
    path.ends_with(".zip")
        || path.ends_with(".tar.gz")
        || path.ends_with(".tar.bz2")
        || path.ends_with(".tar.xz")
        || path.ends_with(".tgz")
        || path.ends_with(".7z")
        || path.ends_with(".rar")
        || path.ends_with(".deb")
        || path.ends_with(".apk")
}

/// Check if a file path looks like a viewable media file.
fn is_media(path: &str) -> bool {
    path.ends_with(".pdf")
        || path.ends_with(".png")
        || path.ends_with(".jpg")
        || path.ends_with(".jpeg")
        || path.ends_with(".mp4")
        || path.ends_with(".mkv")
}

/// Check if a process name looks like a terminal emulator or shell.
fn is_terminal_process(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("foot")
        || lower.contains("alacritty")
        || lower.contains("kitty")
        || lower.contains("xterm")
        || lower.contains("terminal")
        || lower.contains("konsole")
}
