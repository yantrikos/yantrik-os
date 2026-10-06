//! What each partition is, and whether Yantrik OS may be installed into it.
//!
//! Only two kinds of partition can ever be the root: an empty one, and a placeholder made for
//! the purpose (FAT or exFAT labelled exactly `YANTRIK`, which is what macOS's Disk Utility
//! makes when the APFS container is shrunk). Both only once the disk itself was read and showed
//! them empty (`table::Checked::Empty`): a table parsed from text, or a partition that could not
//! be read, offers nothing. Everything else is kept, and a kind is read from the partition type
//! and the filesystem both: a partition typed APFS is macOS's even when nothing on it can be
//! probed.

use crate::table::{human, Checked, DiskTable, Part, Run};

pub use crate::segment::{preselect, segments, Segment};

/// The smallest root Yantrik OS is installed into: 20 GB.
pub const MIN_BYTES: u64 = 20_000_000_000;
/// The least room the EFI system partition must have left for Yantrik's boot loader: 32 MB.
pub const ESP_MIN_FREE: u64 = 32_000_000;
/// The only label a placeholder carries.
pub const PLACEHOLDER_LABEL: &str = "YANTRIK";
/// What [`Kind::Other`] says of a partition that could not be shown to be empty.
pub const UNREADABLE: &str = "unreadable";

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
    /// FAT or exFAT labelled exactly YANTRIK, read and found empty: made to be installed over.
    Placeholder,
    /// Nothing on it at all, read and found so, typed as plain data.
    Unformatted,
    /// What the installer itself is running from.
    InstallMedium,
    /// Anything else: a filesystem that is not a placeholder, a type we do not know, a
    /// placeholder holding files, or a partition that could not be shown empty ([`UNREADABLE`]).
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

/// Exactly `YANTRIK`. Not `YANTRIK-backup`, not `Yantrik-Photos`, not the installer's own
/// `YANTRIK-INS`: a label that merely starts the same way may be someone's data.
pub fn is_placeholder_label(label: &str) -> bool {
    label == PLACEHOLDER_LABEL
}

/// Whether a partition has an Apple GPT type: APFS, HFS+, or any other of Apple's.
pub fn is_apple_type(p: &Part) -> bool {
    p.type_uuid.ends_with(guid::APPLE_SUFFIX)
}

const LINUX_FS: &[&str] = &[
    "ext2", "ext3", "ext4", "xfs", "btrfs", "f2fs", "jfs", "reiserfs", "bcachefs", "swap",
    "crypto_LUKS", "LVM2_member", "linux_raid_member", "zfs_member",
];

/// What `p` is, from its GPT type, its filesystem and what reading it found.
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
        // Labelled for Yantrik, but only empty is a placeholder: what is on it was looked at.
        return match &p.checked {
            Checked::Empty => Kind::Placeholder,
            Checked::Holds(_) => Kind::Other(fs.to_string()),
            Checked::NotLooked | Checked::Unreadable(_) => Kind::Other(UNREADABLE.into()),
        };
    }
    if fs.is_empty() {
        // Only the plain data types: an empty partition of a type something else owns (a
        // Linux root or /home with a filesystem blkid could not read) is not empty.
        if t == guid::MS_BASIC_DATA || t == guid::LINUX_FS {
            return if p.checked == Checked::Empty { Kind::Unformatted } else { Kind::Other(UNREADABLE.into()) };
        }
        if [guid::LINUX_ROOT_X86_64, guid::LINUX_HOME].contains(&t) {
            return Kind::LinuxData("Linux".into());
        }
        return Kind::Other(if t.is_empty() { "an unknown partition type".into() } else { format!("partition type {t}") });
    }
    if let Checked::Unreadable(_) = p.checked {
        return Kind::Other(UNREADABLE.into());
    }
    Kind::Other(fs.to_string())
}

