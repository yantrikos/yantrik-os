//! Reading a real disk and carrying out a plan on it.
//!
//! Every command goes through a runner the caller supplies, which returns the command's
//! standard output or why it failed: the desktop installer runs them through sudo, the
//! command-line tool as itself.

use crate::plan::{self, Plan, TargetSpec};
use crate::table::DiskTable;

/// Runs a command, returning its standard output, or why it failed.
pub type Runner<'a> = &'a dyn Fn(&str, &[&str]) -> Result<String, String>;

/// A plan carried out: the devices the installer formats and mounts.
#[derive(Debug, Clone)]
pub struct Placed {
    /// The existing EFI system partition. Mounted at /boot/efi, never formatted.
    pub esp: String,
    pub root: String,
    pub boot: Option<String>,
    pub plan: Plan,
}

/// The table of `disk` as it is now: parted for the geometry, lsblk for filesystems and mount
/// points, and `blkid -p` for any partition lsblk could say nothing about (lsblk reads udev's
/// records, and a system without udev, or a partition udev has not seen yet, has none).
pub fn read_table(disk: &str, run: Runner) -> Result<DiskTable, String> {
    let parted = run("parted", &["-j", "-s", disk, "unit", "s", "print", "free"])
        .map_err(|e| format!("reading the partition table of {disk}: {e}"))?;
    let lsblk = run("lsblk", &["-J", "-o", "PATH,NAME,FSTYPE,LABEL,MOUNTPOINT", disk]).unwrap_or_default();
    let mut t = DiskTable::parse(&parted, &lsblk)?;
    for p in &mut t.parts {
        if p.fstype.is_empty() {
            // Exit status 2 is "nothing found": an empty partition, and no error.
            if let Ok(out) = run("blkid", &["-p", "-o", "export", &p.path]) {
                for line in out.lines() {
                    match line.split_once('=') {
                        Some(("TYPE", v)) => p.fstype = v.to_string(),
                        Some(("LABEL", v)) if p.label.is_empty() => p.label = v.to_string(),
                        _ => {}
                    }
                }
            }
        }
    }
    let mounts = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    let swaps = std::fs::read_to_string("/proc/swaps").unwrap_or_default();
    for p in &mut t.parts {
        if p.mountpoint.is_empty() {
            p.mountpoint = mount_of(&mounts, &swaps, &p.path).unwrap_or_default();
        }
    }
    Ok(t)
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

/// Carry out installing into `spec` on `disk`: read the table again, refuse unless it is the one
/// the person was shown (`shown`), make or wipe the target, read the table once more and check
/// that nothing else moved, and say which devices are the root (and /boot) and the EFI
/// partition. Nothing is formatted here except by the wipe of the chosen placeholder.
pub fn apply(
    disk: &str,
    spec: TargetSpec,
    encrypt: bool,
    efi_boot: bool,
    shown: &str,
    run: Runner,
) -> Result<Placed, String> {
    let before = read_table(disk, run)?;
    let plan = plan::plan(&before, spec, encrypt, efi_boot, shown)?;
    for (argv, required) in plan.partition_commands() {
        let args: Vec<&str> = argv[1..].iter().map(String::as_str).collect();
        if let Err(e) = run(&argv[0], &args) {
            if required {
                return Err(format!("`{}` failed: {e}", argv.join(" ")));
            }
        }
    }
    // The kernel re-reads the table (partitions in use elsewhere on the disk, such as the
    // installer's own, are left as they are) and udev makes the device nodes.
    let _ = run("partprobe", &[disk]);
    let _ = run("udevadm", &["settle", "--timeout=10"]);
    let after = read_table(disk, run)?;
    unchanged_elsewhere(&before, &after, &plan)?;

    let made = |s: u64| after.parts.iter().find(|p| p.run.start == s).map(|p| p.path.clone());
    let root = plan
        .device(plan.root, &made)
        .ok_or("the new root partition is not in the table after writing it")?;
    let boot = match plan.boot {
        Some(b) => Some(plan.device(b, &made).ok_or("the new /boot partition is not in the table after writing it")?),
        None => None,
    };
    for dev in std::iter::once(&root).chain(boot.iter()) {
        wait_for(dev)?;
    }
    Ok(Placed { esp: plan.esp_path.clone(), root, boot, plan })
}

/// After writing: every partition that was not the target is exactly where it was, with the
/// same type and the same filesystem, and every new one is inside the plan's region.
pub fn unchanged_elsewhere(before: &DiskTable, after: &DiskTable, plan: &Plan) -> Result<(), String> {
    let target = match plan.spec {
        TargetSpec::Partition(n) => Some(n),
        TargetSpec::Free { .. } => None,
    };
    let old: Vec<_> = before.parts.iter().filter(|p| Some(p.number) != target).collect();
    for p in &old {
        let same = after.parts.iter().any(|q| {
            q.number == p.number && q.run == p.run && q.type_uuid == p.type_uuid && q.fstype == p.fstype && q.label == p.label
        });
        if !same {
            return Err(format!(
                "after writing, {} is not as it was; stop and check the disk before anything else is done",
                p.path
            ));
        }
    }
    for q in &after.parts {
        let existed = old.iter().any(|p| p.number == q.number && p.run == q.run);
        if !existed && !plan.region.contains(&q.run) {
            return Err(format!(
                "after writing, {} (sectors {}-{}) lies outside the space chosen",
                q.path, q.run.start, q.run.end
            ));
        }
    }
    Ok(())
}

fn wait_for(dev: &str) -> Result<(), String> {
    for _ in 0..20 {
        if std::path::Path::new(dev).exists() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    Err(format!("{dev} did not appear after the table was written"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mounted_partition_is_found_by_its_name() {
        let mounts = "/dev/sda4 /run/live/medium vfat ro 0 0\n/dev/sdb1 /media/My\\040Disk exfat rw 0 0\n";
        let swaps = "Filename\tType\tSize\tUsed\tPriority\n/dev/sdc2 partition 1000 0 -2\n";
        assert_eq!(mount_of(mounts, swaps, "/dev/sda4").as_deref(), Some("/run/live/medium"));
        assert_eq!(mount_of(mounts, swaps, "/dev/sdb1").as_deref(), Some("/media/My Disk"));
        assert_eq!(mount_of(mounts, swaps, "/dev/sdc2").as_deref(), Some("[SWAP]"));
        assert_eq!(mount_of(mounts, swaps, "/dev/sda3"), None);
    }
}
