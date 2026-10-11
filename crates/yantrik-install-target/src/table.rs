//! A disk's partition table, read from `parted -j -s <disk> unit s print free` (the geometry,
//! the disk's GUID, the GPT type, GUID, name and flags of each partition, and the runs of free
//! space between them) and `lsblk -J` (the filesystem, label and mount point of each partition).
//!
//! What only reading the disk itself can tell — whether a partition is truly empty, what a
//! placeholder holds, the MBR in sector 0, the room left on the EFI partition — is filled in by
//! `read::read_table`. A table parsed from text alone has looked at none of it, and nothing on
//! it can be chosen.

use serde_json::Value;

/// A run of sectors, both ends included, as parted prints them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    pub start: u64,
    pub end: u64,
}

impl Run {
    pub fn sectors(&self) -> u64 {
        self.end.saturating_sub(self.start) + 1
    }
    pub fn contains(&self, other: &Run) -> bool {
        other.start >= self.start && other.end <= self.end
    }
    pub fn overlaps(&self, other: &Run) -> bool {
        self.start <= other.end && other.start <= self.end
    }
}

/// One partition.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub number: u32,
    pub run: Run,
    /// The GPT partition type, lowercase; empty when parted did not say.
    pub type_uuid: String,
    /// The partition's own GPT GUID, lowercase; empty when parted did not say.
    pub uuid: String,
    /// The GPT partition name ("EFI System Partition", "Container").
    pub name: String,
    pub flags: Vec<String>,
    /// The kernel's name for it: `/dev/sda3`, `/dev/nvme0n1p3`.
    pub path: String,
    /// What blkid says is on it ("vfat", "apfs", "ext4"); empty when nothing was found.
    pub fstype: String,
    pub label: String,
    /// Where it is mounted now, if anywhere.
    pub mountpoint: String,
    /// What looking inside it found. Only an empty partition or an empty placeholder can be
    /// installed into, and only `read::read_table` looks.
    pub checked: Checked,
}

/// What reading a partition's bytes found, beyond the name of its filesystem.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Checked {
    /// Nobody looked: a table parsed from text, or a partition of a kind that is kept anyway.
    #[default]
    NotLooked,
    /// Nothing on it at all (blkid found nothing, wipefs no signature, the first and last MiB
    /// are zeros), or, for a YANTRIK placeholder, nothing on it but macOS's own housekeeping.
    Empty,
    /// It could not be shown to be empty; why, in words.
    Unreadable(String),
    /// A YANTRIK placeholder with files a person may want: their paths, as found.
    Holds(Vec<String>),
}

/// What only the disk itself can say about the table as a whole, read by `read::read_table`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probed {
    /// On USB or marked removable, or it could not be told: never where a target is chosen for
    /// the person.
    pub external: bool,
    /// Bytes free on the EFI system partition; `None` when it could not be looked at.
    pub esp_free: Option<u64>,
    /// Why sector 0 is not a plain protective MBR (a hybrid MBR, as Boot Camp makes), or `None`.
    pub mbr_problem: Option<String>,
}

/// A disk and its partitions, in the order they sit on it.
#[derive(Debug, Clone, PartialEq)]
pub struct DiskTable {
    /// `/dev/sda`.
    pub path: String,
    pub model: String,
    /// The partition table: "gpt", "msdos", "unknown".
    pub label: String,
    /// The disk's GPT GUID, lowercase: a rewritten table gets a new one. Empty when parted did
    /// not say, and then nothing on the disk can be chosen.
    pub guid: String,
    /// The disk's size in logical sectors.
    pub sectors: u64,
    pub sector_size: u64,
    pub parts: Vec<Part>,
    pub free: Vec<Run>,
    /// `None` until the disk itself was read (`read::read_table`).
    pub probed: Option<Probed>,
}

/// The kernel's name for partition `num` of `disk`: a disk whose name ends in a digit gets a
/// `p` before the number (`nvme0n1p3`, `mmcblk0p1`, `loop0p2`), any other does not (`sda3`).
pub fn partition_path(disk: &str, num: u32) -> String {
    if disk.ends_with(|c: char| c.is_ascii_digit()) {
        format!("{disk}p{num}")
    } else {
        format!("{disk}{num}")
    }
}

