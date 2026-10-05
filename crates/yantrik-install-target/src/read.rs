//! Reading a real disk, read-only: the table from parted, and what is inside each partition
//! from blkid, wipefs and the bytes themselves.
//!
//! Every command goes through a runner the caller supplies, which says how the command exited
//! and what it printed: the desktop installer runs them through sudo, the command-line tool as
//! itself. How it exited matters: `blkid -p` exits 2 for "nothing found", and also for a device
//! that is not there, so nothing is called empty on its exit status alone.
//!
//! Every answer here fails closed. A partition is empty only when blkid found nothing, wipefs
//! found no signature and its first and last MiB are zeros; a placeholder is empty only when it
//! was mounted read-only and held nothing but macOS's own housekeeping. Anything that could not
//! be read is `Checked::Unreadable`, and can never be chosen.

use crate::classify;
use crate::table::{disk_name, Checked, DiskTable, Part, Probed};

/// What a command did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ran {
    /// Its exit status; `None` when it could not be started or a signal ended it.
    pub code: Option<i32>,
    /// Standard output as text. Bytes that are not UTF-8 become U+FFFD; a NUL stays one NUL.
    pub stdout: String,
    pub stderr: String,
}

impl Ran {
    /// A command that exited `code` having printed `stdout`.
    pub fn exited(code: i32, stdout: &str) -> Ran {
        Ran { code: Some(code), stdout: stdout.into(), stderr: String::new() }
    }

    /// Its output when it exited 0, or why it did not.
    pub fn ok(self, cmd: &str) -> Result<String, String> {
        match self.code {
            Some(0) => Ok(self.stdout),
            Some(c) => Err(format!("{cmd} exited {c}: {}", self.stderr.trim())),
            None => Err(format!("{cmd} did not finish: {}", self.stderr.trim())),
        }
    }
}

/// Runs a command and says how it went.
pub type Runner<'a> = &'a dyn Fn(&str, &[&str]) -> Ran;

/// Run a command and keep its output only if it succeeded.
pub fn ok(run: Runner, cmd: &str, args: &[&str]) -> Result<String, String> {
    run(cmd, args).ok(cmd)
}

const MIB: u64 = 1024 * 1024;
/// Where a partition is mounted, read-only, to look inside it.
const LOOK_DIR: &str = "/run/yantrik-install-target";

/// The table of `disk` as it is now: parted for the geometry, lsblk for mount points, `blkid -p`
/// for what is on every partition (lsblk reads udev's records, which may be stale or missing),
/// and for the partitions that might be chosen, a look at what is inside them.
pub fn read_table(disk: &str, run: Runner) -> Result<DiskTable, String> {
    let parted = ok(run, "parted", &["-j", "-s", disk, "unit", "s", "print", "free"])
        .map_err(|e| format!("reading the partition table of {disk}: {e}"))?;
    let lsblk = ok(run, "lsblk", &["-J", "-o", "PATH,NAME,FSTYPE,LABEL,MOUNTPOINT", disk]).unwrap_or_default();
    let mut t = DiskTable::parse(&parted, &lsblk)?;
    let mounts = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    let swaps = std::fs::read_to_string("/proc/swaps").unwrap_or_default();
    for p in &mut t.parts {
        if p.mountpoint.is_empty() {
            p.mountpoint = mount_of(&mounts, &swaps, &p.path).unwrap_or_default();
        }
    }
    let sector_size = t.sector_size;
    for p in &mut t.parts {
        probe_filesystem(p, sector_size, run);
        if (p.fstype == "vfat" || p.fstype == "exfat")
            && classify::is_placeholder_label(&p.label)
            && p.checked == Checked::NotLooked
        {
            p.checked = look_inside(p, run);
        }
    }
    t.probed = Some(probe_disk(&t, run));
    Ok(t)
}

/// What `blkid -p` says is on `p`, which overrides lsblk's word; and, when it finds nothing, the
/// other two checks that make a partition empty.
fn probe_filesystem(p: &mut Part, sector_size: u64, run: Runner) {
    // -D: not the partition table's entry for it, which every GPT partition has, so that
    // "nothing found" means nothing on the partition itself.
    let r = run("blkid", &["-p", "-D", "-o", "export", &p.path]);
    match r.code {
        Some(0) => {
            let field = |k: &str| r.stdout.lines().find_map(|l| l.strip_prefix(k)).map(str::to_string);
            match field("TYPE=") {
                Some(ty) => {
                    p.fstype = ty;
                    p.label = field("LABEL=").unwrap_or_default();
                }
                None => {
                    p.checked = Checked::Unreadable("blkid found something on it that is not a filesystem".into())
                }
            }
        }
        Some(2) if !p.fstype.is_empty() => {
            p.checked = Checked::Unreadable(format!("lsblk says {} but blkid -p finds nothing", p.fstype));
        }
        Some(2) => p.checked = empty_check(&p.path, p.run.sectors().saturating_mul(sector_size), run),
        Some(code) => {
            p.checked = Checked::Unreadable(format!("blkid -p could not say what is on it (exit {code}: {})", r.stderr.trim()))
        }
        None => p.checked = Checked::Unreadable(format!("blkid -p did not finish: {}", r.stderr.trim())),
    }
}

