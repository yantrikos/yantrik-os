//! What the machine's readings add up to, in the words every surface that reports them uses.
//!
//! The System Monitor window drew "Healthy", a full bar marked "100%" and "All systems nominal"
//! directly above its own disk card showing `/` at 95% full, in red. The score was a guess from
//! two numbers, the sentence was reassurance, and neither said what had been checked. What
//! replaces them says only what was measured, which limits were checked, and which of those were
//! reached — so the claim "No limits reached" can be read off the screen and checked.
//!
//! The window, the system-monitor service's `describe` and the shell's System screen all speak
//! these words, and an agent reading one must not be told something different by another. That
//! makes them part of what the surfaces promise, which is why they live with the contracts: the
//! one crate all three already share.

/// A disk this full needs its owner: a full root filesystem stops logins, updates and saves.
pub const DISK_LIMIT_PERCENT: f64 = 90.0;

/// Memory this full is where the kernel starts reclaiming hard and then killing processes.
pub const MEMORY_LIMIT_PERCENT: f64 = 90.0;

/// The filesystems that hold a person's data — the ones a "disk nearly full" is about.
///
/// Pseudo filesystems are left out, and so are the ones that are full by construction: a
/// squashfs or an ISO image always reads 100%, and calling that a disk nearly full would put a
/// warning on every live session. The service's mount reader and the window's fallback both
/// filter with this, so the two paths agree on what a disk is.
pub fn is_real_filesystem(fs: &str) -> bool {
    matches!(fs, "ext4" | "ext3" | "ext2" | "xfs" | "btrfs" | "f2fs" | "vfat" | "ntfs" | "zfs")
}

/// One mounted filesystem, as measured.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Disk {
    pub mount: String,
    pub total_bytes: u64,
    /// Total less what an ordinary user could still write.
    pub used_bytes: u64,
}

impl Disk {
    pub fn percent(&self) -> f64 {
        if self.total_bytes == 0 {
            0.0
        } else {
            self.used_bytes as f64 / self.total_bytes as f64 * 100.0
        }
    }

    fn free_bytes(&self) -> u64 {
        self.total_bytes.saturating_sub(self.used_bytes)
    }

    /// "Disk" for the root filesystem, which is what a person means by the disk; the mount
    /// point for any other, so two of them are never the same word.
    fn name(&self) -> String {
        if self.mount == "/" {
            "Disk".to_string()
        } else {
            format!("Disk {}", self.mount)
        }
    }
}

/// Whatever one screen measured. A reading left out is not checked and not reported — a
/// limit nothing measured is never said to be clear.
#[derive(Clone, Debug, Default)]
pub struct Readings {
    /// Already in the screen's own words ("6d 5h"), so it reads the same as the rest of it.
    pub uptime: Option<String>,
    /// The busy share over `cpu_window_ms`. `None` until there is an interval to measure it
    /// over: one read of the counters is the average since boot, and would read 8% through a
    /// minute at 100%. The row then says "CPU —", and nothing else on it depends on CPU.
    pub cpu_percent: Option<f64>,
    pub cpu_window_ms: Option<u64>,
    /// `(total, used)`, where used is total less what is available.
    pub memory: Option<(u64, u64)>,
    /// `(total, used)`. A total of zero is a machine with no swap.
    pub swap: Option<(u64, u64)>,
    /// Real filesystems only (see [`is_real_filesystem`]).
    pub disks: Vec<Disk>,
    pub load_1: Option<f64>,
    /// Zero when the count is not known, and then load is reported but not checked.
    pub cores: usize,
}

impl Readings {
    /// Everything a system-monitor snapshot measured. Uptime is left to the caller, which says
    /// it in its own screen's words.
    pub fn from_snapshot(s: &crate::system_monitor::SystemSnapshot) -> Readings {
        Readings {
            uptime: None,
            cpu_percent: s.cpu.measured_percent(),
            cpu_window_ms: s.cpu.window_ms,
            memory: Some((s.memory.total_bytes, s.memory.used_bytes)),
            swap: Some((s.memory.swap_total_bytes, s.memory.swap_used_bytes)),
            disks: s
                .disks
                .iter()
                .filter(|d| is_real_filesystem(&d.filesystem))
                .map(|d| Disk { mount: d.mount_point.clone(), total_bytes: d.total_bytes, used_bytes: d.used_bytes })
                .collect(),
            load_1: Some(s.cpu.load_avg_1),
            cores: s.cpu.cores.len(),
        }
    }
}

