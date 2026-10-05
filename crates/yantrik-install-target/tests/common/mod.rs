//! Fixture tables, as parsed and as `read::read_table` would leave them.
#![allow(dead_code)]

use yantrik_install_target::classify::{guid, is_placeholder_label};
use yantrik_install_target::{Checked, DiskTable, Probed};

/// `<name>.parted.json` with `<lsblk>.lsblk.json`, parsed: nothing on it was looked at.
pub fn parsed(name: &str, lsblk: &str) -> DiskTable {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/");
    let parted = std::fs::read_to_string(format!("{dir}{name}.parted.json")).unwrap();
    let lsblk = std::fs::read_to_string(format!("{dir}{lsblk}.lsblk.json")).unwrap();
    DiskTable::parse(&parted, &lsblk).unwrap()
}

/// What reading the disk itself finds when it is internal, has a protective MBR and room on its
/// EFI partition, and every YANTRIK placeholder and blank partition on it is empty.
pub fn looked(mut t: DiskTable) -> DiskTable {
    t.probed = Some(Probed { external: false, esp_free: Some(150_000_000), mbr_problem: None });
    for p in &mut t.parts {
        let placeholder = (p.fstype == "vfat" || p.fstype == "exfat") && is_placeholder_label(&p.label);
        let blank = p.fstype.is_empty() && (p.type_uuid == guid::MS_BASIC_DATA || p.type_uuid == guid::LINUX_FS);
        if placeholder || blank {
            p.checked = Checked::Empty;
        }
    }
    t
}

/// A fixture as `read_table` would leave it.
pub fn table(name: &str, lsblk: &str) -> DiskTable {
    looked(parsed(name, lsblk))
}

/// The same disk, on USB.
pub fn on_usb(mut t: DiskTable) -> DiskTable {
    if let Some(p) = &mut t.probed {
        p.external = true;
    }
    t
}