/// Whether `text`, read as `len` bytes, is all zeros.
pub fn all_zero(text: &str, len: u64) -> bool {
    text.len() as u64 == len && text.bytes().all(|b| b == 0)
}

/// After blkid found nothing: no signature for wipefs, and the first and last MiB blank.
pub fn empty_check(path: &str, bytes: u64, run: Runner) -> Checked {
    let sigs = run("wipefs", &["-n", "--noheadings", "-O", "TYPE", path]);
    match sigs.code {
        Some(0) if sigs.stdout.trim().is_empty() => {}
        Some(0) => {
            let found: Vec<&str> = sigs.stdout.split_whitespace().collect();
            return Checked::Unreadable(format!("wipefs finds {}", found.join(", ")));
        }
        _ => return Checked::Unreadable(format!("wipefs -n could not read it: {}", sigs.stderr.trim())),
    }
    if bytes == 0 {
        return Checked::Unreadable("it has no size".into());
    }
    let span = bytes.min(MIB);
    for (which, offset) in [("first", 0), ("last", bytes - span)] {
        let r = run(
            "dd",
            &[
                &format!("if={path}"),
                &format!("bs={span}"),
                "count=1",
                &format!("skip={offset}"),
                "iflag=skip_bytes,fullblock",
                "status=none",
            ],
        );
        if r.code != Some(0) {
            return Checked::Unreadable(format!("its {which} MiB could not be read: {}", r.stderr.trim()));
        }
        if !all_zero(&r.stdout, span) {
            return Checked::Unreadable(format!("its {which} MiB is not blank"));
        }
    }
    Checked::Empty
}

/// What on a placeholder is not macOS's own housekeeping, from `find -printf '%y %P\0'`: the
/// Spotlight index, the FSEvents log, `.DS_Store`, AppleDouble `._` files, and `.Trashes` with
/// nothing in it but folders. A file in `.Trashes` was someone's, and counts.
pub fn foreign_files(listing: &str) -> Vec<String> {
    listing
        .split('\0')
        .filter(|e| !e.is_empty())
        .filter_map(|entry| {
            let (kind, path) = entry.split_once(' ').unwrap_or(("?", entry));
            let top = path.split('/').next().unwrap_or(path);
            let base = path.rsplit('/').next().unwrap_or(path);
            let housekeeping = matches!(top, ".fseventsd" | ".Spotlight-V100")
                || base == ".DS_Store"
                || base.starts_with("._")
                || (top == ".Trashes" && kind == "d");
            (!housekeeping).then(|| path.to_string())
        })
        .collect()
}

/// Mount a YANTRIK placeholder read-only (or read it where it is already mounted) and say
/// whether it holds anything a person put there.
pub fn look_inside(p: &Part, run: Runner) -> Checked {
    if p.mountpoint == "[SWAP]" {
        return Checked::Unreadable("it is in use as swap".into());
    }
    let here = format!("{LOOK_DIR}/{}", disk_name(&p.path));
    let (dir, mounted_here) = if p.mountpoint.is_empty() { (here, true) } else { (p.mountpoint.clone(), false) };
    if mounted_here {
        let mounted = ok(run, "mkdir", &["-p", &dir])
            .and_then(|_| ok(run, "mount", &["-o", "ro,nosuid,nodev,noexec", "-t", &p.fstype, &p.path, &dir]));
        if let Err(e) = mounted {
            let _ = run("rmdir", &[&dir]);
            return Checked::Unreadable(format!("it could not be mounted read-only to look inside: {e}"));
        }
    }
    let listing = ok(run, "find", &[&dir, "-xdev", "-mindepth", "1", "-printf", "%y %P\\0"]);
    if mounted_here {
        if let Err(e) = ok(run, "umount", &[&dir]) {
            return Checked::Unreadable(format!("it was left mounted at {dir} after looking inside: {e}"));
        }
        let _ = run("rmdir", &[&dir]);
    }
    match listing {
        Ok(listing) => {
            let files = foreign_files(&listing);
            if files.is_empty() { Checked::Empty } else { Checked::Holds(files) }
        }
        Err(e) => Checked::Unreadable(format!("what is on it could not be listed: {e}")),
    }
}

