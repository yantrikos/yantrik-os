//! Booting from an EFI system partition that other systems share.
//!
//! Yantrik's GRUB always goes to `\EFI\yantrik\grubx64.efi`. The two shared things are decided
//! here: the removable-media fallback `\EFI\BOOT\BOOTX64.EFI`, which a Mac's Option-key menu
//! shows as "EFI Boot" and which another system may already own, and the firmware's NVRAM boot
//! entries, which on a Mac decide what starts when nobody holds a key.

/// Whether this machine is a Mac, by any one of: DMI's vendor (`dmi`: `/sys/class/dmi/id/`'s
/// `sys_vendor`, and `board_vendor` and `bios_vendor` with it, or `None` when `sys_vendor` could
/// not be read), the firmware's vendor (`/sys/firmware/efi/fw_vendor`), or an Apple partition
/// type (APFS, HFS+, any other of Apple's) on the disk being installed to.
///
/// It fails toward Apple: a machine taken for a Mac has nothing written to its NVRAM, which costs
/// a PC its boot entry, while a Mac taken for a PC would have its startup disk changed. So a
/// machine whose DMI cannot be read is a Mac.
pub fn is_apple_machine(dmi: Option<&str>, fw_vendor: Option<&str>, apple_types_on_disk: bool) -> bool {
    let says_apple = |v: Option<&str>| v.is_some_and(|v| v.to_ascii_lowercase().contains("apple"));
    apple_types_on_disk || dmi.is_none() || says_apple(dmi) || says_apple(fw_vendor)
}

/// What is at `\EFI\BOOT\BOOTX64.EFI` on an EFI partition, before anything is written to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Existing {
    /// Nothing: a directory on the way holds no such name.
    Nothing,
    /// There, at this path, in whatever case it was written.
    At(String),
    /// A directory on the way could not be read: counted as one that is there and is not ours.
    Unknown(String),
}

/// Find `\EFI\BOOT\BOOTX64.EFI` under `esp` by listing each directory on the way and matching
/// names without regard to case, as FAT does (`efi/boot/bootx64.efi` is the same file). `list`
/// gives one directory's names, or why it could not.
pub fn find_fallback(esp: &str, list: &dyn Fn(&str) -> Result<Vec<String>, String>) -> Existing {
    let mut path = esp.trim_end_matches('/').to_string();
    for want in ["EFI", "BOOT", "BOOTX64.EFI"] {
        let names = match list(&path) {
            Ok(names) => names,
            Err(e) => return Existing::Unknown(format!("{path} could not be read: {e}")),
        };
        match names.iter().find(|n| n.trim_end_matches(['\r', '\n']).eq_ignore_ascii_case(want)) {
            Some(name) => path = format!("{path}/{}", name.trim_end_matches(['\r', '\n'])),
            None => return Existing::Nothing,
        }
    }
    Existing::At(path)
}

/// Next to `\EFI\BOOT\BOOTX64.EFI` when Yantrik wrote it: the digest of the file it wrote. A
/// fallback whose digest matches is ours to replace; any other is not, even one that Yantrik
/// wrote once and something else has since replaced.
pub const OWNER_FILE: &str = "EFI/BOOT/YANTRIK.OWN";

/// What [`OWNER_FILE`] says for a BOOTX64.EFI of these bytes: `sha256=<hex>`, the line the text
/// installer writes with sha256sum.
pub fn owner_record(bootx64: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let hex: String = Sha256::digest(bootx64).iter().map(|b| format!("{b:02x}")).collect();
    owner_line(&hex)
}

/// [`OWNER_FILE`]'s line for a BOOTX64.EFI whose sha256 is `hex` (as sha256sum prints it).
pub fn owner_line(hex: &str) -> String {
    format!("sha256={}\n", hex.trim().to_ascii_lowercase())
}

/// Whether the BOOTX64.EFI with these bytes is the one Yantrik wrote, by `owner` (the contents
/// of [`OWNER_FILE`], empty when there is none).
pub fn is_ours(bootx64: &[u8], owner: &str) -> bool {
    !owner.trim().is_empty() && owner.trim() == owner_record(bootx64).trim()
}

/// [`is_ours`], from the file's sha256 as sha256sum prints it rather than its bytes.
pub fn is_ours_by_digest(hex: &str, owner: &str) -> bool {
    !hex.trim().is_empty() && !owner.trim().is_empty() && owner.trim() == owner_line(hex).trim()
}

/// What to do about `\EFI\BOOT\BOOTX64.EFI`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fallback {
    /// None is there, or the one there is ours: write it (`grub-install --removable`).
    Write,
    /// Another system's is there: leave it, and say how Yantrik is started instead.
    Keep(String),
}

