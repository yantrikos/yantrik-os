//! WiFi tools — wifi_scan, wifi_connect, wifi_status.
//! Uses `nmcli` (NetworkManager) or `iwctl` (iwd) depending on what's available.
//! Falls back to `wpa_cli` for Alpine minimal installs.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(WifiScanTool));
    reg.register(Box::new(WifiConnectTool));
    reg.register(Box::new(WifiStatusTool));
    reg.register(Box::new(WifiDisconnectTool));
}

/// Detect which WiFi manager is available.
fn wifi_backend() -> &'static str {
    if std::process::Command::new("nmcli").arg("--version").output().map(|o| o.status.success()).unwrap_or(false) {
        "nmcli"
    } else if std::process::Command::new("iwctl").arg("--version").output().map(|o| o.status.success()).unwrap_or(false) {
        "iwctl"
    } else {
        "wpa_cli"
    }
}

// ── WiFi Scan ──

pub struct WifiScanTool;

impl Tool for WifiScanTool {
    fn name(&self) -> &'static str { "wifi_scan" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "wifi" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "wifi_scan",
                "description": "Scan for nearby Wi-Fi networks",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        match wifi_backend() {
            "nmcli" => {
                // Trigger rescan then list
                let _ = std::process::Command::new("nmcli")
                    .args(["device", "wifi", "rescan"])
                    .output();
                match std::process::Command::new("nmcli")
                    .args(["-t", "-f", "SSID,SIGNAL,SECURITY", "device", "wifi", "list"])
                    .output()
                {
                    Ok(o) if o.status.success() => {
                        let text = String::from_utf8_lossy(&o.stdout);
                        let mut networks = Vec::new();
                        for line in text.lines() {
                            let parts: Vec<&str> = line.split(':').collect();
                            if parts.len() >= 3 && !parts[0].is_empty() {
                                networks.push(format!(
                                    "  {} ({}% signal, {})",
                                    parts[0], parts[1], parts[2]
                                ));
                            }
                        }
                        if networks.is_empty() {
                            "No WiFi networks found.".to_string()
                        } else {
                            // Deduplicate SSIDs
                            networks.dedup();
                            networks.truncate(20);
                            format!("Available networks:\n{}", networks.join("\n"))
                        }
                    }
                    Ok(o) => format!("Scan failed: {}", String::from_utf8_lossy(&o.stderr)),
                    Err(e) => format!("Error: {e}"),
                }
            }
            "iwctl" => {
                let _ = std::process::Command::new("iwctl")
                    .args(["station", "wlan0", "scan"])
                    .output();
                match std::process::Command::new("iwctl")
                    .args(["station", "wlan0", "get-networks"])
                    .output()
                {
                    Ok(o) if o.status.success() => {
                        let text = String::from_utf8_lossy(&o.stdout);
                        if text.trim().is_empty() {
                            "No WiFi networks found.".to_string()
                        } else {
                            let trunc = if text.len() > 2000 { &text[..text.floor_char_boundary(2000)] } else { &text };
                            trunc.to_string()
                        }
                    }
                    Ok(o) => format!("Scan failed: {}", String::from_utf8_lossy(&o.stderr)),
                    Err(e) => format!("Error: {e}"),
                }
            }
            _ => {
                // wpa_cli
                let _ = std::process::Command::new("wpa_cli")
                    .args(["-i", "wlan0", "scan"])
                    .output();
                std::thread::sleep(std::time::Duration::from_secs(2));
                match std::process::Command::new("wpa_cli")
                    .args(["-i", "wlan0", "scan_results"])
                    .output()
                {
                    Ok(o) if o.status.success() => {
                        let text = String::from_utf8_lossy(&o.stdout);
                        if text.trim().is_empty() {
                            "No WiFi networks found.".to_string()
                        } else {
                            let trunc = if text.len() > 2000 { &text[..text.floor_char_boundary(2000)] } else { &text };
                            trunc.to_string()
                        }
                    }
                    Ok(o) => format!("Scan failed: {}", String::from_utf8_lossy(&o.stderr)),
                    Err(e) => format!("Error: {e}"),
                }
            }
        }
    }
}

// ── WiFi Connect ──

pub struct WifiConnectTool;

