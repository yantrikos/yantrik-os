//! Hardware detection & AI recommendation engine.
//!
//! Probes the system for GPU, RAM, CPU, disk, battery, network, and local
//! AI runtimes (Ollama, llama-server) to recommend the best setup mode.

use serde::{Deserialize, Serialize};
use std::process::Command;

// ── GPU Profile ──────────────────────────────────────────────────────────

/// Detected GPU information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuProfile {
    /// GPU vendor: "nvidia", "amd", "intel", "unknown".
    pub vendor: String,
    /// GPU model name (e.g. "NVIDIA GeForce RTX 3090 Ti").
    pub model: String,
    /// VRAM in megabytes (0 if unknown).
    pub vram_mb: u64,
    /// Whether this is a dedicated (discrete) GPU.
    pub is_dedicated: bool,
    /// Driver version string (empty if unknown).
    pub driver_version: String,
}

// ── Local runtime detection ──────────────────────────────────────────────

/// Kind of locally-detected AI runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeKind {
    /// Ollama (localhost:11434)
    Ollama,
    /// llama-server or llama.cpp server
    LlamaServer,
}

impl std::fmt::Display for RuntimeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeKind::Ollama => write!(f, "Ollama"),
            RuntimeKind::LlamaServer => write!(f, "llama-server"),
        }
    }
}

/// A locally-detected AI inference runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalRuntime {
    /// Runtime type.
    pub kind: RuntimeKind,
    /// Base URL where the runtime is reachable.
    pub url: String,
    /// Models available on this runtime.
    pub models: Vec<String>,
}

// ── Hardware Profile ─────────────────────────────────────────────────────

/// Complete hardware profile of the system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareProfile {
    /// Detected GPUs (may be empty on headless/CPU-only systems).
    pub gpus: Vec<GpuProfile>,
    /// Total system RAM in megabytes.
    pub ram_mb: u64,
    /// Number of logical CPU cores.
    pub cpu_cores: usize,
    /// Free disk space in gigabytes (on the primary partition).
    pub disk_free_gb: u64,
    /// Whether the system is running on battery power.
    pub on_battery: bool,
    /// Whether the system has internet connectivity.
    pub has_internet: bool,
    /// Locally-detected AI runtimes.
    pub local_runtimes: Vec<LocalRuntime>,
}

impl HardwareProfile {
    /// Total VRAM across all dedicated GPUs (in MB).
    pub fn total_vram_mb(&self) -> u64 {
        self.gpus
            .iter()
            .filter(|g| g.is_dedicated)
            .map(|g| g.vram_mb)
            .sum()
    }

    /// Whether any NVIDIA GPU is present.
    pub fn has_nvidia(&self) -> bool {
        self.gpus.iter().any(|g| g.vendor == "nvidia")
    }

    /// Whether Ollama is running locally.
    pub fn has_ollama(&self) -> bool {
        self.local_runtimes.iter().any(|r| r.kind == RuntimeKind::Ollama)
    }

    /// All models available across all local runtimes.
    pub fn all_local_models(&self) -> Vec<String> {
        self.local_runtimes
            .iter()
            .flat_map(|r| r.models.clone())
            .collect()
    }
}

// ── Detection ────────────────────────────────────────────────────────────

/// Detect the complete hardware profile.
///
/// This probes GPUs, RAM, CPU, disk, battery, network, and local runtimes.
/// Each probe has a timeout so the total detection time is bounded.
pub fn detect_hardware() -> HardwareProfile {
    let gpus = detect_gpus();
    let (ram_mb, cpu_cores) = detect_cpu_ram();
    let disk_free_gb = detect_disk_free();
    let on_battery = detect_battery();
    let has_internet = detect_internet();
    let local_runtimes = detect_local_runtimes();

    let profile = HardwareProfile {
        gpus,
        ram_mb,
        cpu_cores,
        disk_free_gb,
        on_battery,
        has_internet,
        local_runtimes,
    };

    tracing::info!(
        gpus = profile.gpus.len(),
        vram_mb = profile.total_vram_mb(),
        ram_mb = profile.ram_mb,
        cpu_cores = profile.cpu_cores,
        disk_free_gb = profile.disk_free_gb,
        on_battery = profile.on_battery,
        has_internet = profile.has_internet,
        runtimes = profile.local_runtimes.len(),
        "hardware profile detected"
    );

    profile
}

