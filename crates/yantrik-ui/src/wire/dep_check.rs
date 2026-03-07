//! Dependency checker — detect missing optional CLI tools at startup.
//!
//! Provides `has_command()` for quick checks and `check_all()` for a startup
//! summary of which optional tools are available.

use std::collections::HashMap;
use std::sync::OnceLock;

/// Cached results of command availability checks.
static AVAILABLE: OnceLock<HashMap<&'static str, bool>> = OnceLock::new();

/// Optional CLI tools that Yantrik apps may use.
const OPTIONAL_DEPS: &[(&str, &str)] = &[
    ("mpv", "Media Player, Music Player"),
    ("nmcli", "Network Manager, WiFi toggle"),
    ("brightnessctl", "Brightness control"),
    ("amixer", "Volume control"),
    ("chromium", "Browser-based apps"),
    ("firefox-esr", "Browser"),
    ("docker", "Container Manager"),
    ("podman", "Container Manager"),
    ("grim", "Screenshots"),
    ("slurp", "Region screenshots"),
    ("wl-copy", "Clipboard"),
    ("wlrctl", "Window management"),
    ("foot", "Terminal"),
    ("curl", "Weather, Downloads"),
];

/// Check if a command is available on the system (cached).
pub fn has_command(cmd: &str) -> bool {
    let map = AVAILABLE.get_or_init(|| {
        let mut m = HashMap::new();
        for (name, _) in OPTIONAL_DEPS {
            m.insert(*name, command_exists(name));
        }
        m
    });
    map.get(cmd).copied().unwrap_or_else(|| command_exists(cmd))
}

fn command_exists(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Log a startup summary of available/missing optional tools.
pub fn log_dep_summary() {
    let mut missing = Vec::new();
    let mut present = 0;

    for (cmd, usage) in OPTIONAL_DEPS {
        if has_command(cmd) {
            present += 1;
        } else {
            missing.push((*cmd, *usage));
        }
    }

    tracing::info!(
        available = present,
        missing = missing.len(),
        "Optional dependency check"
    );
    for (cmd, usage) in &missing {
        tracing::debug!(cmd, usage, "Optional tool not found");
    }
}

/// Get a user-friendly message for a missing dependency.
pub fn missing_msg(cmd: &str) -> String {
    format!("Install '{}' to use this feature (apk add {})", cmd, cmd)
}
