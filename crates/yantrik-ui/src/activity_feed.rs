//! Activity accumulator — aggregates system events into time-bucketed digests.
//!
//! Runs entirely on the Slint main thread. Called from system_poll.rs on every
//! 3-second tick. Heavy formatting happens only in flush(), called by the hourly timer.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Instant;

use yantrik_os::{SystemEvent, SystemSnapshot};

/// A process session within one bucket.
#[derive(Debug, Clone)]
pub struct ProcessSession {
    pub name: String,
    pub runtime_secs: u64,
    pub exit_code: Option<i32>,
    started_at: Instant,
}

/// Statistics collected across one time bucket (default: 1 hour).
#[derive(Debug, Clone)]
pub struct ActivityBucket {
    pub bucket_start_ts: f64,

    // Network
    pub network_drops: u32,
    pub network_connects: u32,
    pub ssids_seen: Vec<String>,

    // Battery
    pub battery_min: u8,
    pub charging_started: bool,
    pub was_discharging: bool,

    // CPU
    pub cpu_samples: Vec<f32>,
    pub cpu_sustained_high_secs: u32,

    // Memory
    pub memory_samples: Vec<f32>,

    // Disk
    pub disk_available: HashMap<String, u64>,
    pub disk_total: HashMap<String, u64>,

    // Processes
    pub process_sessions: HashMap<String, ProcessSession>,
    running_pids: HashMap<u32, (String, Instant)>,

    // User presence
    pub user_active_secs: u64,
    pub user_idle_secs: u64,
    last_active_instant: Option<Instant>,

    // Notifications
    pub notification_count: u32,
    pub critical_notifications: u32,

    // File activity
    pub file_creates: u32,
    pub file_modifies: u32,
    pub file_deletes: u32,
}

impl Default for ActivityBucket {
    fn default() -> Self {
        Self {
            bucket_start_ts: unix_now(),
            network_drops: 0,
            network_connects: 0,
            ssids_seen: Vec::new(),
            battery_min: 100,
            charging_started: false,
            was_discharging: false,
            cpu_samples: Vec::new(),
            cpu_sustained_high_secs: 0,
            memory_samples: Vec::new(),
            disk_available: HashMap::new(),
            disk_total: HashMap::new(),
            process_sessions: HashMap::new(),
            running_pids: HashMap::new(),
            user_active_secs: 0,
            user_idle_secs: 0,
            last_active_instant: Some(Instant::now()),
            notification_count: 0,
            critical_notifications: 0,
            file_creates: 0,
            file_modifies: 0,
            file_deletes: 0,
        }
    }
}

/// A detected system issue to be stored as a persistent memory.
#[derive(Debug, Clone)]
pub struct DetectedIssue {
    pub text: String,
    pub importance: f64,
    /// Decay in seconds. 0.0 = permanent.
    pub decay: f64,
}

/// The activity accumulator — lives in AppContext, called each 3s tick.
pub struct ActivityAccumulator {
    current: ActivityBucket,
    last_context_hash: u64,
    issue_cooldowns: HashMap<String, Instant>,
}

impl ActivityAccumulator {
    pub fn new() -> Self {
        Self {
            current: ActivityBucket::default(),
            last_context_hash: 0,
            issue_cooldowns: HashMap::new(),
        }
    }