/// Why sector 0 is not a plain protective MBR, from `od -An -tx1 -v -j446 -N66` of the disk: its
/// four partition entries and the 55 AA signature. A protective MBR has one entry of type EE and
/// nothing else; a hybrid MBR (Boot Camp's, or gdisk's) lists real partitions beside it, and a
/// partition made in the GPT alone would leave the two disagreeing.
pub fn mbr_problem(disk: &str, od_hex: &str) -> Option<String> {
    let bytes: Vec<u8> = od_hex.split_whitespace().filter_map(|b| u8::from_str_radix(b, 16).ok()).collect();
    if bytes.len() != 66 || bytes[64] != 0x55 || bytes[65] != 0xaa {
        return Some(format!("sector 0 of {disk} holds no readable MBR beside its GPT; it is left as it is"));
    }
    let types: Vec<u8> = (0..4).map(|i| bytes[i * 16 + 4]).collect();
    let others: Vec<String> = types.iter().filter(|t| **t != 0 && **t != 0xee).map(|t| format!("{t:02X}")).collect();
    if !others.is_empty() {
        return Some(format!(
            "{disk} has a hybrid MBR (entries of type {} beside its GPT, as Boot Camp makes); a partition added to the GPT alone would leave the two disagreeing, so nothing is installed beside it",
            others.join(", ")
        ));
    }
    if !types.contains(&0xee) {
        return Some(format!("sector 0 of {disk} has no protective MBR entry for its GPT; it is left as it is"));
    }
    None
}

/// What only the disk as a whole can say: whether it is on USB or removable, the room left on
/// its EFI partition, and its MBR.
fn probe_disk(t: &DiskTable, run: Runner) -> Probed {
    let removable = std::fs::read_to_string(format!("/sys/block/{}/removable", t.name()));
    let tran = ok(run, "lsblk", &["-dno", "TRAN", &t.path]);
    let external = match (removable, tran) {
        (Ok(rm), Ok(tran)) => rm.trim() != "0" || tran.trim() == "usb",
        _ => true,
    };
    let mbr_problem = match ok(run, "od", &["-An", "-tx1", "-v", "-j446", "-N66", &t.path]) {
        Ok(hex) => mbr_problem(&t.path, &hex),
        Err(e) => Some(format!("sector 0 of {} could not be read: {e}", t.path)),
    };
    Probed { external, esp_free: t.esp().and_then(|esp| esp_free(esp, run)), mbr_problem }
}

/// The bytes free on the EFI partition, mounted read-only to look (or where it already is).
fn esp_free(esp: &Part, run: Runner) -> Option<u64> {
    if esp.fstype != "vfat" || esp.mountpoint == "[SWAP]" {
        return None;
    }
    let here = format!("{LOOK_DIR}/{}", disk_name(&esp.path));
    let (dir, mounted_here) = if esp.mountpoint.is_empty() { (here, true) } else { (esp.mountpoint.clone(), false) };
    if mounted_here {
        ok(run, "mkdir", &["-p", &dir]).ok()?;
        if ok(run, "mount", &["-o", "ro,nosuid,nodev,noexec", "-t", "vfat", &esp.path, &dir]).is_err() {
            let _ = run("rmdir", &[&dir]);
            return None;
        }
    }
    let df = ok(run, "df", &["-B1", "--output=avail", &dir]);
    if mounted_here {
        // Left mounted, it reads as in use, and every later look refuses the disk.
        ok(run, "umount", &[&dir]).ok()?;
        let _ = run("rmdir", &[&dir]);
    }
    df.ok()?.lines().nth(1)?.trim().parse().ok()
}

/// Where `device` is mounted, from /proc/self/mounts (or "[SWAP]" from /proc/swaps). A mount
/// recorded by a link (/dev/disk/by-uuid/...) counts for the device the link points to.
pub fn mount_of(mounts: &str, swaps: &str, device: &str) -> Option<String> {
    let same = |dev: &str| {
        dev == device
            || (dev.starts_with("/dev/")
                && std::fs::canonicalize(dev).map(|c| c.to_string_lossy() == device).unwrap_or(false))
    };
    for line in mounts.lines() {
        let mut f = line.split_whitespace();
        if let (Some(dev), Some(at)) = (f.next(), f.next()) {
            if same(dev) {
                return Some(at.replace("\\040", " "));
            }
        }
    }
    swaps.lines().skip(1).any(|l| l.split_whitespace().next().is_some_and(same)).then(|| "[SWAP]".into())
}

#[cfg(test)]
mod tests;