/// Detect GPUs by parsing nvidia-smi output.
fn detect_gpus() -> Vec<GpuProfile> {
    let mut gpus = Vec::new();

    // Try nvidia-smi for NVIDIA GPUs
    if let Ok(output) = Command::new("nvidia-smi")
        .args(["--query-gpu=name,memory.total,driver_version", "--format=csv,noheader,nounits"])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
                if parts.len() >= 3 {
                    let model = parts[0].to_string();
                    let vram_mb = parts[1].parse::<u64>().unwrap_or(0);
                    let driver_version = parts[2].to_string();

                    gpus.push(GpuProfile {
                        vendor: "nvidia".to_string(),
                        model,
                        vram_mb,
                        is_dedicated: true,
                        driver_version,
                    });
                }
            }
        }
    }

    // On Linux, also try lspci for AMD/Intel GPUs if no NVIDIA found
    #[cfg(target_os = "linux")]
    if gpus.is_empty() {
        if let Ok(output) = Command::new("lspci").output() {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    let lower = line.to_lowercase();
                    if lower.contains("vga") || lower.contains("3d") || lower.contains("display") {
                        let (vendor, is_dedicated) = if lower.contains("nvidia") {
                            ("nvidia".to_string(), true)
                        } else if lower.contains("amd") || lower.contains("radeon") {
                            ("amd".to_string(), true)
                        } else if lower.contains("intel") {
                            ("intel".to_string(), false) // Intel iGPU
                        } else {
                            ("unknown".to_string(), false)
                        };

                        // Extract model from lspci line (rough)
                        let model = line
                            .split(':')
                            .last()
                            .unwrap_or(line)
                            .trim()
                            .to_string();

                        gpus.push(GpuProfile {
                            vendor,
                            model,
                            vram_mb: 0, // Can't reliably get VRAM from lspci
                            is_dedicated,
                            driver_version: String::new(),
                        });
                    }
                }
            }
        }
    }

    // On Windows, try WMIC for non-NVIDIA GPUs if nvidia-smi didn't find any
    #[cfg(target_os = "windows")]
    if gpus.is_empty() {
        if let Ok(output) = Command::new("wmic")
            .args(["path", "win32_VideoController", "get", "Name,AdapterRAM,DriverVersion", "/format:csv"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines().skip(1) {
                    let parts: Vec<&str> = line.split(',').collect();
                    if parts.len() >= 4 {
                        let adapter_ram = parts[1].trim().parse::<u64>().unwrap_or(0);
                        let driver_version = parts[2].trim().to_string();
                        let name = parts[3].trim().to_string();

                        if name.is_empty() { continue; }

                        let lower = name.to_lowercase();
                        let (vendor, is_dedicated) = if lower.contains("nvidia") {
                            ("nvidia".to_string(), true)
                        } else if lower.contains("amd") || lower.contains("radeon") {
                            ("amd".to_string(), true)
                        } else if lower.contains("intel") {
                            ("intel".to_string(), false)
                        } else {
                            ("unknown".to_string(), false)
                        };

                        gpus.push(GpuProfile {
                            vendor,
                            model: name,
                            vram_mb: adapter_ram / (1024 * 1024),
                            is_dedicated,
                            driver_version,
                        });
                    }
                }
            }
        }
    }

    gpus
}

/// Detect RAM and CPU cores using sysinfo.
fn detect_cpu_ram() -> (u64, usize) {
    use sysinfo::System;

    let mut sys = System::new();
    sys.refresh_memory();
    sys.refresh_cpu_all();

    let ram_mb = sys.total_memory() / (1024 * 1024);
    let cpu_cores = sys.cpus().len();

    (ram_mb, cpu_cores)
}

/// Detect free disk space on the primary partition.
fn detect_disk_free() -> u64 {
    use sysinfo::Disks;

    let disks = Disks::new_with_refreshed_list();

    // Find the largest/primary disk
    let mut max_free = 0u64;
    for disk in disks.list() {
        let free_gb = disk.available_space() / (1024 * 1024 * 1024);
        if free_gb > max_free {
            max_free = free_gb;
        }
    }

    max_free
}

/// Detect whether the system is on battery power.
fn detect_battery() -> bool {
    // On Linux, check /sys/class/power_supply/
    #[cfg(target_os = "linux")]
    {
        if let Ok(entries) = std::fs::read_dir("/sys/class/power_supply") {
            for entry in entries.flatten() {
                let status_path = entry.path().join("status");
                if let Ok(status) = std::fs::read_to_string(&status_path) {
                    let status = status.trim().to_lowercase();
                    if status == "discharging" {
                        return true;
                    }
                }
            }
        }
        false
    }

    // On Windows, use WMIC
    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = Command::new("wmic")
            .args(["path", "Win32_Battery", "get", "BatteryStatus", "/format:value"])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // BatteryStatus=1 means discharging
            if stdout.contains("BatteryStatus=1") {
                return true;
            }
        }
        false
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        false
    }
}