    /// Called every 3s from system_poll.rs for each event. Pure accumulation — O(1).
    pub fn ingest(&mut self, event: &SystemEvent) {
        let now = Instant::now();
        match event {
            SystemEvent::NetworkChanged { connected, ssid, .. } => {
                if *connected {
                    self.current.network_connects += 1;
                    if let Some(s) = ssid {
                        let clean: String =
                            s.chars().filter(|c| !c.is_control()).take(32).collect();
                        if !self.current.ssids_seen.contains(&clean) {
                            self.current.ssids_seen.push(clean);
                        }
                    }
                } else {
                    self.current.network_drops += 1;
                }
            }
            SystemEvent::BatteryChanged { level, charging, .. } => {
                self.current.battery_min = self.current.battery_min.min(*level);
                if *charging {
                    self.current.charging_started = true;
                } else {
                    self.current.was_discharging = true;
                }
            }
            SystemEvent::CpuPressure { usage_percent } => {
                self.current.cpu_samples.push(*usage_percent);
                if *usage_percent >= 90.0 {
                    // Each CpuPressure fires every ~10s (resource_poll_secs)
                    self.current.cpu_sustained_high_secs += 10;
                }
            }
            SystemEvent::MemoryPressure { used_bytes, total_bytes } => {
                if *total_bytes > 0 {
                    let pct = *used_bytes as f32 / *total_bytes as f32 * 100.0;
                    self.current.memory_samples.push(pct);
                }
            }
            SystemEvent::DiskPressure {
                mount_point,
                available_bytes,
                total_bytes,
            } => {
                self.current
                    .disk_available
                    .insert(mount_point.clone(), *available_bytes);
                self.current
                    .disk_total
                    .insert(mount_point.clone(), *total_bytes);
            }
            SystemEvent::ProcessStarted { name, pid, .. } => {
                let safe: String = name.chars().filter(|c| !c.is_control()).take(50).collect();
                self.current.running_pids.insert(*pid, (safe, now));
            }
            SystemEvent::ProcessStopped {
                name,
                pid,
                exit_code,
            } => {
                if let Some((pname, started)) = self.current.running_pids.remove(pid) {
                    let runtime = now.duration_since(started).as_secs();
                    let safe: String =
                        name.chars().filter(|c| !c.is_control()).take(50).collect();
                    let session = self
                        .current
                        .process_sessions
                        .entry(pname)
                        .or_insert_with(|| ProcessSession {
                            name: safe,
                            runtime_secs: 0,
                            exit_code: None,
                            started_at: started,
                        });
                    session.runtime_secs += runtime;
                    if let Some(code) = exit_code {
                        session.exit_code = Some(*code);
                    }
                }
            }
            SystemEvent::UserIdle { idle_seconds } => {
                if let Some(last) = self.current.last_active_instant {
                    let active = now.duration_since(last).as_secs();
                    self.current.user_active_secs += active;
                }
                self.current.last_active_instant = None;
                self.current.user_idle_secs += *idle_seconds;
            }
            SystemEvent::UserResumed => {
                self.current.last_active_instant = Some(now);
            }
            SystemEvent::NotificationReceived { urgency, .. } => {
                self.current.notification_count += 1;
                if *urgency >= 2 {
                    self.current.critical_notifications += 1;
                }
            }
            SystemEvent::FileChanged { kind, .. } => {
                use yantrik_os::FileChangeKind;
                match kind {
                    FileChangeKind::Created => self.current.file_creates += 1,
                    FileChangeKind::Modified => self.current.file_modifies += 1,
                    FileChangeKind::Deleted => self.current.file_deletes += 1,
                    FileChangeKind::Renamed { .. } => self.current.file_modifies += 1,
                }
            }
            SystemEvent::KeybindTriggered { .. } => {}
        }
    }

    /// Detect issues from the current event. Returns an issue if anomaly found.
    /// Uses 1-hour cooldown per issue key to prevent flooding.
    pub fn detect_issue(
        &mut self,
        event: &SystemEvent,
        snap: &SystemSnapshot,
    ) -> Option<DetectedIssue> {
        let now = Instant::now();
        let cooldown = std::time::Duration::from_secs(3600);

        // Evict expired cooldowns
        self.issue_cooldowns
            .retain(|_, t| now.duration_since(*t) < cooldown);

        let result = self.classify_issue(event, snap)?;

        // Cooldown check — key on first 40 chars of text
        let key = format!("issue:{}", &result.text[..result.text.len().min(40)]);
        if self.issue_cooldowns.contains_key(&key) {
            return None;
        }
        self.issue_cooldowns.insert(key, now);

        Some(result)
    }

