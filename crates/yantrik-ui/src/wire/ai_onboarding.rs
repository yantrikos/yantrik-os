//! The first boot's hardware scan: what this machine has, measured, for the control surface's
//! `describe` of the wizard (control_installer.rs) and a caller deciding where to install.
//!
//! The provider screens this file used to drive (onboarding.slint's AI mode, provider, key and
//! connection test) are no longer mounted: the first run (onboarding_first_run.slint, wired by
//! wire/onboarding.rs) asks who does the thinking now, lists the account's models and lets the
//! person choose one. Nothing here saves a provider or a model any more.
//!
//! Everything here must report what is actually true. Earlier revisions hardcoded every hardware
//! check to `true`.

use slint::ComponentHandle;

use crate::app_context::AppContext;
use crate::App;

/// The minimums. The README's hardware table carries the same numbers in its
/// Minimum column, and the test at the end of this file holds the two together,
/// because they drifted apart once already (#268): the docs said one thing, the
/// wizard checked another, and nobody could say which was the requirement.
/// The disk figure is the whole size of the disk the installer will write to —
/// not free space on the live session, which describes the stick, not the target.
const MIN_CPU_CORES: usize = 2;
const MIN_RAM_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MIN_DISK_BYTES: u64 = 6 * 1024 * 1024 * 1024;

/// Start the hardware scan.
pub fn wire(ui: &App, ctx: &AppContext) {
    // Hardware scan — real measurements from the system observer.
    let ui_weak = ui.as_weak();
    let configured_url = ctx.llm_base_url.clone();
    slint::Timer::single_shot(std::time::Duration::from_millis(100), move || {
        let Some(ui) = ui_weak.upgrade() else { return };

        // CPU count is available without waiting on the observer.
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0);
        ui.set_onboard_ai_hw_cpu_ok(cpus >= MIN_CPU_CORES);
        ui.set_onboard_ai_hw_cpu_label(
            if cpus > 0 { format!("{cpus} cores") } else { "Unknown".into() }.into(),
        );

        let gpu = detect_gpu();
        ui.set_onboard_ai_hw_gpu_ok(gpu.is_some());
        ui.set_onboard_ai_hw_gpu_label(gpu.clone().unwrap_or_else(|| "Not detected".into()).into());

        // RAM is read one-shot rather than waiting on the observer, whose
        // resource poll is 10s by default — far longer than the user watches
        // this screen, so the scan used to time out into "Unknown". The disk
        // row measures the install target and is a one-shot of its own.
        let weak = ui.as_weak();
        let has_gpu = gpu.is_some();
        let local_worth_it = matches!(
            gpu.as_deref(),
            Some("NVIDIA") | Some("AMD ROCm") | Some("AMD (amdgpu)")
        );
        std::thread::spawn(move || {
            let hw = read_hardware();
            let runtime = probe_runtime(configured_url.as_deref());
            let (disk_ok, disk_label) = measure_install_target();
            {
                let weak = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = weak.upgrade() else { return };
                    ui.set_onboard_ai_hw_ram_ok(hw.ram_total >= MIN_RAM_BYTES);
                    ui.set_onboard_ai_hw_ram_label(format!("{} GB", gib(hw.ram_total)).into());
                    ui.set_onboard_ai_hw_disk_ok(disk_ok);
                    ui.set_onboard_ai_hw_disk_label(disk_label.into());
                    ui.set_onboard_ai_hw_network_ok(hw.network);
                });
            }
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = weak.upgrade() else { return };
                ui.set_onboard_ai_hw_runtime_ok(runtime);

                // Recommend on the GPU, which is the durable fact — a missing
                // runtime is installable in a minute. Previously a failed
                // runtime probe alone forced "cloud", so the scan could say
                // "GPU available" and the very next screen badge Cloud as
                // Recommended. Only "local"/"cloud" are rendered by the UI.
                // Integrated graphics is a GPU and still the wrong place to run a
                // model, so "has an accelerator" and "should run locally" are not the
                // same question. Only discrete NVIDIA/AMD earns the local recommendation.
                let recommend = if local_worth_it { "local" } else { "cloud" };
                ui.set_onboard_ai_hw_recommend(recommend.into());
                tracing::info!(
                    gpu = has_gpu,
                    local_worth_it,
                    runtime,
                    recommend,
                    "Onboarding: scan complete"
                );
            });
        });
    });
}

struct Measured {
    ram_total: u64,
    network: bool,
}

/// One-shot hardware read. Same source the observer uses (sysinfo), taken
/// directly so the scan does not depend on the observer's poll cycle.
fn read_hardware() -> Measured {
    use sysinfo::System;

    let mut sys = System::new();
    sys.refresh_memory();

    Measured {
        ram_total: sys.total_memory(),
        // Reachability, not link state: what matters on this screen is
        // whether a cloud provider could be contacted at all.
        network: network_reachable(),
    }
}

