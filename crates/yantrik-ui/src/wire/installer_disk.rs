//! The installer's disk: its partitions, the encrypted root (#400 step b), the filesystems and
//! their mounts, and what the installed system needs to find them again: fstab, crypttab, and an
//! initramfs that asks for the passphrase.
//!
//! An encrypted install keeps /boot outside the encryption, so GRUB reads the kernel and the
//! initramfs without a key of its own; the initramfs asks for the passphrase (the person's
//! password, typed on the keyboard they chose) and opens the root before anything on it runs.
//!
//! The passphrase reaches cryptsetup on its standard input, never as an argument, where every
//! process on the live system could read it from /proc.

use std::process::{Command, Stdio};

use super::installer::{chroot_cmd, run_cmd, sudo_write};

/// Where the opened root appears, /dev/mapper/<this>, on every encrypted install.
pub const CRYPT_NAME: &str = "yantrik-root";
/// The root filesystem's label, encrypted or not: fstab finds the root by it.
const ROOT_LABEL: &str = "YANTRIK";
const BOOT_LABEL: &str = "YANTRIK_BOOT";
const CRYPT_LABEL: &str = "YANTRIK_CRYPT";

/// Which partitions to make, in order, as `parted` arguments after `-s <disk>`, and which
/// number each ends up as.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub parted: Vec<Vec<&'static str>>,
    pub efi: Option<u8>,
    /// A /boot of its own, only when the root is encrypted.
    pub boot: Option<u8>,
    pub root: u8,
}

/// The partitions for a firmware and a choice of encryption.
///
/// Unencrypted it is the layout every install had before: an EFI system partition (or a BIOS
/// boot partition) and one root. Encrypted, a 1 GiB /boot sits between them, so GRUB never needs
/// the key.
pub fn plan(is_efi: bool, encrypt: bool) -> Plan {
    let mut parted: Vec<Vec<&'static str>> = vec![vec!["mklabel", "gpt"]];
    let root_start;
    if is_efi {
        parted.push(vec!["mkpart", "EFI", "fat32", "1MiB", "513MiB"]);
        parted.push(vec!["set", "1", "esp", "on"]);
        root_start = if encrypt { "1537MiB" } else { "513MiB" };
        if encrypt {
            parted.push(vec!["mkpart", "boot", "ext4", "513MiB", "1537MiB"]);
        }
    } else {
        parted.push(vec!["mkpart", "biosboot", "", "1MiB", "2MiB"]);
        parted.push(vec!["set", "1", "bios_grub", "on"]);
        root_start = if encrypt { "1026MiB" } else { "2MiB" };
        if encrypt {
            parted.push(vec!["mkpart", "boot", "ext4", "2MiB", "1026MiB"]);
        }
    }
    parted.push(vec!["mkpart", "root", "ext4", root_start, "100%"]);
    Plan {
        parted,
        efi: is_efi.then_some(1),
        boot: encrypt.then_some(2),
        root: if encrypt { 3 } else { 2 },
    }
}

/// The disk as partitioned and formatted.
#[derive(Debug, Clone, Default)]
pub struct Layout {
    /// The EFI system partition, empty on a BIOS machine.
    pub efi_part: String,
    pub boot_part: Option<String>,
    /// The LUKS2 partition, when the root is encrypted.
    pub luks_part: Option<String>,
    /// What the root filesystem is on: the partition, or /dev/mapper/yantrik-root.
    pub root_dev: String,
}

impl Layout {
    pub fn encrypted(&self) -> bool {
        self.luks_part.is_some()
    }
}

/// `/dev/sda` + 2 = `/dev/sda2`; `/dev/nvme0n1` + 2 = `/dev/nvme0n1p2`.
pub fn partition_name(disk: &str, num: u8) -> String {
    if disk.contains("nvme") || disk.contains("mmcblk") {
        format!("{disk}p{num}")
    } else {
        format!("{disk}{num}")
    }
}

