//! Which firmware entry starts the Yantrik that was found on this computer's disk (#1165).
//!
//! A label alone is not enough: an earlier install, a firmware leftover on another disk, or the
//! live disc's own `yantrik-live` entry can carry a Yantrik-looking name. The entry chosen here is
//! the one whose device path (`HD(n,GPT,<partition guid>,...)` in `efibootmgr -v`) is the EFI
//! system partition of the very disk the install was detected on, under one of the two labels the
//! installer gives its entry. Anything short of one clear answer is no answer: the screen then
//! sends the person to the firmware's boot menu instead of guessing.

/// The GPT partition type of an EFI system partition.
const ESP_TYPE: &str = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";

/// Firmware entry labels the installer's two paths give: efibootmgr's, and grub-install's id.
/// Matched whole, so `yantrik-live` and `Yantrik OS (old)` never qualify.
const ENTRY_LABELS: [&str; 2] = ["Yantrik OS", "yantrik"];

/// One row of `lsblk -rno NAME,FSTYPE,LABEL,PKNAME,PARTUUID,PARTTYPE,RM`. Empty columns stay
/// empty: lsblk -r separates with single spaces and escapes the spaces inside values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    pub fstype: String,
    pub label: String,
    /// The disk this partition is on.
    pub parent: String,
    pub partuuid: String,
    pub parttype: String,
    pub removable: bool,
}

pub const LSBLK_COLUMNS: &str = "NAME,FSTYPE,LABEL,PKNAME,PARTUUID,PARTTYPE,RM";

pub fn rows(lsblk: &str) -> Vec<Row> {
    lsblk
        .lines()
        .filter_map(|line| {
            let mut f = line.split(' ');
            let mut next = || f.next().map(str::to_string);
            Some(Row {
                name: next().filter(|n| !n.is_empty())?,
                fstype: next()?,
                label: next()?,
                parent: next().unwrap_or_default(),
                partuuid: next().unwrap_or_default().to_ascii_lowercase(),
                parttype: next().unwrap_or_default().to_ascii_lowercase(),
                removable: next().as_deref() == Some("1"),
            })
        })
        .collect()
}

/// The partition guid of the one EFI system partition on `disk`; `None` when it has none or
/// several (then no entry can be said to be its own).
pub fn esp_guid(rows: &[Row], disk: &str) -> Option<String> {
    let mut esps = rows.iter().filter(|r| r.parent == disk && r.parttype == ESP_TYPE && !r.partuuid.is_empty());
    let first = esps.next()?;
    esps.next().is_none().then(|| first.partuuid.clone())
}

