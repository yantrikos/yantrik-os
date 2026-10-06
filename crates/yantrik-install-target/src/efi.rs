//! Booting from an EFI system partition that other systems share.
//!
//! Yantrik's GRUB always goes to `\EFI\yantrik\grubx64.efi`. The two shared things are decided
//! here: the removable-media fallback directory `\EFI\BOOT`, whose `BOOTX64.EFI` a Mac's
//! Option-key menu shows as "EFI Boot" and whose files another system may already own (Yantrik
//! writes exactly [`SHIM_SET`] there, never over another system's file, and records each in
//! [`OWNER_NAME`]), and the firmware's NVRAM boot entries, which on a Mac decide what starts when
//! nobody holds a key.

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

/// The files Yantrik writes in `\EFI\BOOT`, each with the file in `\EFI\yantrik` (what the named
/// grub-install just wrote) it is a copy of. Nothing else is ever written there, and none of
/// these is written over another system's file: see [`write_fallback`].
///
/// With shim, which the image carries (grub-efi-amd64-signed and shim-signed come in through
/// grub-efi-amd64-bin's recommends; the VM rehearsal found shim's files on the disk), all four:
///
/// - `BOOTX64.EFI`: shim, the file the firmware starts from the removable-media path;
/// - `grubx64.efi`: the signed GRUB shim starts, looked for beside shim;
/// - `mmx64.efi`: MokManager. Not needed for an ordinary boot, but shim starts it, from beside
///   itself, whenever a key enrolment is pending, and does not go on to GRUB when it cannot;
/// - `grub.cfg`: the three lines that find /boot by UUID. The signed GRUB's built-in prefix is
///   the distribution's own directory, which is not on this disk, so it reads the grub.cfg
///   beside itself.
///
/// grub-install `--removable` also writes `BOOTX64.CSV`. It is left out: only `fbx64.efi`
/// reads it, fbx64.efi is not written here (so shim goes straight to grubx64.efi), and fbx64.efi
/// only reads CSVs in vendor directories, never in `\EFI\BOOT`, anyway.
pub const SHIM_SET: [(&str, &str); 4] = [
    ("BOOTX64.EFI", "shimx64.efi"),
    ("grubx64.efi", "grubx64.efi"),
    ("mmx64.efi", "mmx64.efi"),
    ("grub.cfg", "grub.cfg"),
];

/// Without shim: GRUB itself as `BOOTX64.EFI`, and `grub.cfg` beside it when `\EFI\yantrik` has
/// one (a signed GRUB reads it; an unsigned one has its config built in, and grub-install
/// writes none).
pub const GRUB_SET: [(&str, &str); 2] = [("BOOTX64.EFI", "grubx64.efi"), ("grub.cfg", "grub.cfg")];

/// Every name Yantrik may write in `\EFI\BOOT`: checked, before anything is written, on every
/// install, whichever set it will write.
pub const FALLBACK_NAMES: [&str; 4] = ["BOOTX64.EFI", "grubx64.efi", "mmx64.efi", "grub.cfg"];

/// Never Yantrik's, and never written beside: shim starts an `fbx64.efi` it finds beside itself,
/// and fbx64.efi writes NVRAM boot entries, which a Mac's must never get.
pub const NEVER_BESIDE: [&str; 1] = ["fbx64.efi"];

/// In `\EFI\BOOT`, beside the files Yantrik wrote: what they are. One line per file, as
/// sha256sum prints it, `<sha256 in lowercase hex><two spaces><name>`, so
/// `cd EFI/BOOT && sha256sum -c YANTRIK.OWN` checks them. A file there is ours when its sha256 is
/// listed here; any other is another system's, even one Yantrik wrote once and something has
/// replaced since. An older install wrote one line, `sha256=<hex>`, for BOOTX64.EFI alone; its
/// digest still counts.
pub const OWNER_NAME: &str = "YANTRIK.OWN";

/// [`OWNER_NAME`]'s contents for these files: `(name, sha256 hex)`.
pub fn owner_text(files: &[(String, String)]) -> String {
    files.iter().map(|(name, hex)| format!("{}  {name}\n", hex.trim().to_ascii_lowercase())).collect()
}

