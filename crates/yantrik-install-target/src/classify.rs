//! What each partition is, and whether Yantrik OS may be installed into it.
//!
//! Only two kinds of partition can ever be the root: an empty one, and a placeholder made for
//! the purpose (FAT or exFAT labelled `YANTRIK` or `YANTRIK-*`, which is what macOS's Disk
//! Utility makes when the APFS container is shrunk). Everything else is kept, and a kind is
//! read from the partition type and the filesystem both: a partition typed APFS is macOS's even
//! when nothing on it can be probed.

use crate::table::{human, DiskTable, Part, Run};

/// The smallest root Yantrik OS is installed into: 20 GB.
pub const MIN_BYTES: u64 = 20_000_000_000;
/// Free space smaller than this is not drawn at all: GPT alignment leaves gaps of a few MB
/// between partitions, and macOS leaves 128 MiB ones.
const DRAWN_FREE_BYTES: u64 = 1_000_000_000;

/// GPT partition types, lowercase.
pub mod guid {
    pub const ESP: &str = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";
    pub const APFS: &str = "7c3457ef-0000-11aa-aa11-00306543ecac";
    pub const HFS_PLUS: &str = "48465300-0000-11aa-aa11-00306543ecac";
    /// Every Apple type (boot, RAID, Core Storage, label) ends this way.
    pub const APPLE_SUFFIX: &str = "-11aa-aa11-00306543ecac";
    pub const MS_BASIC_DATA: &str = "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7";
    pub const MS_RESERVED: &str = "e3c9e316-0b5c-4db8-817d-f92df00215ae";
    pub const WINDOWS_RECOVERY: &str = "de94bba4-06d1-4d40-a16a-bfd50179d6ac";
    pub const LINUX_FS: &str = "0fc63daf-8483-4772-8e79-3d69d8477de4";
    pub const LINUX_SWAP: &str = "0657fd6d-a4ab-43c4-84e5-0933c84b4f4f";
    pub const LINUX_LVM: &str = "e6d6d379-f507-44c2-a23c-238f2a3df928";
    pub const LINUX_RAID: &str = "a19d880f-05fc-4d3b-a006-743f0f84911e";
    pub const LINUX_LUKS: &str = "ca7d7ccb-63ed-4c53-861c-1742536059cc";
    pub const LINUX_ROOT_X86_64: &str = "4f68bce3-e8cd-4db1-96e7-fbcaf984b709";
    pub const LINUX_HOME: &str = "933ac7e1-2eb4-4f13-b844-0e14e2aef915";
    pub const BIOS_BOOT: &str = "21686148-6449-6e6f-744e-656564454649";
}

/// What a partition is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// The EFI system partition: Yantrik's boot loader goes beside what is there.
    Esp,
    Apfs,
    HfsPlus,
    /// Another Apple partition: a Recovery HD, Core Storage, an Apple RAID member.
    AppleOther,
    Windows,
    /// A Linux filesystem, swap, LVM, RAID or LUKS: it may hold someone's data.
    LinuxData(String),
    /// FAT or exFAT labelled YANTRIK or YANTRIK-*: made to be installed over.
    Placeholder,
    /// Nothing on it that blkid recognises, typed as plain data.
    Unformatted,
    /// What the installer itself is running from.
    InstallMedium,
    /// Anything else: a filesystem that is not a placeholder, a type we do not know.
    Other(String),
}

impl Kind {
    /// A word for the screens and the control surface.
    pub fn name(&self) -> &'static str {
        match self {
            Kind::Esp => "esp",
            Kind::Apfs | Kind::HfsPlus | Kind::AppleOther => "macos",
            Kind::Windows => "windows",
            Kind::LinuxData(_) => "linux",
            Kind::Placeholder => "placeholder",
            Kind::Unformatted => "unformatted",
            Kind::InstallMedium => "medium",
            Kind::Other(_) => "other",
        }
    }

    /// A kind the installer never installs over, whatever its size or state.
    pub fn kept(&self) -> bool {
        !matches!(self, Kind::Placeholder | Kind::Unformatted)
    }
}

