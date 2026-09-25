//! AI Onboarding wiring — first-boot hardware scan + provider setup.
//!
//! Everything here runs exactly once, before the user has any other way to
//! reach the system, so it must report what is actually true. Earlier
//! revisions hardcoded every hardware check to `true`, simulated the
//! connection test with a 2s sleep and an unconditional "success", and
//! dropped the submitted API key on the floor — a user who typed a real key
//! landed on a desktop that could not think, with nothing on screen saying so.
//!
//! Detection and validation now share the same code paths Settings uses
//! (`wire::settings::{provider_preset, test_provider_connection}`,
//! `ProviderStore`) so the two screens cannot drift apart.

use slint::ComponentHandle;

use crate::app_context::AppContext;
use crate::wire::settings::{
    provider_preset, test_provider_connection, ProviderStore, ProviderStoreEntry,
};
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

/// Wire AI onboarding callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    // AI mode selected (local/cloud/later)
    ui.on_onboard_ai_mode_selected(move |mode| {
        tracing::info!(mode = %mode, "Onboarding: AI mode selected");
    });

    // Provider selected — persist it immediately. A local runtime has no API
    // key step, so `on_onboard_ai_api_key_submitted` never fires for it; if
    // selection did not save, "Test Connection" would find no provider and the
    // user's choice would be lost on the way to the desktop.
    let ui_weak = ui.as_weak();
    let configured_for_select = ctx.llm_base_url.clone();
    ui.on_onboard_ai_provider_selected(move |provider| {
        let Some(ui) = ui_weak.upgrade() else { return };
        let id = provider.to_string();
        save_primary(&id, None, configured_for_select.as_deref());
        ui.set_onboard_ai_test_status("".into());
        ui.set_onboard_ai_test_model("".into());
    });

    // API key submitted — re-save the same provider, now carrying the key.
    let ui_weak = ui.as_weak();
    let configured_for_save = ctx.llm_base_url.clone();
    ui.on_onboard_ai_api_key_submitted(move |provider, key| {
        let Some(ui) = ui_weak.upgrade() else { return };
        let id = provider.to_string();
        let key_str = key.to_string();
        let key_opt = if key_str.is_empty() { None } else { Some(key_str) };
        if !save_primary(&id, key_opt, configured_for_save.as_deref()) {
            ui.set_onboard_ai_test_status("error".into());
        }
    });

    // Test AI connection — actually contact the provider.
    let ui_weak = ui.as_weak();
    ui.on_onboard_ai_test_connection(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        ui.set_onboard_ai_test_status("testing".into());

        // Test whatever onboarding just saved as primary; fall back to the
        // configured endpoint if the user has not chosen a provider yet.
        let store = ProviderStore::load();
        let Some(primary) = store.primary().cloned() else {
            tracing::warn!("Onboarding: connection test with no provider configured");
            ui.set_onboard_ai_test_status("error".into());
            return;
        };

        let weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = test_provider_connection(
                &primary.base_url,
                primary.api_key.as_deref(),
                &primary.auth_type,
            );
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = weak.upgrade() else { return };
                if result.success {
                    ui.set_onboard_ai_test_status("success".into());
                    ui.set_onboard_ai_test_latency_ms(result.latency_ms);
                    ui.set_onboard_ai_test_privacy(privacy_of(&primary.base_url).into());
                    ui.set_onboard_ai_test_model(primary.name.clone().into());
                    tracing::info!(
                        provider = %primary.provider_type,
                        latency_ms = result.latency_ms,
                        "Onboarding: provider reachable"
                    );
                } else {
                    ui.set_onboard_ai_test_status("error".into());
                    ui.set_onboard_ai_test_model(result.message.clone().into());
                    tracing::warn!(
                        provider = %primary.provider_type,
                        error = %result.message,
                        "Onboarding: provider unreachable"
                    );
                }
            });
        });
    });

    // Skip AI setup
    ui.on_onboard_ai_skip_setup(move || {
        tracing::info!("Onboarding: AI setup skipped, using bundled fallback");
    });

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
/// installer's own listing, whose first candidate its picker preselects
/// (`wire::installer`), sized with `lsblk -b`. The listing is run here rather
/// than read off the picker's selection property because the scan and the
/// picker are populated on separate threads, and UI properties are not ours
/// to read from this one.
fn first_install_candidate() -> Option<(String, Option<u64>)> {
    let disk = super::installer::detect_disks().into_iter().next()?;
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
            tracing::info!(endpoint = %url, "Onboarding: LLM runtime reachable");
            return true;
        }
    }
    tracing::info!(tried = candidates.len(), "Onboarding: no LLM runtime reachable");
    false
}

/// Anthropic uses `x-api-key`; the rest of the presets are bearer-token.
/// Shared with the installer, which rebuilds the wizard's provider entry for
/// the installed user and must not guess the auth scheme differently.
pub(crate) fn auth_type_for(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "x-api-key",
        _ if is_local_runtime(provider) => "none",
        _ => "bearer",
    }
}

/// Providers served by a runtime the user hosts, whose preset endpoint is a
/// localhost guess rather than a fixed vendor URL.
fn is_local_runtime(provider: &str) -> bool {
    matches!(provider, "ollama" | "llamacpp" | "lmstudio" | "vllm")
}

/// Persist `provider` as the primary, replacing any existing primary.
/// Returns false when there is no endpoint to save.
///
/// Called both on selection (no key yet) and on key submission, so a local
/// runtime — which never reaches the key step — is still saved.
fn save_primary(provider: &str, api_key: Option<String>, configured: Option<&str>) -> bool {
    let (name, preset_url) = provider_preset(provider);

    // Local-runtime presets hardcode localhost. When the config points the
    // companion elsewhere — the documented remote-Ollama setup — that is the
    // endpoint that actually works, so prefer it. Otherwise onboarding would
    // save an endpoint the hardware scan just proved unreachable.
    let url: String = match (is_local_runtime(provider), configured) {
        (true, Some(cfg)) if !cfg.is_empty() => cfg.to_string(),
        _ => preset_url.to_string(),
    };

    if url.is_empty() {
        tracing::warn!(provider, "Onboarding: no known endpoint for provider, not saving");
        return false;
    }

    let has_key = api_key.is_some();
    let entry = ProviderStoreEntry {
        id: format!("{provider}-onboarding"),
        name: name.to_string(),
        provider_type: provider.to_string(),
        base_url: url.clone(),
        api_key,
        auth_type: auth_type_for(provider).to_string(),
        is_primary: true,
        is_fallback: false,
    };

    let mut store = ProviderStore::load();
    store.entries.retain(|e| !e.is_primary);
    store.entries.push(entry);
    store.save();

    // Log presence, never the key itself.
    tracing::info!(provider, endpoint = %url, has_key, "Onboarding: provider saved as primary");
    true
}

/// Whether requests to this endpoint leave the machine.
fn privacy_of(base_url: &str) -> &'static str {
    if base_url.contains("localhost") || base_url.contains("127.0.0.1") {
        "local"
    } else if base_url.starts_with("http://") {
        "network"
    } else {
        "cloud"
    }
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