/// The digests [`OWNER_NAME`] lists, lowercase, in either format. Anything else is ignored.
pub fn owned_digests(owner: &str) -> Vec<String> {
    owner
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            let hex = l.strip_prefix("sha256=").unwrap_or_else(|| l.split_whitespace().next().unwrap_or(""));
            (hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit())).then(|| hex.to_ascii_lowercase())
        })
        .collect()
}

/// The sha256 of these bytes, as sha256sum prints it.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// An EFI partition, mounted, as the caller reaches it: directly as root (the command-line tool),
/// or through sudo (the desktop installer). Paths are absolute.
pub trait EspFiles {
    /// One directory's names.
    fn list(&self, dir: &str) -> Result<Vec<String>, String>;
    /// A file's sha256, lowercase hex.
    fn sha256(&self, path: &str) -> Result<String, String>;
    fn read(&self, path: &str) -> Result<String, String>;
    fn copy(&self, from: &str, to: &str) -> Result<(), String>;
    fn write(&self, path: &str, text: &str) -> Result<(), String>;
    fn remove(&self, path: &str) -> Result<(), String>;
    fn mkdir(&self, path: &str) -> Result<(), String>;
}

/// The name in `names` that is `want` to FAT, which ignores case.
fn named<'a>(names: &'a [String], want: &str) -> Option<&'a str> {
    names.iter().map(|n| n.trim_end_matches(['\r', '\n'])).find(|n| n.eq_ignore_ascii_case(want))
}

/// `\EFI\BOOT` as it is, found by listing each directory and matching names without regard to
/// case (`efi/boot` is the same directory).
struct BootDir {
    /// `\EFI`'s path, or where it goes.
    efi: String,
    efi_there: bool,
    /// `\EFI\BOOT`'s path, or where it goes.
    boot: String,
    boot_there: bool,
    names: Vec<String>,
}

fn boot_dir(esp: &dyn EspFiles, root: &str) -> Result<BootDir, String> {
    let root = root.trim_end_matches('/');
    let at_root = esp.list(root).map_err(|e| format!("{root} could not be read: {e}"))?;
    let Some(efi) = named(&at_root, "EFI").map(|n| format!("{root}/{n}")) else {
        let efi = format!("{root}/EFI");
        return Ok(BootDir { boot: format!("{efi}/BOOT"), efi, efi_there: false, boot_there: false, names: vec![] });
    };
    let in_efi = esp.list(&efi).map_err(|e| format!("{efi} could not be read: {e}"))?;
    let Some(boot) = named(&in_efi, "BOOT").map(|n| format!("{efi}/{n}")) else {
        return Ok(BootDir { boot: format!("{efi}/BOOT"), efi, efi_there: true, boot_there: false, names: vec![] });
    };
    let names = esp.list(&boot).map_err(|e| format!("{boot} could not be read: {e}"))?;
    Ok(BootDir { efi, efi_there: true, boot, boot_there: true, names })
}

/// The files in `\EFI\BOOT` that stop Yantrik writing there: each of [`FALLBACK_NAMES`] that is
/// there and whose sha256 [`OWNER_NAME`] does not list (or could not be read), and any of
/// [`NEVER_BESIDE`]. Alongside, the ones that are ours, with their digests.
fn sort_out(esp: &dyn EspFiles, dir: &BootDir) -> (Vec<String>, Vec<(String, String)>) {
    let owner = named(&dir.names, OWNER_NAME)
        .map(|n| esp.read(&format!("{}/{n}", dir.boot)).unwrap_or_default())
        .unwrap_or_default();
    let digests = owned_digests(&owner);
    let (mut foreign, mut ours) = (Vec::new(), Vec::new());
    for want in FALLBACK_NAMES {
        let Some(name) = named(&dir.names, want) else { continue };
        match esp.sha256(&format!("{}/{name}", dir.boot)) {
            Ok(hex) if digests.contains(&hex.to_ascii_lowercase()) => ours.push((name.to_string(), hex)),
            _ => foreign.push(name.to_string()),
        }
    }
    foreign.extend(NEVER_BESIDE.iter().filter_map(|w| named(&dir.names, w)).map(String::from));
    (foreign, ours)
}