/// The Disk row of the hardware scan.
///
/// This row used to report the free space of the live system's root filesystem.
/// On a live USB that is the session's overlay on the stick it booted from, a
/// number that says nothing about the disk being installed to — the only disk
/// this screen cares about. A machine could fail the check beside an empty
/// 500 GB disk, and a machine with no target disk at all could pass it on the
/// strength of a tmpfs.
///
/// So it measures the target instead: in installer mode, the disk the picker
/// preselects, sized with lsblk. Booted live with nothing to install to, it
/// says that plainly rather than printing a number that answers nothing.
fn measure_install_target() -> (bool, String) {
    let installer_mode = std::path::Path::new("/opt/yantrik/.installer-mode").exists();
    let target = if installer_mode { first_install_candidate() } else { None };
    disk_row(installer_mode, target)
}

/// The disk the wizard will install to unless the person picks another: the
/// installer's own listing and its own preselection (`wire::installer::preselect`,
/// which picks only a lone internal disk without macOS, never a USB one), sized with `lsblk -b`;
/// `None` when it picks nothing. The listing is run here rather
/// than read off the picker's selection property because the scan and the
/// picker are populated on separate threads, and UI properties are not ours
/// to read from this one.
fn first_install_candidate() -> Option<(String, Option<u64>)> {
    let disks = super::installer::detect_disks();
    let disk = super::installer::preselect(&disks)?;
    Some((disk.name.clone(), disk_size_bytes(&disk.name)))
}

/// The size of `/dev/<name>` in bytes, from `lsblk -b -dn -o SIZE`.
fn disk_size_bytes(name: &str) -> Option<u64> {
    let output = std::process::Command::new("lsblk")
        .args(["-b", "-dn", "-o", "SIZE", &format!("/dev/{name}")])
        .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_size_bytes(&String::from_utf8_lossy(&output.stdout))
}

/// `lsblk -b` prints one number of bytes and nothing else. Anything that is
/// not exactly that is treated as no answer rather than guessed at — in
/// particular lsblk's human sizes ("80G"), which are what `-b` is not.
fn parse_size_bytes(out: &str) -> Option<u64> {
    out.trim().parse::<u64>().ok()
}

/// The scan's verdict on the install target, kept apart from the I/O that
/// measures it so it can be tested without a disk. `target` is the name and
/// byte-size of the disk the installer would write to, or `None` when no
/// candidate was found.
fn disk_row(installer_mode: bool, target: Option<(String, Option<u64>)>) -> (bool, String) {
    if !installer_mode {
        // Running live: nothing will be written to any disk, so no disk can
        // fail this check — and the row says that instead of implying one was
        // measured.
        return (true, "Running live — no disk needed".into());
    }
    let Some((name, size)) = target else {
        return (false, "No disk found to install to".into());
    };
    let Some(size) = size else {
        return (false, format!("{name} — could not read its size"));
    };
    if size >= MIN_DISK_BYTES {
        (true, format!("{} GB ({name})", gib(size)))
    } else {
        (false, format!("{} GB ({name}) — needs {} GB", gib(size), gib(MIN_DISK_BYTES)))
    }
}

/// Cheap reachability check — a TCP connect, no DNS-only or ICMP assumptions.
fn network_reachable() -> bool {
    use std::net::{SocketAddr, TcpStream};
    let timeout = std::time::Duration::from_secs(2);
    ["1.1.1.1:443", "8.8.8.8:443"].iter().any(|addr| {
        addr.parse::<SocketAddr>()
            .ok()
            .is_some_and(|sa| TcpStream::connect_timeout(&sa, timeout).is_ok())
    })
}

fn gib(bytes: u64) -> u64 {
    bytes / (1024 * 1024 * 1024)
}

/// Detect a usable GPU. Returns a human-readable label when one is found.
/// What accelerator this machine actually has, if any.
///
/// This used to return Some("Available") for the mere existence of a
/// /dev/dri/renderD* node. That is not evidence of an accelerator: every VM with
/// virtio-gpu publishes one and renders on the host CPU through llvmpipe, and so
/// does a server whose only graphics is its BMC. On a Proxmox VM the screen
/// consequently reported a GPU and recommended local inference, where a model
/// would have run on software rasterisation.
///
/// So the render node is only a starting point — the driver behind it decides.
fn detect_gpu() -> Option<String> {
    if std::path::Path::new("/proc/driver/nvidia/version").exists()
        || std::path::Path::new("/dev/nvidia0").exists()
    {
        return Some("NVIDIA".into());
    }
    if std::path::Path::new("/sys/class/kfd").exists() {
        return Some("AMD ROCm".into());
    }
    for entry in std::fs::read_dir("/sys/class/drm").into_iter().flatten().flatten() {
        if !entry.file_name().to_string_lossy().starts_with("renderD") {
            continue;
        }
        let Ok(uevent) = std::fs::read_to_string(entry.path().join("device/uevent")) else {
            continue;
        };
        let driver = uevent
            .lines()
            .find_map(|l| l.strip_prefix("DRIVER="))
            .unwrap_or("")
            .trim();
        // Named explicitly, because the interesting case is everything NOT here:
        // virtio-pci, virtio_gpu, vmwgfx, qxl, bochs-drm, simpledrm, mgag200 and
        // ast all expose a render node and none of them can run a model.
        match driver {
            "amdgpu" => return Some("AMD (amdgpu)".into()),
            "nouveau" => return Some("NVIDIA (nouveau)".into()),
            "i915" | "xe" => return Some("Intel integrated".into()),
            _ => {}
        }
    }
    None
}

