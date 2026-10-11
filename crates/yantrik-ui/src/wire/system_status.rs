//! The System screen's STATUS row: uptime, load, and the limits the readings were checked
//! against — `machine_status`'s account, the one System Monitor and its `describe` give, so the
//! shell and the app never say different things about the same machine.
//!
//! The STATUS card sat empty on the review's screen. CPU and memory have bars of their own above
//! it, so the row shows what nothing else on the screen does, and still checks every limit.

use yantrik_ipc_contracts::machine_status::{Disk, Fact, MachineStatus, Readings};

use crate::App;

const ROW: &[Fact] = &[Fact::Uptime, Fact::Load];

/// Put the row on screen. Shared by the live poll and the on-entry population, as the memory
/// readouts are, so the two paths cannot drift apart.
pub(crate) fn update(ui: &App, snap: &yantrik_os::SystemSnapshot) {
    let uptime = super::about::read_uptime();
    let status = assess(snap, uptime_reading(uptime), load_1(), cores());
    ui.set_sys_status_lead(status.lead.as_str().into());
    ui.set_sys_status_rest(status.rest.as_str().into());
    ui.set_sys_status_thresholds(status.thresholds.as_str().into());
    ui.set_sys_status_needs_you(status.needs_you());
}

fn assess(snap: &yantrik_os::SystemSnapshot, uptime: Option<String>, load_1: Option<f64>, cores: usize) -> MachineStatus {
    // A zero total is the snapshot before its first reading, not a machine with no memory or
    // no disk; a limit nothing measured is left out rather than said to be clear.
    let memory_measured = snap.memory_total_bytes > 0;
    let readings = Readings {
        uptime,
        // CPU has its own bar above this card, and no limit is checked against it.
        cpu_percent: None,
        cpu_window_ms: None,
        memory: memory_measured.then_some((snap.memory_total_bytes, snap.memory_used_bytes)),
        swap: memory_measured.then_some((snap.swap_total_bytes, snap.swap_used_bytes)),
        // The shell's observer measures the root filesystem and nothing else.
        disks: (snap.disk_total_bytes > 0)
            .then(|| Disk {
                mount: "/".into(),
                total_bytes: snap.disk_total_bytes,
                used_bytes: snap.disk_total_bytes.saturating_sub(snap.disk_available_bytes),
            })
            .into_iter()
            .collect(),
        load_1,
        cores,
    };
    MachineStatus::assess(&readings, ROW)
}

/// About's reader answers an em dash when /proc/uptime could not be read; that is no reading.
fn uptime_reading(text: String) -> Option<String> {
    (text != "\u{2014}").then_some(text)
}

/// The 1-minute load average, the first field of /proc/loadavg.
fn load_1() -> Option<f64> {
    std::fs::read_to_string("/proc/loadavg").ok()?.split_whitespace().next()?.parse().ok()
}

/// Online CPUs: the `cpuN` lines of /proc/stat. Not `available_parallelism`, which is this
/// process's affinity, while the load average counts the whole machine.
fn cores() -> usize {
    std::fs::read_to_string("/proc/stat")
        .map(|stat| {
            stat.lines()
                .filter(|l| l.starts_with("cpu") && l.as_bytes().get(3).is_some_and(u8::is_ascii_digit))
                .count()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1024 * 1024 * 1024;

    fn snap(disk_free_gb: u64) -> yantrik_os::SystemSnapshot {
        yantrik_os::SystemSnapshot {
            memory_total_bytes: 16 * GB,
            memory_used_bytes: 8 * GB,
            disk_total_bytes: 32 * GB,
            disk_available_bytes: disk_free_gb * GB,
            ..Default::default()
        }
    }

    #[test]
    fn the_status_card_says_uptime_load_and_that_no_limit_was_reached() {
        let s = assess(&snap(20), Some("6d 5h 3m".into()), Some(1.15), 4);
        assert_eq!(s.line(), "Uptime 6d 5h 3m · Load 1.15 · No limits reached");
        assert_eq!(s.thresholds, "Limits: / at 90% full, memory at 90% used, 1-minute load above 4 (the core count)");
    }

    #[test]
    fn a_full_root_disk_leads_the_card() {
        let s = assess(&snap(1), Some("2h 1m".into()), Some(0.5), 4);
        assert!(s.needs_you());
        assert_eq!(s.line(), "Disk nearly full · 1.0 GB free (97%) · Uptime 2h 1m · Load 0.50");
    }

    #[test]
    fn before_the_first_reading_nothing_is_claimed() {
        let s = assess(&yantrik_os::SystemSnapshot::default(), None, None, 0);
        assert_eq!(s.line(), "");
        assert!(!s.needs_you());
    }

    #[test]
    fn an_unreadable_uptime_is_no_uptime() {
        assert_eq!(uptime_reading("\u{2014}".into()), None);
        assert_eq!(uptime_reading("3h 2m".into()).as_deref(), Some("3h 2m"));
    }
}
