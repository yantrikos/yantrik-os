//! The plan for one target: the exact commands, and the guards every plan passes before anyone
//! sees it.
//!
//! A plan never contains `mklabel`. Free space gets one new partition (two when the root is
//! encrypted and needs a /boot beside it) with exact start and end sectors inside the run. A
//! placeholder is reformatted where it is, or, when encrypting, removed and remade as two
//! partitions inside exactly its old extent. The EFI system partition is never in a command
//! except to be read.

use crate::classify::{self, guid, Kind};
use crate::table::{partition_path, DiskTable, Run};

/// What the person chose on one disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetSpec {
    /// An existing partition, by number.
    Partition(u32),
    /// Free space, or part of it, in sectors, both ends included.
    Free { start: u64, end: u64 },
}

/// `sda3` for a partition, `sda@<start>-<end>` for free space: what the screens and the control
/// surface hold.
pub fn target_id(disk: &str, spec: &TargetSpec) -> String {
    let disk = crate::table::disk_name(disk);
    match spec {
        TargetSpec::Partition(n) => partition_path(disk, *n),
        TargetSpec::Free { start, end } => format!("{disk}@{start}-{end}"),
    }
}

/// The disk and the target in `sda3`, `/dev/nvme0n1p2` or `sda@2048-409599`.
pub fn parse_target_id(id: &str) -> Result<(String, TargetSpec), String> {
    let id = id.trim().trim_start_matches("/dev/");
    if let Some((disk, range)) = id.split_once('@') {
        let (s, e) = range
            .split_once('-')
            .ok_or_else(|| format!("`{id}`: free space is written disk@START-END, in sectors"))?;
        let parse = |v: &str| v.trim().trim_end_matches('s').parse::<u64>();
        let (Ok(start), Ok(end)) = (parse(s), parse(e)) else {
            return Err(format!("`{id}`: START and END are sector numbers"));
        };
        if disk.is_empty() || start > end {
            return Err(format!("`{id}` is not a run of sectors on a disk"));
        }
        return Ok((disk.to_string(), TargetSpec::Free { start, end }));
    }
    let digits = id.len() - id.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    let (prefix, number) = id.split_at(id.len() - digits);
    // nvme0n1p3 → nvme0n1; sda3 → sda. A name that is all disk (sda, nvme0n1) has no partition.
    let disk = match prefix.strip_suffix('p') {
        Some(d) if d.ends_with(|c: char| c.is_ascii_digit()) => d,
        _ => prefix,
    };
    // Disks whose own names end in a digit take a `p` before the partition's number, so
    // `nvme0n1` and `loop0` are whole disks, not partition 1 of `nvme0n` or `loop`.
    let numbered_family = ["nvme", "mmcblk", "loop", "nbd", "md"].iter().any(|f| disk.starts_with(f));
    let whole = prefix.ends_with(|c: char| c.is_ascii_digit())
        || number.is_empty()
        || (numbered_family && !disk.ends_with(|c: char| c.is_ascii_digit()));
    if disk.is_empty() || whole {
        return Err(format!(
            "`{id}` is a whole disk; name a partition (sda3, nvme0n1p3) or free space (sda@START-END)"
        ));
    }
    let n = number.parse().map_err(|_| format!("`{id}`: no partition number"))?;
    Ok((disk.to_string(), TargetSpec::Partition(n)))
}

/// A partition a plan refers to: one that exists now, or one it makes, known by its first
/// sector until the table has been read again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartRef {
    Existing(u32),
    StartingAt(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Root,
    /// /boot, outside the encryption, when the root is encrypted.
    Boot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// `parted -s -a none <disk> unit s mkpart <name> ext4 <start>s <end>s`.
    MkPart { role: Role, start: u64, end: u64 },
    /// `parted -s <disk> rm <n>`: only the chosen placeholder, only to remake it as two.
    Remove(u32),
    /// `wipefs -a <partition>`: only the chosen placeholder or empty partition.
    Wipe(u32),
    /// `parted -s <disk> type <n> <Linux filesystem>`: the chosen partition, so macOS and
    /// Windows read it as Linux's. Best effort (parted 3.6 and later).
    SetLinuxType(u32),
    /// Make the filesystem (or, for an encrypted root, the LUKS2 container). The installer
    /// runs these itself; they are here so the whole plan can be read and checked.
    Format { part: PartRef, role: Role, encrypted: bool },
}