    fn classify_issue(
        &self,
        event: &SystemEvent,
        snap: &SystemSnapshot,
    ) -> Option<DetectedIssue> {
        match event {
            SystemEvent::BatteryChanged {
                level,
                charging: false,
                time_to_empty_mins,
            } if *level <= 15 => {
                let detail = time_to_empty_mins
                    .map(|m| format!(", ~{}min left", m))
                    .unwrap_or_default();
                Some(DetectedIssue {
                    text: format!("Critical battery: {}%{}", level, detail),
                    importance: 0.9,
                    decay: 0.0, // permanent
                })
            }
            SystemEvent::CpuPressure { usage_percent }
                if self.current.cpu_sustained_high_secs >= 60 && *usage_percent >= 90.0 =>
            {
                Some(DetectedIssue {
                    text: format!(
                        "CPU sustained above 90% for {}+ seconds (current: {:.0}%)",
                        self.current.cpu_sustained_high_secs, usage_percent
                    ),
                    importance: 0.8,
                    decay: 604800.0,
                })
            }
            SystemEvent::MemoryPressure {
                used_bytes,
                total_bytes,
            } if *total_bytes > 0 => {
                let pct = *used_bytes as f32 / *total_bytes as f32 * 100.0;
                if pct >= 85.0 {
                    Some(DetectedIssue {
                        text: format!(
                            "Memory pressure: {:.0}% used ({}/{}MB)",
                            pct,
                            *used_bytes / (1024 * 1024),
                            *total_bytes / (1024 * 1024)
                        ),
                        importance: 0.8,
                        decay: 604800.0,
                    })
                } else {
                    None
                }
            }
            SystemEvent::DiskPressure {
                mount_point,
                available_bytes,
                total_bytes,
            } if *total_bytes > 0 => {
                let avail_pct = *available_bytes as f64 / *total_bytes as f64 * 100.0;
                if avail_pct <= 10.0 {
                    Some(DetectedIssue {
                        text: format!(
                            "Disk almost full on {}: only {:.1}GB free ({:.0}% used)",
                            mount_point,
                            *available_bytes as f64 / 1_000_000_000.0,
                            100.0 - avail_pct
                        ),
                        importance: 0.85,
                        decay: 0.0, // permanent
                    })
                } else {
                    None
                }
            }
            SystemEvent::NetworkChanged {
                connected: false, ..
            } if snap.network_connected => Some(DetectedIssue {
                text: "Network dropped".into(),
                importance: 0.7,
                decay: 604800.0,
            }),
            SystemEvent::ProcessStopped {
                name,
                exit_code: Some(code),
                ..
            } if *code != 0 && *code != 1 => {
                let safe: String = name.chars().filter(|c| !c.is_control()).take(50).collect();
                Some(DetectedIssue {
                    text: format!("App crashed: {} (exit code {})", safe, code),
                    importance: 0.75,
                    decay: 604800.0,
                })
            }
            _ => None,
        }
    }

    /// Check if the system context string needs to be updated.
    /// Uses a cheap hash to avoid redundant bridge.set_system_context() calls.
    pub fn context_changed(&mut self, snap: &SystemSnapshot) -> bool {
        let mut h = DefaultHasher::new();
        snap.battery_level.hash(&mut h);
        snap.battery_charging.hash(&mut h);
        snap.network_connected.hash(&mut h);
        (snap.cpu_usage_percent as u32).hash(&mut h);
        (snap.memory_usage_percent() as u32).hash(&mut h);
        snap.disk_available_bytes.hash(&mut h);
        snap.running_processes.len().hash(&mut h);
        snap.user_idle.hash(&mut h);
        let hash = h.finish();
        if hash != self.last_context_hash {
            self.last_context_hash = hash;
            true
        } else {
            false
        }
    }

