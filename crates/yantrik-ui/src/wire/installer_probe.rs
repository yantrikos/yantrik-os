//! Asking the disks and the firmware what they hold, for the installer, through sudo
//! (crates/yantrik-install-target reads; this only runs its commands). Every answer here fails
//! closed: a disk that cannot be read holds macOS, a machine that cannot be told is a Mac, a
//! target that cannot be read again is refused.

use std::process::Command;

use yantrik_install_target::apply::{self, Ran};
use yantrik_install_target::{classify, efi, parse_target_id, Run, TargetSpec};

/// Run a command through sudo and say how it went. crates/yantrik-install-target needs the exit
/// status itself (blkid's 2 is "nothing found"), which `installer::run_cmd` folds into an error.
pub fn run_ran(cmd: &str, args: &[&str]) -> Ran {
    match Command::new("sudo")
        .arg(cmd)
        .args(args)
        .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
        .output()
    {
        Ok(out) => Ran {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        },
        Err(e) => Ran { code: None, stdout: String::new(), stderr: format!("failed to run sudo {cmd}: {e}") },
    }
}

/// Whether this machine is a Mac (efi::is_apple_machine): DMI's vendors, the firmware's
/// vendor, or an Apple partition type on `disk`. A disk named but unreadable counts as having one.
pub fn is_apple(disk: Option<&str>) -> bool {
    let read = |p: &str| std::fs::read_to_string(p).ok();
    let dmi = read("/sys/class/dmi/id/sys_vendor").map(|sys| {
        let more = ["/sys/class/dmi/id/board_vendor", "/sys/class/dmi/id/bios_vendor"].map(read);
        std::iter::once(sys).chain(more.into_iter().flatten()).collect::<Vec<_>>().join(" ")
    });
    let fw = read("/sys/firmware/efi/fw_vendor");
    let apple_types = disk.is_some_and(|d| {
        apply::read_table(d, &run_ran).map(|t| t.parts.iter().any(classify::is_apple_type)).unwrap_or(true)
    });
    efi::is_apple_machine(dmi.as_deref(), fw.as_deref(), apple_types)
}

/// Whether `disk` (`/dev/sda`) holds macOS, read now: an Apple GPT type or an APFS or HFS+
/// filesystem by `blkid -p` (classify::kind_of), not only what udev told lsblk. A disk whose
/// table cannot be read holds macOS unless it is blank: blkid finds nothing on it and wipefs no
/// signature. Erasing it then takes the confirmation that names it.
pub fn holds_macos(disk: &str) -> bool {
    match apply::read_table(disk, &run_ran) {
        Ok(t) => t.parts.iter().any(|p| classify::kind_of(p).name() == "macos"),
        Err(e) => {
            let blank = run_ran("blkid", &["-p", disk]).code == Some(2) && {
                let sigs = run_ran("wipefs", &["-n", "--noheadings", "-O", "TYPE", disk]);
                sigs.code == Some(0) && sigs.stdout.trim().is_empty()
            };
            tracing::info!(disk, error = %e, blank, "Installer: no table read; a disk that is not blank counts as macOS's");
            !blank
        }
    }
}

/// The target `id` on its disk read again now, as the planner will read it right before
/// writing: the placeholder mounted read-only and looked inside, an empty partition's bytes
/// read. Refused unless it may still be installed into and the table is the one `fingerprint`
/// was taken of.
pub fn recheck(id: &str, fingerprint: &str, efi_boot: bool) -> Result<(), String> {
    let (disk, spec) = parse_target_id(id)?;
    let disk = format!("/dev/{disk}");
    let t = apply::read_table(&disk, &run_ran)?;
    if t.fingerprint() != fingerprint {
        return Err(format!("the partition table on {disk} changed since it was scanned; nothing was chosen"));
    }
    let problem = match spec {
        TargetSpec::Partition(n) => match t.part(n) {
            Some(p) => classify::partition_problem(&t, p, efi_boot),
            None => Some(format!("{disk} has no partition {n}")),
        },
        TargetSpec::Free { start, end } => match t.free.iter().any(|r| r.contains(&Run { start, end })) {
            true => classify::free_problem(&t, &Run { start, end }, efi_boot),
            false => Some(format!("sectors {start}-{end} are not free space on {disk}")),
        },
    };
    problem.map_or(Ok(()), |why| Err(format!("refused: {why}")))
}
