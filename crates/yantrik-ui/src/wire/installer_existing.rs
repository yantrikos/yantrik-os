//! A Yantrik already installed on this computer's disk, found when the live installer starts
//! (#1165). On Proxmox and QEMU the ISO is still attached after an install, so the machine boots
//! the installer again and the person thinks the install looped. The Welcome screen then says
//! the install is there and offers to start it.
//!
//! The disk is only read: lsblk's udev records, never a mount, and a LUKS volume is recognised by
//! the label the installer gave its header, never opened. Starting it is a one-shot UEFI BootNext
//! to the firmware entry that points at that disk's EFI partition (installer_boot_entry.rs); with
//! no clear entry there is no button and the screen says how to choose the disk by hand. A
//! failure is said on the screen, never only logged.

use slint::ComponentHandle;
use yantrik_install_target::apply::Ran;

use super::installer_boot_entry as entries;
use super::installer_disk::{CRYPT_LABEL, ROOT_LABEL};
use super::installer_probe::run_ran;
use crate::App;

/// How to start the install by hand, for every case the button cannot.
const BY_HAND: &str = "restart and choose the disk in your computer's boot menu (often F12 or Esc)";

/// What was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The partition, `sda2`.
    pub device: String,
    /// `BootNNNN`'s number, when exactly one firmware entry starts this disk's install.
    pub boot_entry: Option<String>,
    /// The install is on a removable drive (a USB stick or card), not the computer's own disk.
    pub removable: bool,
}

/// Yantrik's own partition among `rows`: the root filesystem the installer labels, or the LUKS
/// container it labels when it encrypts. The label alone is not enough: the live ISO and the
/// Mac's placeholder carry YANTRIK on other filesystems.
fn installed_row(rows: &[entries::Row]) -> Option<&entries::Row> {
    rows.iter().find(|r| (r.fstype == "ext4" && r.label == ROOT_LABEL) || (r.fstype == "crypto_LUKS" && r.label == CRYPT_LABEL))
}

/// The partition of Yantrik's own making in `lsblk -rno NAME,FSTYPE,LABEL,...`.
pub fn installed_partition(lsblk: &str) -> Option<String> {
    installed_row(&entries::rows(lsblk)).map(|r| r.name.clone())
}

/// What the install is and which firmware entry starts it, from the two listings.
pub fn find(lsblk: &str, efibootmgr_verbose: Option<&str>) -> Option<Found> {
    let rows = entries::rows(lsblk);
    let root = installed_row(&rows)?;
    let boot_entry = efibootmgr_verbose
        .zip(entries::esp_guid(&rows, &root.parent))
        .and_then(|(listing, esp)| entries::entry_for(listing, &esp));
    Some(Found { device: root.name.clone(), boot_entry, removable: root.removable })
}

/// What the Welcome screen says.
pub fn sentence(found: &Found) -> String {
    let place = if found.removable { "on a removable drive" } else { "on this computer" };
    let head = format!("Yantrik is already installed {place} (/dev/{}).", found.device);
    if found.boot_entry.is_some() {
        format!("{head} Start it, or install again. If the installer disc or ISO is still attached, remove it first.")
    } else {
        format!("{head} To use it, remove the installer disc (or ISO), then {BY_HAND}. Or install again.")
    }
}

fn detect() -> Option<Found> {
    let listing = run_ran("lsblk", &["-rno", entries::LSBLK_COLUMNS]);
    if listing.code != Some(0) {
        return None;
    }
    let verbose = std::path::Path::new("/sys/firmware/efi")
        .exists()
        .then(|| run_ran("efibootmgr", &["-v"]))
        .filter(|r| r.code == Some(0));
    find(&listing.stdout, verbose.as_ref().map(|r| r.stdout.as_str()))
}