pub(crate) fn fs_word(fs: &str) -> &str {
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
    if t.guid.is_empty() {
        return Some(format!(
            "parted did not report the GUID of {}'s partition table, which is how the installer makes sure the table is never rewritten",
            t.path
        ));
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
    let Some(probed) = &t.probed else {
        return Some(format!("{} was not read closely enough to install beside what is on it", t.path));
    };
    if let Some(why) = &probed.mbr_problem {
        return Some(why.clone());
    }
    match probed.esp_free {
        None => Some(format!(
            "the EFI system partition {} could not be looked at, read-only, to see how much room it has",
            esp.path
        )),
        Some(free) if free < ESP_MIN_FREE => Some(format!(
            "the EFI system partition {} has {} free; Yantrik's boot loader needs {}",
            esp.path,
            human(free),
            human(ESP_MIN_FREE)
        )),
        Some(_) => None,
    }
}

/// The first few of a placeholder's files, for a sentence.
fn some_of(files: &[String]) -> String {
    let shown: Vec<&str> = files.iter().take(4).map(String::as_str).collect();
    let more = files.len().saturating_sub(shown.len());
    if more == 0 { shown.join(", ") } else { format!("{} and {more} more", shown.join(", ")) }
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
        Kind::Other(what) => Some(match &p.checked {
            Checked::Holds(files) if is_placeholder_label(&p.label) => format!(
                "{path} is labelled YANTRIK but holds files ({}); only an empty placeholder is installed into, so move them off it in macOS first",
                some_of(files)
            ),
            Checked::Unreadable(why) => format!("{path} could not be shown to be empty ({why}); it is kept"),
            _ if what == UNREADABLE => {
                format!("{path} was not looked inside, so it cannot be shown to be empty; it is kept")
            }
            _ => format!(
                "{path} holds {}{}; only an empty partition, or a FAT or exFAT placeholder labelled exactly YANTRIK, can be installed into",
                fs_word(what),
                if p.label.is_empty() { String::new() } else { format!(" labelled {}", p.label) }
            ),
        }),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fat(label: &str, checked: Checked) -> Part {
        Part {
            number: 4,
            run: Run { start: 2048, end: 100_000_000 },
            type_uuid: guid::MS_BASIC_DATA.into(),
            uuid: String::new(),
            name: String::new(),
            flags: vec![],
            path: "/dev/sda4".into(),
            fstype: "vfat".into(),
            label: label.into(),
            mountpoint: String::new(),
            checked,
        }
    }

    #[test]
    fn only_the_label_yantrik_itself_is_a_placeholder() {
        assert!(is_placeholder_label("YANTRIK"));
        for not in ["YANTRIK-backup", "Yantrik-Photos", "YANTRIK-INS", "yantrik", "Yantrik", "YANTRIK ", " YANTRIK", "YANTRIKS", ""] {
            assert!(!is_placeholder_label(not), "{not:?}");
            assert_ne!(kind_of(&fat(not, Checked::Empty)), Kind::Placeholder, "{not:?}");
        }
        assert_eq!(kind_of(&fat("YANTRIK", Checked::Empty)), Kind::Placeholder);
    }

    #[test]
    fn a_placeholder_nobody_looked_inside_or_that_holds_files_is_kept() {
        assert_eq!(kind_of(&fat("YANTRIK", Checked::NotLooked)), Kind::Other(UNREADABLE.into()));
        assert_eq!(kind_of(&fat("YANTRIK", Checked::Unreadable("mount failed".into()))), Kind::Other(UNREADABLE.into()));
        let holds = fat("YANTRIK", Checked::Holds(vec!["Photos/IMG_0001.HEIC".into()]));
        assert_eq!(kind_of(&holds), Kind::Other("vfat".into()));
        assert!(kind_of(&holds).kept());
    }

    #[test]
    fn an_empty_partition_is_empty_only_once_read_and_found_so() {
        let mut p = fat("", Checked::NotLooked);
        p.fstype.clear();
        assert_eq!(kind_of(&p), Kind::Other(UNREADABLE.into()), "parsed from text: nobody looked");
        p.checked = Checked::Unreadable("its first MiB is not blank".into());
        assert_eq!(kind_of(&p), Kind::Other(UNREADABLE.into()));
        p.checked = Checked::Empty;
        assert_eq!(kind_of(&p), Kind::Unformatted);
        // Typed as someone's Linux root: never empty, however blank it reads.
        p.type_uuid = guid::LINUX_ROOT_X86_64.into();
        assert_eq!(kind_of(&p).name(), "linux");
    }
}