pub fn is_esp(p: &Part) -> bool {
    p.type_uuid == guid::ESP || p.flags.iter().any(|f| f == "esp")
}

/// `YANTRIK` or `YANTRIK-anything`, in any case (an exFAT label keeps its case; FAT's is upper).
pub fn is_placeholder_label(label: &str) -> bool {
    let upper = label.trim().to_ascii_uppercase();
    upper == "YANTRIK" || (upper.starts_with("YANTRIK-") && upper.len() > "YANTRIK-".len())
}

const LINUX_FS: &[&str] = &[
    "ext2", "ext3", "ext4", "xfs", "btrfs", "f2fs", "jfs", "reiserfs", "bcachefs", "swap",
    "crypto_LUKS", "LVM2_member", "linux_raid_member", "zfs_member",
];

/// What `p` is, from its GPT type and its filesystem.
pub fn kind_of(p: &Part) -> Kind {
    let t = p.type_uuid.as_str();
    let fs = p.fstype.as_str();
    if fs == "iso9660" || p.mountpoint == "/run/live/medium" {
        return Kind::InstallMedium;
    }
    if is_esp(p) {
        return Kind::Esp;
    }
    if t == guid::APFS || fs == "apfs" {
        return Kind::Apfs;
    }
    if t == guid::HFS_PLUS || fs == "hfsplus" || fs == "hfs" {
        return Kind::HfsPlus;
    }
    if t.ends_with(guid::APPLE_SUFFIX) {
        return Kind::AppleOther;
    }
    if fs == "ntfs" || fs == "BitLocker" || t == guid::MS_RESERVED || t == guid::WINDOWS_RECOVERY {
        return Kind::Windows;
    }
    if LINUX_FS.contains(&fs) {
        return Kind::LinuxData(fs.to_string());
    }
    if [guid::LINUX_SWAP, guid::LINUX_LVM, guid::LINUX_RAID, guid::LINUX_LUKS, guid::BIOS_BOOT].contains(&t) {
        return Kind::LinuxData(if t == guid::BIOS_BOOT { "BIOS boot".into() } else { "Linux".into() });
    }
    if (fs == "vfat" || fs == "exfat") && is_placeholder_label(&p.label) {
        return Kind::Placeholder;
    }
    if fs.is_empty() {
        // Only the plain data types: an empty partition of a type something else owns (a
        // Linux root or /home with a filesystem blkid could not read) is not empty.
        if t == guid::MS_BASIC_DATA || t == guid::LINUX_FS {
            return Kind::Unformatted;
        }
        if [guid::LINUX_ROOT_X86_64, guid::LINUX_HOME].contains(&t) {
            return Kind::LinuxData("Linux".into());
        }
        return Kind::Other(if t.is_empty() { "an unknown partition type".into() } else { format!("partition type {t}") });
    }
    Kind::Other(fs.to_string())
}

fn fs_word(fs: &str) -> &str {
    match fs {
        "vfat" => "FAT32",
        "exfat" => "exFAT",
        "ntfs" => "NTFS",
        other => other,
    }
}

/// How a partition is named on the Disk screen's bar.
pub fn title(p: &Part, kind: &Kind) -> String {
    let labelled = |what: String| if p.label.is_empty() { what } else { format!("{} ({what})", p.label) };
    match kind {
        Kind::Esp => "EFI system".into(),
        Kind::Apfs => "macOS (APFS)".into(),
        Kind::HfsPlus => "macOS (HFS+)".into(),
        Kind::AppleOther => if p.name.is_empty() { "macOS".into() } else { format!("macOS ({})", p.name) },
        Kind::Windows => labelled("Windows".into()),
        Kind::LinuxData(fs) => labelled(format!("Linux, {fs}")),
        Kind::Placeholder => labelled(fs_word(&p.fstype).to_string()),
        Kind::Unformatted => "Empty partition".into(),
        Kind::InstallMedium => labelled("this installer".into()),
        Kind::Other(what) => labelled(fs_word(what).to_string()),
    }
}

