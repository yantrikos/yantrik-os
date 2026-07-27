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

/// Minimums from the README's stated hardware requirements.
const MIN_RAM_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MIN_DISK_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Wire AI onboarding callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    // AI mode selected (local/cloud/later)
    ui.on_onboard_ai_mode_selected(move |mode| {
        tracing::info!(mode = %mode, "Onboarding: AI mode selected");
    });

    // Provider selected during onboarding — prefill the endpoint we will test.
    let ui_weak = ui.as_weak();
    ui.on_onboard_ai_provider_selected(move |provider| {
        let Some(ui) = ui_weak.upgrade() else { return };
        let id = provider.to_string();
        let (name, url) = provider_preset(&id);
        tracing::info!(provider = %id, name, url, "Onboarding: AI provider selected");
        ui.set_onboard_ai_test_status("".into());
        ui.set_onboard_ai_test_model("".into());
    });

    // API key submitted — persist it, so the choice survives to first use.
    let ui_weak = ui.as_weak();
    ui.on_onboard_ai_api_key_submitted(move |provider, key| {
        let Some(ui) = ui_weak.upgrade() else { return };
        let id = provider.to_string();
        let key_str = key.to_string();
        let (name, url) = provider_preset(&id);

        if url.is_empty() {
            tracing::warn!(provider = %id, "Onboarding: no known endpoint for provider, not saving");
            ui.set_onboard_ai_test_status("error".into());
            return;
        }

        let entry = ProviderStoreEntry {
            id: format!("{}-onboarding", id),
            name: name.to_string(),
            provider_type: id.clone(),
            base_url: url.to_string(),
            api_key: if key_str.is_empty() { None } else { Some(key_str) },
            auth_type: auth_type_for(&id).to_string(),
            is_primary: true,
            is_fallback: false,
        };

        let mut store = ProviderStore::load();
        store.entries.retain(|e| !e.is_primary);
        store.entries.push(entry);
        store.save();

        // Log presence, never the key itself.
        tracing::info!(
            provider = %id,
            endpoint = url,
            has_key = !key.is_empty(),
            "Onboarding: provider saved as primary"
        );
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
        ui.set_onboard_ai_hw_cpu_ok(cpus >= 2);
        ui.set_onboard_ai_hw_cpu_label(
            if cpus > 0 { format!("{cpus} cores") } else { "Unknown".into() }.into(),
        );

        let gpu = detect_gpu();
        ui.set_onboard_ai_hw_gpu_ok(gpu.is_some());
        ui.set_onboard_ai_hw_gpu_label(gpu.clone().unwrap_or_else(|| "Not detected".into()).into());

        // RAM/disk are read one-shot rather than waiting on the observer,
        // whose resource poll is 10s by default — far longer than the user
        // watches this screen, so the scan used to time out into "Unknown".
        let weak = ui.as_weak();
        let has_gpu = gpu.is_some();
        std::thread::spawn(move || {
            let hw = read_hardware();
            let runtime = probe_runtime(configured_url.as_deref());
            {
                let weak = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = weak.upgrade() else { return };
                    ui.set_onboard_ai_hw_ram_ok(hw.ram_total >= MIN_RAM_BYTES);
                    ui.set_onboard_ai_hw_ram_label(format!("{} GB", gib(hw.ram_total)).into());
                    ui.set_onboard_ai_hw_disk_ok(hw.disk_free >= MIN_DISK_BYTES);
                    ui.set_onboard_ai_hw_disk_label(
                        format!("{} GB free", gib(hw.disk_free)).into(),
                    );
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
                let recommend = if has_gpu { "local" } else { "cloud" };
                ui.set_onboard_ai_hw_recommend(recommend.into());
                tracing::info!(gpu = has_gpu, runtime, recommend, "Onboarding: scan complete");
            });
        });
    });
}

struct Measured {
    ram_total: u64,
    disk_free: u64,
    network: bool,
}

/// One-shot hardware read. Same source the observer uses (sysinfo), taken
/// directly so the scan does not depend on the observer's poll cycle.
fn read_hardware() -> Measured {
    use sysinfo::{Disks, System};

    let mut sys = System::new();
    sys.refresh_memory();
    let ram_total = sys.total_memory();

    // Root mount, falling back to the largest disk we can see.
    let disks = Disks::new_with_refreshed_list();
    let mut disk_free = 0u64;
    for disk in &disks {
        if disk.mount_point().to_string_lossy() == "/" {
            disk_free = disk.available_space();
            break;
        }
        disk_free = disk_free.max(disk.available_space());
    }

    Measured {
        ram_total,
        disk_free,
        // Reachability, not link state: what matters on this screen is
        // whether a cloud provider could be contacted at all.
        network: network_reachable(),
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
fn detect_gpu() -> Option<String> {
    if std::path::Path::new("/proc/driver/nvidia/version").exists()
        || std::path::Path::new("/dev/nvidia0").exists()
    {
        return Some("NVIDIA".into());
    }
    if std::path::Path::new("/sys/class/kfd").exists() {
        return Some("AMD ROCm".into());
    }
    // Any DRM render node means some accelerator is present.
    if let Ok(entries) = std::fs::read_dir("/dev/dri") {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with("renderD") {
                return Some("Available".into());
            }
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
fn auth_type_for(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "x-api-key",
        "ollama" | "llamacpp" | "lmstudio" | "vllm" => "none",
        _ => "bearer",
    }
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