/// The readings a screen puts on its status row. A screen that already draws CPU and memory
/// in bars of their own can leave them off; the limits are checked either way.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fact {
    Uptime,
    Cpu,
    Memory,
    Disk,
    Swap,
    Load,
}

/// A limit that was reached, with the measurement that reached it.
#[derive(Clone, Debug, PartialEq)]
pub enum Limit {
    Disk { mount: String, percent: f64, free_bytes: u64 },
    Memory { percent: f64, available_bytes: u64 },
    Load { load: f64, cores: usize },
}

impl Limit {
    /// What the person reads first on the row, beside the warning glyph. A full disk carries
    /// its percentage here, so the readings after it need not name the same disk again.
    pub fn sentence(&self) -> String {
        match self {
            Limit::Disk { mount, percent, free_bytes } => {
                let disk = Disk { mount: mount.clone(), ..Default::default() };
                format!("{} nearly full · {} free ({}%)", disk.name(), bytes(*free_bytes), whole(*percent))
            }
            Limit::Memory { available_bytes, .. } => {
                format!("Memory nearly full · {} available", bytes(*available_bytes))
            }
            Limit::Load { load, cores } => format!("Load {load:.2} above {cores} cores"),
        }
    }

    /// The same limit for a caller: the measurement, the threshold it crossed, and the sentence.
    pub fn json(&self) -> serde_json::Value {
        match self {
            Limit::Disk { mount, percent, free_bytes } => serde_json::json!({
                "limit": "disk",
                "mount": mount,
                "percent": whole(*percent),
                "threshold_percent": DISK_LIMIT_PERCENT,
                "free": bytes(*free_bytes),
                "sentence": self.sentence(),
            }),
            Limit::Memory { percent, available_bytes } => serde_json::json!({
                "limit": "memory",
                "percent": whole(*percent),
                "threshold_percent": MEMORY_LIMIT_PERCENT,
                "available": bytes(*available_bytes),
                "sentence": self.sentence(),
            }),
            Limit::Load { load, cores } => serde_json::json!({
                "limit": "load",
                "load_1": (load * 100.0).round() / 100.0,
                "threshold": cores,
                "sentence": self.sentence(),
            }),
        }
    }
}

/// The status row, and the same thing for a caller.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MachineStatus {
    pub limits: Vec<Limit>,
    /// The limits reached, in sentences; empty when none was.
    pub lead: String,
    /// The readings, and "No limits reached" after them when that was checked and is true.
    pub rest: String,
    /// What was checked, so the claim on the row can be checked too.
    pub thresholds: String,
}

impl MachineStatus {
    pub fn assess(r: &Readings, shown: &[Fact]) -> MachineStatus {
        let limits = limits(r);
        let checks = checks(r);
        let mut rest = facts(r, shown);
        if limits.is_empty() && !checks.is_empty() {
            rest.push(NO_LIMITS.to_string());
        }
        MachineStatus {
            lead: limits.iter().map(Limit::sentence).collect::<Vec<_>>().join(SEP),
            limits,
            rest: rest.join(SEP),
            thresholds: if checks.is_empty() {
                "No limits checked: nothing measured yet".to_string()
            } else {
                format!("Limits: {}", checks.join(", "))
            },
        }
    }

    /// Whether the row wears the amber of "needs you".
    pub fn needs_you(&self) -> bool {
        !self.limits.is_empty()
    }

    /// The row as one line: what needs the person first, then the readings.
    pub fn line(&self) -> String {
        match (self.lead.is_empty(), self.rest.is_empty()) {
            (true, _) => self.rest.clone(),
            (false, true) => self.lead.clone(),
            (false, false) => format!("{}{SEP}{}", self.lead, self.rest),
        }
    }

    /// The status word, derived from nothing but the limits on the row.
    pub fn verdict(&self) -> String {
        match self.limits.len() {
            0 => NO_LIMITS.to_string(),
            1 => "1 limit reached".to_string(),
            n => format!("{n} limits reached"),
        }
    }

