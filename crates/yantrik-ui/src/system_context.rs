//! System context — snapshot formatting for LLM, event→memory conversion, config loading.

use std::path::PathBuf;

/// Format a SystemSnapshot into a compact string for LLM context injection.
/// Kept short (~100 tokens) to fit the token budget.
pub fn format_system_context(snap: &yantrik_os::SystemSnapshot) -> String {
    let mut parts = Vec::new();

    // Battery
    let charge_str = if snap.battery_charging { " (charging)" } else { "" };
    parts.push(format!("Battery: {}%{}", snap.battery_level, charge_str));

    // Network — sanitize SSID (WiFi names are attacker-controlled in public spaces)
    if snap.network_connected {
        let raw_ssid = snap.network_ssid.as_deref().unwrap_or("connected");
        let safe_ssid: String = raw_ssid.chars().filter(|c| !c.is_control()).take(32).collect();
        parts.push(format!("WiFi: {}", safe_ssid));
    } else {
        parts.push("WiFi: disconnected".to_string());
    }

    // CPU & memory
    if snap.cpu_usage_percent > 0.0 {
        parts.push(format!("CPU: {:.0}%", snap.cpu_usage_percent));
    }
    if snap.memory_total_bytes > 0 {
        let used_mb = snap.memory_used_bytes / (1024 * 1024);
        let total_mb = snap.memory_total_bytes / (1024 * 1024);
        parts.push(format!("RAM: {}/{}MB ({:.0}%)", used_mb, total_mb, snap.memory_usage_percent()));
    }

    // Running processes (top 5 by name) — sanitize process names
    if !snap.running_processes.is_empty() {
        let names: Vec<String> = snap.running_processes.iter().take(5)
            .map(|p| p.name.chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == '.').take(30).collect())
            .collect();
        parts.push(format!("Apps: {}", names.join(", ")));
    }

    // User idle
    if snap.user_idle && snap.idle_seconds > 60 {
        parts.push(format!("User idle: {}m", snap.idle_seconds / 60));
    }

    parts.join("\n")
}

/// Convert a system event into a memory record (text, domain, importance).
/// Returns None for events that aren't worth remembering (routine resource polls).
pub fn event_to_memory(event: &yantrik_os::SystemEvent) -> Option<(String, String, f64)> {
    use yantrik_os::SystemEvent;
    match event {
        SystemEvent::BatteryChanged { level, charging, .. } => {
            if *charging {
                Some((
                    format!("Battery started charging at {}%", level),
                    "system/battery".into(),
                    0.3,
                ))
            } else if *level <= 20 {
                Some((
                    format!("Battery low at {}%", level),
                    "system/battery".into(),
                    0.6,
                ))
            } else {
                None
            }
        }
        SystemEvent::NetworkChanged { connected, ssid, .. } => {
            let text = if *connected {
                let safe_ssid = ssid.as_ref()
                    .map(|s| {
                        let clean: String = s.chars().filter(|c| !c.is_control()).take(32).collect();
                        format!(" '{}'", clean)
                    })
                    .unwrap_or_default();
                format!("Connected to network{}", safe_ssid)
            } else {
                "Network disconnected".into()
            };
            Some((text, "system/network".into(), 0.4))
        }
        SystemEvent::NotificationReceived { app, summary, .. } => {
            // Sanitize notification content — D-Bus notifications are untrusted external input.
            // Truncate and strip control chars to prevent injection via crafted notification.
            let safe_app: String = app.chars().filter(|c| !c.is_control()).take(50).collect();
            let safe_summary: String = summary.chars().filter(|c| !c.is_control()).take(100).collect();
            Some((
                format!("Notification from {}: {}", safe_app, safe_summary),
                "system/notification".into(),
                0.5,
            ))
        }
        SystemEvent::FileChanged { path, kind } => {
            // Truncate file paths to prevent oversized memory entries
            let safe_path: String = path.chars().take(200).collect();
            let action = match kind {
                yantrik_os::FileChangeKind::Created => "created",
                yantrik_os::FileChangeKind::Modified => "modified",
                yantrik_os::FileChangeKind::Deleted => "deleted",
                yantrik_os::FileChangeKind::Renamed { to } => {
                    let safe_to: String = to.chars().take(200).collect();
                    return Some((
                        format!("File renamed: {} → {}", safe_path, safe_to),
                        "system/files".into(),
                        0.3,
                    ));
                }
            };
            Some((
                format!("File {}: {}", action, safe_path),
                "system/files".into(),
                0.3,
            ))
        }
        SystemEvent::ProcessStarted { name, .. } => {
            let safe_name: String = name.chars().filter(|c| !c.is_control()).take(50).collect();
            Some((
                format!("App opened: {}", safe_name),
                "system/process".into(),
                0.2,
            ))
        }
        SystemEvent::ProcessStopped { name, .. } => {
            let safe_name: String = name.chars().filter(|c| !c.is_control()).take(50).collect();
            Some((
                format!("App closed: {}", safe_name),
                "system/process".into(),
                0.2,
            ))
        }
        SystemEvent::UserIdle { idle_seconds } if *idle_seconds > 300 => {
            Some((
                format!("User idle for {} minutes", idle_seconds / 60),
                "system/presence".into(),
                0.2,
            ))
        }
        SystemEvent::UserResumed => {
            Some((
                "User returned".into(),
                "system/presence".into(),
                0.3,
            ))
        }
        _ => None,
    }
}

/// Load system observer config from the YAML file.
/// Falls back to defaults (mock mode) if not present.
pub fn load_system_config(path: Option<PathBuf>) -> yantrik_os::SystemObserverConfig {
    let Some(p) = path else {
        return yantrik_os::SystemObserverConfig {
            mock: true,
            ..Default::default()
        };
    };

    let contents = match std::fs::read_to_string(&p) {
        Ok(c) => c,
        Err(_) => {
            return yantrik_os::SystemObserverConfig {
                mock: true,
                ..Default::default()
            };
        }
    };

    let yaml: serde_yaml::Value = match serde_yaml::from_str(&contents) {
        Ok(v) => v,
        Err(_) => {
            return yantrik_os::SystemObserverConfig {
                mock: true,
                ..Default::default()
            };
        }
    };

    match yaml.get("system") {
        Some(sys_val) => {
            serde_yaml::from_value(sys_val.clone()).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "Invalid system config, using defaults");
                yantrik_os::SystemObserverConfig {
                    mock: true,
                    ..Default::default()
                }
            })
        }
        None => {
            tracing::info!("No 'system' section in config, using mock mode");
            yantrik_os::SystemObserverConfig {
                mock: true,
                ..Default::default()
            }
        }
    }
}
