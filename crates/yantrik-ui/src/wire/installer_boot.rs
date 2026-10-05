//! GRUB on an EFI machine, whether the installer made the EFI partition or found it shared.
//!
//! Two installs, and both used to be needed everywhere. The first writes \EFI\yantrik and,
//! where allowed, a firmware boot entry for it. The second writes \EFI\BOOT\BOOTX64.EFI, the
//! removable-media path every UEFI implementation tries when it has no entry of its own, and the
//! one a Mac's Option-key menu shows as "EFI Boot".
//!
//! Only the first used to run, with `--no-nvram`, so the disk had a bootloader in a directory
//! nothing had been told to look in: the machine installed, rebooted, and came back up on the
//! installation media.
//!
//! Installed into a partition (wire/installer_partition.rs) the EFI partition is shared, so the
//! decisions are crates/yantrik-install-target's `efi` rules: a Mac's NVRAM is never written
//! (macOS stays what starts; Yantrik is picked with Option), another system's BOOTX64.EFI is
//! never replaced, and on any other UEFI machine the new entry goes last unless the person
//! asked for first.

use yantrik_install_target::efi::{self, Existing, Fallback, Nvram};
use yantrik_install_target::{parse_target_id, TargetSpec};

use super::installer::{chroot_cmd, run_cmd, sudo_write};
use super::installer_disk::Layout;

const NAMED: &[&str] =
    &["grub-install", "--target=x86_64-efi", "--efi-directory=/boot/efi", "--bootloader-id=yantrik"];

/// Install GRUB to \EFI\yantrik\grubx64.efi, and the fallback where it may go. `Ok(Some(note))`
/// when the person has something to know: how Yantrik starts when the fallback was left alone,
/// or that the firmware took no entry.
pub fn install_efi(
    mount_dir: &str,
    disk: &str,
    layout: &Layout,
    external: bool,
    boot_first: bool,
) -> Result<Option<String>, String> {
    let apple = super::installer_partition::is_apple(Some(disk));
    let nvram = efi::nvram(apple, external, layout.in_partition, boot_first);
    tracing::info!(apple, external, in_partition = layout.in_partition, ?nvram, "Installer: installing GRUB for EFI");
    let mut notes: Vec<String> = Vec::new();

    // What is at \EFI\BOOT\BOOTX64.EFI, asked before any grub-install writes to the partition:
    // each directory listed and its names matched without regard to case, as FAT matches them.
    // A directory that cannot be listed counts as holding another system's loader.
    let esp = format!("{mount_dir}/boot/efi");
    let existing = efi::find_fallback(&esp, &|dir| {
        run_cmd("ls", &["-A1", "--", dir]).map(|out| out.lines().map(String::from).collect())
    });
    let ours = match &existing {
        Existing::At(path) => fallback_is_ours(&esp, path),
        Existing::Nothing | Existing::Unknown(_) => false,
    };
    let fallback = efi::fallback(layout.in_partition, existing != Existing::Nothing, ours, apple);
    tracing::info!(?existing, ours, ?fallback, "Installer: the EFI partition's fallback loader, before writing");

    let mut no_nvram: Vec<&str> = NAMED.to_vec();
    no_nvram.push("--no-nvram");
    if nvram == (Nvram::Entry { first: true }) {
        // Firmware that will not take a new entry is normal enough — a locked-down board, or
        // efivars mounted read-only. It costs the named entry, not the install, because the
        // removable path does not need NVRAM at all.
        if let Err(e) = chroot_cmd(mount_dir, NAMED) {
            tracing::warn!(error = %e, "could not register a UEFI boot entry; the removable path will carry the boot");
            chroot_cmd(mount_dir, &no_nvram)?;
        }
    } else {
        chroot_cmd(mount_dir, &no_nvram)?;
    }
    if nvram == (Nvram::Entry { first: false }) {
        if let Err(e) = add_entry_last(mount_dir, disk, layout) {
            tracing::warn!(error = %e, "could not add a UEFI boot entry for Yantrik OS");
            notes.push("The firmware took no boot entry for Yantrik OS; choose it from the firmware's boot menu.".into());
        }
    }

    match fallback {
        Fallback::Write => {
            // Not optional once chosen: its failure is the install's failure. --no-nvram: the
            // removable path is found by the firmware without an entry, and this one is never
            // to touch a Mac's NVRAM, whatever grub-install would do by default.
            chroot_cmd(
                mount_dir,
                &["grub-install", "--target=x86_64-efi", "--efi-directory=/boot/efi", "--removable", "--no-nvram"],
            )?;
            // Check the file, not the exit code. grub-install has been known to report success
            // having written nothing useful.
            let written = format!("{esp}/EFI/BOOT/BOOTX64.EFI");
            let digest = run_cmd("sha256sum", &[&written]).map_err(|_| {
                "grub-install reported success but left no EFI/BOOT/BOOTX64.EFI on the EFI partition; \
                 the disk would not boot"
                    .to_string()
            })?;
            let hex = digest.split_whitespace().next().unwrap_or("");
            sudo_write(&format!("{esp}/{}", efi::OWNER_FILE), &efi::owner_line(hex))?;
            tracing::info!("Installer: EFI fallback bootloader present");
            if apple && layout.in_partition {
                notes.push(
                    "macOS still starts by default. To start Yantrik OS, hold Option at the chime and choose EFI Boot."
                        .into(),
                );
            }
        }
        Fallback::Keep(note) => {
            tracing::info!(existing = ?existing, "Installer: another system's fallback loader left in place");
            notes.push(note);
        }
    }
    Ok((!notes.is_empty()).then(|| notes.join(" ")))
}