    /// The fields a `describe` adds for this, the same on every surface that reports the machine.
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "status": self.line(),
            "limits_reached": self.limits.iter().map(Limit::json).collect::<Vec<_>>(),
            "thresholds": self.thresholds,
        })
    }
}

/// Whether a disk's bar wears amber: at the limit and only there. Never red — red is for an
/// action that destroys something, and a full disk is a thing the person can act on.
pub fn disk_needs_you(percent: f64) -> bool {
    whole(percent) >= DISK_LIMIT_PERCENT
}

const SEP: &str = " · ";
const NO_LIMITS: &str = "No limits reached";

/// Percentages are compared as they are shown. A disk at 89.6% reads "90%" on its card, and a
/// row saying no limit was reached beside it would be checked against the number on screen.
fn whole(percent: f64) -> f64 {
    percent.round()
}

fn limits(r: &Readings) -> Vec<Limit> {
    let mut out = Vec::new();
    for d in r.disks.iter().filter(|d| disk_needs_you(d.percent())) {
        out.push(Limit::Disk { mount: d.mount.clone(), percent: d.percent(), free_bytes: d.free_bytes() });
    }
    if let Some((total, used)) = r.memory.filter(|(t, _)| *t > 0) {
        let percent = used as f64 / total as f64 * 100.0;
        if whole(percent) >= MEMORY_LIMIT_PERCENT {
            out.push(Limit::Memory { percent, available_bytes: total.saturating_sub(used) });
        }
    }
    if let Some(load) = r.load_1.filter(|_| r.cores > 0) {
        if load > r.cores as f64 {
            out.push(Limit::Load { load, cores: r.cores });
        }
    }
    out
}

/// The limits these readings can be checked against, in words.
fn checks(r: &Readings) -> Vec<String> {
    let mut out = Vec::new();
    match r.disks.as_slice() {
        [] => {}
        [one] => out.push(format!("{} at {DISK_LIMIT_PERCENT}% full", one.mount)),
        _ => out.push(format!("any disk at {DISK_LIMIT_PERCENT}% full")),
    }
    if r.memory.is_some_and(|(t, _)| t > 0) {
        out.push(format!("memory at {MEMORY_LIMIT_PERCENT}% used"));
    }
    if r.load_1.is_some() && r.cores > 0 {
        out.push(format!("1-minute load above {} (the core count)", r.cores));
    }
    out
}

fn facts(r: &Readings, shown: &[Fact]) -> Vec<String> {
    shown
        .iter()
        .filter_map(|fact| match fact {
            Fact::Uptime => r.uptime.as_ref().map(|u| format!("Uptime {u}")),
            Fact::Cpu => Some(cpu_fact(r.cpu_percent, r.cpu_window_ms)),
            Fact::Memory => r
                .memory
                .filter(|(t, _)| *t > 0)
                .map(|(t, u)| format!("Memory {}%", whole(u as f64 / t as f64 * 100.0))),
            // The fullest one: it is the one that will run out first. When it is at the limit
            // its percentage already leads the row, and "Disk" twice on one line reads as two
            // disks.
            Fact::Disk => r
                .disks
                .iter()
                .max_by(|a, b| a.percent().total_cmp(&b.percent()))
                .filter(|d| !disk_needs_you(d.percent()))
                .map(|d| format!("{} {}% full", d.name(), whole(d.percent()))),
            Fact::Swap => r.swap.map(|(total, used)| match total {
                0 => "Swap none".to_string(),
                _ => format!("Swap {}%", whole(used as f64 / total as f64 * 100.0)),
            }),
            Fact::Load => r.load_1.map(|l| format!("Load {l:.2}")),
        })
        .collect()
}

/// Past this, a share of CPU is an average over a stretch nobody would call "now", and says so.
const CPU_NOW_MS: u64 = 10_000;

/// "CPU 8%" over the poll's few seconds; "CPU 8% over the last 5 min" when the interval is long
/// (a `describe` asked minutes after the last); "CPU —" with no interval at all.
fn cpu_fact(percent: Option<f64>, window_ms: Option<u64>) -> String {
    match (percent, window_ms) {
        (Some(p), Some(ms)) if ms > CPU_NOW_MS => {
            let secs = ms / 1000;
            let span = if secs < 120 { format!("{secs} s") } else { format!("{} min", secs / 60) };
            format!("CPU {}% over the last {span}", whole(p))
        }
        (Some(p), _) => format!("CPU {}%", whole(p)),
        (None, _) => "CPU —".to_string(),
    }
}

