//! Installing into one partition or one run of free space, beside macOS or Windows
//! (crates/yantrik-install-target): the Disk screen's scan, and the step that takes the place of
//! partitioning a whole disk.
//!
//! The planner reads the table again right before writing and refuses when it is not the one the
//! person was shown, so the fingerprint the scan took travels with the choice to the install.

use std::process::Command;

use yantrik_install_target::{apply, classify, efi, parse_target_id, Segment};

use super::installer::run_cmd;
use super::installer_disk::{self, Layout};

/// A disk the scan could read a GPT from.
#[derive(Debug, Clone)]
pub struct ScannedDisk {
    pub name: String,
    /// "APPLE HDD · 1 TB (/dev/sda)".
    pub title: String,
    pub fingerprint: String,
}

/// Every partition and run of free space a person could be shown, disk by disk.
#[derive(Debug, Clone, Default)]
pub struct Scan {
    pub disks: Vec<ScannedDisk>,
    pub segments: Vec<Segment>,
    /// Started by UEFI: installing into a partition needs it.
    pub efi: bool,
    /// A Mac: its NVRAM is never written, so there is no "start Yantrik by default" to offer.
    pub apple: bool,
}

pub fn efi_boot() -> bool {
    std::path::Path::new("/sys/firmware/efi").exists()
}

pub fn is_apple() -> bool {
    std::fs::read_to_string("/sys/class/dmi/id/sys_vendor").map(|v| efi::is_apple(&v)).unwrap_or(false)
}

/// The disks a partition may be chosen on, from `lsblk -J -d -o NAME,TYPE,RO,SIZE,MODEL,FSTYPE`:
/// whole, writable disks with something in them, except a stick that is nothing but the
/// installer's ISO. Unlike the whole-disk list (installer.rs `disks_from_lsblk`), a disk the
/// installer is running from is kept: a Mac booted from its YANTRIK-INS partition runs from its
/// internal disk, and that partition is shown and refused rather than the disk hidden.
pub fn candidate_disks(lsblk_json: &str) -> Vec<(String, String)> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(lsblk_json) else { return Vec::new() };
    let flag = |x: &serde_json::Value| x.as_bool().unwrap_or(false) || x.as_str() == Some("1");
    v["blockdevices"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter(|d| d["type"].as_str() == Some("disk") && !flag(&d["ro"]))
        .filter(|d| d["size"].as_str() != Some("0B") && d["fstype"].as_str() != Some("iso9660"))
        .filter_map(|d| {
            let name = d["name"].as_str()?.to_string();
            let model = d["model"].as_str().unwrap_or("").trim();
            let size = d["size"].as_str().unwrap_or("?");
            let model = if model.is_empty() { "Disk" } else { model };
            Some((name.clone(), format!("{model} · {size} (/dev/{name})")))
        })
        .collect()
}

/// Read every candidate disk's table. A disk with no GPT (blank, or MBR) has nothing to install
/// into and is left to the whole-disk list.
pub fn scan() -> Scan {
    let mut out = Scan { efi: efi_boot(), apple: is_apple(), ..Scan::default() };
    let listing = Command::new("lsblk")
        .args(["-J", "-d", "-o", "NAME,TYPE,RO,SIZE,MODEL,FSTYPE", "-e", "7,11"])
        .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    for (name, title) in candidate_disks(&listing) {
        match apply::read_table(&format!("/dev/{name}"), &run_cmd) {
            Ok(t) if t.label == "gpt" => {
                out.segments.extend(classify::segments(&t, out.efi));
                out.disks.push(ScannedDisk { name, title, fingerprint: t.fingerprint() });
            }
            Ok(t) => tracing::info!(disk = %name, label = %t.label, "Installer: no GPT, nothing to install into"),
            Err(e) => tracing::info!(disk = %name, error = %e, "Installer: could not read a partition table"),
        }
    }
    tracing::info!(
        disks = out.disks.len(),
        eligible = out.segments.iter().filter(|s| s.eligible).count(),
        efi = out.efi,
        apple = out.apple,
        "Installer: partitions scanned"
    );
    out
}

/// The fingerprint of `disk`'s table in the screens' `sda=<fp> nvme0n1=<fp>` list, or "".
pub fn fingerprint_for(list: &str, disk: &str) -> String {
    list.split_whitespace()
        .find_map(|pair| pair.strip_prefix(disk)?.strip_prefix('=').map(String::from))
        .unwrap_or_default()
}