/// Probe the LLM runtime at the endpoint the companion is actually configured
/// to use, not an assumed localhost. A deployment with Ollama on another host
/// — the documented setup — previously reported "Ollama not found" while the
/// companion was concurrently using that same Ollama successfully.
fn probe_runtime(configured: Option<&str>) -> bool {
    let mut candidates: Vec<String> = Vec::new();
    if let Some(url) = configured {
        let base = url.trim_end_matches('/');
        let base = base.strip_suffix("/v1").unwrap_or(base);
        candidates.push(format!("{base}/api/tags"));
        candidates.push(format!("{base}/v1/models"));
    }
    candidates.push("http://localhost:11434/api/tags".into());

    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(3))
        .build();

    for url in &candidates {
        if agent.get(url).call().is_ok() {
            tracing::info!(endpoint = %yantrik_app_runtime::place::address_of(url), "Onboarding: LLM runtime reachable");
            return true;
        }
    }
    tracing::info!(tried = candidates.len(), "Onboarding: no LLM runtime reachable");
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    /// The README's hardware table carries a Minimum column, and the wizard
    /// checks against the constants above it. This reads the README and holds
    /// the two to each other: #268 was the docs saying 6 GB, the wizard
    /// checking 4, and no way to tell which was the requirement.
    #[test]
    fn minimums_match_readme_hardware_table() {
        let readme = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../README.md"),
        )
        .expect("README.md sits in the repository root");

        fn minimum_cell(readme: &str, row: &str) -> String {
            let prefix = format!("| **{row}**");
            let line = readme
                .lines()
                .find(|l| l.starts_with(&prefix))
                .unwrap_or_else(|| panic!("README's hardware table has no {row} row"));
            line.split('|')
                .nth(2)
                .unwrap_or_else(|| panic!("the {row} row has no Minimum cell"))
                .trim()
                .to_string()
        }

        fn leading_number(cell: &str) -> u64 {
            let digits: String = cell.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits
                .parse()
                .unwrap_or_else(|_| panic!("Minimum cell {cell:?} does not start with a number"))
        }

        assert_eq!(
            leading_number(&minimum_cell(&readme, "CPU")) as usize,
            MIN_CPU_CORES
        );
        assert_eq!(leading_number(&minimum_cell(&readme, "RAM")) * GIB, MIN_RAM_BYTES);
        assert_eq!(
            leading_number(&minimum_cell(&readme, "Disk")) * GIB,
            MIN_DISK_BYTES,
            "the README's disk minimum and the wizard's check must be the one number"
        );
    }

    /// The Disk row reports the disk the installer would write to. The live
    /// session's own free space — what the row used to read — appears nowhere
    /// in these inputs, so a row that borrowed it could not pass.
    #[test]
    fn disk_row_measures_the_install_target() {
        // A target above the minimum passes and names itself.
        let (ok, label) = disk_row(true, Some(("sda".into(), Some(240_057_409_536))));
        assert!(ok, "{label}");
        assert!(label.contains("sda") && label.contains("223 GB"), "{label}");

        // A target below the minimum fails and says what the minimum is.
        let (ok, label) = disk_row(true, Some(("sdb".into(), Some(MIN_DISK_BYTES - 1))));
        assert!(!ok, "{label}");
        assert!(
            label.contains("sdb") && label.contains(&format!("needs {} GB", MIN_DISK_BYTES / GIB)),
            "{label}"
        );

        // Exactly the minimum passes.
        let (ok, _) = disk_row(true, Some(("sda".into(), Some(MIN_DISK_BYTES))));
        assert!(ok);

        // No candidate disk fails, rather than passing on the live session's space.
        let (ok, label) = disk_row(true, None);
        assert!(!ok, "{label}");
        assert!(label.contains("No disk"), "{label}");

        // A disk whose size lsblk will not report is not a pass either.
        let (ok, label) = disk_row(true, Some(("sda".into(), None)));
        assert!(!ok, "{label}");
        assert!(label.contains("could not read"), "{label}");

        // Booted live with nothing to install to, the row says that instead of
        // reporting on a requirement that does not apply.
        let (ok, label) = disk_row(false, None);
        assert!(ok, "{label}");
        assert!(label.contains("live"), "{label}");
    }

    #[test]
    fn parses_lsblk_byte_sizes() {
        assert_eq!(parse_size_bytes("240057409536\n"), Some(240_057_409_536));
        assert_eq!(parse_size_bytes("  80015632224  "), Some(80_015_632_224));
        assert_eq!(parse_size_bytes(""), None);
        // lsblk's human size is what `-b` exists to avoid; it is not a byte
        // count and must not be read as one.
        assert_eq!(parse_size_bytes("80G"), None);
    }
}