/// Everything installing into one target will do to the disk.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// `/dev/sda`.
    pub disk: String,
    /// The table this plan was made against ([`DiskTable::fingerprint`]).
    pub fingerprint: String,
    pub spec: TargetSpec,
    pub encrypt: bool,
    /// Where every change happens: the free run's aligned part, or the placeholder's extent.
    pub region: Run,
    /// The EFI system partition the boot loader goes beside. Mounted, never formatted.
    pub esp: u32,
    pub esp_path: String,
    pub steps: Vec<Step>,
    pub root: PartRef,
    pub boot: Option<PartRef>,
    pub sentence: String,
}

/// /boot, when the root is encrypted: 1 GiB, as on a whole-disk install.
pub const BOOT_BYTES: u64 = 1024 * 1024 * 1024;
pub const ROOT_LABEL: &str = "YANTRIK";
pub const BOOT_LABEL: &str = "YANTRIK_BOOT";

fn part_name(role: Role) -> &'static str {
    match role {
        Role::Root => "yantrik-root",
        Role::Boot => "yantrik-boot",
    }
}

/// The plan for installing into `spec` on the disk `t` describes. Refused when the table is not
/// the one the person was shown (`shown`, a [`DiskTable::fingerprint`]), when the target may not
/// be installed into, or when the plan would break any guard in [`check`].
pub fn plan(t: &DiskTable, spec: TargetSpec, encrypt: bool, efi_boot: bool, shown: &str) -> Result<Plan, String> {
    let now = t.fingerprint();
    if now != shown {
        return Err(format!(
            "the partition table on {} has changed since it was shown (shown {shown}, now {now}); \
             nothing was written. Look at the disk again and choose again",
            t.path
        ));
    }
    if let Some(why) = classify::disk_problem(t, efi_boot) {
        return Err(why);
    }
    let esp = t.esp().ok_or_else(|| format!("{} has no EFI system partition", t.path))?;
    let a = t.align();
    let boot_sectors = (BOOT_BYTES / t.sector_size).div_ceil(a) * a;
    let (region, steps, root, boot) = match spec {
        TargetSpec::Partition(n) => {
            let p = t.part(n).ok_or_else(|| format!("{} has no partition {n}", t.path))?;
            if let Some(why) = classify::partition_problem(t, p, efi_boot) {
                return Err(why);
            }
            let region = p.run;
            if encrypt {
                let root_start = (region.start + boot_sectors).div_ceil(a) * a;
                if root_start >= region.end {
                    return Err(format!("{} is too small for /boot and an encrypted root", p.path));
                }
                // Wiped first, so the placeholder's FAT signature does not linger at the start
                // of the /boot made over it.
                let steps = vec![
                    Step::Wipe(n),
                    Step::Remove(n),
                    Step::MkPart { role: Role::Boot, start: region.start, end: root_start - 1 },
                    Step::MkPart { role: Role::Root, start: root_start, end: region.end },
                    Step::Format { part: PartRef::StartingAt(region.start), role: Role::Boot, encrypted: false },
                    Step::Format { part: PartRef::StartingAt(root_start), role: Role::Root, encrypted: true },
                ];
                (region, steps, PartRef::StartingAt(root_start), Some(PartRef::StartingAt(region.start)))
            } else {
                let steps = vec![
                    Step::Wipe(n),
                    Step::SetLinuxType(n),
                    Step::Format { part: PartRef::Existing(n), role: Role::Root, encrypted: false },
                ];
                (region, steps, PartRef::Existing(n), None)
            }
        }
        TargetSpec::Free { start, end } => {
            let asked = Run { start, end };
            if start > end || !t.free.iter().any(|r| r.contains(&asked)) {
                let runs: Vec<String> = t.free.iter().map(|r| format!("{}-{}", r.start, r.end)).collect();
                return Err(format!(
                    "sectors {start}-{end} are not free space on {}; its free runs are {}",
                    t.path,
                    if runs.is_empty() { "none".into() } else { runs.join(", ") }
                ));
            }
            if let Some(why) = classify::free_problem(t, &asked, efi_boot) {
                return Err(why);
            }
            let region = classify::aligned(t, &asked).ok_or("no aligned space is left in that run")?;
            if encrypt {
                let root_start = region.start + boot_sectors;
                let steps = vec![
                    Step::MkPart { role: Role::Boot, start: region.start, end: root_start - 1 },
                    Step::MkPart { role: Role::Root, start: root_start, end: region.end },
                    Step::Format { part: PartRef::StartingAt(region.start), role: Role::Boot, encrypted: false },
                    Step::Format { part: PartRef::StartingAt(root_start), role: Role::Root, encrypted: true },
                ];
                (region, steps, PartRef::StartingAt(root_start), Some(PartRef::StartingAt(region.start)))
            } else {
                let steps = vec![
                    Step::MkPart { role: Role::Root, start: region.start, end: region.end },
                    Step::Format { part: PartRef::StartingAt(region.start), role: Role::Root, encrypted: false },
                ];
                (region, steps, PartRef::StartingAt(region.start), None)
            }
        }
    };
    let what = match spec {
        TargetSpec::Partition(n) => partition_path(&t.path, n),
        TargetSpec::Free { .. } => format!("a new partition in the free space on {}", t.path),
    };
    let plan = Plan {
        disk: t.path.clone(),
        fingerprint: now,
        spec,
        encrypt,
        region,
        esp: esp.number,
        esp_path: esp.path.clone(),
        steps,
        root,
        boot,
        sentence: format!(
            "Yantrik OS will be installed into {what} ({}). Nothing else on this disk changes.",
            crate::table::human(t.bytes(region.sectors()))
        ),
    };
    check(&plan, t)?;
    Ok(plan)
}