/// Put a scan on the Disk screen: each disk's bar, what can be chosen, and the fingerprints the
/// install is checked against. A placeholder labelled YANTRIK, or free space beside another
/// system, is chosen for the person (classify::preselect); otherwise the screen opens on erasing
/// a disk, as it always did.
pub fn show(ui: &crate::App, scan: &Scan) {
    use slint::{ModelRc, VecModel};
    let disks: Vec<crate::InstallerTargetDisk> = scan
        .disks
        .iter()
        .map(|d| crate::InstallerTargetDisk { name: d.name.clone().into(), title: d.title.clone().into() })
        .collect();
    let segments: Vec<crate::InstallerSegment> = scan
        .segments
        .iter()
        .map(|s| crate::InstallerSegment {
            id: s.id.clone().into(),
            disk: s.disk.clone().into(),
            kind: s.kind.clone().into(),
            title: s.title.clone().into(),
            size: s.size.clone().into(),
            share: s.share,
            kept: s.kept,
            eligible: s.eligible,
            reason: s.reason.clone().into(),
            sentence: s.sentence.clone().into(),
        })
        .collect();
    let eligible: Vec<&str> = scan.segments.iter().filter(|s| s.eligible).map(|s| s.id.as_str()).collect();
    let prints: Vec<String> = scan.disks.iter().map(|d| format!("{}={}", d.name, d.fingerprint)).collect();
    ui.set_onboard_target_disks(ModelRc::new(VecModel::from(disks)));
    ui.set_onboard_segments(ModelRc::new(VecModel::from(segments)));
    ui.set_onboard_eligible_targets(eligible.join(" ").into());
    ui.set_onboard_table_fingerprints(prints.join(" ").into());
    ui.set_onboard_boot_first_offered(scan.efi && !scan.apple);
    if ui.get_onboard_install_target().is_empty() {
        if let Some(id) = classify::preselect(&scan.segments) {
            ui.set_onboard_install_target(id.into());
            ui.set_onboard_into_partition(true);
        }
    }
    ui.set_onboard_partitions_scanned(true);
}

/// Make the chosen target ready: the table read again and compared with `fingerprint`, the
/// partition made (or the placeholder wiped), then the filesystems on it. The EFI partition is
/// the disk's own and is only ever mounted.
pub fn prepare(
    target: &str,
    fingerprint: &str,
    encrypt: bool,
    passphrase: &str,
    progress: &dyn Fn(i32, &str),
) -> Result<Layout, String> {
    if encrypt && passphrase.is_empty() {
        return Err("encrypting the disk needs a password, and none was given".into());
    }
    if fingerprint.is_empty() {
        return Err("the disk was never scanned, so there is nothing to check it against".into());
    }
    let (disk, spec) = parse_target_id(target)?;
    let disk = format!("/dev/{disk}");
    let _ = run_cmd("cryptsetup", &["close", installer_disk::CRYPT_NAME]);
    progress(2, "Checking the disk has not changed...");
    let placed = apply::apply(&disk, spec, encrypt, efi_boot(), fingerprint, &run_cmd)?;
    tracing::info!(
        disk = %disk, target, root = %placed.root, esp = %placed.esp, boot = ?placed.boot,
        "Installer: partition made beside what is on the disk"
    );
    progress(8, "Partition ready");
    let keeps_macos = apply::read_table(&disk, &run_cmd)
        .map(|t| t.parts.iter().any(|p| classify::kind_of(p).name() == "macos"))
        .unwrap_or(false);
    let layout = Layout {
        efi_part: placed.esp,
        boot_part: placed.boot,
        luks_part: None,
        root_dev: placed.root,
        in_partition: true,
        keeps_macos,
    };
    installer_disk::make_filesystems(layout, encrypt, passphrase, progress)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_disk_the_installer_runs_from_is_still_a_candidate() {
        // The Mac booted from its own YANTRIK-INS partition: sda holds the live medium, and
        // still has its YANTRIK placeholder to install into. A dd'ed ISO stick does not.
        let json = r#"{"blockdevices":[
            {"name":"sda","type":"disk","ro":false,"size":"931.5G","model":"APPLE HDD HTS541010A9E662","fstype":null},
            {"name":"sdb","type":"disk","ro":false,"size":"28.9G","model":"Flash Drive","fstype":"iso9660"},
            {"name":"sdc","type":"disk","ro":false,"size":"0B","model":"SD Card Reader","fstype":null},
            {"name":"sr0","type":"rom","ro":true,"size":"3G","model":"DVD","fstype":"iso9660"}
        ]}"#;
        let disks = candidate_disks(json);
        assert_eq!(disks, [("sda".to_string(), "APPLE HDD HTS541010A9E662 · 931.5G (/dev/sda)".to_string())]);
    }

    #[test]
    fn a_disks_fingerprint_is_found_by_its_whole_name() {
        let list = "sda=00aa sdab=11bb nvme0n1=22cc";
        assert_eq!(fingerprint_for(list, "sda"), "00aa");
        assert_eq!(fingerprint_for(list, "sdab"), "11bb");
        assert_eq!(fingerprint_for(list, "nvme0n1"), "22cc");
        assert_eq!(fingerprint_for(list, "sdb"), "");
        assert_eq!(fingerprint_for("", "sda"), "");
    }
}