/// Partition `disk`, encrypt its root if asked (with `passphrase`), and make the filesystems.
pub fn prepare(
    disk: &str,
    is_efi: bool,
    encrypt: bool,
    passphrase: &str,
    progress: &dyn Fn(i32, &str),
) -> Result<Layout, String> {
    if encrypt && passphrase.is_empty() {
        return Err("encrypting the disk needs a password, and none was given".into());
    }
    // A failed attempt earlier in this session can leave the encrypted root open, and the disk
    // cannot be partitioned again under it.
    let _ = run_cmd("cryptsetup", &["close", CRYPT_NAME]);
    progress(2, "Partitioning disk...");
    let plan = plan(is_efi, encrypt);
    for step in &plan.parted {
        let mut args = vec!["-s", disk];
        args.extend(step.iter().copied());
        run_cmd("parted", &args)?;
    }
    let root_part = partition_name(disk, plan.root);
    progress(8, "Disk partitioned");

    // The kernel re-reads the table and udev makes the device nodes; wait for them.
    let _ = run_cmd("partprobe", &[disk]);
    let _ = run_cmd("udevadm", &["settle", "--timeout=10"]);
    if !std::path::Path::new(&root_part).exists() {
        std::thread::sleep(std::time::Duration::from_secs(2));
    }

    let mut layout = Layout {
        efi_part: plan.efi.map(|n| partition_name(disk, n)).unwrap_or_default(),
        boot_part: plan.boot.map(|n| partition_name(disk, n)),
        luks_part: None,
        root_dev: root_part.clone(),
    };

    if !layout.efi_part.is_empty() {
        progress(10, "Formatting EFI partition (FAT32)...");
        run_cmd("mkfs.fat", &["-F32", &layout.efi_part])?;
    }
    if let Some(boot) = &layout.boot_part {
        run_cmd("mkfs.ext4", &["-q", "-F", "-L", BOOT_LABEL, boot])?;
    }
    if encrypt {
        progress(11, "Encrypting the disk...");
        // Argon2id, LUKS2's default, sized by cryptsetup's own benchmark of this machine.
        with_secret(
            "cryptsetup",
            &["luksFormat", "--type", "luks2", "--batch-mode", "--label", CRYPT_LABEL,
              "--key-file=-", &root_part],
            passphrase,
        )?;
        with_secret("cryptsetup", &["open", "--key-file=-", &root_part, CRYPT_NAME], passphrase)?;
        layout.luks_part = Some(root_part);
        layout.root_dev = format!("/dev/mapper/{CRYPT_NAME}");
    }
    progress(12, "Formatting root partition (ext4)...");
    if let Err(e) = run_cmd("mkfs.ext4", &["-q", "-F", "-L", ROOT_LABEL, &layout.root_dev]) {
        if layout.encrypted() {
            let _ = run_cmd("cryptsetup", &["close", CRYPT_NAME]);
        }
        return Err(e);
    }
    progress(15, "Filesystems formatted");
    Ok(layout)
}

/// Mount the root at `mount_dir`, then /boot and the EFI partition under it.
pub fn mount(layout: &Layout, mount_dir: &str) -> Result<(), String> {
    run_cmd("mkdir", &["-p", mount_dir])?;
    run_cmd("mount", &[&layout.root_dev, mount_dir])?;
    if let Some(boot) = &layout.boot_part {
        let at = format!("{mount_dir}/boot");
        run_cmd("mkdir", &["-p", &at])?;
        run_cmd("mount", &[boot, &at])?;
    }
    if !layout.efi_part.is_empty() {
        let at = format!("{mount_dir}/boot/efi");
        run_cmd("mkdir", &["-p", &at])?;
        run_cmd("mount", &[&layout.efi_part, &at])?;
    }
    Ok(())
}