/// `/dev/sda` → `sda`.
pub fn disk_name(path: &str) -> &str {
    path.trim_start_matches("/dev/")
}

fn sectors_of(v: &Value) -> Option<u64> {
    v.as_str()?.trim().strip_suffix('s')?.parse().ok()
}

impl DiskTable {
    /// The table from parted's JSON (in sectors) and lsblk's (`-J -o PATH,FSTYPE,LABEL,MOUNTPOINT`
    /// or more). lsblk may be `""` or carry nothing for a partition: its fields stay empty.
    pub fn parse(parted_json: &str, lsblk_json: &str) -> Result<DiskTable, String> {
        let parted: Value =
            serde_json::from_str(parted_json).map_err(|e| format!("parted's answer is not JSON: {e}"))?;
        let disk = &parted["disk"];
        let path = disk["path"].as_str().ok_or("parted named no disk")?.to_string();
        let sectors = sectors_of(&disk["size"]).ok_or("parted gave no size in sectors")?;
        let sector_size = disk["logical-sector-size"].as_u64().unwrap_or(512);
        let label = disk["label"].as_str().unwrap_or("unknown").to_string();
        let guid = disk["uuid"].as_str().unwrap_or("").to_lowercase();
        let model = disk["model"].as_str().unwrap_or("").trim().to_string();
        let probes = probes(lsblk_json);

        let mut parts = Vec::new();
        let mut free = Vec::new();
        for p in disk["partitions"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            let (Some(start), Some(end)) = (sectors_of(&p["start"]), sectors_of(&p["end"])) else {
                return Err(format!("parted gave a partition without sectors: {p}"));
            };
            let run = Run { start, end };
            if p["type"].as_str() == Some("free") {
                free.push(run);
                continue;
            }
            let number = p["number"].as_u64().ok_or("parted gave a partition without a number")? as u32;
            let part_path = partition_path(&path, number);
            let probe = probes.iter().find(|(p, _)| *p == part_path).map(|(_, v)| v.clone());
            let probe = probe.unwrap_or_default();
            parts.push(Part {
                number,
                run,
                type_uuid: p["type-uuid"].as_str().unwrap_or("").to_lowercase(),
                uuid: p["uuid"].as_str().unwrap_or("").to_lowercase(),
                name: p["name"].as_str().unwrap_or("").to_string(),
                flags: p["flags"]
                    .as_array()
                    .map(|f| f.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                    .unwrap_or_default(),
                path: part_path,
                fstype: probe.0,
                label: probe.1,
                mountpoint: probe.2,
                checked: Checked::NotLooked,
            });
        }
        parts.sort_by_key(|p| p.run.start);
        Ok(DiskTable { path, model, label, guid, sectors, sector_size, parts, free, probed: None })
    }

    pub fn name(&self) -> &str {
        disk_name(&self.path)
    }

    pub fn bytes(&self, sectors: u64) -> u64 {
        sectors.saturating_mul(self.sector_size)
    }

    /// The last sector a partition may use: GPT keeps its backup header and entries (33
    /// sectors at 512 bytes, 5 at 4096) at the end of the disk.
    pub fn last_usable(&self) -> u64 {
        let backup = 1 + (128 * 128 + self.sector_size - 1) / self.sector_size;
        self.sectors.saturating_sub(backup + 1)
    }

    /// The first sector a partition may use: the protective MBR, then GPT's header and entries
    /// (34 sectors in all at 512 bytes, 6 at 4096).
    pub fn first_usable(&self) -> u64 {
        2 + (128 * 128 + self.sector_size - 1) / self.sector_size
    }

    /// One mebibyte in sectors: where new partitions start and end.
    pub fn align(&self) -> u64 {
        (1024 * 1024 / self.sector_size).max(1)
    }

    pub fn part(&self, number: u32) -> Option<&Part> {
        self.parts.iter().find(|p| p.number == number)
    }

    /// The EFI system partition the disk boots from, if it has one.
    pub fn esp(&self) -> Option<&Part> {
        self.parts.iter().find(|p| crate::classify::is_esp(p))
    }

    /// What the person was shown, in sixteen hex digits: the table (its kind, its GUID, the
    /// disk's size, every partition's number, extent, type, GUID, name and flags) and what is on
    /// each partition. Two reads of an unchanged disk agree; a table rewritten, or a partition
    /// added, moved, removed, retyped, renamed, reflagged or reformatted between them, does not.
    pub fn fingerprint(&self) -> String {
        let mut text = format!("{}|{}|{}|{}", self.label, self.guid, self.sectors, self.sector_size);
        for p in &self.parts {
            text.push_str(&format!(
                "|{}:{}-{}:{}:{}:{:?}:{:?}:{}:{}",
                p.number, p.run.start, p.run.end, p.type_uuid, p.uuid, p.name, p.flags, p.fstype, p.label
            ));
        }
        format!("{:016x}", fnv1a64(text.as_bytes()))
    }
}

/// FNV-1a, 64 bits: a short, dependency-free digest. It tells two tables apart; it is not a
/// security boundary (the person's own disk is not an adversary here).
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// (path, (fstype, label, mountpoint)) for every device in lsblk's JSON, children included.
fn probes(lsblk_json: &str) -> Vec<(String, (String, String, String))> {
    fn walk(v: &Value, out: &mut Vec<(String, (String, String, String))>) {
        for dev in v.as_array().map(Vec::as_slice).unwrap_or(&[]) {
            let s = |k: &str| dev[k].as_str().unwrap_or("").to_string();
            let path = if dev["path"].is_string() { s("path") } else { format!("/dev/{}", s("name")) };
            // util-linux 2.37 on prints `mountpoints`, a list; older ones `mountpoint`.
            let mount = dev["mountpoints"]
                .as_array()
                .and_then(|m| m.iter().find_map(|x| x.as_str().map(String::from)))
                .unwrap_or_else(|| s("mountpoint"));
            out.push((path, (s("fstype"), s("label"), mount)));
            walk(&dev["children"], out);
        }
    }
    let mut out = Vec::new();
    if let Ok(v) = serde_json::from_str::<Value>(lsblk_json) {
        walk(&v["blockdevices"], &mut out);
    }
    out
}

/// A size as a person reads it in macOS's Disk Utility: decimal units, one decimal, none when
/// it is `.0` (209.7 MB, 200 GB, 1 TB).
pub fn human(bytes: u64) -> String {
    let units = [(1e12, "TB"), (1e9, "GB"), (1e6, "MB"), (1e3, "kB")];
    let b = bytes as f64;
    for (scale, unit) in units {
        if b >= scale {
            let text = format!("{:.1}", b / scale);
            let text = text.strip_suffix(".0").map(String::from).unwrap_or(text);
            return format!("{text} {unit}");
        }
    }
    format!("{bytes} B")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partitions_are_named_the_way_the_kernel_names_them() {
        assert_eq!(partition_path("/dev/sda", 3), "/dev/sda3");
        assert_eq!(partition_path("/dev/nvme0n1", 3), "/dev/nvme0n1p3");
        assert_eq!(partition_path("/dev/mmcblk0", 1), "/dev/mmcblk0p1");
        assert_eq!(partition_path("/dev/loop7", 2), "/dev/loop7p2");
    }

    #[test]
    fn sizes_read_the_way_macos_prints_them() {
        assert_eq!(human(209_715_200), "209.7 MB");
        assert_eq!(human(200_000_000_000), "200 GB");
        assert_eq!(human(1_000_204_886_016), "1 TB");
        assert_eq!(human(8_000_000_000), "8 GB");
        assert_eq!(human(21_474_836_480), "21.5 GB");
    }

    #[test]
    fn the_ends_of_a_gpt_disk_are_kept_for_its_headers() {
        let t = DiskTable {
            path: "/dev/sda".into(),
            model: String::new(),
            label: "gpt".into(),
            guid: String::new(),
            sectors: 4_194_304,
            sector_size: 512,
            parts: vec![],
            free: vec![],
            probed: None,
        };
        // 4194304 sectors: the backup header is the last, its 32 sectors of entries before it.
        assert_eq!(t.last_usable(), 4_194_304 - 34);
        // The protective MBR, the header, then 32 sectors of entries: 34 is the first free one.
        assert_eq!(t.first_usable(), 34);
        let t4k = DiskTable { sector_size: 4096, sectors: 1_000_000, ..t };
        assert_eq!(t4k.last_usable(), 1_000_000 - 6);
        assert_eq!(t4k.first_usable(), 6);
        assert_eq!(t4k.align(), 256);
    }
}