/// Whether the BOOTX64.EFI at `path` is the one Yantrik wrote: its sha256 is the one recorded
/// beside it.
fn fallback_is_ours(esp: &str, path: &str) -> bool {
    let owner = run_cmd("cat", &[&format!("{esp}/{}", efi::OWNER_FILE)]).unwrap_or_default();
    let digest = run_cmd("sha256sum", &[path]).unwrap_or_default();
    efi::is_ours_by_digest(digest.split_whitespace().next().unwrap_or(""), &owner)
}

/// A firmware entry for \EFI\yantrik, last in the boot order: what was starting keeps starting.
fn add_entry_last(mount_dir: &str, disk: &str, layout: &Layout) -> Result<(), String> {
    let name = layout.efi_part.trim_start_matches("/dev/");
    let Ok((_, TargetSpec::Partition(part))) = parse_target_id(name) else {
        return Err(format!("cannot tell which partition {} is", layout.efi_part));
    };
    let shim = std::path::Path::new(&format!("{mount_dir}/boot/efi/EFI/yantrik/shimx64.efi")).exists();
    let loader = if shim { "\\EFI\\yantrik\\shimx64.efi" } else { "\\EFI\\yantrik\\grubx64.efi" };
    let listing = run_cmd("efibootmgr", &[])?;
    let order = listing
        .lines()
        .find_map(|l| l.strip_prefix("BootOrder:"))
        .unwrap_or("")
        .trim()
        .to_string();
    // -C: create the entry without putting it in the boot order; the order is set below.
    let made = run_cmd(
        "efibootmgr",
        &["-C", "-d", disk, "-p", &part.to_string(), "-L", "Yantrik OS", "-l", loader],
    )?;
    let num = efi::entry_number(&made, "Yantrik OS").ok_or("efibootmgr did not list the new entry")?;
    run_cmd("efibootmgr", &["-o", &efi::boot_order_with(&order, &num, false)])?;
    tracing::info!(entry = %num, "Installer: UEFI entry added, last in the boot order");
    Ok(())
}

/// The boot menu's macOS entry, on a Mac installed beside macOS (efi::GRUB_MACOS_SCRIPT). Written
/// before update-grub. A failure costs the entry, never the install: Option at the chime still
/// reaches macOS. A disk that keeps macOS has Apple partition types, which makes the machine a
/// Mac by efi::is_apple_machine; nothing else need be asked.
pub fn write_macos_entry(mount_dir: &str, layout: &Layout) {
    if !(layout.in_partition && layout.keeps_macos) {
        return;
    }
    let path = format!("{mount_dir}/etc/grub.d/35_yantrik_macos");
    let written = sudo_write(&path, efi::GRUB_MACOS_SCRIPT).and_then(|_| run_cmd("chmod", &["0755", &path]));
    if let Err(e) = written {
        tracing::warn!(error = %e, "could not add the macOS entry to the boot menu");
    }
}