/// Unmount what `mount` mounted and close the encrypted root. Best effort: it runs after a
/// failure as well as after a success.
pub fn release(layout: &Layout, mount_dir: &str) {
    let _ = run_cmd("umount", &["-l", &format!("{mount_dir}/boot/efi")]);
    let _ = run_cmd("umount", &["-l", &format!("{mount_dir}/boot")]);
    let _ = run_cmd("umount", &["-l", mount_dir]);
    let _ = run_cmd("sync", &[]);
    if layout.encrypted() {
        // A lazy unmount may still hold the device a moment.
        for _ in 0..5 {
            if run_cmd("cryptsetup", &["close", CRYPT_NAME]).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
}

/// A filesystem's UUID, as blkid reads it.
fn uuid_of(device: &str) -> Result<String, String> {
    let uuid = run_cmd("blkid", &["-s", "UUID", "-o", "value", device])?.trim().to_string();
    if uuid.is_empty() {
        return Err(format!("{device} has no UUID"));
    }
    Ok(uuid)
}

/// The installed system's /etc/fstab. The root by its label, the others by UUID: device names
/// move between boots (a second disk, a USB stick), and a UUID does not.
pub fn fstab_text(efi_uuid: Option<&str>, boot_uuid: Option<&str>) -> String {
    let mut fstab = format!("LABEL={ROOT_LABEL}  /           ext4  defaults,noatime  0  1\n");
    if let Some(uuid) = boot_uuid {
        fstab.push_str(&format!("UUID={uuid}  /boot       ext4  defaults,noatime  0  2\n"));
    }
    if let Some(uuid) = efi_uuid {
        fstab.push_str(&format!("UUID={uuid}  /boot/efi   vfat  umask=0077        0  2\n"));
    }
    fstab
}

/// The installed system's /etc/crypttab: the root, opened with a passphrase asked for at boot
/// (`none`), in the initramfs. No `discard`: it would tell the disk, and anyone reading it
/// later, which blocks hold nothing.
pub fn crypttab_text(luks_uuid: &str) -> String {
    format!("{CRYPT_NAME} UUID={luks_uuid} none luks,initramfs\n")
}

/// Write fstab and, for an encrypted root, crypttab and the initramfs settings that make it ask.
pub fn write_system_files(layout: &Layout, mount_dir: &str) -> Result<(), String> {
    let efi_uuid = if layout.efi_part.is_empty() { None } else { Some(uuid_of(&layout.efi_part)?) };
    let boot_uuid = match &layout.boot_part {
        Some(boot) => Some(uuid_of(boot)?),
        None => None,
    };
    sudo_write(
        &format!("{mount_dir}/etc/fstab"),
        &fstab_text(efi_uuid.as_deref(), boot_uuid.as_deref()),
    )?;
    let Some(luks) = &layout.luks_part else {
        return Ok(());
    };
    sudo_write(&format!("{mount_dir}/etc/crypttab"), &crypttab_text(&uuid_of(luks)?))?;
    // Nothing else to write: cryptsetup-initramfs's own conf-hook turns on KEYMAP=y, so the
    // keymap in /etc/default/keyboard goes into the initramfs beside the unlock. Rewriting the
    // package's configuration files would only stop a later upgrade at a conffile prompt.
    // verify_initramfs checks that it happened.
    Ok(())
}

/// Check that each initramfs on the installed /boot opens the root. Without it the machine
/// installs, reboots, and stops at "cannot find root" with nothing asking for a passphrase.
pub fn verify_initramfs(mount_dir: &str) -> Result<(), String> {
    let images: Vec<String> = std::fs::read_dir(format!("{mount_dir}/boot"))
        .map_err(|e| format!("reading the installed /boot: {e}"))?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("initrd.img-"))
        .collect();
    if images.is_empty() {
        return Err("the installed /boot has no initramfs".into());
    }
    for image in images {
        let listing = chroot_cmd(mount_dir, &["lsinitramfs", &format!("/boot/{image}")])?;
        if let Some(missing) = initramfs_missing(&listing) {
            return Err(format!(
                "{image} has no {missing}; the encrypted disk would not open at boot"
            ));
        }
    }
    Ok(())
}

/// What an initramfs listing lacks to open an encrypted root, if anything: the unlock itself,
/// the prompt, and the keymap. Without the keymap a passphrase typed on an AZERTY keyboard is read
/// as QWERTY, and there is no other key to the disk.
pub fn initramfs_missing(listing: &str) -> Option<&'static str> {
    let lines: Vec<&str> = listing.lines().map(str::trim_end).collect();
    let has = |suffix: &str| lines.iter().any(|l| l.ends_with(suffix));
    let has_keymap = lines
        .iter()
        .any(|l| l.contains("etc/console-setup/cached_") && l.ends_with(".kmap"));
    if !has("cryptroot/crypttab") {
        return Some("cryptroot/crypttab");
    }
    if !has("sbin/cryptsetup") {
        return Some("cryptsetup");
    }
    if !has("cryptsetup/askpass") {
        return Some("the passphrase prompt (askpass)");
    }
    if !has_keymap || !has("bin/loadkeys") {
        return Some("the keyboard's keymap");
    }
    None
}

