//! What a person is shown before the installer writes a disk: the sentence on the approval card
//! for `installer_install` and `installer_choose_target` (`Action::explain`), and confirming
//! that macOS may be erased (`installer_erase_macos`), which used to be one field of
//! `installer_set` among the name and the timezone, at the grade of filling in a name.
//!
//! The sentences name the device and say whether it is erased: "Install into /dev/sda4 (199 GB,
//! YANTRIK, FAT32); nothing else changes", or "ERASE /dev/sda, which holds macOS". They are read
//! from the installer as it is when the card is built, which is what installer_install acts on.

use slint::{ComponentHandle, Model};
use yantrik_app_runtime::control::{Action, App as ControlSurface, Param};

use crate::installer_rules;
use crate::App;

/// What installer_install would do, as the installer holds it now.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InstallView {
    pub into_partition: bool,
    pub target: String,
    /// The card of the segment `target` names, when the scan offered it as one that may be chosen.
    pub target_card: Option<String>,
    /// Why the segment `target` names may not be chosen, when the scan refused it.
    pub target_refusal: Option<String>,
    /// The disk chosen to erase, `sda`.
    pub disk: String,
    /// "931.5G, APPLE HDD HTS541010A9E662", when the installer listed it.
    pub disk_line: Option<String>,
    pub disk_holds_macos: bool,
}

/// The approval card's sentence for installer_install.
pub fn install_card(v: &InstallView) -> String {
    if v.into_partition {
        if v.target.trim().is_empty() {
            return "Install Yantrik OS beside what is on a disk, but nothing is chosen to install into; this call is refused".into();
        }
        if let Some(why) = &v.target_refusal {
            return format!("Install into /dev/{}, which is refused: {why}; this call is refused", v.target.trim_start_matches("/dev/"));
        }
        if let Some(card) = &v.target_card {
            return card.clone();
        }
        return match yantrik_install_target::parse_target_id(&v.target) {
            Ok((disk, yantrik_install_target::TargetSpec::Free { start, end })) => {
                format!("Install into a new partition at sectors {start}-{end} of /dev/{disk}; nothing else changes")
            }
            _ => format!("Install into /dev/{}; nothing else changes", v.target.trim_start_matches("/dev/")),
        };
    }
    let disk = v.disk.trim().trim_start_matches("/dev/");
    if disk.is_empty() {
        return "ERASE a whole disk and install Yantrik OS, but no disk is chosen; this call is refused".into();
    }
    if v.disk_holds_macos {
        return format!("ERASE /dev/{disk}, which holds macOS; macOS and everything else on it is lost");
    }
    match &v.disk_line {
        Some(line) => format!("ERASE /dev/{disk} ({line}); everything on it is lost"),
        None => format!("ERASE /dev/{disk}; everything on it is lost"),
    }
}

/// The approval card's sentence for installer_choose_target, from its arguments and the
/// segments the scan found (`(id, card, eligible)`; a refused segment's card is its refusal).
pub fn choose_card(args: &serde_json::Value, offered: &[(String, String, bool)], erase_disk: &str) -> String {
    if args["whole_disk"].as_bool() == Some(true) {
        let disk = if erase_disk.is_empty() { "none chosen yet".to_string() } else { format!("/dev/{erase_disk}") };
        return format!("Switch the installer to ERASING a whole disk ({disk}) instead of installing beside what is there");
    }
    if let Some(p) = args["partition"].as_str().map(|p| p.trim().trim_start_matches("/dev/")).filter(|p| !p.is_empty()) {
        return match offered.iter().find(|(id, _, _)| id == p) {
            Some((_, card, true)) => format!("Choose where to install: {card}. Nothing is written until installer_install"),
            Some((_, why, false)) => format!("Choose {p} to install into; this call is refused: {why}"),
            None => format!("Choose {p} to install into; the installer did not offer it, so this call is refused"),
        };
    }
    if let (Some(start), Some(end)) = (args["free_start"].as_u64(), args["free_end"].as_u64()) {
        let disk = args["disk"].as_str().unwrap_or("").trim().trim_start_matches("/dev/");
        return format!(
            "Choose a new partition at sectors {start}-{end} of /dev/{disk} to install into; nothing else on it changes, and nothing is written until installer_install"
        );
    }
    match args["boot_first"].as_bool() {
        Some(true) => "Put Yantrik OS first in this machine's boot order once it is installed".into(),
        Some(false) => "Put Yantrik OS last in this machine's boot order once it is installed".into(),
        None => String::new(),
    }
}