/// What to do about `\EFI\BOOT`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fallback {
    /// Nothing there is another system's: write Yantrik's set.
    Write,
    /// Something there is another system's, or could not be read: write nothing there at all,
    /// and say this to the person.
    Keep(String),
}

/// `foreign` is what [`check_fallback`] found in `\EFI\BOOT` that is not Yantrik's.
pub fn fallback(foreign: &[String], apple: bool) -> Fallback {
    if foreign.is_empty() {
        return Fallback::Write;
    }
    let what = if foreign.iter().any(|f| f.eq_ignore_ascii_case("BOOTX64.EFI")) {
        "The EFI partition already has a \\EFI\\BOOT\\BOOTX64.EFI that is not Yantrik's, so it was left alone.".to_string()
    } else {
        format!(
            "The EFI partition's \\EFI\\BOOT already holds {} that {} not Yantrik's, so nothing was written there.",
            foreign.join(" and "),
            if foreign.len() == 1 { "is" } else { "are" }
        )
    };
    Fallback::Keep(format!("{what} {}", how_to_start(apple)))
}

fn how_to_start(apple: bool) -> &'static str {
    if apple {
        "Start Yantrik OS from rEFInd if it is installed, or add \\EFI\\yantrik\\grubx64.efi to your boot manager."
    } else {
        "Yantrik OS starts from its own entry in the firmware's boot menu."
    }
}

/// Whether Yantrik may write its set in `\EFI\BOOT` under the EFI partition mounted at `root`.
/// Asked before any grub-install writes to the partition. A directory that cannot be read is
/// one holding another system's files.
pub fn check_fallback(esp: &dyn EspFiles, root: &str, apple: bool) -> Fallback {
    match boot_dir(esp, root) {
        Ok(dir) => fallback(&sort_out(esp, &dir).0, apple),
        Err(e) => Fallback::Keep(unreadable(&e, apple)),
    }
}

fn unreadable(why: &str, apple: bool) -> String {
    format!("The EFI partition's \\EFI\\BOOT could not be read ({why}), so nothing was written there. {}", how_to_start(apple))
}

/// What [`write_fallback`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Written {
    /// These files, `(name, sha256)`, are in `\EFI\BOOT` and listed in [`OWNER_NAME`].
    Set(Vec<(String, String)>),
    /// Nothing was written in `\EFI\BOOT`; why, for the person.
    Kept(String),
}

