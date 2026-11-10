//! A Yantrik already installed on this computer's disk, found when the live installer starts
//! (#1165). On Proxmox and QEMU the ISO is still attached after an install, so the machine boots
//! the installer again and the person thinks the install looped. The Welcome screen then says
//! the install is there and offers to start it.
//!
//! The disk is only read: lsblk's udev records, never a mount, and a LUKS volume is recognised by
//! the label the installer gave its header, never opened. Starting it is a one-shot UEFI BootNext
//! to Yantrik's own firmware entry; with no such entry the screen says what to do by hand.

use slint::ComponentHandle;
use yantrik_install_target::efi;

use super::installer_disk::{CRYPT_LABEL, ROOT_LABEL};
use super::installer_probe::run_ran;
use crate::App;

/// Firmware entry labels the installer's two paths give: efibootmgr's, and grub-install's id.
const ENTRY_LABELS: [&str; 2] = ["Yantrik OS", "yantrik"];

/// What was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The partition, `sda2`.
    pub device: String,
    /// `BootNNNN`'s number, when the firmware has Yantrik's entry to start by.
    pub boot_entry: Option<String>,
}

/// The partition of Yantrik's own making in `lsblk -rno NAME,FSTYPE,LABEL`: the root filesystem
/// the installer labels, or the LUKS container it labels when it encrypts. The label alone is not
/// enough: the live ISO and the Mac's placeholder carry YANTRIK on other filesystems.
pub fn installed_partition(lsblk: &str) -> Option<String> {
    lsblk.lines().find_map(|line| {
        let mut f = line.split(' ');
        let (name, fstype, label) = (f.next()?, f.next()?, f.next()?);
        let ours = (fstype == "ext4" && label == ROOT_LABEL) || (fstype == "crypto_LUKS" && label == CRYPT_LABEL);
        ours.then(|| name.to_string())
    })
}

/// The number of Yantrik's entry in `efibootmgr`'s listing.
pub fn boot_entry(listing: &str) -> Option<String> {
    ENTRY_LABELS.iter().find_map(|label| efi::entry_number(listing, label))
}

/// What the Welcome screen says.
pub fn sentence(found: &Found) -> String {
    let head = format!("Yantrik is already installed on this computer (/dev/{}).", found.device);
    if found.boot_entry.is_some() {
        format!("{head} Start it, or install again. If the installer disc or ISO is still attached, remove it first.")
    } else {
        format!("{head} Remove the installer disc (or ISO), then restart to use it. Or install again.")
    }
}

fn detect() -> Option<Found> {
    let listing = run_ran("lsblk", &["-rno", "NAME,FSTYPE,LABEL"]);
    let device = installed_partition(&listing.stdout).filter(|_| listing.code == Some(0))?;
    let boot_entry = std::path::Path::new("/sys/firmware/efi")
        .exists()
        .then(|| run_ran("efibootmgr", &[]))
        .filter(|r| r.code == Some(0))
        .and_then(|r| boot_entry(&r.stdout));
    Some(Found { device, boot_entry })
}

/// BootNext to Yantrik's entry, then restart. BootNext is used once and leaves the order alone.
fn start(entry: &str) -> Result<(), String> {
    let set = run_ran("efibootmgr", &["-n", entry]);
    if set.code != Some(0) {
        return Err(format!("the firmware refused BootNext: {}", set.stderr.trim()));
    }
    run_ran("reboot", &["-f"]);
    Ok(())
}

/// Look for an install in the background, say so on the Welcome screen, and answer its button.
pub fn wire(ui: &App) {
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let found = detect();
        tracing::info!(?found, "Installer: an install already on this computer");
        let _ = slint::invoke_from_event_loop(move || {
            let (Some(ui), Some(found)) = (weak.upgrade(), found) else { return };
            ui.set_onboard_existing_install(sentence(&found).into());
            ui.set_onboard_existing_startable(found.boot_entry.is_some());
            ui.on_onboard_start_installed(move || {
                if let Some(entry) = &found.boot_entry {
                    if let Err(e) = start(entry) {
                        tracing::warn!(error = %e, "Installer: could not start the installed Yantrik");
                    }
                }
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIVE: &str = "sr0 iso9660 YANTRIK\nloop0 squashfs \nsda  \n";

    #[test]
    fn the_live_iso_alone_is_no_install() {
        assert_eq!(installed_partition(LIVE), None, "YANTRIK on iso9660 is the installer's own disc");
    }

    #[test]
    fn a_plain_install_is_its_labelled_ext4_root() {
        let rows = format!("{LIVE}sdb1 vfat \nsdb2 ext4 YANTRIK\n");
        assert_eq!(installed_partition(&rows).as_deref(), Some("sdb2"));
    }

    #[test]
    fn an_encrypted_install_is_known_by_its_luks_label_and_not_opened() {
        let rows = format!("{LIVE}vda1 vfat \nvda2 ext4 YANTRIK_BOOT\nvda3 crypto_LUKS YANTRIK_CRYPT\n");
        assert_eq!(installed_partition(&rows).as_deref(), Some("vda3"));
    }

    #[test]
    fn a_label_on_another_filesystem_or_an_unlabelled_luks_is_not_ours() {
        assert_eq!(installed_partition("sda1 vfat YANTRIK\nsda2 crypto_LUKS \nsda3 ext4 data\nsda4 crypto_LUKS other\n"), None);
        assert_eq!(installed_partition("sda1 ext4 YANTRIK_BOOT\n"), None, "/boot alone says nothing");
        assert_eq!(installed_partition(""), None);
    }

    #[test]
    fn the_firmware_entry_is_found_by_either_label_the_installer_gives() {
        let partition = "BootCurrent: 0001\nBootOrder: 0001,0003\nBoot0001* UEFI QEMU DVD-ROM\nBoot0003* Yantrik OS\tHD(1,GPT)\n";
        assert_eq!(boot_entry(partition).as_deref(), Some("0003"));
        let grub = "BootOrder: 0001,0002\nBoot0001* UEFI QEMU DVD-ROM\nBoot0002* yantrik\tHD(1,GPT)\n";
        assert_eq!(boot_entry(grub).as_deref(), Some("0002"));
        assert_eq!(boot_entry("BootOrder: 0001\nBoot0001* UEFI QEMU DVD-ROM\n"), None);
    }

    #[test]
    fn the_welcome_screen_says_what_to_do_with_and_without_an_entry() {
        let with = sentence(&Found { device: "sda2".into(), boot_entry: Some("0003".into()) });
        assert!(with.contains("already installed") && with.contains("/dev/sda2") && with.contains("Start it"), "{with}");
        let without = sentence(&Found { device: "vda3".into(), boot_entry: None });
        assert!(without.contains("Remove the installer disc (or ISO)"), "{without}");
        assert!(!without.contains("Start it"), "no button to promise");
    }
}
