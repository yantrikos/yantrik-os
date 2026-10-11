//! What the window says the readings add up to — on its status row and in `describe`.
//!
//! This was a health score: "Healthy", 100%, "All systems nominal" whenever CPU and memory were
//! both under 90%, drawn above a disk card showing / at 95% full in red. Nothing on screen said
//! what the score was made of, and an agent reading `describe` was told the same. Both now carry
//! `machine_status`'s account: what was measured, which limits were checked, which were reached.

use yantrik_app_runtime::control::View;
use yantrik_ipc_contracts::machine_status::{disk_needs_you, Fact, MachineStatus, Readings};
use yantrik_ipc_contracts::system_monitor::{DiskInfo, SystemSnapshot};

/// The readings on the window's status row. Load has its own line in the CPU card; it is
/// checked all the same.
const ROW: &[Fact] = &[Fact::Cpu, Fact::Memory, Fact::Disk, Fact::Swap];

pub fn assess(snap: &SystemSnapshot) -> MachineStatus {
    MachineStatus::assess(&Readings::from_snapshot(snap), ROW)
}

/// Whether a disk card's bar wears amber. The row's own rule, so the two never disagree.
pub fn disk_bar_needs_you(disk: &DiskInfo) -> bool {
    disk_needs_you(disk.usage_percent)
}

/// The one-line summary `describe` leads with: the row, then what the row leaves to the cards.
pub fn summary(status: &MachineStatus, memory_used: &str, memory_total: &str, uptime: &str) -> String {
    let line = status.line();
    let line = if line.is_empty() { "no reading yet".to_string() } else { line };
    format!("System — {line}; memory {memory_used} of {memory_total}; up {uptime}")
}

/// `describe`'s `cpu_percent`: the share over `cpu_window_ms`, or null when there is no interval
/// yet. It was the since-boot average, reported as if it were now.
pub fn cpu_percent(percent: f32, window_ms: Option<u64>) -> Option<f64> {
    window_ms.map(|_| (percent * 10.0).round() as f64 / 10.0)
}

/// `describe`'s status fields.
///
/// `health` and `health_summary` are kept for callers that read them, and now carry the status
/// word and the row — derived from the limits on screen and nothing else. `health_score` is gone:
/// it was the number nothing measured. `status`, `limits_reached` and `thresholds` are new, and
/// are the same fields the system-monitor service's `describe` answers with.
pub fn with_status(view: View, status: &MachineStatus) -> View {
    let fields = status.json();
    view.with("health", status.verdict())
        .with("health_summary", status.line())
        .with("status", fields["status"].clone())
        .with("limits_reached", fields["limits_reached"].clone())
        .with("thresholds", fields["thresholds"].clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_ipc_contracts::system_monitor::{CpuCore, CpuInfo, MemoryInfo};

    const GB: u64 = 1024 * 1024 * 1024;

    fn snap(disk_used_gb: f64) -> SystemSnapshot {
        let total = (31.3 * GB as f64) as u64;
        let used = (disk_used_gb * GB as f64) as u64;
        SystemSnapshot {
            cpu: CpuInfo {
                overall_percent: 8.0,
                cores: (0..4).map(|id| CpuCore { id, usage_percent: 8.0 }).collect(),
                load_avg_1: 1.15,
                load_avg_5: 1.0,
                load_avg_15: 0.9,
                window_ms: Some(2000),
            },
            memory: MemoryInfo {
                total_bytes: 16 * GB,
                used_bytes: 8 * GB,
                usage_percent: 50.0,
                swap_total_bytes: 0,
                swap_used_bytes: 0,
                available_bytes: 8 * GB,
                cached_bytes: 0,
                buffers_bytes: 0,
            },
            disks: vec![DiskInfo {
                mount_point: "/".into(),
                device: "/dev/sda1".into(),
                filesystem: "ext4".into(),
                total_bytes: total,
                used_bytes: used,
                usage_percent: used as f64 / total as f64 * 100.0,
            }],
            networks: vec![],
            uptime_secs: 3600,
        }
    }

    fn described(snap: &SystemSnapshot) -> serde_json::Value {
        let status = assess(snap);
        let view = with_status(View::new(summary(&status, "8.0 GB", "16.0 GB", "1h 0m")), &status);
        serde_json::to_value(&view).unwrap()
    }

    #[test]
    fn the_row_with_no_limit_reached_says_so_and_what_was_checked() {
        let status = assess(&snap(10.0));
        assert_eq!(status.line(), "CPU 8% · Memory 50% · Disk 32% full · Swap none · No limits reached");
        assert!(status.thresholds.starts_with("Limits: / at 90% full"));
        assert!(!status.needs_you());
    }

    #[test]
    fn the_row_with_the_disk_at_95_percent_leads_with_it() {
        let status = assess(&snap(29.6));
        assert!(status.needs_you());
        assert_eq!(status.lead, "Disk nearly full · 1.7 GB free (95%)");
    }

    #[test]
    fn the_row_with_several_limits_reached_names_each() {
        let mut s = snap(29.6);
        s.memory.used_bytes = 15 * GB;
        s.cpu.load_avg_1 = 6.0;
        let status = assess(&s);
        assert_eq!(status.verdict(), "3 limits reached");
        assert_eq!(
            status.lead,
            "Disk nearly full · 1.7 GB free (95%) · Memory nearly full · 1.0 GB available · Load 6.00 above 4 cores"
        );
    }

    #[test]
    fn a_disk_bar_is_amber_at_90_percent_and_not_below() {
        let mut disk = snap(0.0).disks.remove(0);
        for (percent, amber) in [(95.0, true), (90.0, true), (89.4, false), (50.0, false)] {
            disk.usage_percent = percent;
            assert_eq!(disk_bar_needs_you(&disk), amber, "{percent}%");
        }
    }

    #[test]
    fn describe_never_calls_the_machine_healthy_or_nominal() {
        for used in [10.0, 29.6] {
            let text = described(&snap(used)).to_string().to_lowercase();
            assert!(!text.contains("healthy"), "{text}");
            assert!(!text.contains("nominal"), "{text}");
            assert!(!text.contains("health_score"), "the score nothing measured is gone: {text}");
        }
    }

    #[test]
    fn before_the_second_sample_there_is_no_cpu_figure_anywhere() {
        // The first reading's CPU is the counters over themselves: the average since boot.
        let mut first = snap(10.0);
        first.cpu.window_ms = None;
        let status = assess(&first);
        assert_eq!(status.line(), "CPU — · Memory 50% · Disk 32% full · Swap none · No limits reached");
        assert_eq!(cpu_percent(8.0, None), None);
        assert_eq!(cpu_percent(8.04, Some(2000)), Some(8.0));
    }

    #[test]
    fn describe_reports_the_rows_words_and_the_tripped_limit() {
        let view = described(&snap(29.6));
        let line = "Disk nearly full · 1.7 GB free (95%) · CPU 8% · Memory 50% · Swap none";
        assert_eq!(view["state"]["status"], line);
        assert_eq!(view["state"]["health_summary"], line);
        assert_eq!(view["state"]["health"], "1 limit reached");
        assert_eq!(view["state"]["limits_reached"][0]["mount"], "/");
        assert!(view["summary"].as_str().unwrap().starts_with(&format!("System — {line};")));
    }
}