/// Check internet connectivity with a 500ms timeout.
fn detect_internet() -> bool {
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration;

    // Try connecting to Cloudflare DNS (1.1.1.1:443)
    if let Ok(mut addrs) = "1.1.1.1:443".to_socket_addrs() {
        if let Some(addr) = addrs.next() {
            if TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok() {
                return true;
            }
        }
    }

    // Fallback: try Google DNS
    if let Ok(mut addrs) = "8.8.8.8:443".to_socket_addrs() {
        if let Some(addr) = addrs.next() {
            if TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok() {
                return true;
            }
        }
    }

    false
}

/// Detect locally-running AI inference runtimes.
fn detect_local_runtimes() -> Vec<LocalRuntime> {
    let mut runtimes = Vec::new();

    // Probe Ollama (localhost:11434)
    if let Some(runtime) = probe_ollama("http://localhost:11434") {
        runtimes.push(runtime);
    }

    // Probe common llama-server ports
    for port in [8080, 8341, 8000] {
        if let Some(runtime) = probe_llama_server(&format!("http://localhost:{}", port)) {
            runtimes.push(runtime);
        }
    }

    runtimes
}

/// Probe an Ollama instance at the given URL.
fn probe_ollama(base_url: &str) -> Option<LocalRuntime> {
    let url = format!("{}/api/tags", base_url);

    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(std::time::Duration::from_millis(500)))
            .build(),
    );

    let resp = agent.get(&url).call().ok()?;
    let json: serde_json::Value = resp.into_body().read_json().ok()?;

    let models = json["models"]
        .as_array()?
        .iter()
        .filter_map(|m| m["name"].as_str().map(String::from))
        .collect();

    Some(LocalRuntime {
        kind: RuntimeKind::Ollama,
        url: base_url.to_string(),
        models,
    })
}

/// Probe a llama-server instance at the given URL.
fn probe_llama_server(base_url: &str) -> Option<LocalRuntime> {
    let url = format!("{}/health", base_url);

    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(std::time::Duration::from_millis(500)))
            .build(),
    );

    let resp = agent.get(&url).call().ok()?;

    // llama-server /health returns {"status":"ok"} or similar
    let json: serde_json::Value = resp.into_body().read_json().ok()?;
    let status = json["status"].as_str().unwrap_or("");
    if status != "ok" && status != "no slot available" {
        // "no slot available" still means the server is running
        return None;
    }

    // Try to get model info from /v1/models if available
    let models_url = format!("{}/v1/models", base_url);
    let mut models = Vec::new();
    if let Ok(resp) = agent.get(&models_url).call() {
        if let Ok(json) = resp.into_body().read_json::<serde_json::Value>() {
            if let Some(data) = json["data"].as_array() {
                models = data
                    .iter()
                    .filter_map(|m| m["id"].as_str().map(String::from))
                    .collect();
            }
        }
    }

    Some(LocalRuntime {
        kind: RuntimeKind::LlamaServer,
        url: base_url.to_string(),
        models,
    })
}

// ── Recommendation Engine ────────────────────────────────────────────────

/// Recommended setup mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SetupMode {
    /// Full local inference — GPU with sufficient VRAM.
    Local,
    /// Mix of local (fast/small) and cloud (powerful) models.
    Hybrid,
    /// Primarily cloud-based inference.
    Cloud,
    /// CPU-only local fallback (slow but functional).
    CPUFallback,
}

impl std::fmt::Display for SetupMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SetupMode::Local => write!(f, "Local"),
            SetupMode::Hybrid => write!(f, "Hybrid"),
            SetupMode::Cloud => write!(f, "Cloud"),
            SetupMode::CPUFallback => write!(f, "CPU Fallback"),
        }
    }
}

/// A setup recommendation based on hardware detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupRecommendation {
    /// Recommended setup mode.
    pub mode: SetupMode,
    /// Confidence score (0.0 to 1.0).
    pub confidence: f32,
    /// Human-readable reasons for this recommendation.
    pub reasons: Vec<String>,
    /// Recommended model (if a specific model is suggested).
    pub model_recommendation: Option<String>,
    /// Detailed scores for each mode (for debugging/UI).
    pub scores: ModeScores,
}

/// Raw scores for each setup mode.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModeScores {
    pub local: f32,
    pub hybrid: f32,
    pub cloud: f32,
    pub cpu_fallback: f32,
}