/// `existing` is whether a BOOTX64.EFI is on the partition, `ours` whether it is Yantrik's.
/// A whole-disk install made the partition itself, so it always writes.
pub fn fallback(in_partition: bool, existing: bool, ours: bool, apple: bool) -> Fallback {
    if !in_partition || !existing || ours {
        return Fallback::Write;
    }
    Fallback::Keep(if apple {
        "The EFI partition already has a \\EFI\\BOOT\\BOOTX64.EFI that is not Yantrik's, so it was left \
         alone. Start Yantrik OS from rEFInd if it is installed, or add \\EFI\\yantrik\\grubx64.efi to \
         your boot manager."
            .into()
    } else {
        "The EFI partition already has a \\EFI\\BOOT\\BOOTX64.EFI that is not Yantrik's, so it was left \
         alone. Yantrik OS starts from its own entry in the firmware's boot menu."
            .into()
    })
}

/// What to ask of the firmware's NVRAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nvram {
    /// Nothing at all: no entry, no change to the boot order.
    Untouched,
    /// An entry for `\EFI\yantrik\grubx64.efi`; first in the boot order only when `first`.
    Entry { first: bool },
}

/// A Mac's NVRAM is never written: macOS stays what starts by default, and Yantrik is reached
/// with the Option key. A USB disk gets no entry either (it moves between machines). Installed
/// into a partition on any other UEFI machine, the entry goes last unless the person asked for
/// first. A whole-disk install is first, as it always was.
pub fn nvram(apple: bool, external: bool, in_partition: bool, boot_first: bool) -> Nvram {
    if apple || external {
        return Nvram::Untouched;
    }
    Nvram::Entry { first: !in_partition || boot_first }
}

/// The boot order with `new` added: first when asked, otherwise last; never a duplicate.
/// `order` is efibootmgr's `BootOrder:` value (`0000,0003,0001`).
pub fn boot_order_with(order: &str, new: &str, first: bool) -> String {
    let mut entries: Vec<&str> = order.split(',').map(str::trim).filter(|e| !e.is_empty() && *e != new).collect();
    if first {
        entries.insert(0, new);
    } else {
        entries.push(new);
    }
    entries.join(",")
}

/// The `BootNNNN` efibootmgr printed for the entry it just made with `label`, from its listing.
pub fn entry_number(efibootmgr_output: &str, label: &str) -> Option<String> {
    efibootmgr_output.lines().rev().find_map(|l| {
        let rest = l.strip_prefix("Boot")?;
        let (num, tail) = rest.split_at(rest.find(|c: char| !c.is_ascii_hexdigit())?);
        (num.len() == 4 && tail.trim_start_matches('*').trim().starts_with(label)).then(|| num.to_string())
    })
}

/// `/etc/grub.d/35_yantrik_macos` on a Mac installed beside macOS. GRUB cannot read APFS, so it
/// cannot chainload macOS's `boot.efi` from inside the container; the entry restarts the Mac
/// instead, and since its NVRAM was never touched, macOS is what it starts.
pub const GRUB_MACOS_SCRIPT: &str = r#"#!/bin/sh
# Written by the Yantrik OS installer, which installed Yantrik beside macOS on this disk.
# GRUB cannot read APFS, so this does not load macOS itself: it restarts the Mac, and the Mac
# starts macOS, which is still its startup disk. Hold Option at the chime to choose instead.
cat <<'EOF'
menuentry 'macOS (restarts this Mac into macOS)' --class macosx {
	reboot
}
EOF
"#;