/// Why the disk as a whole cannot take an install into one of its partitions.
pub fn disk_problem(t: &DiskTable, efi_boot: bool) -> Option<String> {
    if !efi_boot {
        return Some("this computer started in BIOS mode; installing beside another system needs UEFI".into());
    }
    if t.label != "gpt" {
        return Some(if t.label == "unknown" || t.label.is_empty() {
            format!("{} has no partition table", t.path)
        } else {
            format!("{} has an {} partition table; installing into a partition needs GPT", t.path, t.label)
        });
    }
    let Some(esp) = t.esp() else {
        return Some(format!("{} has no EFI system partition to boot from", t.path));
    };
    if esp.fstype != "vfat" {
        let found = if esp.fstype.is_empty() { "nothing readable".to_string() } else { esp.fstype.clone() };
        return Some(format!(
            "the EFI system partition {} holds {found}, not FAT; it is never formatted, so it cannot be used",
            esp.path
        ));
    }
    None
}

/// Why `p` cannot be the root, or `None` when it can.
pub fn partition_problem(t: &DiskTable, p: &Part, efi_boot: bool) -> Option<String> {
    if let Some(why) = disk_problem(t, efi_boot) {
        return Some(why);
    }
    let kind = kind_of(p);
    let path = &p.path;
    let refused = match &kind {
        Kind::Esp => Some(format!(
            "{path} is the EFI system partition; Yantrik OS puts its boot loader beside what is there, and never installs into it or formats it"
        )),
        Kind::Apfs => Some(format!("{path} holds macOS (APFS); it is kept")),
        Kind::HfsPlus => Some(format!("{path} holds macOS (HFS+); it is kept")),
        Kind::AppleOther => Some(format!("{path} is part of macOS; it is kept")),
        Kind::Windows => Some(format!("{path} holds Windows; it is kept")),
        Kind::LinuxData(what) => Some(format!("{path} holds a Linux filesystem ({what}) that may have data; it is kept")),
        Kind::InstallMedium => Some(format!("{path} is what this installer is running from")),
        Kind::Other(what) => Some(format!(
            "{path} holds {}{}; only an empty partition, or a FAT or exFAT placeholder labelled YANTRIK, can be installed into",
            fs_word(what),
            if p.label.is_empty() { String::new() } else { format!(" labelled {}", p.label) }
        )),
        Kind::Placeholder | Kind::Unformatted => None,
    };
    if refused.is_some() {
        return refused;
    }
    if !p.mountpoint.is_empty() {
        return Some(format!("{path} is in use (mounted at {}); unmount it first", p.mountpoint));
    }
    let bytes = t.bytes(p.run.sectors());
    if bytes < MIN_BYTES {
        return Some(format!("{path} is {}; Yantrik OS needs at least {}", human(bytes), human(MIN_BYTES)));
    }
    None
}

/// The part of a free run a partition is made in: from the first MiB boundary to the last one,
/// and never into the space GPT keeps at the end of the disk. `None` when nothing is left.
pub fn aligned(t: &DiskTable, run: &Run) -> Option<Run> {
    let a = t.align();
    let start = run.start.max(a).div_ceil(a) * a;
    let end = run.end.min(t.last_usable());
    let end = ((end + 1) / a * a).checked_sub(1)?;
    (end > start).then_some(Run { start, end })
}

/// Why free space `run` (as parted showed it, or any part of it) cannot take the install.
pub fn free_problem(t: &DiskTable, run: &Run, efi_boot: bool) -> Option<String> {
    if let Some(why) = disk_problem(t, efi_boot) {
        return Some(why);
    }
    let usable = aligned(t, run).map(|r| t.bytes(r.sectors())).unwrap_or(0);
    if usable < MIN_BYTES {
        return Some(format!(
            "the free space at sectors {}-{} is {}; Yantrik OS needs at least {}",
            run.start, run.end, human(usable), human(MIN_BYTES)
        ));
    }
    None
}

