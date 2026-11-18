//! A reading, as the cards say it: the numbers formatted the way the window has always drawn
//! them. Plain data, made once when a reading lands; `render` draws it and `describe` reads the
//! same strings, so the person and the Mind are given the same figures.

use yantrik_ipc_contracts::system_monitor::SystemSnapshot;

use crate::report;

/// What the cards show. `Default` is what the window shows before the first reading: the
/// screen's own property defaults (`system_monitor.slint`), so `describe` answers as it did.
#[derive(Debug, Clone, PartialEq)]
pub struct Readout {
    /// The share over the interval since the last reading; 0 until there is one.
    pub cpu_usage: f32,
    /// False until two readings give an interval to measure over.
    pub cpu_measured: bool,
    pub load_avg: [String; 3],
    /// Empty until the CPU is measured: one read of the counters is the average since boot.
    pub cores: Vec<Core>,
    pub memory_usage: f32,
    pub memory_used: String,
    pub memory_total: String,
    pub memory_available: String,
    pub memory_cached: String,
    pub memory_buffers: String,
    pub swap_usage: f32,
    pub swap_used: String,
    pub swap_total: String,
    pub disks: Vec<Disk>,
    pub interfaces: Vec<Interface>,
    pub uptime: String,
}

impl Default for Readout {
    fn default() -> Readout {
        Readout {
            cpu_usage: 0.0,
            cpu_measured: false,
            load_avg: ["0.00".into(), "0.00".into(), "0.00".into()],
            cores: Vec::new(),
            memory_usage: 0.0,
            memory_used: String::new(),
            memory_total: String::new(),
            memory_available: String::new(),
            memory_cached: String::new(),
            memory_buffers: String::new(),
            swap_usage: 0.0,
            swap_used: String::new(),
            swap_total: String::new(),
            disks: Vec::new(),
            interfaces: Vec::new(),
            uptime: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Core {
    pub id: i32,
    pub usage: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Disk {
    pub mount_point: String,
    pub filesystem: String,
    pub used: String,
    pub total: String,
    pub usage_percent: f32,
    /// At the limit the status row checks: the bar wears amber.
    pub needs_you: bool,
}

/// One network interface. There is no address: neither the service's snapshot nor `sysinfo`
/// carries one (`NetworkInterface` in the contract has no field for it).
#[derive(Debug, Clone, PartialEq)]
pub struct Interface {
    pub name: String,
    pub rx_bytes: String,
    pub tx_bytes: String,
    pub rx_speed: String,
    pub tx_speed: String,
}

impl Readout {
    pub fn from_snapshot(snap: &SystemSnapshot) -> Readout {
        // CPU — a share over the interval since the last reading, and no figure until there is
        // one: a single read of the counters is the average since boot, which reads 8% through
        // a minute at 100%. The per-core bars wait for the same interval.
        let measured = snap.cpu.measured_percent();
        let cores = snap
            .cpu
            .cores
            .iter()
            .filter(|_| measured.is_some())
            .map(|c| Core {
                id: c.id as i32,
                usage: c.usage_percent as f32,
            })
            .collect();
        let swap_pct = if snap.memory.swap_total_bytes > 0 {
            (snap.memory.swap_used_bytes as f64 / snap.memory.swap_total_bytes as f64) * 100.0
        } else {
            0.0
        };
        Readout {
            cpu_usage: measured.unwrap_or(0.0) as f32,
            cpu_measured: measured.is_some(),
            load_avg: [
                format!("{:.2}", snap.cpu.load_avg_1),
                format!("{:.2}", snap.cpu.load_avg_5),
                format!("{:.2}", snap.cpu.load_avg_15),
            ],
            cores,
            memory_usage: snap.memory.usage_percent as f32,
            memory_used: format_bytes(snap.memory.used_bytes),
            memory_total: format_bytes(snap.memory.total_bytes),
            memory_available: format_bytes(snap.memory.available_bytes),
            memory_cached: format_bytes(snap.memory.cached_bytes),
            memory_buffers: format_bytes(snap.memory.buffers_bytes),
            swap_usage: swap_pct as f32,
            swap_used: format_bytes(snap.memory.swap_used_bytes),
            swap_total: format_bytes(snap.memory.swap_total_bytes),
            disks: snap
                .disks
                .iter()
                .map(|d| Disk {
                    mount_point: d.mount_point.clone(),
                    filesystem: d.filesystem.clone(),
                    used: format_bytes(d.used_bytes),
                    total: format_bytes(d.total_bytes),
                    usage_percent: d.usage_percent as f32,
                    needs_you: report::disk_bar_needs_you(d),
                })
                .collect(),
            interfaces: snap
                .networks
                .iter()
                .map(|n| Interface {
                    name: n.name.clone(),
                    rx_bytes: format_bytes(n.rx_bytes),
                    tx_bytes: format_bytes(n.tx_bytes),
                    rx_speed: format_rate(n.rx_rate_bps),
                    tx_speed: format_rate(n.tx_rate_bps),
                })
                .collect(),
            uptime: format_uptime(snap.uptime_secs),
        }
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    }
}

pub fn format_rate(bps: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    if bps >= MB {
        format!("{:.1} MB/s", bps as f64 / MB as f64)
    } else if bps >= KB {
        format!("{:.1} KB/s", bps as f64 / KB as f64)
    } else {
        format!("{} B/s", bps)
    }
}

pub fn format_uptime(secs: u64) -> String {
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    if days > 0 {
        format!("{}d {}h {}m", days, hours, mins)
    } else if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else {
        format!("{}m", mins)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_rates_and_uptimes_read_as_they_always_have() {
        assert_eq!(format_bytes(2048), "2 KB");
        assert_eq!(format_bytes(3 * 1024 * 1024), "3.0 MB");
        assert_eq!(format_bytes(16 * 1024 * 1024 * 1024), "16.0 GB");
        assert_eq!(format_rate(100), "100 B/s");
        assert_eq!(format_rate(2048), "2.0 KB/s");
        assert_eq!(format_rate(5 * 1024 * 1024), "5.0 MB/s");
        assert_eq!(format_uptime(59), "0m");
        assert_eq!(format_uptime(3 * 3600 + 25 * 60), "3h 25m");
        assert_eq!(format_uptime(2 * 86400 + 3600 + 60), "2d 1h 1m");
    }

    #[test]
    fn before_the_first_reading_the_cards_hold_the_screens_own_defaults() {
        let empty = Readout::default();
        assert_eq!(empty.load_avg, ["0.00", "0.00", "0.00"]);
        assert!(!empty.cpu_measured && empty.memory_used.is_empty() && empty.uptime.is_empty());
    }
}
