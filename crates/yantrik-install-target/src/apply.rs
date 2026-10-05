//! Carrying out a plan on a real disk.
//!
//! Every command goes through a runner the caller supplies (`read::Runner`): the desktop
//! installer runs them through sudo, the command-line tool as itself.

use crate::plan::{self, PartRef, Plan, TargetSpec};
use crate::read::{ok, Runner};
use crate::table::{disk_name, DiskTable, Run};

pub use crate::read::{mount_of, read_table, Ran};

/// A plan carried out: the devices the installer formats and mounts.
#[derive(Debug, Clone)]
pub struct Placed {
    /// The existing EFI system partition. Mounted at /boot/efi, never formatted.
    pub esp: String,
    pub root: String,
    pub boot: Option<String>,
    pub plan: Plan,
}

/// Carry out installing into `spec` on `disk`: read the table again, refuse unless it is the one
/// the person was shown (`shown`), check the chosen partition is where the kernel says it is,
/// make or wipe the target, read the table once more and check that nothing else moved, and
/// check every partition about to be formatted is exactly where the plan put it. Nothing is
/// formatted here except by the wipe of the chosen placeholder.
///
/// A command that fails part way does not end the checking: the table is read again, and the
/// error says whether everything else on the disk is still where it was.
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
    if let TargetSpec::Partition(n) = spec {
        let p = before.part(n).ok_or_else(|| format!("{disk} has no partition {n}"))?;
        check_extent(&p.path, p.run, before.sector_size, run)
            .map_err(|e| format!("{e}; nothing was written"))?;
    }
    for (argv, required) in plan.partition_commands() {
        let args: Vec<&str> = argv[1..].iter().map(String::as_str).collect();
        let done = run(&argv[0], &args).ok(&argv[0]);
        if let (Err(e), true) = (done, required) {
            settle(disk, run);
            let checked = read_table(disk, run).and_then(|after| unchanged_elsewhere(&before, &after, &plan));
            let state = match checked {
                Ok(()) => "every other partition on the disk is still exactly where it was".to_string(),
                Err(why) => why,
            };
            return Err(format!("`{}` failed: {e}. Checked afterwards: {state}", argv.join(" ")));
        }
    }
    settle(disk, run);
    let after = read_table(disk, run)?;
    unchanged_elsewhere(&before, &after, &plan)?;

    let made = |s: u64| after.parts.iter().find(|p| p.run.start == s).map(|p| p.path.clone());
    let mut placed = Vec::new();
    for (what, r) in [("root", Some(plan.root)), ("/boot", plan.boot)] {
        let Some(r) = r else { continue };
        let dev = plan
            .device(r, &made)
            .ok_or_else(|| format!("the new {what} partition is not in the table after writing it"))?;
        let extent = plan.extent(r, &before).ok_or_else(|| format!("the plan has no extent for the {what} partition"))?;
        wait_for(&dev, run)?;
        check_extent(&dev, extent, after.sector_size, run).map_err(|e| format!("{e}; it was not formatted"))?;
        placed.push(dev);
    }
    let root = placed.remove(0);
    Ok(Placed { esp: plan.esp_path.clone(), root, boot: placed.pop(), plan })
}

/// The kernel re-reads the table (partitions in use elsewhere on the disk, such as the
/// installer's own, are left as they are) and udev makes the device nodes.
fn settle(disk: &str, run: Runner) {
    let _ = run("partprobe", &[disk]);
    let _ = run("udevadm", &["settle", "--timeout=10"]);
}

