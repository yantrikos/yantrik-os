//! The Disk screen's view of a disk: each partition and run of free space in order, whether it
//! may be chosen, and what is chosen before anyone chooses.

use crate::classify::{self, Kind};
use crate::table::{human, DiskTable, Part};

/// Free space smaller than this is not drawn at all: GPT alignment leaves gaps of a few MB
/// between partitions, and macOS leaves 128 MiB ones.
const DRAWN_FREE_BYTES: u64 = 1_000_000_000;

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
    /// What the Disk screen says about it: once chosen, what installing does; for one that
    /// cannot be chosen, why not (the same words as `reason`), never an install sentence.
    pub sentence: String,
    /// What an approval card says installing into it does, naming the device and what is on
    /// it: "Install into /dev/sda4 (199 GB, YANTRIK, FAT32); nothing else changes". For one that
    /// cannot be chosen, its refusal, as `sentence`.
    pub card: String,
    /// `/dev/sda3`; empty for free space.
    pub device: String,
    /// On a disk known to be neither on USB nor removable.
    pub internal: bool,
    pub start: u64,
    pub end: u64,
}

/// What installing does, for what may be chosen; for what may not, why not. A refused
/// segment never carries words that read as though it would be installed into.
fn or_refusal(refused: &str, install: impl FnOnce() -> String) -> String {
    if refused.is_empty() { install() } else { refused.to_string() }
}

fn sentence(what: &str, bytes: u64) -> String {
    format!("Yantrik OS will be installed into {what} ({}). Nothing else on this disk changes.", human(bytes))
}

/// What is on a partition, for the approval card: its label and filesystem for a placeholder.
fn contents(p: &Part, kind: &Kind) -> String {
    match kind {
        Kind::Placeholder => format!("{}, {}", p.label, classify::fs_word(&p.fstype)),
        Kind::Unformatted => "empty".into(),
        other => classify::title(p, other),
    }
}

/// The disk's partitions and free space in order, each with whether it can be chosen.
pub fn segments(t: &DiskTable, efi_boot: bool) -> Vec<Segment> {
    let disk = t.name().to_string();
    let share = |sectors: u64| sectors as f32 / t.sectors.max(1) as f32;
    let internal = t.probed.as_ref().is_some_and(|p| !p.external);
    let mut out: Vec<Segment> = Vec::new();
    for p in &t.parts {
        let kind = classify::kind_of(p);
        let bytes = t.bytes(p.run.sectors());
        let reason = classify::partition_problem(t, p, efi_boot).unwrap_or_default();
        out.push(Segment {
            id: crate::plan::target_id(&disk, &crate::plan::TargetSpec::Partition(p.number)),
            disk: disk.clone(),
            kind: kind.name().into(),
            title: classify::title(p, &kind),
            bytes,
            size: human(bytes),
            share: share(p.run.sectors()),
            kept: kind.kept(),
            eligible: reason.is_empty(),
            sentence: or_refusal(&reason, || sentence(&p.path, bytes)),
            card: or_refusal(&reason, || {
                format!("Install into {} ({}, {}); nothing else changes", p.path, human(bytes), contents(p, &kind))
            }),
            reason,
            device: p.path.clone(),
            internal,
            start: p.run.start,
            end: p.run.end,
        });
    }
    for run in &t.free {
        let bytes = t.bytes(run.sectors());
        if bytes < DRAWN_FREE_BYTES {
            continue;
        }
        let usable = classify::aligned(t, run).map(|r| t.bytes(r.sectors())).unwrap_or(0);
        let reason = classify::free_problem(t, run, efi_boot).unwrap_or_default();
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
            sentence: or_refusal(&reason, || sentence(&format!("a new partition in the free space on {}", t.path), usable)),
            card: or_refusal(&reason, || {
                format!(
                    "Install into a new partition in the free space on {} ({}); nothing else changes",
                    t.path,
                    human(usable)
                )
            }),
            reason,
            device: String::new(),
            internal,
            start: run.start,
            end: run.end,
        });
    }
    out.sort_by_key(|s| s.start);
    out
}

/// The target chosen before anyone chooses, or `None`, and the Disk screen offers erasing a disk
/// as it always did.
///
/// A placeholder labelled YANTRIK, when there is exactly one on an internal disk that holds
/// macOS or the EFI partition (one beside macOS first): never on a USB or removable disk, which
/// may be someone's backup, and never a guess between two. Otherwise the largest free run on an
/// internal disk that keeps another system.
pub fn preselect(segments: &[Segment]) -> Option<String> {
    let disk_has = |disk: &str, kinds: &[&str]| segments.iter().any(|s| s.disk == disk && kinds.contains(&s.kind.as_str()));
    let placeholders: Vec<&Segment> = segments
        .iter()
        .filter(|s| s.eligible && s.internal && s.kind == "placeholder" && disk_has(&s.disk, &["macos", "esp"]))
        .collect();
    let beside_macos: Vec<&Segment> = placeholders.iter().copied().filter(|s| disk_has(&s.disk, &["macos"])).collect();
    match (beside_macos.as_slice(), placeholders.as_slice()) {
        ([one], _) | ([], [one]) => return Some(one.id.clone()),
        ([], []) => {}
        _ => return None,
    }
    segments
        .iter()
        .filter(|s| s.eligible && s.internal && s.kind == "free" && disk_has(&s.disk, &["macos", "windows", "linux"]))
        .max_by_key(|s| s.bytes)
        .map(|s| s.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(id: &str, disk: &str, kind: &str, eligible: bool, internal: bool) -> Segment {
        Segment {
            id: id.into(),
            disk: disk.into(),
            kind: kind.into(),
            title: String::new(),
            bytes: 100,
            size: String::new(),
            share: 0.1,
            kept: !eligible,
            eligible,
            reason: String::new(),
            sentence: String::new(),
            card: String::new(),
            device: String::new(),
            internal,
            start: 0,
            end: 0,
        }
    }

    #[test]
    fn a_placeholder_on_a_usb_disk_is_never_chosen_for_the_person() {
        let usb = [seg("sdb1", "sdb", "esp", false, false), seg("sdb2", "sdb", "placeholder", true, false)];
        assert_eq!(preselect(&usb), None);
        // Not known to be internal (the disk could not be asked) counts as USB.
        let internal = [seg("sda1", "sda", "esp", false, true), seg("sda2", "sda", "macos", false, true), seg("sda4", "sda", "placeholder", true, true)];
        assert_eq!(preselect(&internal).as_deref(), Some("sda4"));
        let both: Vec<Segment> = internal.iter().chain(usb.iter()).cloned().collect();
        assert_eq!(preselect(&both).as_deref(), Some("sda4"));
    }

    #[test]
    fn two_placeholders_beside_macos_are_a_choice_for_the_person() {
        let two = [
            seg("sda1", "sda", "esp", false, true),
            seg("sda2", "sda", "macos", false, true),
            seg("sda3", "sda", "placeholder", true, true),
            seg("sda4", "sda", "placeholder", true, true),
            seg("sda@1-2", "sda", "free", true, true),
        ];
        assert_eq!(preselect(&two), None, "and not the free space either");
        // One beside macOS wins over one on a second internal disk with only an EFI partition.
        let mut spread = two[..3].to_vec();
        spread.extend([seg("sdc1", "sdc", "esp", false, true), seg("sdc2", "sdc", "placeholder", true, true)]);
        assert_eq!(preselect(&spread).as_deref(), Some("sda3"));
    }
}