/// The guards, checked over every plan against the table it was made for. Independent of how
/// the plan was put together, so a mistake in [`plan`] is caught here rather than on a disk.
pub fn check(plan: &Plan, t: &DiskTable) -> Result<(), String> {
    let fail = |why: String| Err(format!("refusing a plan that would {why}"));
    if plan.disk != t.path {
        return fail(format!("write to {} while planned for {}", t.path, plan.disk));
    }
    if plan.commands().iter().any(|c| c.iter().any(|w| w == "mklabel")) {
        return fail("rewrite the partition table".into());
    }
    let target = match plan.spec {
        TargetSpec::Partition(n) => {
            let Some(p) = t.part(n) else { return fail(format!("touch a partition {n} that does not exist")) };
            if !matches!(classify::kind_of(p), Kind::Placeholder | Kind::Unformatted) {
                return fail(format!("install over {} ({})", p.path, classify::kind_of(p).name()));
            }
            if plan.region != p.run {
                return fail(format!("reach outside {}", p.path));
            }
            Some(n)
        }
        TargetSpec::Free { .. } => {
            if !t.free.iter().any(|r| r.contains(&plan.region)) {
                return fail(format!("write outside free space (sectors {}-{})", plan.region.start, plan.region.end));
            }
            None
        }
    };
    // Nothing but the target is inside the region.
    if let Some(p) = t.parts.iter().find(|p| Some(p.number) != target && p.run.overlaps(&plan.region)) {
        return fail(format!("overlap {}", p.path));
    }
    if plan.region.end > t.last_usable() || plan.region.start < t.first_usable() {
        return fail("write into the space GPT keeps for itself".into());
    }
    let mut made: Vec<Run> = Vec::new();
    let mut removed = false;
    for step in &plan.steps {
        match *step {
            Step::MkPart { start, end, .. } => {
                let run = Run { start, end };
                if start > end || !plan.region.contains(&run) {
                    return fail(format!("make a partition at {start}-{end}, outside {}-{}", plan.region.start, plan.region.end));
                }
                if made.iter().any(|m| m.overlaps(&run)) {
                    return fail("make two partitions that overlap".into());
                }
                // A partition is made only where nothing is, or where the target was removed.
                if target.is_some() && !removed {
                    return fail("make a partition over one that is still there".into());
                }
                made.push(run);
            }
            Step::Remove(n) | Step::Wipe(n) | Step::SetLinuxType(n) => {
                if Some(n) != target {
                    return fail(format!("change partition {n}, which was not chosen"));
                }
                if matches!(step, Step::Remove(_)) {
                    removed = true;
                } else if removed {
                    return fail(format!("change partition {n} after removing it"));
                }
            }
            Step::Format { part, .. } => match part {
                PartRef::Existing(n) if Some(n) != target || n == plan.esp => {
                    return fail(format!("format partition {n}, which was not chosen"));
                }
                PartRef::StartingAt(s) if !made.iter().any(|m| m.start == s) => {
                    return fail(format!("format something at sector {s} that the plan did not make"));
                }
                _ => {}
            },
        }
    }
    if t.part(plan.esp).map(|p| p.type_uuid != guid::ESP && !p.flags.iter().any(|f| f == "esp")).unwrap_or(true) {
        return fail("boot from a partition that is not the EFI system partition".into());
    }
    let roots = plan.steps.iter().filter(|s| matches!(s, Step::Format { role: Role::Root, .. })).count();
    if roots != 1 {
        return fail(format!("format {roots} roots"));
    }
    Ok(())
}