/// The entry `efibootmgr -v` lists under an installer label whose device path is the partition
/// `esp` guid, as `BootNNNN`'s four digits. Entries that agree are one answer (the same ESP boots
/// the same loader); a label match that points elsewhere, or nowhere readable, is skipped.
pub fn entry_for(listing: &str, esp: &str) -> Option<String> {
    let esp = esp.to_ascii_lowercase();
    ENTRY_LABELS.iter().find_map(|wanted| {
        listing.lines().find_map(|line| {
            let rest = line.strip_prefix("Boot")?;
            let end = rest.find(|c: char| !c.is_ascii_hexdigit())?;
            let (num, tail) = rest.split_at(end);
            if num.len() != 4 {
                return None;
            }
            let body = tail.strip_prefix('*').unwrap_or(tail).trim_start();
            let (label, path) = body.split_once('\t').unwrap_or_else(|| (body.split("HD(").next().unwrap_or(body), body));
            let guid = path.split("HD(").nth(1)?.split(',').nth(2)?.to_ascii_lowercase();
            (label.trim() == *wanted && guid == esp).then(|| num.to_string())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ESP: &str = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";
    const G1: &str = "5f1b6e0a-2d4c-4a0b-9b53-1c2e8a7d9f10";
    const G2: &str = "9a77c3d2-0e11-4c55-8f0a-77d1b2c3e4f5";

    fn lsblk() -> String {
        format!(
            "sr0 iso9660 YANTRIK    0\nloop0 squashfs     0\nsda      0\nsda1 vfat  sda {G1} {ESP} 0\nsda2 ext4 YANTRIK sda 11111111-2222 0fc63daf-8483-4772-8e79-3d69d8477de4 0\n"
        )
    }

    /// Proxmox OVMF with an efidisk: the CD, the UiApp, and the installed entry.
    fn proxmox() -> String {
        format!("BootCurrent: 0001\nTimeout: 0 seconds\nBootOrder: 0001,0000,0002\nBoot0000* UiApp\tFvVol(7cb8bdc9-f8eb-4f34-aaea-3ee4af6516a1)/FvFile(462caa21-7614-4503-836e-8ab6f4662331)\nBoot0001* UEFI QEMU DVD-ROM QM00003 \tPciRoot(0x0)/Pci(0x1f,0x2)/Sata(0,65535,0)/CDROM(1,0x1c8,0x2a0)N.....YM....R,Y.\nBoot0002* Yantrik OS\tHD(1,GPT,{G1},0x800,0x100000)/File(\\EFI\\yantrik\\grubx64.efi)\n")
    }

    #[test]
    fn the_rows_keep_their_empty_columns() {
        let r = rows(&lsblk());
        assert_eq!(r.len(), 5);
        assert_eq!(r[0].fstype, "iso9660");
        assert_eq!(r[3].parent, "sda");
        assert_eq!(r[3].partuuid, G1);
        assert!(!r[3].removable);
        assert!(rows("sdb1 ext4 YANTRIK sdb x y 1\n")[0].removable);
    }

    /// `lsblk -r` writes a space inside a value as `\x20`, so the columns stay in place.
    #[test]
    fn a_label_with_a_space_keeps_the_columns_in_place() {
        let r = rows(&format!("sda1 vfat EFI\\x20System sda {G1} {ESP} 0\n"));
        assert_eq!((r[0].label.as_str(), r[0].parent.as_str(), r[0].parttype.as_str()), ("EFI\\x20System", "sda", ESP));
        assert_eq!(esp_guid(&r, "sda").as_deref(), Some(G1));
    }

    #[test]
    fn the_esp_of_the_disk_is_its_one_efi_partition() {
        assert_eq!(esp_guid(&rows(&lsblk()), "sda").as_deref(), Some(G1));
        assert_eq!(esp_guid(&rows(&lsblk()), "sdb"), None, "another disk has none");
        let two = format!("{}sda3 vfat  sda {G2} {ESP} 0\n", lsblk());
        assert_eq!(esp_guid(&rows(&two), "sda"), None, "two ESPs on one disk is not a clear answer");
    }

    #[test]
    fn proxmox_ovmf_gives_the_installed_entry() {
        assert_eq!(entry_for(&proxmox(), G1).as_deref(), Some("0002"));
        assert_eq!(entry_for(&proxmox(), &G1.to_uppercase()).as_deref(), Some("0002"), "case does not matter");
    }

    #[test]
    fn a_real_laptop_with_windows_and_grub_ids_is_matched_by_its_disk() {
        let laptop = format!(
            "BootCurrent: 0003\nBootOrder: 0003,0000,0001\nBoot0000* Windows Boot Manager\tHD(1,GPT,0cb8fa1e-1111-2222-3333-444455556666,0x800,0x82000)/File(\\EFI\\Microsoft\\Boot\\bootmgfw.efi)WINDOWS.........x...B.C.D.O.B.J.E.C.T.=.{{9dea862c-5cdd-4e70-acc1-f32b344d4795}}...\nBoot0001* UEFI: SanDisk\tPciRoot(0x0)/Pci(0x14,0x0)/USB(3,0)/HD(1,MBR,0x1234,0x800,0x3b)\nBoot0003* yantrik\tHD(2,GPT,{G1},0x96800,0x100000)/File(\\EFI\\yantrik\\shimx64.efi)\n"
        );
        assert_eq!(entry_for(&laptop, G1).as_deref(), Some("0003"));
        assert_eq!(entry_for(&laptop, G2), None, "another disk's ESP finds nothing");
    }

    #[test]
    fn of_two_yantrik_entries_the_one_on_this_disk_wins() {
        let two = format!(
            "BootOrder: 0005,0004\nBoot0004* Yantrik OS\tHD(1,GPT,{G2},0x800,0x100000)/File(\\EFI\\yantrik\\grubx64.efi)\nBoot0005* Yantrik OS\tHD(1,GPT,{G1},0x800,0x100000)/File(\\EFI\\yantrik\\grubx64.efi)\n"
        );
        assert_eq!(entry_for(&two, G1).as_deref(), Some("0005"));
        assert_eq!(entry_for(&two, G2).as_deref(), Some("0004"));
    }

    #[test]
    fn the_live_entry_and_lookalike_labels_never_match() {
        let live = format!(
            "Boot0001* yantrik-live\tHD(1,GPT,{G1},0x800,0x100000)/File(\\EFI\\boot\\bootx64.efi)\nBoot0002* Yantrik OS (old)\tHD(1,GPT,{G1},0x800,0x100000)\nBoot0003* My yantrik\tHD(1,GPT,{G1},0x800,0x100000)\n"
        );
        assert_eq!(entry_for(&live, G1), None);
    }

    #[test]
    fn an_entry_with_no_readable_disk_path_is_no_match() {
        assert_eq!(entry_for("Boot0002* Yantrik OS\tFvVol(1)/FvFile(2)\nBoot0003* Yantrik OS\n", G1), None);
        assert_eq!(entry_for("", G1), None);
    }
}
