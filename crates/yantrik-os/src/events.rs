//! System events — the vocabulary of what the OS observes.
//!
//! Every system change is expressed as a `SystemEvent`. These flow from
//! the SystemObserver thread to the main thread via crossbeam channel.
//! Features consume events to produce urges.

use serde::{Deserialize, Serialize};

/// A single system event observed by the SystemObserver.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SystemEvent {
    // ── Power ──
    BatteryChanged {
        level: u8,
        charging: bool,
        /// Estimated minutes until empty (if discharging).
        time_to_empty_mins: Option<u32>,
    },

    // ── Network ──
    NetworkChanged {
        connected: bool,
        ssid: Option<String>,
        /// Signal strength 0-100.
        signal: Option<u8>,
    },

    // ── Notifications ──
    NotificationReceived {
        app: String,
        summary: String,
        body: String,
        /// D-Bus urgency hint: 0=low, 1=normal, 2=critical.
        urgency: u8,
    },

    // ── File system ──
    FileChanged {
        path: String,
        kind: FileChangeKind,
    },

    // ── Processes ──
    ProcessStarted {
        name: String,
        pid: u32,
        /// CPU usage at discovery time (0.0 - 100.0+).
        cpu_percent: f32,
    },
    ProcessStopped {
        name: String,
        pid: u32,
        /// Exit code if available.
        exit_code: Option<i32>,
    },

    // ── Resource pressure ──
    CpuPressure {
        /// Overall CPU usage percentage.
        usage_percent: f32,
    },
    MemoryPressure {
        /// Used memory in bytes (total - available).
        used_bytes: u64,
        /// Total memory in bytes.
        total_bytes: u64,
        /// Cached/buffers memory in bytes (available - free).
        cached_bytes: u64,
        /// Free memory in bytes (MemFree, not available).
        free_bytes: u64,
        /// Swap used in bytes.
        swap_used_bytes: u64,
        /// Swap total in bytes.
        swap_total_bytes: u64,
    },
    DiskPressure {
        mount_point: String,
        /// Available space in bytes.
        available_bytes: u64,
        /// Total space in bytes.
        total_bytes: u64,
    },

    // ── User presence ──
    UserIdle {
        idle_seconds: u64,
    },
    UserResumed,

    // ── Keybind ──
    /// A global keybind was triggered via D-Bus (from labwc).
    KeybindTriggered {
        /// Action identifier, e.g. "open-lens", "lock-screen", "open-terminal".
        action: String,
    },
}

/// What kind of file system change occurred.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileChangeKind {
    Created,
    Modified,
    Deleted,
    Renamed { to: String },
}

/// A snapshot of current system state, built from accumulated events.
#[derive(Debug, Clone, Default)]
pub struct SystemSnapshot {
    // Power
    pub battery_level: u8,
    pub battery_charging: bool,
    pub battery_time_to_empty_mins: Option<u32>,

    // Network
    pub network_connected: bool,
    pub network_ssid: Option<String>,
    pub network_signal: Option<u8>,

    // Resources
    pub cpu_usage_percent: f32,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    pub memory_cached_bytes: u64,
    pub memory_free_bytes: u64,
    pub swap_used_bytes: u64,
    pub swap_total_bytes: u64,

    // Disk
    pub disk_available_bytes: u64,
    pub disk_total_bytes: u64,

    // Process tracking
    pub running_processes: Vec<ProcessInfo>,

    // Idle tracking
    pub idle_seconds: u64,
    pub user_idle: bool,
}

/// Basic info about a running process.
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub name: String,
    pub pid: u32,
    pub cpu_percent: f32,
}

impl SystemSnapshot {
    /// Apply a system event to update the snapshot.
    pub fn apply(&mut self, event: &SystemEvent) {
        match event {
            SystemEvent::BatteryChanged { level, charging, time_to_empty_mins } => {
                self.battery_level = *level;
                self.battery_charging = *charging;
                self.battery_time_to_empty_mins = *time_to_empty_mins;
            }
            SystemEvent::NetworkChanged { connected, ssid, signal } => {
                self.network_connected = *connected;
                self.network_ssid = ssid.clone();
                self.network_signal = *signal;
            }
            SystemEvent::CpuPressure { usage_percent } => {
                self.cpu_usage_percent = *usage_percent;
            }
            SystemEvent::MemoryPressure {
                used_bytes,
                total_bytes,
                cached_bytes,
                free_bytes,
                swap_used_bytes,
                swap_total_bytes,
            } => {
                self.memory_used_bytes = *used_bytes;
                self.memory_total_bytes = *total_bytes;
                self.memory_cached_bytes = *cached_bytes;
                self.memory_free_bytes = *free_bytes;
                self.swap_used_bytes = *swap_used_bytes;
                self.swap_total_bytes = *swap_total_bytes;
            }
            SystemEvent::DiskPressure { available_bytes, total_bytes, .. } => {
                self.disk_available_bytes = *available_bytes;
                self.disk_total_bytes = *total_bytes;
            }
            SystemEvent::ProcessStarted { name, pid, cpu_percent } => {
                // Only add if not already tracked
                if !self.running_processes.iter().any(|p| p.pid == *pid) {
                    self.running_processes.push(ProcessInfo {
                        name: name.clone(),
                        pid: *pid,
                        cpu_percent: *cpu_percent,
                    });
                }
            }
            SystemEvent::ProcessStopped { pid, .. } => {
                self.running_processes.retain(|p| p.pid != *pid);
            }
            SystemEvent::UserIdle { idle_seconds } => {
                self.idle_seconds = *idle_seconds;
                self.user_idle = true;
            }
            SystemEvent::UserResumed => {
                self.idle_seconds = 0;
                self.user_idle = false;
            }
            // File and notification events don't update the snapshot —
            // they're consumed directly by features.
            _ => {}
        }
    }

    /// Memory usage as a percentage (0.0 - 100.0).
    pub fn memory_usage_percent(&self) -> f32 {
        if self.memory_total_bytes == 0 {
            0.0
        } else {
            (self.memory_used_bytes as f64 / self.memory_total_bytes as f64 * 100.0) as f32
        }
    }

    /// Disk used as a percentage (0.0 - 100.0).
    pub fn disk_used_percent(&self) -> f32 {
        if self.disk_total_bytes == 0 {
            0.0
        } else {
            let avail = self.disk_available_bytes as f64;
            let total = self.disk_total_bytes as f64;
            ((1.0 - avail / total) * 100.0) as f32
        }
    }
}