/// Run `cmd` as root with `secret` on its standard input, and nothing of it anywhere else.
fn with_secret(cmd: &str, args: &[&str], secret: &str) -> Result<(), String> {
    use std::io::Write;
    tracing::debug!(cmd, args = ?args, "installer: running command with a secret on stdin");
    let mut child = Command::new("sudo")
        .arg(cmd)
        .args(args)
        .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run sudo {cmd}: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(secret.as_bytes())
            .map_err(|e| format!("{cmd}: could not hand it the passphrase: {e}"))?;
        // Dropped here: the end of input is where the key ends.
    }
    let out = child.wait_with_output().map_err(|e| format!("waiting for {cmd}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("{cmd} {} failed: {}", args.first().unwrap_or(&""), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unencrypted_disk_is_laid_out_as_it_always_was() {
        let efi = plan(true, false);
        assert_eq!((efi.efi, efi.boot, efi.root), (Some(1), None, 2));
        assert_eq!(efi.parted.last().unwrap(), &vec!["mkpart", "root", "ext4", "513MiB", "100%"]);
        let bios = plan(false, false);
        assert_eq!((bios.efi, bios.boot, bios.root), (None, None, 2));
        assert_eq!(bios.parted.last().unwrap(), &vec!["mkpart", "root", "ext4", "2MiB", "100%"]);
    }

    #[test]
    fn an_encrypted_disk_keeps_boot_outside_the_encryption() {
        let efi = plan(true, true);
        assert_eq!((efi.efi, efi.boot, efi.root), (Some(1), Some(2), 3));
        assert!(efi.parted.contains(&vec!["mkpart", "boot", "ext4", "513MiB", "1537MiB"]));
        assert_eq!(efi.parted.last().unwrap(), &vec!["mkpart", "root", "ext4", "1537MiB", "100%"]);
        let bios = plan(false, true);
        assert_eq!((bios.efi, bios.boot, bios.root), (None, Some(2), 3));
        assert!(bios.parted.contains(&vec!["mkpart", "boot", "ext4", "2MiB", "1026MiB"]));
        // The table first, every time.
        assert_eq!(bios.parted[0], vec!["mklabel", "gpt"]);
    }

    #[test]
    fn partitions_are_named_the_way_the_kernel_names_them() {
        assert_eq!(partition_name("/dev/sda", 3), "/dev/sda3");
        assert_eq!(partition_name("/dev/nvme0n1", 3), "/dev/nvme0n1p3");
        assert_eq!(partition_name("/dev/mmcblk0", 1), "/dev/mmcblk0p1");
    }

    #[test]
    fn fstab_finds_everything_but_the_root_by_uuid() {
        let text = fstab_text(Some("AB12-CD34"), Some("1111-boot"));
        assert!(text.starts_with("LABEL=YANTRIK  /  "));
        assert!(text.contains("UUID=1111-boot  /boot  "));
        assert!(text.contains("UUID=AB12-CD34  /boot/efi   vfat  umask=0077"));
        assert!(!text.contains("/dev/"), "no device names: they move between boots");
        assert_eq!(fstab_text(None, None).lines().count(), 1);
    }

    #[test]
    fn crypttab_asks_for_the_passphrase_in_the_initramfs() {
        let line = crypttab_text("0f0e-uuid");
        let fields: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(fields, ["yantrik-root", "UUID=0f0e-uuid", "none", "luks,initramfs"]);
        assert!(!line.contains("discard"));
    }

    /// What lsinitramfs listed for the installed disk on VM 540 (trixie), trimmed.
    const INSTALLED: &str = "cryptroot\ncryptroot/crypttab\netc/console-setup\n\
        etc/console-setup/cached_UTF-8_del.kmap\nscripts/local-top/cryptroot\n\
        usr/bin/loadkeys\nusr/bin/setupcon\nusr/lib/cryptsetup/askpass\nusr/sbin/cryptsetup\n";

    #[test]
    fn an_initramfs_that_cannot_open_the_root_is_caught() {
        assert_eq!(initramfs_missing(INSTALLED), None);
        let without = |gone: &str| {
            INSTALLED.lines().filter(|l| !l.contains(gone)).collect::<Vec<_>>().join("\n")
        };
        assert_eq!(initramfs_missing(&without("crypttab")), Some("cryptroot/crypttab"));
        assert_eq!(initramfs_missing(&without("sbin/cryptsetup")), Some("cryptsetup"));
        assert_eq!(initramfs_missing(&without("askpass")), Some("the passphrase prompt (askpass)"));
        assert_eq!(initramfs_missing(&without(".kmap")), Some("the keyboard's keymap"));
        assert_eq!(initramfs_missing(&without("loadkeys")), Some("the keyboard's keymap"));
    }
}