/// Generate a setup recommendation from a hardware profile.
///
/// Uses deterministic scoring:
/// - Local: VRAM >= 8GB (+3), RAM >= 16GB (+2), Ollama detected (+3), on_battery (-2)
/// - Cloud: internet (+3), weak local hardware (+3)
/// - Hybrid: moderate hardware + internet
/// - CPUFallback: no GPU, no internet
pub fn recommend_setup(hw: &HardwareProfile) -> SetupRecommendation {
    let mut local_score: f32 = 0.0;
    let mut cloud_score: f32 = 0.0;
    let mut hybrid_score: f32 = 0.0;
    let mut reasons = Vec::new();

    let total_vram = hw.total_vram_mb();

    // ── GPU scoring ──
    if total_vram >= 24_000 {
        local_score += 5.0;
        reasons.push(format!("Excellent GPU: {}MB VRAM — can run large models locally", total_vram));
    } else if total_vram >= 8_000 {
        local_score += 3.0;
        hybrid_score += 1.0;
        reasons.push(format!("Good GPU: {}MB VRAM — suitable for medium models locally", total_vram));
    } else if total_vram >= 4_000 {
        local_score += 1.0;
        hybrid_score += 2.0;
        reasons.push(format!("Modest GPU: {}MB VRAM — small models locally, large models via cloud", total_vram));
    } else if total_vram > 0 {
        hybrid_score += 1.0;
        cloud_score += 1.0;
        reasons.push(format!("Limited GPU: {}MB VRAM — cloud recommended for best performance", total_vram));
    } else {
        cloud_score += 2.0;
        reasons.push("No dedicated GPU detected — cloud or CPU fallback".to_string());
    }

    // ── RAM scoring ──
    if hw.ram_mb >= 32_000 {
        local_score += 2.0;
        reasons.push(format!("Plenty of RAM: {}GB", hw.ram_mb / 1024));
    } else if hw.ram_mb >= 16_000 {
        local_score += 1.0;
        reasons.push(format!("Adequate RAM: {}GB", hw.ram_mb / 1024));
    } else {
        cloud_score += 1.0;
        reasons.push(format!("Limited RAM: {}GB — cloud may provide better experience", hw.ram_mb / 1024));
    }

    // ── Local runtime scoring ──
    if hw.has_ollama() {
        local_score += 3.0;
        let model_count = hw.local_runtimes
            .iter()
            .filter(|r| r.kind == RuntimeKind::Ollama)
            .flat_map(|r| &r.models)
            .count();
        reasons.push(format!("Ollama detected with {} model(s) — ready for local inference", model_count));
    }
    if hw.local_runtimes.iter().any(|r| r.kind == RuntimeKind::LlamaServer) {
        local_score += 2.0;
        reasons.push("llama-server detected — local inference available".to_string());
    }

    // ── Network scoring ──
    if hw.has_internet {
        cloud_score += 3.0;
        hybrid_score += 2.0;
        reasons.push("Internet connectivity confirmed".to_string());
    } else {
        cloud_score -= 10.0; // Can't do cloud without internet
        hybrid_score -= 5.0;
        reasons.push("No internet — local-only or CPU fallback".to_string());
    }

    // ── Battery scoring ──
    if hw.on_battery {
        local_score -= 2.0;
        cloud_score += 1.0;
        reasons.push("Running on battery — cloud saves power".to_string());
    }

    // ── Disk scoring ──
    if hw.disk_free_gb < 5 {
        local_score -= 2.0;
        reasons.push(format!("Low disk space: {}GB free — may not fit local models", hw.disk_free_gb));
    }

    // ── Hybrid is the average of local and cloud ──
    hybrid_score += (local_score + cloud_score) / 3.0;

    // ── CPU fallback score ──
    let cpu_fallback_score = if local_score <= 0.0 && cloud_score <= 0.0 {
        3.0
    } else if !hw.has_internet && total_vram == 0 {
        5.0
    } else {
        -1.0
    };

    // ── Pick the best mode ──
    let scores = ModeScores {
        local: local_score,
        hybrid: hybrid_score,
        cloud: cloud_score,
        cpu_fallback: cpu_fallback_score,
    };

    let (mode, best_score) = [
        (SetupMode::Local, local_score),
        (SetupMode::Hybrid, hybrid_score),
        (SetupMode::Cloud, cloud_score),
        (SetupMode::CPUFallback, cpu_fallback_score),
    ]
    .into_iter()
    .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    .unwrap_or((SetupMode::Cloud, 0.0));

    // Confidence: normalize best score relative to total
    let total = local_score.abs() + hybrid_score.abs() + cloud_score.abs() + cpu_fallback_score.abs();
    let confidence = if total > 0.0 {
        (best_score / total).clamp(0.0, 1.0)
    } else {
        0.5
    };

    // ── Model recommendation ──
    let model_recommendation = recommend_model(hw, mode);

    SetupRecommendation {
        mode,
        confidence,
        reasons,
        model_recommendation,
        scores,
    }
}