impl Plan {
    /// The device a reference names: an existing partition's path, or, for one the plan makes,
    /// what `made` says is at that sector (`None` before the table has been read again).
    pub fn device(&self, r: PartRef, made: &dyn Fn(u64) -> Option<String>) -> Option<String> {
        match r {
            PartRef::Existing(n) => Some(partition_path(&self.disk, n)),
            PartRef::StartingAt(s) => made(s),
        }
    }

    fn argv(&self, step: &Step, made: &dyn Fn(u64) -> Option<String>) -> Vec<String> {
        let disk = self.disk.clone();
        let words = |w: &[&str]| w.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        match *step {
            Step::MkPart { role, start, end } => {
                let mut v = words(&["parted", "-s", "-a", "none", &disk, "unit", "s", "mkpart", part_name(role), "ext4"]);
                v.push(format!("{start}s"));
                v.push(format!("{end}s"));
                v
            }
            Step::Remove(n) => words(&["parted", "-s", &disk, "rm", &n.to_string()]),
            Step::Wipe(n) => words(&["wipefs", "-a", &partition_path(&disk, n)]),
            Step::SetLinuxType(n) => words(&["parted", "-s", &disk, "type", &n.to_string(), guid::LINUX_FS]),
            Step::Format { part, role, encrypted } => {
                let dev = self.device(part, made).unwrap_or_else(|| match part {
                    PartRef::StartingAt(s) => format!("<the new partition at sector {s}>"),
                    PartRef::Existing(n) => partition_path(&disk, n),
                });
                match (role, encrypted) {
                    (Role::Root, true) => words(&["cryptsetup", "luksFormat", "--type", "luks2", &dev]),
                    (Role::Root, false) => words(&["mkfs.ext4", "-q", "-F", "-L", ROOT_LABEL, &dev]),
                    (Role::Boot, _) => words(&["mkfs.ext4", "-q", "-F", "-L", BOOT_LABEL, &dev]),
                }
            }
        }
    }

    /// Every command, in order, for reading and for the tests.
    pub fn commands(&self) -> Vec<Vec<String>> {
        self.steps.iter().map(|s| self.argv(s, &|_| None)).collect()
    }

    /// The commands that change the table or wipe the target: what [`crate::apply::apply`] runs.
    /// `SetLinuxType` is flagged best effort.
    pub fn partition_commands(&self) -> Vec<(Vec<String>, bool)> {
        self.steps
            .iter()
            .filter(|s| !matches!(s, Step::Format { .. }))
            .map(|s| (self.argv(s, &|_| None), !matches!(s, Step::SetLinuxType(_))))
            .collect()
    }

    /// The format commands with the real devices, once the table has been read again.
    pub fn format_commands(&self, made: &dyn Fn(u64) -> Option<String>) -> Vec<Vec<String>> {
        self.steps
            .iter()
            .filter(|s| matches!(s, Step::Format { .. }))
            .map(|s| self.argv(s, made))
            .collect()
    }
}
