//! `yantrik-install-target`: the install-into-a-partition planner for the text installer and
//! for checking it against a real (loop) device. Run as root.
//!
//!   yantrik-install-target scan [--uefi|--bios] DISK...
//!       Each disk's partitions and free space as JSON, with what can be chosen and the table's
//!       fingerprint, which `plan` and `apply` must be handed back.
//!   yantrik-install-target plan [--uefi|--bios] [--encrypt] TARGET FINGERPRINT
//!       The commands installing into TARGET would run, without running any.
//!   yantrik-install-target apply [--uefi|--bios] [--encrypt] TARGET FINGERPRINT
//!       Re-read the table, refuse if it changed, make or wipe the target, check nothing else
//!       moved, and print the devices as JSON: {"esp": ..., "root": ..., "boot": ...}.
//!
//!   yantrik-install-target grub-macos-entry
//!       The /etc/grub.d script for a Mac installed beside macOS (efi::GRUB_MACOS_SCRIPT).
//!
//! TARGET is a partition (`sda3`, `nvme0n1p4`) or free space (`sda@START-END`, in sectors), as
//! `scan` names them. Nothing is formatted except a placeholder's wipe; the caller makes the
//! filesystems on the devices `apply` prints.

use std::process::{Command, ExitCode};

use serde_json::json;
use yantrik_install_target::{apply, classify, parse_target_id, plan, DiskTable};

fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd)
        .args(args)
        .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
        .output()
        .map_err(|e| format!("could not run {cmd}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!("{cmd} failed: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

fn scan(disk: &str, efi: bool) -> serde_json::Value {
    let t: DiskTable = match apply::read_table(disk, &run) {
        Ok(t) => t,
        Err(e) => return json!({ "disk": disk, "problem": e, "segments": [] }),
    };
    let segments: Vec<_> = classify::segments(&t, efi)
        .into_iter()
        .map(|s| {
            json!({
                "id": s.id, "kind": s.kind, "title": s.title, "size": s.size, "bytes": s.bytes,
                "kept": s.kept, "eligible": s.eligible, "reason": s.reason, "sentence": s.sentence,
                "device": s.device, "start": s.start, "end": s.end,
            })
        })
        .collect();
    json!({
        "disk": t.path, "model": t.model, "label": t.label, "sectors": t.sectors,
        "sector_size": t.sector_size, "fingerprint": t.fingerprint(),
        "problem": classify::disk_problem(&t, efi),
        "segments": segments,
    })
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let efi = if flag("--uefi") {
        true
    } else if flag("--bios") {
        false
    } else {
        std::path::Path::new("/sys/firmware/efi").exists()
    };
    let encrypt = flag("--encrypt");
    let words: Vec<&str> = args.iter().map(String::as_str).filter(|a| !a.starts_with("--")).collect();
    if words.as_slice() == ["grub-macos-entry"] {
        print!("{}", yantrik_install_target::efi::GRUB_MACOS_SCRIPT);
        return ExitCode::SUCCESS;
    }
    let result: Result<serde_json::Value, String> = match words.as_slice() {
        ["scan", disks @ ..] if !disks.is_empty() => Ok(json!(disks.iter().map(|d| scan(d, efi)).collect::<Vec<_>>())),
        [verb @ ("plan" | "apply"), target, fingerprint] => (|| {
            let (disk, spec) = parse_target_id(target)?;
            let disk = format!("/dev/{disk}");
            if *verb == "plan" {
                let t = apply::read_table(&disk, &run)?;
                let p = plan(&t, spec, encrypt, efi, fingerprint)?;
                Ok(json!({ "sentence": p.sentence, "commands": p.commands() }))
            } else {
                let placed = apply::apply(&disk, spec, encrypt, efi, fingerprint, &run)?;
                Ok(json!({ "esp": placed.esp, "root": placed.root, "boot": placed.boot, "sentence": placed.plan.sentence }))
            }
        })(),
        _ => Err("usage: yantrik-install-target scan DISK... | plan TARGET FINGERPRINT | apply TARGET FINGERPRINT \
                  [--encrypt] [--uefi|--bios]"
            .into()),
    };
    match result {
        Ok(v) => {
            println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("yantrik-install-target: {e}");
            ExitCode::FAILURE
        }
    }
}