/// Recommend a specific model based on hardware and setup mode.
fn recommend_model(hw: &HardwareProfile, mode: SetupMode) -> Option<String> {
    let total_vram = hw.total_vram_mb();

    match mode {
        SetupMode::Local | SetupMode::Hybrid => {
            if total_vram >= 48_000 {
                Some("qwen3.5:27b-nothink".to_string())
            } else if total_vram >= 24_000 {
                Some("qwen3.5:14b-nothink".to_string())
            } else if total_vram >= 8_000 {
                Some("qwen3.5:7b-nothink".to_string())
            } else if total_vram >= 4_000 {
                Some("qwen3.5:4b-nothink".to_string())
            } else if hw.ram_mb >= 16_000 {
                // CPU inference with enough RAM
                Some("qwen3.5:4b-nothink".to_string())
            } else {
                Some("qwen3.5:0.8b-nothink".to_string())
            }
        }
        SetupMode::Cloud => {
            // Suggest a cloud model
            Some("gpt-4o-mini".to_string())
        }
        SetupMode::CPUFallback => {
            Some("qwen3.5:0.8b-nothink".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_profile(vram_mb: u64, ram_mb: u64, has_ollama: bool, has_internet: bool) -> HardwareProfile {
        let mut runtimes = Vec::new();
        if has_ollama {
            runtimes.push(LocalRuntime {
                kind: RuntimeKind::Ollama,
                url: "http://localhost:11434".to_string(),
                models: vec!["qwen3.5:7b-nothink".to_string()],
            });
        }

        HardwareProfile {
            gpus: if vram_mb > 0 {
                vec![GpuProfile {
                    vendor: "nvidia".to_string(),
                    model: "Test GPU".to_string(),
                    vram_mb,
                    is_dedicated: true,
                    driver_version: "999.0".to_string(),
                }]
            } else {
                Vec::new()
            },
            ram_mb,
            cpu_cores: 8,
            disk_free_gb: 100,
            on_battery: false,
            has_internet,
            local_runtimes: runtimes,
        }
    }

    #[test]
    fn test_powerful_local_recommends_local() {
        let hw = make_profile(24_000, 32_000, true, true);
        let rec = recommend_setup(&hw);
        assert_eq!(rec.mode, SetupMode::Local);
        assert!(rec.confidence > 0.3);
    }

    #[test]
    fn test_no_gpu_no_internet_recommends_cpu_fallback() {
        let hw = make_profile(0, 8_000, false, false);
        let rec = recommend_setup(&hw);
        assert_eq!(rec.mode, SetupMode::CPUFallback);
    }

    #[test]
    fn test_no_gpu_with_internet_recommends_cloud() {
        let hw = make_profile(0, 8_000, false, true);
        let rec = recommend_setup(&hw);
        assert_eq!(rec.mode, SetupMode::Cloud);
    }

    #[test]
    fn test_model_recommendation_large_vram() {
        let hw = make_profile(48_000, 64_000, true, true);
        let rec = recommend_setup(&hw);
        assert_eq!(rec.model_recommendation, Some("qwen3.5:27b-nothink".to_string()));
    }

    #[test]
    fn test_model_recommendation_medium_vram() {
        let hw = make_profile(8_000, 16_000, true, true);
        let rec = recommend_setup(&hw);
        assert_eq!(rec.model_recommendation, Some("qwen3.5:7b-nothink".to_string()));
    }

    #[test]
    fn test_total_vram() {
        let hw = HardwareProfile {
            gpus: vec![
                GpuProfile { vendor: "nvidia".to_string(), model: "RTX 3090".to_string(), vram_mb: 24_000, is_dedicated: true, driver_version: String::new() },
                GpuProfile { vendor: "nvidia".to_string(), model: "RTX 3090".to_string(), vram_mb: 24_000, is_dedicated: true, driver_version: String::new() },
            ],
            ram_mb: 64_000,
            cpu_cores: 16,
            disk_free_gb: 500,
            on_battery: false,
            has_internet: true,
            local_runtimes: Vec::new(),
        };
        assert_eq!(hw.total_vram_mb(), 48_000);
    }
}