/// After writing: the table is the same table (its kind, GUID and size), every partition that
/// was not the target is exactly where it was with the same type, GUID, name, flags and
/// filesystem, and every new one is inside the plan's region.
pub fn unchanged_elsewhere(before: &DiskTable, after: &DiskTable, plan: &Plan) -> Result<(), String> {
    if (&before.label, &before.guid, before.sectors, before.sector_size)
        != (&after.label, &after.guid, after.sectors, after.sector_size)
    {
        return Err(format!(
            "after writing, the partition table of {} is not the one it was (GUID {} then, {} now); stop and check the disk before anything else is done",
            before.path, before.guid, after.guid
        ));
    }
    let target = match plan.spec {
        TargetSpec::Partition(n) => Some(n),
        TargetSpec::Free { .. } => None,
    };
    let old: Vec<_> = before.parts.iter().filter(|p| Some(p.number) != target).collect();
    for p in &old {
        let same = after.parts.iter().any(|q| {
            (q.number, q.run, &q.type_uuid, &q.uuid, &q.name, &q.flags, &q.fstype, &q.label)
                == (p.number, p.run, &p.type_uuid, &p.uuid, &p.name, &p.flags, &p.fstype, &p.label)
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

/// Whether the kernel's own record of `dev` (/sys/class/block/<dev>/{start,size}, always in
/// 512-byte units) is exactly `want`, in the table's sectors of `sector_size` bytes.
pub fn check_extent(dev: &str, want: Run, sector_size: u64, run: Runner) -> Result<(), String> {
    let sys = format!("/sys/class/block/{}", disk_name(dev));
    let read = |f: &str| -> Result<u64, String> {
        let text = ok(run, "cat", &[&format!("{sys}/{f}")])?;
        text.trim().parse().map_err(|_| format!("{sys}/{f} says `{}`", text.trim()))
    };
    let (start, size) = match (read("start"), read("size")) {
        (Ok(s), Ok(n)) => (s, n),
        (Err(e), _) | (_, Err(e)) => return Err(format!("the kernel's record of {dev} could not be read ({e})")),
    };
    let per = sector_size / 512;
    let (want_start, want_size) = (want.start * per, want.sectors() * per);
    if (start, size) != (want_start, want_size) {
        return Err(format!(
            "the kernel has {dev} at 512-byte sectors {start}+{size}, not {want_start}+{want_size} as planned"
        ));
    }
    Ok(())
}

fn wait_for(dev: &str, run: Runner) -> Result<(), String> {
    for _ in 0..20 {
        if run("test", &["-b", dev]).code == Some(0) {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    Err(format!("{dev} did not appear after the table was written"))
}

impl Plan {
    /// Where the partition a reference names is meant to be: an existing one's extent in
    /// `before`, or the extent the plan makes for a new one.
    pub fn extent(&self, r: PartRef, before: &DiskTable) -> Option<Run> {
        match r {
            PartRef::Existing(n) => before.part(n).map(|p| p.run),
            PartRef::StartingAt(s) => self.steps.iter().find_map(|step| match *step {
                plan::Step::MkPart { start, end, .. } if start == s => Some(Run { start, end }),
                _ => None,
            }),
        }
    }
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

    #[test]
    fn the_kernels_extent_must_be_the_planned_one() {
        let sys = |start: &'static str, size: &'static str| {
            move |cmd: &str, args: &[&str]| match (cmd, args.first().copied().unwrap_or("")) {
                ("cat", p) if p.ends_with("/start") => Ran::exited(0, start),
                ("cat", p) if p.ends_with("/size") => Ran::exited(0, size),
                _ => Ran::exited(1, ""),
            }
        };
        let want = Run { start: 2048, end: 4095 };
        assert!(check_extent("/dev/sda4", want, 512, &sys("2048\n", "2048\n")).is_ok());
        let e = check_extent("/dev/sda4", want, 512, &sys("2048\n", "2047\n")).unwrap_err();
        assert!(e.contains("not 2048+2048"), "{e}");
        assert!(check_extent("/dev/sda4", want, 512, &sys("4096", "2048")).is_err());
        // 4096-byte sectors: the kernel still counts in 512.
        assert!(check_extent("/dev/sda4", Run { start: 256, end: 511 }, 4096, &sys("2048", "2048")).is_ok());
        let unreadable = |_: &str, _: &[&str]| Ran::exited(1, "");
        assert!(check_extent("/dev/sda4", want, 512, &unreadable).unwrap_err().contains("could not be read"));
        assert!(check_extent("/dev/sda4", want, 512, &sys("lots", "2048")).is_err());
    }
}