/// Write Yantrik's set in `\EFI\BOOT` ([`SHIM_SET`], or [`GRUB_SET`] when `\EFI\yantrik` has no
/// shim) by copying from `\EFI\yantrik`, after the named grub-install. Checked again first, as
/// [`check_fallback`]: if any of [`FALLBACK_NAMES`] is there and not ours, or [`NEVER_BESIDE`]
/// is there, nothing is written. Our own files are written over, and one of ours this set no
/// longer has is removed.
///
/// [`OWNER_NAME`] lists every file written. It is written first listing both what is there and
/// what is about to be, so a copy cut short leaves nothing of ours unlisted, and again at the end
/// listing exactly the set. Each copy is checked against its source's digest. `Err` only when the
/// set could not be written; the disk may then not start from the removable path.
pub fn write_fallback(esp: &dyn EspFiles, root: &str, apple: bool) -> Result<Written, String> {
    let dir = match boot_dir(esp, root) {
        Ok(dir) => dir,
        Err(e) => return Ok(Written::Kept(unreadable(&e, apple))),
    };
    let (foreign, ours) = sort_out(esp, &dir);
    if let Fallback::Keep(note) = fallback(&foreign, apple) {
        return Ok(Written::Kept(note));
    }

    let in_efi = if dir.efi_there { esp.list(&dir.efi)? } else { vec![] };
    let yantrik = named(&in_efi, "yantrik")
        .map(|n| format!("{}/{n}", dir.efi))
        .ok_or("the EFI partition has no \\EFI\\yantrik to copy the fallback from")?;
    let sources = esp.list(&yantrik)?;
    let shim = named(&sources, "shimx64.efi").is_some();
    let set: &[(&str, &str)] = if shim { &SHIM_SET } else { &GRUB_SET };
    let mut picks: Vec<(String, String, String)> = Vec::new(); // (name, source path, sha256)
    for (name, from) in set {
        match named(&sources, from) {
            Some(src) => {
                let src = format!("{yantrik}/{src}");
                let hex = esp.sha256(&src)?.to_ascii_lowercase();
                picks.push((name.to_string(), src, hex));
            }
            // Without shim, grub.cfg is copied only when there is one.
            None if !shim && *name == "grub.cfg" => {}
            None => return Err(format!("\\EFI\\yantrik has no {from}; \\EFI\\BOOT would not start")),
        }
    }

    if !dir.efi_there {
        esp.mkdir(&dir.efi)?;
    }
    if !dir.boot_there {
        esp.mkdir(&dir.boot)?;
    }
    // A name already there keeps the case it was written in: FAT has one file either way.
    let target = |name: &str| format!("{}/{}", dir.boot, named(&dir.names, name).unwrap_or(name));
    let owner = target(OWNER_NAME);
    let new: Vec<(String, String)> = picks.iter().map(|(n, _, h)| (n.clone(), h.clone())).collect();
    let mut both = new.clone();
    both.extend(ours.iter().filter(|(n, h)| !new.iter().any(|(m, g)| m.eq_ignore_ascii_case(n) && g == h)).cloned());
    esp.write(&owner, &owner_text(&both))?;
    for (name, src, hex) in &picks {
        let to = target(name);
        esp.copy(src, &to)?;
        let got = esp.sha256(&to)?;
        if !got.eq_ignore_ascii_case(hex) {
            return Err(format!("{to} does not match {src} after copying it"));
        }
    }
    for (name, _) in &ours {
        if !new.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)) {
            esp.remove(&format!("{}/{name}", dir.boot))?;
        }
    }
    esp.write(&owner, &owner_text(&new))?;
    Ok(Written::Set(new))
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

    /// An EFI partition in memory: a path to `Some(bytes)` is a file, to `None` a directory.
    #[derive(Default)]
    struct Fake {
        tree: std::cell::RefCell<std::collections::BTreeMap<String, Option<Vec<u8>>>>,
        unreadable: Vec<String>,
    }

    impl Fake {
        /// A Mac's ESP after the named grub-install with shim: firmware and rEFInd files, and
        /// `\EFI\yantrik` with everything grub-install puts there.
        fn mac() -> Fake {
            let f = Fake::default();
            for d in ["/esp", "/esp/EFI", "/esp/EFI/APPLE", "/esp/EFI/refind", "/esp/EFI/yantrik"] {
                f.tree.borrow_mut().insert(d.into(), None);
            }
            f.put("/esp/EFI/APPLE/log.txt", b"macOS firmware log");
            f.put("/esp/EFI/refind/refind_x64.efi", b"rEFInd");
            for (n, b) in [
                ("shimx64.efi", &b"shim"[..]),
                ("grubx64.efi", b"signed grub"),
                ("mmx64.efi", b"mok manager"),
                ("fbx64.efi", b"fallback"),
                ("BOOTX64.CSV", b"shimx64.efi,yantrik,,"),
                ("grub.cfg", b"search.fs_uuid 1234 root\n"),
            ] {
                f.put(&format!("/esp/EFI/yantrik/{n}"), b);
            }
            f
        }
        fn put(&self, path: &str, bytes: &[u8]) {
            self.tree.borrow_mut().insert(path.into(), Some(bytes.to_vec()));
        }
        fn get(&self, path: &str) -> Option<Vec<u8>> {
            self.tree.borrow().get(path).cloned().flatten()
        }
        fn mkboot(&self) {
            self.tree.borrow_mut().insert("/esp/EFI/BOOT".into(), None);
        }
        /// Every file under `dir`, by name, with its bytes.
        fn files_in(&self, dir: &str) -> Vec<(String, Vec<u8>)> {
            let pre = format!("{dir}/");
            self.tree
                .borrow()
                .iter()
                .filter_map(|(p, b)| Some((p.strip_prefix(&pre)?.to_string(), b.clone()?)))
                .filter(|(n, _)| !n.contains('/'))
                .collect()
        }
        fn names_in(&self, dir: &str) -> Vec<String> {
            self.files_in(dir).into_iter().map(|(n, _)| n).collect()
        }
    }

    impl EspFiles for Fake {
        fn list(&self, dir: &str) -> Result<Vec<String>, String> {
            if self.unreadable.iter().any(|d| d == dir) || self.tree.borrow().get(dir) != Some(&None) {
                return Err("Permission denied".into());
            }
            let pre = format!("{dir}/");
            Ok(self.tree.borrow().keys().filter_map(|p| p.strip_prefix(&pre)).filter(|n| !n.contains('/')).map(String::from).collect())
        }
        fn sha256(&self, path: &str) -> Result<String, String> {
            self.get(path).map(|b| sha256_hex(&b)).ok_or_else(|| format!("{path}: no such file"))
        }
        fn read(&self, path: &str) -> Result<String, String> {
            self.get(path).map(|b| String::from_utf8_lossy(&b).into_owned()).ok_or_else(|| format!("{path}: no such file"))
        }
        fn copy(&self, from: &str, to: &str) -> Result<(), String> {
            let b = self.get(from).ok_or("no source")?;
            self.put(to, &b);
            Ok(())
        }
        fn write(&self, path: &str, text: &str) -> Result<(), String> {
            self.put(path, text.as_bytes());
            Ok(())
        }
        fn remove(&self, path: &str) -> Result<(), String> {
            self.tree.borrow_mut().remove(path).map(|_| ()).ok_or_else(|| "no such file".into())
        }
        fn mkdir(&self, path: &str) -> Result<(), String> {
            self.tree.borrow_mut().insert(path.into(), None);
            Ok(())
        }
    }

    const SET: [&str; 5] = ["BOOTX64.EFI", "YANTRIK.OWN", "grub.cfg", "grubx64.efi", "mmx64.efi"];

    /// Every line of YANTRIK.OWN names a file in `\EFI\BOOT` whose sha256 it is.
    fn owner_checks(f: &Fake) -> Vec<String> {
        let owner = String::from_utf8(f.get("/esp/EFI/BOOT/YANTRIK.OWN").expect("YANTRIK.OWN")).unwrap();
        owner
            .lines()
            .map(|l| {
                let (hex, name) = l.split_once("  ").expect("sha256sum's format");
                assert_eq!(f.sha256(&format!("/esp/EFI/BOOT/{name}")).unwrap(), hex, "{name}");
                name.to_string()
            })
            .collect()
    }

    #[test]
    fn exactly_the_set_is_written_and_each_files_digest_is_recorded() {
        let f = Fake::mac();
        assert_eq!(check_fallback(&f, "/esp", true), Fallback::Write);
        let Written::Set(set) = write_fallback(&f, "/esp", true).unwrap() else { panic!("kept") };
        let names: Vec<&str> = set.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, FALLBACK_NAMES, "the files written are the list");
        assert_eq!(f.names_in("/esp/EFI/BOOT"), SET, "and nothing else: no BOOTX64.CSV, no fbx64.efi");
        assert_eq!(owner_checks(&f), FALLBACK_NAMES, "each one's sha256 is in YANTRIK.OWN");
        assert_eq!(f.get("/esp/EFI/BOOT/BOOTX64.EFI").unwrap(), b"shim", "BOOTX64.EFI is shim");
        assert_eq!(f.get("/esp/EFI/BOOT/grubx64.efi").unwrap(), b"signed grub");
        assert_eq!(f.get("/esp/EFI/refind/refind_x64.efi").unwrap(), b"rEFInd");
    }

    #[test]
    fn without_shim_grub_alone_is_the_fallback() {
        let f = Fake::mac();
        for n in ["shimx64.efi", "mmx64.efi", "fbx64.efi", "BOOTX64.CSV", "grub.cfg"] {
            f.remove(&format!("/esp/EFI/yantrik/{n}")).unwrap();
        }
        let Written::Set(set) = write_fallback(&f, "/esp", false).unwrap() else { panic!("kept") };
        assert_eq!(set, vec![("BOOTX64.EFI".to_string(), sha256_hex(b"signed grub"))]);
        assert_eq!(f.names_in("/esp/EFI/BOOT"), ["BOOTX64.EFI", "YANTRIK.OWN"]);
        assert_eq!(owner_checks(&f), ["BOOTX64.EFI"]);
    }

    #[test]
    fn a_shim_missing_one_of_its_files_is_an_error_not_half_a_fallback() {
        let f = Fake::mac();
        f.remove("/esp/EFI/yantrik/grubx64.efi").unwrap();
        let err = write_fallback(&f, "/esp", true).unwrap_err();
        assert!(err.contains("grubx64.efi"), "{err}");
        assert!(f.list("/esp/EFI/BOOT").is_err(), "nothing was written: {:?}", f.names_in("/esp/EFI/BOOT"));
    }

    #[test]
    fn another_systems_file_in_efi_boot_stops_the_whole_write_and_is_left_byte_for_byte() {
        for (name, bytes) in [
            ("grub.cfg", &b"configfile (hd0,gpt5)/boot/grub/grub.cfg\n"[..]),
            ("grubx64.efi", b"ubuntu's grub"),
            ("BOOTX64.EFI", b"rEFInd"),
            ("bootx64.efi", b"rEFInd, lower case"),
            ("MMX64.EFI", b"another mok manager"),
            ("fbx64.efi", b"another fallback"),
        ] {
            let f = Fake::mac();
            f.mkboot();
            f.put(&format!("/esp/EFI/BOOT/{name}"), bytes);
            let before = f.files_in("/esp/EFI/BOOT");
            let Fallback::Keep(note) = check_fallback(&f, "/esp", true) else { panic!("{name}: not kept") };
            assert!(note.contains("not Yantrik's") && note.contains("rEFInd"), "{name}: {note}");
            if name.eq_ignore_ascii_case("BOOTX64.EFI") {
                assert!(note.starts_with("The EFI partition already has a \\EFI\\BOOT\\BOOTX64.EFI"), "{note}");
            } else {
                assert!(note.contains(name) && note.contains("nothing was written there"), "{note}");
            }
            assert_eq!(write_fallback(&f, "/esp", true).unwrap(), Written::Kept(note), "{name}");
            assert_eq!(f.files_in("/esp/EFI/BOOT"), before, "{name}: \\EFI\\BOOT changed");
        }
    }

    #[test]
    fn a_file_yantrik_wrote_once_and_something_replaced_is_not_ours() {
        let f = Fake::mac();
        write_fallback(&f, "/esp", true).unwrap();
        f.put("/esp/EFI/BOOT/grub.cfg", b"another system's\n");
        let before = f.files_in("/esp/EFI/BOOT");
        let Written::Kept(note) = write_fallback(&f, "/esp", false).unwrap() else { panic!("written") };
        assert!(note.contains("grub.cfg that is not Yantrik's"), "{note}");
        assert!(note.contains("firmware's boot menu"), "{note}");
        assert_eq!(f.files_in("/esp/EFI/BOOT"), before);
    }

    #[test]
    fn a_reinstall_over_our_own_files_is_allowed_and_rerecorded() {
        let f = Fake::mac();
        write_fallback(&f, "/esp", true).unwrap();
        // A newer GRUB in the second install.
        f.put("/esp/EFI/yantrik/grubx64.efi", b"newer signed grub");
        assert_eq!(check_fallback(&f, "/esp", true), Fallback::Write);
        let Written::Set(_) = write_fallback(&f, "/esp", true).unwrap() else { panic!("kept") };
        assert_eq!(f.get("/esp/EFI/BOOT/grubx64.efi").unwrap(), b"newer signed grub");
        assert_eq!(f.names_in("/esp/EFI/BOOT"), SET);
        assert_eq!(owner_checks(&f), FALLBACK_NAMES);

        // Reinstalled without shim: our shim-era files that the new set lacks go.
        for n in ["shimx64.efi", "mmx64.efi", "grub.cfg"] {
            f.remove(&format!("/esp/EFI/yantrik/{n}")).unwrap();
        }
        write_fallback(&f, "/esp", true).unwrap();
        assert_eq!(f.names_in("/esp/EFI/BOOT"), ["BOOTX64.EFI", "YANTRIK.OWN"]);
        assert_eq!(owner_checks(&f), ["BOOTX64.EFI"]);
    }

    #[test]
    fn files_yantrik_does_not_write_are_left_alone_beside_its_set() {
        let f = Fake::mac();
        f.mkboot();
        f.put("/esp/EFI/BOOT/BOOTX64.CSV", b"left by an earlier grub-install --removable");
        f.put("/esp/EFI/BOOT/README.txt", b"someone's note");
        write_fallback(&f, "/esp", true).unwrap();
        assert_eq!(f.get("/esp/EFI/BOOT/BOOTX64.CSV").unwrap(), b"left by an earlier grub-install --removable");
        assert_eq!(f.get("/esp/EFI/BOOT/README.txt").unwrap(), b"someone's note");
        assert_eq!(owner_checks(&f), FALLBACK_NAMES, "and not listed as ours");
    }

    #[test]
    fn efi_boot_is_found_in_any_case_and_an_unreadable_one_stops_the_write() {
        let f = Fake::mac();
        f.tree.borrow_mut().insert("/esp/EFI/boot".into(), None);
        f.put("/esp/EFI/boot/bootx64.efi", b"rEFInd");
        assert!(matches!(check_fallback(&f, "/esp/", true), Fallback::Keep(n) if n.contains("BOOTX64.EFI")));

        let mut f = Fake::mac();
        f.mkboot();
        f.unreadable.push("/esp/EFI/BOOT".into());
        let Fallback::Keep(note) = check_fallback(&f, "/esp", true) else { panic!("written") };
        assert!(note.contains("/esp/EFI/BOOT could not be read") && note.contains("nothing was written"), "{note}");
        assert!(matches!(write_fallback(&f, "/esp", true).unwrap(), Written::Kept(_)));
        assert!(f.files_in("/esp/EFI/BOOT").is_empty());

        // A lower-case \EFI\BOOT of ours is written into where it is, not beside it.
        let f = Fake::mac();
        f.tree.borrow_mut().insert("/esp/EFI/boot".into(), None);
        write_fallback(&f, "/esp", true).unwrap();
        assert_eq!(f.names_in("/esp/EFI/boot"), SET);
        assert!(f.list("/esp/EFI/BOOT").is_err());
    }

    #[test]
    fn the_owner_record_is_sha256sums_format_and_the_older_line_still_counts() {
        // `printf 'grub image' | sha256sum`
        let hex = "977898ce6cb0ef1775b824aa7aed1e3d028e9cf44d4d5a8bf93efbf21736472e";
        assert_eq!(sha256_hex(b"grub image"), hex);
        let text = owner_text(&[("BOOTX64.EFI".into(), hex.to_uppercase()), ("grub.cfg".into(), "ab".repeat(32))]);
        assert_eq!(text, format!("{hex}  BOOTX64.EFI\n{}  grub.cfg\n", "ab".repeat(32)));
        assert_eq!(owned_digests(&text), vec![hex.to_string(), "ab".repeat(32)]);
        assert_eq!(owned_digests(&format!("sha256={hex}\n")), vec![hex.to_string()], "an older install's line");
        assert!(owned_digests("").is_empty());
        assert!(owned_digests("sha256=\nnot a digest  BOOTX64.EFI\n").is_empty());
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