    /// Flush the current bucket and return a formatted digest string.
    /// Called from the hourly timer. Resets the current bucket.
    pub fn flush(&mut self) -> String {
        let bucket = std::mem::replace(&mut self.current, ActivityBucket::default());
        format_digest(&bucket)
    }
}

/// Format a bucket into a compact, LLM-friendly digest string.
fn format_digest(b: &ActivityBucket) -> String {
    let mut parts = Vec::new();

    // Timestamp — compute local hour from bucket_start_ts
    let hour = (b.bucket_start_ts as u64 / 3600) % 24;
    parts.push(format!("Hour starting {:02}:00", hour));

    // Battery
    let bat_status = if b.charging_started {
        "charging"
    } else if b.was_discharging {
        "discharging"
    } else {
        "stable"
    };
    if b.battery_min < 100 {
        parts.push(format!("Battery: min {}% ({})", b.battery_min, bat_status));
    }

    // Network
    if b.network_drops > 0 {
        parts.push(format!(
            "Network: {} drop(s), {} reconnect(s)",
            b.network_drops, b.network_connects
        ));
    } else if !b.ssids_seen.is_empty() {
        parts.push(format!("Network: connected ({})", b.ssids_seen.join(", ")));
    }

    // CPU
    if !b.cpu_samples.is_empty() {
        let avg = b.cpu_samples.iter().sum::<f32>() / b.cpu_samples.len() as f32;
        let max = b
            .cpu_samples
            .iter()
            .cloned()
            .fold(f32::NEG_INFINITY, f32::max);
        let high_str = if b.cpu_sustained_high_secs > 0 {
            format!(", {}s >90%", b.cpu_sustained_high_secs)
        } else {
            String::new()
        };
        parts.push(format!("CPU: avg {:.0}%, peak {:.0}%{}", avg, max, high_str));
    }

    // Memory
    if !b.memory_samples.is_empty() {
        let avg = b.memory_samples.iter().sum::<f32>() / b.memory_samples.len() as f32;
        let max = b
            .memory_samples
            .iter()
            .cloned()
            .fold(f32::NEG_INFINITY, f32::max);
        parts.push(format!("RAM: avg {:.0}%, peak {:.0}%", avg, max));
    }

    // Disk
    for (mount, avail) in &b.disk_available {
        if let Some(total) = b.disk_total.get(mount) {
            if *total > 0 {
                let free_pct = *avail as f64 / *total as f64 * 100.0;
                parts.push(format!("Disk {}: {:.0}% free", mount, free_pct));
            }
        }
    }

    // Processes — top 5 by runtime
    if !b.process_sessions.is_empty() {
        let mut sessions: Vec<&ProcessSession> = b.process_sessions.values().collect();
        sessions.sort_by(|a, b| b.runtime_secs.cmp(&a.runtime_secs));
        let summaries: Vec<String> = sessions
            .iter()
            .take(5)
            .map(|s| {
                let mins = s.runtime_secs / 60;
                if mins > 0 {
                    format!("{} ({}min)", s.name, mins)
                } else {
                    s.name.clone()
                }
            })
            .collect();
        parts.push(format!("Apps: {}", summaries.join(", ")));
    }

    // User presence
    let total_secs = b.user_active_secs + b.user_idle_secs;
    if total_secs > 0 {
        let active_pct = b.user_active_secs * 100 / total_secs;
        parts.push(format!("User: {}% active", active_pct));
    }

    // Notifications
    if b.notification_count > 0 {
        parts.push(format!(
            "Notifications: {} ({} critical)",
            b.notification_count, b.critical_notifications
        ));
    }

    // File activity
    let file_total = b.file_creates + b.file_modifies + b.file_deletes;
    if file_total > 0 {
        parts.push(format!(
            "Files: +{} ~{} -{}",
            b.file_creates, b.file_modifies, b.file_deletes
        ));
    }

    parts.join("\n")
}

fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