/// "1.7 GB", "512 MB": free space is read at a glance, not to the byte.
fn bytes(n: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = MB * 1024.0;
    let n = n as f64;
    if n >= GB {
        format!("{:.1} GB", n / GB)
    } else {
        format!("{:.0} MB", n / MB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1024 * 1024 * 1024;
    const APP_ROW: &[Fact] = &[Fact::Cpu, Fact::Memory, Fact::Disk, Fact::Swap];

    fn disk(mount: &str, total_gb: f64, used_gb: f64) -> Disk {
        Disk { mount: mount.into(), total_bytes: (total_gb * GB as f64) as u64, used_bytes: (used_gb * GB as f64) as u64 }
    }

    fn calm() -> Readings {
        Readings {
            uptime: Some("6d 5h".into()),
            cpu_percent: Some(8.2),
            cpu_window_ms: Some(2000),
            memory: Some((16 * GB, 8 * GB)),
            swap: Some((0, 0)),
            disks: vec![disk("/", 31.3, 10.0)],
            load_1: Some(1.15),
            cores: 4,
        }
    }

    #[test]
    fn nothing_reached_says_so_after_the_readings_and_shows_what_was_checked() {
        let s = MachineStatus::assess(&calm(), APP_ROW);
        assert_eq!(s.line(), "CPU 8% · Memory 50% · Disk 32% full · Swap none · No limits reached");
        assert!(!s.needs_you());
        assert_eq!(s.verdict(), "No limits reached");
        assert_eq!(
            s.thresholds,
            "Limits: / at 90% full, memory at 90% used, 1-minute load above 4 (the core count)"
        );
    }

    #[test]
    fn a_disk_at_the_limit_leads_the_row_with_what_is_left() {
        // The review's screen: / at 29.6 of 31.3 GB.
        let r = Readings { disks: vec![disk("/", 31.3, 29.6)], ..calm() };
        let s = MachineStatus::assess(&r, APP_ROW);
        assert!(s.needs_you());
        assert_eq!(s.lead, "Disk nearly full · 1.7 GB free (95%)");
        // The disk is named once: its percentage is folded into the limit, not repeated after.
        assert_eq!(s.line(), "Disk nearly full · 1.7 GB free (95%) · CPU 8% · Memory 50% · Swap none");
        assert_eq!(s.line().matches("Disk").count(), 1);
        assert!(!s.line().contains("No limits reached"));
        assert_eq!(s.verdict(), "1 limit reached");
    }

    #[test]
    fn several_limits_are_each_named() {
        let r = Readings {
            memory: Some((16 * GB, 15 * GB)),
            disks: vec![disk("/", 100.0, 40.0), disk("/home", 100.0, 92.0)],
            load_1: Some(9.5),
            ..calm()
        };
        let s = MachineStatus::assess(&r, APP_ROW);
        assert_eq!(
            s.lead,
            "Disk /home nearly full · 8.0 GB free (92%) · Memory nearly full · 1.0 GB available · Load 9.50 above 4 cores"
        );
        assert_eq!(s.verdict(), "3 limits reached");
        assert_eq!(s.limits.len(), 3);
        assert!(s.thresholds.contains("any disk at 90% full"));
    }

    #[test]
    fn a_limit_is_checked_against_the_number_on_screen() {
        // 89.6% shows as 90%, so it is at the limit; 89.4% shows as 89%, so it is not.
        assert!(disk_needs_you(89.6));
        assert!(!disk_needs_you(89.4));
        assert!(disk_needs_you(95.0));
        assert!(!disk_needs_you(50.0));
    }

    #[test]
    fn load_equal_to_the_core_count_is_not_over_it() {
        let s = MachineStatus::assess(&Readings { load_1: Some(4.0), ..calm() }, APP_ROW);
        assert!(!s.needs_you());
    }

    #[test]
    fn the_shell_row_shows_uptime_and_load_and_still_checks_the_rest() {
        let shell = &[Fact::Uptime, Fact::Load];
        assert_eq!(MachineStatus::assess(&calm(), shell).line(), "Uptime 6d 5h · Load 1.15 · No limits reached");
        let full = Readings { disks: vec![disk("/", 31.3, 29.6)], ..calm() };
        assert_eq!(
            MachineStatus::assess(&full, shell).line(),
            "Disk nearly full · 1.7 GB free (95%) · Uptime 6d 5h · Load 1.15"
        );
    }

    #[test]
    fn nothing_measured_claims_nothing() {
        let s = MachineStatus::assess(&Readings::default(), APP_ROW);
        assert_eq!(s.line(), "CPU —");
        assert!(!s.needs_you());
        assert_eq!(s.thresholds, "No limits checked: nothing measured yet");
    }

    #[test]
    fn no_cpu_figure_without_an_interval_and_nothing_else_moves() {
        let unmeasured = Readings { cpu_percent: None, cpu_window_ms: None, ..calm() };
        let s = MachineStatus::assess(&unmeasured, APP_ROW);
        assert_eq!(s.line(), "CPU — · Memory 50% · Disk 32% full · Swap none · No limits reached");
        // The load limit is checked against the core count, which needs no interval.
        assert!(s.thresholds.contains("1-minute load above 4"));
        let busy = Readings { load_1: Some(9.0), ..unmeasured };
        assert_eq!(MachineStatus::assess(&busy, APP_ROW).lead, "Load 9.00 above 4 cores");
    }

    #[test]
    fn a_share_over_a_long_interval_says_how_long() {
        let r = Readings { cpu_percent: Some(12.0), cpu_window_ms: Some(300_000), ..calm() };
        assert!(MachineStatus::assess(&r, APP_ROW).line().starts_with("CPU 12% over the last 5 min · "));
        let r = Readings { cpu_window_ms: Some(45_000), ..r };
        assert!(MachineStatus::assess(&r, APP_ROW).line().starts_with("CPU 12% over the last 45 s · "));
    }

    #[test]
    fn a_snapshot_without_an_interval_has_no_cpu_figure() {
        use crate::system_monitor::*;
        let mut snap = SystemSnapshot {
            cpu: CpuInfo {
                overall_percent: 8.0,
                cores: vec![CpuCore { id: 0, usage_percent: 8.0 }, CpuCore { id: 1, usage_percent: 8.0 }],
                load_avg_1: 0.5,
                load_avg_5: 0.5,
                load_avg_15: 0.5,
                window_ms: None,
            },
            memory: MemoryInfo {
                total_bytes: 0,
                used_bytes: 0,
                usage_percent: 0.0,
                swap_total_bytes: 0,
                swap_used_bytes: 0,
                available_bytes: 0,
                cached_bytes: 0,
                buffers_bytes: 0,
            },
            disks: vec![],
            networks: vec![],
            uptime_secs: 0,
        };
        let r = Readings::from_snapshot(&snap);
        assert_eq!(r.cpu_percent, None, "the 8% is a since-boot average, not a reading");
        assert_eq!(r.cores, 2);
        snap.cpu.window_ms = Some(2000);
        assert_eq!(Readings::from_snapshot(&snap).cpu_percent, Some(8.0));
    }

    #[test]
    fn the_answer_for_a_caller_carries_the_row_and_the_limits() {
        let r = Readings { disks: vec![disk("/", 31.3, 29.6)], ..calm() };
        let j = MachineStatus::assess(&r, APP_ROW).json();
        assert_eq!(j["status"], "Disk nearly full · 1.7 GB free (95%) · CPU 8% · Memory 50% · Swap none");
        assert_eq!(j["limits_reached"][0]["limit"], "disk");
        assert_eq!(j["limits_reached"][0]["percent"], 95.0);
        assert_eq!(j["limits_reached"][0]["threshold_percent"], 90.0);
        let text = j.to_string().to_lowercase();
        assert!(!text.contains("healthy") && !text.contains("nominal"));
    }

    #[test]
    fn only_real_filesystems_count_as_disks() {
        assert!(is_real_filesystem("ext4"));
        assert!(is_real_filesystem("btrfs"));
        for pseudo in ["squashfs", "iso9660", "tmpfs", "overlay", "proc"] {
            assert!(!is_real_filesystem(pseudo), "{pseudo}");
        }
    }
}