/// One stretch of a disk as the Disk screen draws it: a partition or a run of free space.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// What the installer is handed to choose it: `sda3`, or `sda@<start>-<end>` for free space.
    pub id: String,
    pub disk: String,
    /// [`Kind::name`], or "free".
    pub kind: String,
    pub title: String,
    pub bytes: u64,
    pub size: String,
    /// Its share of the disk, 0 to 1.
    pub share: f32,
    /// Never installed over, whatever happens: macOS, Windows, the EFI partition, data.
    pub kept: bool,
    pub eligible: bool,
    /// Why it cannot be chosen; empty when it can.
    pub reason: String,
    /// What the Disk screen says once it is chosen.
    pub sentence: String,
    /// `/dev/sda3`; empty for free space.
    pub device: String,
    pub start: u64,
    pub end: u64,
}

fn sentence(what: &str, bytes: u64) -> String {
    format!("Yantrik OS will be installed into {what} ({}). Nothing else on this disk changes.", human(bytes))
}

/// The disk's partitions and free space in order, each with whether it can be chosen.
pub fn segments(t: &DiskTable, efi_boot: bool) -> Vec<Segment> {
    let disk = t.name().to_string();
    let share = |sectors: u64| sectors as f32 / t.sectors.max(1) as f32;
    let mut out: Vec<Segment> = Vec::new();
    for p in &t.parts {
        let kind = kind_of(p);
        let bytes = t.bytes(p.run.sectors());
        let reason = partition_problem(t, p, efi_boot).unwrap_or_default();
        out.push(Segment {
            id: crate::plan::target_id(&disk, &crate::plan::TargetSpec::Partition(p.number)),
            disk: disk.clone(),
            kind: kind.name().into(),
            title: title(p, &kind),
            bytes,
            size: human(bytes),
            share: share(p.run.sectors()),
            kept: kind.kept(),
            eligible: reason.is_empty(),
            sentence: sentence(&p.path, bytes),
            reason,
            device: p.path.clone(),
            start: p.run.start,
            end: p.run.end,
        });
    }
    for run in &t.free {
        let bytes = t.bytes(run.sectors());
        if bytes < DRAWN_FREE_BYTES {
            continue;
        }
        let usable = aligned(t, run).map(|r| t.bytes(r.sectors())).unwrap_or(0);
        let reason = free_problem(t, run, efi_boot).unwrap_or_default();
        out.push(Segment {
            id: crate::plan::target_id(&disk, &crate::plan::TargetSpec::Free { start: run.start, end: run.end }),
            disk: disk.clone(),
            kind: "free".into(),
            title: "Free space".into(),
            bytes,
            size: human(bytes),
            share: share(run.sectors()),
            kept: false,
            eligible: reason.is_empty(),
            sentence: sentence(&format!("a new partition in the free space on {}", t.path), usable),
            reason,
            device: String::new(),
            start: run.start,
            end: run.end,
        });
    }
    out.sort_by_key(|s| s.start);
    out
}

/// The target chosen before anyone chooses: a placeholder made for Yantrik (the person said so
/// by labelling it), or else the largest free run on a disk that keeps another system. `None`
/// otherwise, and the Disk screen offers erasing a disk as it always did.
pub fn preselect(segments: &[Segment]) -> Option<String> {
    if let Some(s) = segments.iter().find(|s| s.eligible && s.kind == "placeholder") {
        return Some(s.id.clone());
    }
    let keeps_a_system = |disk: &str| {
        segments.iter().any(|s| s.disk == disk && s.kept && matches!(s.kind.as_str(), "macos" | "windows" | "linux"))
    };
    segments
        .iter()
        .filter(|s| s.eligible && s.kind == "free" && keeps_a_system(&s.disk))
        .max_by_key(|s| s.bytes)
        .map(|s| s.id.clone())
}
