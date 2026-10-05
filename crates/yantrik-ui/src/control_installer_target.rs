//! The installer's "beside what is there" choice on the control surface: which partition, or
//! which free space, Yantrik OS goes into (wire/installer_partition.rs on the screens).
//!
//! The choice is checked against what the Disk screen was shown. A partition the planner keeps —
//! APFS, HFS+, NTFS, a Linux filesystem, the EFI system partition, the installer's own — is
//! refused here, by name and kind, and refused again by the planner right before anything is
//! written, against a fresh read of the disk.

use slint::{ComponentHandle, Model};
use yantrik_app_runtime::control::{Action, App as ControlSurface, Param};

use crate::App;

/// The Disk screen's segments as the control surface reports them.
pub fn targets(ui: &App) -> Vec<serde_json::Value> {
    ui.get_onboard_segments()
        .iter()
        .map(|s| {
            serde_json::json!({
                "id": s.id.to_string(),
                "disk": s.disk.to_string(),
                "kind": s.kind.to_string(),
                "title": s.title.to_string(),
                "size": s.size.to_string(),
                "kept": s.kept,
                "eligible": s.eligible,
                "reason": s.reason.to_string(),
                "sentence": s.sentence.to_string(),
            })
        })
        .collect()
}

/// What a caller may choose from: `(id, kind, eligible, reason)` for each segment.
pub type Offered = (String, String, bool, String);

fn offered(ui: &App) -> Vec<Offered> {
    ui.get_onboard_segments()
        .iter()
        .map(|s| (s.id.to_string(), s.kind.to_string(), s.eligible, s.reason.to_string()))
        .collect()
}

/// 20 GB in 512-byte sectors: a sub-range of free space smaller than this is refused at once.
/// The planner checks again with the disk's real sector size before writing.
const MIN_SECTORS_512: u64 = 20_000_000_000 / 512;

/// The target a caller asked for, as the id the screens hold, or why it cannot be chosen.
/// `partition` names a partition (`sda3`); `disk` with `free_start` and `free_end` (sectors)
/// names free space, all of a run the Disk screen offers or a part of one.
pub fn choose(
    offered: &[Offered],
    partition: Option<&str>,
    disk: Option<&str>,
    free_start: Option<u64>,
    free_end: Option<u64>,
) -> Result<String, String> {
    let choosable: Vec<&str> = offered.iter().filter(|o| o.2).map(|o| o.0.as_str()).collect();
    let listing = if choosable.is_empty() { "nothing".to_string() } else { choosable.join(", ") };
    if let Some(p) = partition.map(|p| p.trim().trim_start_matches("/dev/")).filter(|p| !p.is_empty()) {
        if free_start.is_some() || free_end.is_some() {
            return Err("name a partition or free space, not both".into());
        }
        let Some((id, kind, eligible, reason)) = offered.iter().find(|o| o.0 == p && o.1 != "free") else {
            return Err(format!("no partition `{p}` was found; what may be chosen: {listing}"));
        };
        if !eligible {
            return Err(format!("refused: {reason} ({kind})"));
        }
        return Ok(id.clone());
    }
    let (Some(start), Some(end)) = (free_start, free_end) else {
        return Err("name a partition (partition: sda3), or free space (disk, free_start, free_end in sectors)".into());
    };
    let disk = disk.map(|d| d.trim().trim_start_matches("/dev/")).unwrap_or("");
    if disk.is_empty() {
        return Err("free space needs its disk (disk: sda)".into());
    }
    if start > end {
        return Err(format!("free_start {start} is after free_end {end}"));
    }
    let run = offered.iter().filter(|o| o.1 == "free").find_map(|o| {
        let (d, range) = o.0.split_once('@')?;
        let (s, e) = range.split_once('-')?;
        let (s, e) = (s.parse::<u64>().ok()?, e.parse::<u64>().ok()?);
        (d == disk && start >= s && end <= e).then_some((o, s, e))
    });
    let Some(((id, _, eligible, reason), s, e)) = run else {
        return Err(format!(
            "sectors {start}-{end} on {disk} are not inside free space the installer found; what may be chosen: {listing}"
        ));
    };
    if !eligible {
        return Err(format!("refused: {reason}"));
    }
    if end - start + 1 < MIN_SECTORS_512 {
        return Err(format!("sectors {start}-{end} are under 20 GB; Yantrik OS needs at least 20 GB"));
    }
    Ok(if (start, end) == (s, e) { id.clone() } else { format!("{disk}@{start}-{end}") })
}