/// The installer as installer_install would act on it now.
pub fn install_view(ui: &App) -> InstallView {
    let target = ui.get_onboard_install_target().to_string();
    let disk = ui.get_onboard_selected_disk().to_string();
    let listed = ui.get_onboard_disks().iter().find(|d| d.name.as_str() == disk);
    let segment = ui.get_onboard_segments().iter().find(|s| s.id.as_str() == target);
    InstallView {
        into_partition: ui.get_onboard_into_partition(),
        target_card: segment.as_ref().filter(|s| s.eligible).map(|s| s.card.to_string()).filter(|c| !c.is_empty()),
        target_refusal: segment.as_ref().filter(|s| !s.eligible).map(|s| s.reason.to_string()),
        target,
        disk_line: listed.as_ref().map(|d| format!("{}, {}", d.size, d.model)),
        disk_holds_macos: listed.as_ref().is_some_and(|d| d.holds_macos)
            || installer_rules::disk_in_list(&disk, &ui.get_onboard_macos_disks()),
        disk,
    }
}

/// `installer_erase_macos`: the Disk screen's "Erase macOS on /dev/sdX", by itself, graded
/// `sensitive`. The value is that disk's name, typed again: a yes/no could be carried over to a
/// disk it was never about.
pub fn add_erase_macos(surface: ControlSurface, ui: &App) -> ControlSurface {
    let weak = ui.as_weak();
    let explain_weak = ui.as_weak();
    surface.action(
        Action::new(
            "installer_erase_macos",
            "Confirm that the chosen disk may be erased although it holds macOS: the disk's name, \
             typed again. Empty withdraws the confirmation. installer_install then erases it.",
        )
        .arg(Param::text("disk").describe("the chosen disk's name, e.g. sda; empty to withdraw"))
        .risk("sensitive")
        .explain(move |args| {
            let want = args["disk"].as_str().unwrap_or("").trim().trim_start_matches("/dev/").to_string();
            if want.is_empty() {
                return "Withdraw the confirmation that a disk holding macOS may be erased".into();
            }
            match explain_weak.upgrade() {
                Some(ui) if installer_rules::disk_in_list(&want, &ui.get_onboard_macos_disks()) => {
                    format!("Allow installer_install to ERASE /dev/{want}, which holds macOS; macOS and everything on it would be lost")
                }
                _ => format!("Allow installer_install to ERASE /dev/{want}, said to hold macOS"),
            }
        }),
        move |args| {
            let ui = weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
            if ui.get_onboard_installing() {
                return Err("an install is running; the form cannot be changed".into());
            }
            let want = args["disk"].as_str().unwrap_or("").trim().trim_start_matches("/dev/").to_string();
            if !want.is_empty() {
                if !installer_rules::disk_in_list(&want, &ui.get_onboard_macos_disks()) {
                    return Err(format!("`{want}` is not a disk here that holds macOS"));
                }
                if want != ui.get_onboard_selected_disk().as_str() {
                    return Err(format!("`{want}` is not the chosen disk; choose it with installer_set disk first"));
                }
            }
            ui.set_onboard_erase_macos_disk(want.clone().into());
            Ok(serde_json::json!({
                "erase_macos": want,
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
    use serde_json::json;

    /// An action's declaration as written in `file`: from its `Action::new(` to its handler.
    /// The handlers need a live Slint window, so the grades are pinned against the source, the
    /// way control.rs's `lock_grade_tests` pins `lock`.
    fn declaration(file: &str, name: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(file);
        let whole = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let src = whole.split("#[cfg(test)]").next().unwrap_or_default();
        let at = src.find(&format!("\"{name}\",")).unwrap_or_else(|| panic!("{name} is not declared in {file}"));
        let start = src[..at].rfind("Action::new(").expect("declared with Action::new");
        let end = at + src[at..].find("move |").expect("a handler follows");
        src[start..end].to_string()
    }

    #[test]
    fn choosing_where_to_write_and_erasing_macos_ask_a_person_with_the_device_named() {
        let choose = declaration("control_installer_target.rs", "installer_choose_target");
        assert!(choose.contains(".risk(\"sensitive\")") && choose.contains(".explain("), "{choose}");
        let erase = declaration("control_installer_consent.rs", "installer_erase_macos");
        assert!(erase.contains(".risk(\"sensitive\")") && erase.contains(".explain("), "{erase}");
        let install = declaration("control_installer.rs", "installer_install");
        assert!(install.contains(".risk(\"dangerous\")") && install.contains(".explain("), "{install}");
        // installer_set stays a form field at `standard`, and erase_macos is no field of it.
        let set = declaration("control_installer.rs", "installer_set");
        assert!(!set.contains(".risk("), "{set}");
        let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/control_installer.rs")).unwrap();
        let fields = &src[src.find("const FIELDS").unwrap()..];
        assert!(!fields[..fields.find("];").unwrap()].contains("erase_macos"), "erase_macos is not a writable field");
        assert!(src.contains("erase_macos is not a field: confirm erasing macOS with installer_erase_macos"));
    }

    fn beside(target: &str, card: Option<&str>) -> InstallView {
        InstallView { into_partition: true, target: target.into(), target_card: card.map(String::from), ..InstallView::default() }
    }

    fn erase(disk: &str, line: Option<&str>, macos: bool) -> InstallView {
        InstallView { disk: disk.into(), disk_line: line.map(String::from), disk_holds_macos: macos, ..InstallView::default() }
    }

    #[test]
    fn the_install_card_names_the_device_and_whether_it_erases() {
        let card = "Install into /dev/sda4 (199 GB, YANTRIK, FAT32); nothing else changes";
        assert_eq!(install_card(&beside("sda4", Some(card))), card);
        assert_eq!(
            install_card(&beside("sda@1600000000-1700000000", None)),
            "Install into a new partition at sectors 1600000000-1700000000 of /dev/sda; nothing else changes"
        );
        assert!(install_card(&beside("", None)).contains("refused"));
        let refused = InstallView { target_refusal: Some("/dev/sda1 is the EFI system partition; ...".into()), ..beside("sda1", None) };
        assert_eq!(
            install_card(&refused),
            "Install into /dev/sda1, which is refused: /dev/sda1 is the EFI system partition; ...; this call is refused"
        );
        assert_eq!(install_card(&erase("sda", Some("931.5G, APPLE HDD"), true)), "ERASE /dev/sda, which holds macOS; macOS and everything else on it is lost");
        assert_eq!(install_card(&erase("sdb", Some("1.8T, USB · T7"), false)), "ERASE /dev/sdb (1.8T, USB · T7); everything on it is lost");
        assert!(install_card(&erase("", None, false)).starts_with("ERASE a whole disk") && install_card(&erase("", None, false)).contains("refused"));
        // Every sentence about a beside install says nothing else changes; every erase says ERASE.
        for v in [beside("sda4", Some(card)), beside("sda@1-2", None), beside("nvme0n1p5", None)] {
            assert!(install_card(&v).ends_with("nothing else changes") && !install_card(&v).contains("ERASE"), "{v:?}");
        }
    }

    #[test]
    fn the_choose_card_says_what_is_chosen_and_that_nothing_is_written_yet() {
        let offered = vec![
            ("sda1".to_string(), "/dev/sda1 is the EFI system partition; ...".to_string(), false),
            ("sda4".to_string(), "Install into /dev/sda4 (199 GB, YANTRIK, FAT32); nothing else changes".to_string(), true),
        ];
        let c = choose_card(&json!({ "partition": "/dev/sda4" }), &offered, "");
        assert!(c.contains("/dev/sda4 (199 GB, YANTRIK, FAT32)") && c.contains("Nothing is written until installer_install"), "{c}");
        assert!(choose_card(&json!({ "partition": "sda2" }), &offered, "").contains("did not offer it"));
        // A refused one says it is refused and why, never "Choose where to install".
        let r = choose_card(&json!({ "partition": "sda1" }), &offered, "");
        assert_eq!(r, "Choose sda1 to install into; this call is refused: /dev/sda1 is the EFI system partition; ...");
        assert!(choose_card(&json!({ "whole_disk": true }), &offered, "sda").contains("ERASING a whole disk (/dev/sda)"));
        assert!(choose_card(&json!({ "disk": "sda", "free_start": 2048, "free_end": 9999 }), &offered, "").contains("sectors 2048-9999 of /dev/sda"));
        assert!(choose_card(&json!({ "boot_first": true }), &offered, "").contains("first"));
        assert_eq!(choose_card(&json!({}), &offered, ""), "");
    }
}