/// Whether a disk with these partition kinds keeps macOS: a macOS menu entry is worth writing.
pub fn keeps_macos(kinds: &[&str]) -> bool {
    kinds.contains(&"macos")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mac_is_known_by_any_one_sign_and_an_unreadable_dmi_is_a_mac() {
        assert!(is_apple_machine(Some("Apple Inc.\n"), None, false));
        assert!(is_apple_machine(Some("Apple Computer, Inc."), None, false));
        assert!(is_apple_machine(Some("Acme Clone Co. Apple Inc."), None, false), "board or bios vendor");
        assert!(is_apple_machine(Some("LENOVO"), Some("Apple"), false), "the firmware's vendor");
        assert!(is_apple_machine(Some("LENOVO"), None, true), "APFS or HFS+ on the disk");
        assert!(is_apple_machine(None, None, false), "DMI could not be read");
        assert!(!is_apple_machine(Some("LENOVO"), Some("0x7f6b2018"), false));
        assert!(!is_apple_machine(Some(""), None, false));
    }

    #[test]
    fn the_fallback_loader_is_found_in_any_case_and_an_unreadable_directory_counts_as_one() {
        let tree = |dirs: &'static [(&'static str, &'static [&'static str])]| {
            move |dir: &str| -> Result<Vec<String>, String> {
                dirs.iter()
                    .find(|(d, _)| *d == dir)
                    .map(|(_, names)| names.iter().map(|n| n.to_string()).collect())
                    .ok_or_else(|| "Permission denied".to_string())
            }
        };
        let lower = tree(&[("/esp", &["efi"]), ("/esp/efi", &["APPLE", "boot"]), ("/esp/efi/boot", &["bootx64.efi"])]);
        assert_eq!(find_fallback("/esp/", &lower), Existing::At("/esp/efi/boot/bootx64.efi".into()));
        let none = tree(&[("/esp", &["EFI"]), ("/esp/EFI", &["APPLE"])]);
        assert_eq!(find_fallback("/esp", &none), Existing::Nothing);
        let empty_boot = tree(&[("/esp", &["EFI"]), ("/esp/EFI", &["BOOT"]), ("/esp/EFI/BOOT", &["YANTRIK.OWN"])]);
        assert_eq!(find_fallback("/esp", &empty_boot), Existing::Nothing);
        // The BOOT directory could not be listed: there may be a loader in it, and not ours.
        let unreadable = tree(&[("/esp", &["EFI"]), ("/esp/EFI", &["BOOT"])]);
        let found = find_fallback("/esp", &unreadable);
        assert!(matches!(&found, Existing::Unknown(why) if why.contains("/esp/EFI/BOOT")), "{found:?}");
        assert!(matches!(fallback(true, found != Existing::Nothing, false, true), Fallback::Keep(_)));
    }

    #[test]
    fn a_macs_nvram_is_never_written() {
        assert_eq!(nvram(true, false, true, false), Nvram::Untouched);
        assert_eq!(nvram(true, false, true, true), Nvram::Untouched, "not even when asked");
        assert_eq!(nvram(true, false, false, false), Nvram::Untouched);
        assert_eq!(nvram(false, true, false, false), Nvram::Untouched, "a USB disk gets no entry");
        assert_eq!(nvram(false, false, true, false), Nvram::Entry { first: false });
        assert_eq!(nvram(false, false, true, true), Nvram::Entry { first: true });
        assert_eq!(nvram(false, false, false, false), Nvram::Entry { first: true });
    }

    #[test]
    fn another_systems_fallback_loader_is_left_alone() {
        assert_eq!(fallback(true, false, false, true), Fallback::Write, "a Mac with none gets one");
        assert_eq!(fallback(true, true, true, true), Fallback::Write, "ours is replaced");
        assert!(matches!(fallback(true, true, false, true), Fallback::Keep(m) if m.contains("rEFInd")));
        assert!(matches!(fallback(true, true, false, false), Fallback::Keep(_)));
        assert_eq!(fallback(false, true, false, false), Fallback::Write, "a whole-disk install made the ESP");
    }

    #[test]
    fn ownership_is_the_digest_of_what_was_written() {
        let ours = b"grub image";
        let record = owner_record(ours);
        // `printf 'grub image' | sha256sum`
        assert_eq!(record, "sha256=977898ce6cb0ef1775b824aa7aed1e3d028e9cf44d4d5a8bf93efbf21736472e\n");
        assert!(is_ours(ours, &record));
        assert!(!is_ours(b"rEFInd", &record), "replaced since: not ours any more");
        assert!(!is_ours(ours, ""), "no record: not ours");
        let hex = "977898CE6CB0EF1775B824AA7AED1E3D028E9CF44D4D5A8BF93EFBF21736472E";
        assert!(is_ours_by_digest(hex, &record));
        assert!(!is_ours_by_digest("", &record));
        assert!(!is_ours_by_digest(hex, ""));
    }

    #[test]
    fn a_new_entry_goes_last_unless_asked() {
        assert_eq!(boot_order_with("0000,0003", "0004", false), "0000,0003,0004");
        assert_eq!(boot_order_with("0000,0003", "0004", true), "0004,0000,0003");
        assert_eq!(boot_order_with("0004,0000", "0004", false), "0000,0004");
        assert_eq!(boot_order_with("", "0004", false), "0004");
        let listing = "BootCurrent: 0000\nBootOrder: 0004,0000\nBoot0000* Windows Boot Manager\tHD(1,...)\nBoot0004* Yantrik OS\tHD(1,...)\n";
        assert_eq!(entry_number(listing, "Yantrik OS"), Some("0004".into()));
        assert_eq!(entry_number(listing, "ubuntu"), None);
    }

    #[test]
    fn the_macos_entry_restarts_rather_than_reading_apfs() {
        assert!(GRUB_MACOS_SCRIPT.starts_with("#!/bin/sh"));
        assert!(GRUB_MACOS_SCRIPT.contains("menuentry 'macOS"));
        assert!(GRUB_MACOS_SCRIPT.contains("\treboot"));
        assert!(keeps_macos(&["esp", "macos", "placeholder"]));
        assert!(!keeps_macos(&["esp", "windows"]));
    }
}