/// Why the "beside what is there" choice is not ready, or `None`.
pub fn problem(target: &str, eligible: &str, fingerprint: &str) -> Option<String> {
    if target.is_empty() {
        return Some(if eligible.trim().is_empty() {
            "install_into: nothing on these disks can be installed into; make a FAT32 partition named \
             YANTRIK (20 GB or more) or leave 20 GB free in macOS, or choose whole_disk"
                .into()
        } else {
            format!("install_into: nothing chosen; what may be chosen: {}", eligible.split_whitespace().collect::<Vec<_>>().join(", "))
        });
    }
    if !crate::installer_rules::disk_in_list(target, eligible) {
        return Some(format!("install_into: `{target}` may not be installed into"));
    }
    if fingerprint.is_empty() {
        return Some(format!("install_into: the disk under `{target}` was not scanned"));
    }
    None
}

/// `installer_choose_target`: install beside what is there, into a partition or free space, or
/// go back to erasing a whole disk.
pub fn add(surface: ControlSurface, ui: &App) -> ControlSurface {
    let weak = ui.as_weak();
    surface.action(
        Action::new(
            "installer_choose_target",
            "Install beside what is on a disk: into one partition (an empty one, or a FAT/exFAT \
             placeholder labelled YANTRIK) or into free space, changing nothing else on the disk. \
             macOS, Windows, Linux filesystems and the EFI partition are refused.",
        )
        .arg(Param::text("partition").optional().describe("a partition the installer offers, e.g. sda3"))
        .arg(Param::text("disk").optional().describe("for free space: the disk, e.g. sda"))
        .arg(Param::integer("free_start").optional().describe("for free space: the first sector"))
        .arg(Param::integer("free_end").optional().describe("for free space: the last sector"))
        .arg(Param::flag("whole_disk").optional().describe("true: erase a whole disk instead (the disk field)"))
        .arg(Param::flag("boot_first").optional().describe(
            "not on a Mac: put Yantrik first in the firmware's boot order (otherwise last)",
        )),
        move |args| {
            let ui = weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
            if ui.get_onboard_installing() {
                return Err("an install is running; the form cannot be changed".into());
            }
            if let Some(first) = args["boot_first"].as_bool() {
                if first && !ui.get_onboard_boot_first_offered() {
                    return Err("this machine's boot order is not changed (a Mac, or not UEFI)".into());
                }
                ui.set_onboard_boot_first(first);
            }
            if args["whole_disk"].as_bool() == Some(true) {
                ui.set_onboard_into_partition(false);
                return Ok(serde_json::json!({ "into_partition": false, "disk": ui.get_onboard_selected_disk().to_string() }));
            }
            let partition = args["partition"].as_str();
            let free = (args["free_start"].as_u64(), args["free_end"].as_u64());
            if partition.is_none() && free == (None, None) {
                if args["boot_first"].is_boolean() {
                    return Ok(serde_json::json!({ "boot_first": ui.get_onboard_boot_first() }));
                }
                return Err("name a partition, or free space with disk, free_start and free_end, or whole_disk".into());
            }
            let id = choose(&offered(&ui), partition, args["disk"].as_str(), free.0, free.1)?;
            let eligible = ui.get_onboard_eligible_targets().to_string();
            if !crate::installer_rules::disk_in_list(&id, &eligible) {
                // A part of an offered run: offered now as itself.
                ui.set_onboard_eligible_targets(format!("{eligible} {id}").trim().into());
            }
            ui.set_onboard_install_target(id.clone().into());
            ui.set_onboard_into_partition(true);
            let sentence = ui
                .get_onboard_segments()
                .iter()
                .find(|s| s.id.as_str() == id)
                .map(|s| s.sentence.to_string())
                .unwrap_or_else(|| format!("Yantrik OS will be installed into a new partition at {id}. Nothing else on this disk changes."));
            Ok(serde_json::json!({
                "install_into": id,
                "sentence": sentence,
                "boot_first": ui.get_onboard_boot_first(),
                "blocked_by": crate::control_installer::blocked_by(&ui)
                    .map(serde_json::Value::String)
                    .unwrap_or(serde_json::Value::Null),
            }))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Mac mini after macOS made room: EFI, APFS, the YANTRIK placeholder, the installer's
    /// own YANTRIK-INS, and free space at the end too small to use; plus a disk with room.
    fn mac() -> Vec<Offered> {
        let o = |id: &str, kind: &str, ok: bool, why: &str| (id.to_string(), kind.to_string(), ok, why.to_string());
        vec![
            o("sda1", "esp", false, "/dev/sda1 is the EFI system partition"),
            o("sda2", "macos", false, "/dev/sda2 holds macOS (APFS); it is kept"),
            o("sda3", "placeholder", true, ""),
            o("sda4", "medium", false, "/dev/sda4 is what this installer is running from"),
            o("sda@1950152680-1953525134", "free", false, "the free space is 1.7 GB"),
            o("sdb@2048-500000000", "free", true, ""),
        ]
    }

    #[test]
    fn macos_and_the_efi_partition_are_refused_by_name() {
        let e = choose(&mac(), Some("sda2"), None, None, None).unwrap_err();
        assert!(e.starts_with("refused:") && e.contains("macOS (APFS)"), "{e}");
        let e = choose(&mac(), Some("/dev/sda1"), None, None, None).unwrap_err();
        assert!(e.contains("EFI system partition"), "{e}");
        let e = choose(&mac(), Some("sda4"), None, None, None).unwrap_err();
        assert!(e.contains("running from"), "{e}");
        let e = choose(&mac(), Some("sda9"), None, None, None).unwrap_err();
        assert!(e.contains("no partition `sda9`") && e.contains("sda3"), "{e}");
        assert_eq!(choose(&mac(), Some("sda3"), None, None, None).unwrap(), "sda3");
    }

    #[test]
    fn free_space_is_chosen_whole_or_in_part_and_only_inside_a_run() {
        assert_eq!(choose(&mac(), None, Some("sdb"), Some(2048), Some(500_000_000)).unwrap(), "sdb@2048-500000000");
        assert_eq!(
            choose(&mac(), None, Some("/dev/sdb"), Some(4096), Some(100_000_000)).unwrap(),
            "sdb@4096-100000000"
        );
        let e = choose(&mac(), None, Some("sdb"), Some(0), Some(100_000_000)).unwrap_err();
        assert!(e.contains("not inside free space"), "{e}");
        let e = choose(&mac(), None, Some("sda"), Some(1_950_152_680), Some(1_953_525_134)).unwrap_err();
        assert!(e.starts_with("refused:"), "{e}");
        let e = choose(&mac(), None, Some("sdb"), Some(2048), Some(1_000_000)).unwrap_err();
        assert!(e.contains("under 20 GB"), "{e}");
        assert!(choose(&mac(), Some("sda3"), None, Some(1), Some(2)).is_err(), "not both");
        assert!(choose(&mac(), None, None, Some(2048), Some(500_000_000)).is_err(), "free space needs a disk");
        assert!(choose(&mac(), None, None, None, None).is_err());
    }

    #[test]
    fn the_choice_is_ready_only_when_it_may_be_installed_into() {
        assert_eq!(problem("sda3", "sda3 sdb@2048-500000000", "00aa"), None);
        assert!(problem("", "sda3", "00aa").unwrap().contains("nothing chosen"));
        assert!(problem("", "", "").unwrap().contains("YANTRIK"));
        assert!(problem("sda2", "sda3", "00aa").unwrap().contains("may not be installed into"));
        assert!(problem("sda3", "sda3", "").unwrap().contains("not scanned"));
    }
}