impl Tool for WifiConnectTool {
    fn name(&self) -> &'static str { "wifi_connect" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "wifi" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "wifi_connect",
                "description": "Connect to Wi-Fi by SSID",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "ssid": {"type": "string", "description": "Network name (SSID)"},
                        "password": {"type": "string", "description": "Network password (WPA/WPA2)"}
                    },
                    "required": ["ssid"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let ssid = args.get("ssid").and_then(|v| v.as_str()).unwrap_or_default();
        let password = args.get("password").and_then(|v| v.as_str()).unwrap_or("");

        if ssid.is_empty() {
            return "Error: ssid is required".to_string();
        }

        // Validate no shell metacharacters in SSID/password
        if ssid.contains(|c: char| c == '`' || c == '$' || c == ';' || c == '|' || c == '&') {
            return "Error: SSID contains invalid characters".to_string();
        }

        match wifi_backend() {
            "nmcli" => {
                let mut cmd = std::process::Command::new("nmcli");
                cmd.args(["device", "wifi", "connect", ssid]);
                if !password.is_empty() {
                    cmd.args(["password", password]);
                }
                match cmd.output() {
                    Ok(o) if o.status.success() => {
                        format!("Connected to '{ssid}'")
                    }
                    Ok(o) => {
                        let err = String::from_utf8_lossy(&o.stderr);
                        let out = String::from_utf8_lossy(&o.stdout);
                        format!("Failed to connect: {} {}", out.trim(), err.trim())
                    }
                    Err(e) => format!("Error: {e}"),
                }
            }
            "iwctl" => {
                let mut cmd = std::process::Command::new("iwctl");
                if password.is_empty() {
                    cmd.args(["station", "wlan0", "connect", ssid]);
                } else {
                    cmd.args(["--passphrase", password, "station", "wlan0", "connect", ssid]);
                }
                match cmd.output() {
                    Ok(o) if o.status.success() => format!("Connected to '{ssid}'"),
                    Ok(o) => format!("Failed: {}", String::from_utf8_lossy(&o.stderr)),
                    Err(e) => format!("Error: {e}"),
                }
            }
            _ => "Error: WiFi connection requires nmcli or iwctl. Use wpa_supplicant config for manual setup.".to_string(),
        }
    }
}

// ── WiFi Status ──

pub struct WifiStatusTool;

impl Tool for WifiStatusTool {
    fn name(&self) -> &'static str { "wifi_status" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "wifi" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "wifi_status",
                "description": "Show current Wi-Fi connection details",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let mut info = Vec::new();

        // Try nmcli first
        if let Ok(o) = std::process::Command::new("nmcli")
            .args(["-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device", "status"])
            .output()
        {
            if o.status.success() {
                let text = String::from_utf8_lossy(&o.stdout);
                for line in text.lines() {
                    if line.contains("wifi") {
                        info.push(format!("WiFi: {}", line.replace(':', " | ")));
                    }
                }
            }
        }

        // IP address
        if let Ok(o) = std::process::Command::new("sh")
            .arg("-c")
            .arg("ip -4 addr show scope global | grep inet")
            .output()
        {
            if o.status.success() {
                let text = String::from_utf8_lossy(&o.stdout);
                for line in text.lines() {
                    let trimmed = line.trim();
                    info.push(format!("IP: {trimmed}"));
                }
            }
        }

        // Signal strength via /proc/net/wireless
        if let Ok(content) = std::fs::read_to_string("/proc/net/wireless") {
            for line in content.lines().skip(2) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 {
                    let iface = parts[0].trim_end_matches(':');
                    let quality = parts[2].trim_end_matches('.');
                    info.push(format!("Signal ({iface}): quality {quality}"));
                }
            }
        }

        if info.is_empty() {
            "WiFi status unavailable.".to_string()
        } else {
            info.join("\n")
        }
    }
}

// ── WiFi Disconnect ──

pub struct WifiDisconnectTool;

impl Tool for WifiDisconnectTool {
    fn name(&self) -> &'static str { "wifi_disconnect" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "wifi" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "wifi_disconnect",
                "description": "Disconnect current Wi-Fi",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        match wifi_backend() {
            "nmcli" => {
                match std::process::Command::new("nmcli")
                    .args(["device", "disconnect", "wlan0"])
                    .output()
                {
                    Ok(o) if o.status.success() => "WiFi disconnected.".to_string(),
                    Ok(o) => format!("Failed: {}", String::from_utf8_lossy(&o.stderr)),
                    Err(e) => format!("Error: {e}"),
                }
            }
            "iwctl" => {
                match std::process::Command::new("iwctl")
                    .args(["station", "wlan0", "disconnect"])
                    .output()
                {
                    Ok(o) if o.status.success() => "WiFi disconnected.".to_string(),
                    Ok(o) => format!("Failed: {}", String::from_utf8_lossy(&o.stderr)),
                    Err(e) => format!("Error: {e}"),
                }
            }
            _ => "Error: WiFi disconnect requires nmcli or iwctl.".to_string(),
        }
    }
}