/// BootNext to Yantrik's entry, then restart. BootNext is used once and leaves the order alone.
/// On `Err` the sentence is what to show: if the restart failed after BootNext was set, BootNext
/// is cleared again so a later, unrelated restart does not land somewhere unexpected.
pub fn start_with(entry: &str, run: &dyn Fn(&str, &[&str]) -> Ran) -> Result<(), String> {
    let set = run("efibootmgr", &["-n", entry]);
    if set.code != Some(0) {
        tracing::warn!(stderr = %set.stderr.trim(), "Installer: BootNext was refused");
        return Err(format!("Couldn't set the next boot; {BY_HAND}."));
    }
    let restart = run("reboot", &[]);
    if restart.code == Some(0) {
        return Ok(());
    }
    tracing::warn!(stderr = %restart.stderr.trim(), "Installer: the restart failed after BootNext");
    if run("efibootmgr", &["-N"]).code == Some(0) {
        Err(format!("Couldn't restart, and the next boot was cleared; restart by hand, then {BY_HAND}."))
    } else {
        Err(format!("Couldn't restart; the next boot may still be set to Yantrik. Restart by hand, or {BY_HAND}."))
    }
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
            let weak = ui.as_weak();
            ui.on_onboard_start_installed(move || {
                let (Some(ui), Some(entry)) = (weak.upgrade(), &found.boot_entry) else { return };
                if let Err(said) = start_with(entry, &run_ran) {
                    ui.set_onboard_existing_install(format!("Yantrik is already installed (/dev/{}). {said}", found.device).into());
                    ui.set_onboard_existing_startable(false);
                }
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    const LIVE: &str = "sr0 iso9660 YANTRIK    0\nloop0 squashfs     0\nsda      0\n";
    const ESP: &str = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";
    const G: &str = "5f1b6e0a-2d4c-4a0b-9b53-1c2e8a7d9f10";

    fn disk() -> String {
        format!("{LIVE}sda1 vfat  sda {G} {ESP} 0\nsda2 ext4 YANTRIK sda 1111 0fc63daf-8483-4772-8e79-3d69d8477de4 0\n")
    }
    fn firmware(label: &str) -> String {
        format!("BootOrder: 0002\nBoot0002* {label}\tHD(1,GPT,{G},0x800,0x100000)/File(\\EFI\\yantrik\\grubx64.efi)\n")
    }

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
    fn the_button_is_offered_only_for_the_entry_on_the_installs_own_disk() {
        let found = find(&disk(), Some(&firmware("Yantrik OS"))).unwrap();
        assert_eq!((found.device.as_str(), found.boot_entry.as_deref()), ("sda2", Some("0002")));
        let other = firmware("Yantrik OS").replace(G, "9a77c3d2-0e11-4c55-8f0a-77d1b2c3e4f5");
        assert_eq!(find(&disk(), Some(&other)).unwrap().boot_entry, None, "another disk's entry is not this install's");
        assert_eq!(find(&disk(), Some(&firmware("yantrik-live"))).unwrap().boot_entry, None, "never the live entry");
        assert_eq!(find(&disk(), None).unwrap().boot_entry, None, "BIOS or no efibootmgr: no button");
        let no_esp = format!("{LIVE}sda2 ext4 YANTRIK sda 1111 0fc63daf-8483-4772-8e79-3d69d8477de4 0\n");
        assert_eq!(find(&no_esp, Some(&firmware("Yantrik OS"))).unwrap().boot_entry, None, "no ESP on the disk: no match");
    }

    #[test]
    fn an_install_on_a_removable_drive_says_so() {
        let usb = "sdb1 vfat  sdb 1 c12a7328-f81f-11d2-ba4b-00a0c93ec93b 1\nsdb2 ext4 YANTRIK sdb 2 0fc63daf-8483-4772-8e79-3d69d8477de4 1\n";
        let found = find(usb, None).unwrap();
        assert!(found.removable);
        assert!(sentence(&found).contains("on a removable drive (/dev/sdb2)"), "{}", sentence(&found));
    }

    #[test]
    fn the_welcome_screen_says_what_to_do_with_and_without_an_entry() {
        let with = sentence(&Found { device: "sda2".into(), boot_entry: Some("0003".into()), removable: false });
        assert!(with.contains("already installed") && with.contains("/dev/sda2") && with.contains("Start it"), "{with}");
        let without = sentence(&Found { device: "vda3".into(), boot_entry: None, removable: false });
        assert!(without.contains("remove the installer disc (or ISO)") && without.contains("boot menu (often F12 or Esc)"), "{without}");
        assert!(!without.contains("Start it"), "no button to promise");
    }

    /// Commands run, in order, answering with the exit codes given for each program.
    fn runner(codes: &'static [(&'static str, i32)]) -> (Rc<RefCell<Vec<String>>>, impl Fn(&str, &[&str]) -> Ran) {
        let log = Rc::new(RefCell::new(Vec::new()));
        let seen = log.clone();
        let run = move |cmd: &str, args: &[&str]| {
            let line = format!("{cmd} {}", args.join(" ")).trim().to_string();
            let code = codes.iter().find(|(l, _)| *l == line).map_or(0, |(_, c)| *c);
            seen.borrow_mut().push(line);
            Ran { code: Some(code), stdout: String::new(), stderr: "boom".into() }
        };
        (log, run)
    }

    #[test]
    fn a_refused_bootnext_is_said_and_nothing_restarts() {
        let (log, run) = runner(&[("efibootmgr -n 0002", 1)]);
        let said = start_with("0002", &run).unwrap_err();
        assert!(said.starts_with("Couldn't set the next boot; restart and choose the disk in your computer's boot menu (often F12 or Esc)"), "{said}");
        assert_eq!(*log.borrow(), ["efibootmgr -n 0002"], "no restart after a refusal");
    }

    #[test]
    fn a_failed_restart_clears_bootnext_and_says_so() {
        let (log, run) = runner(&[("reboot", 1)]);
        let said = start_with("0002", &run).unwrap_err();
        assert!(said.contains("next boot was cleared"), "{said}");
        assert_eq!(*log.borrow(), ["efibootmgr -n 0002", "reboot", "efibootmgr -N"]);
        let (_, run) = runner(&[("reboot", 1), ("efibootmgr -N", 1)]);
        assert!(start_with("0002", &run).unwrap_err().contains("may still be set"), "an uncleared BootNext is not hidden");
    }

    #[test]
    fn a_good_start_sets_bootnext_then_restarts_and_clears_nothing() {
        let (log, run) = runner(&[]);
        assert_eq!(start_with("0002", &run), Ok(()));
        assert_eq!(*log.borrow(), ["efibootmgr -n 0002", "reboot"]);
    }
}
