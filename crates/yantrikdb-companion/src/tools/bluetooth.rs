//! Bluetooth tools — bluetooth_scan, bluetooth_pair, bluetooth_connect, bluetooth_disconnect.
//! Uses `bluetoothctl` (BlueZ).

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(BluetoothScanTool));
    reg.register(Box::new(BluetoothPairTool));
    reg.register(Box::new(BluetoothConnectTool));
    reg.register(Box::new(BluetoothDisconnectTool));
    reg.register(Box::new(BluetoothInfoTool));
}

/// Run a bluetoothctl command with timeout.
fn btctl(args: &[&str]) -> Result<String, String> {
    match std::process::Command::new("bluetoothctl")
        .args(args)
        .output()
    {
        Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).to_string()),
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            let out = String::from_utf8_lossy(&o.stdout);
            Err(format!("{} {}", out.trim(), err.trim()))
        }
        Err(e) => Err(format!("bluetoothctl not available: {e}")),
    }
}

/// Validate a Bluetooth MAC address format (XX:XX:XX:XX:XX:XX).
fn valid_mac(addr: &str) -> bool {
    let parts: Vec<&str> = addr.split(':').collect();
    parts.len() == 6 && parts.iter().all(|p| {
        p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit())
    })
}

// ── Bluetooth Scan ──

pub struct BluetoothScanTool;

impl Tool for BluetoothScanTool {
    fn name(&self) -> &'static str { "bluetooth_scan" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "bluetooth" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "bluetooth_scan",
                "description": "Scan for nearby Bluetooth devices. Returns device names and MAC addresses.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "duration": {
                            "type": "integer",
                            "description": "Scan duration in seconds (default: 5, max: 15)"
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let duration = args.get("duration").and_then(|v| v.as_u64()).unwrap_or(5).min(15);

        // Power on adapter
        let _ = btctl(&["power", "on"]);

        // Start scan
        let _ = btctl(&["scan", "on"]);

        // Wait for scan
        std::thread::sleep(std::time::Duration::from_secs(duration));

        // Stop scan
        let _ = btctl(&["scan", "off"]);

        // List devices
        match btctl(&["devices"]) {
            Ok(output) => {
                let devices: Vec<&str> = output.lines()
                    .filter(|l| l.contains("Device"))
                    .collect();
                if devices.is_empty() {
                    "No Bluetooth devices found.".to_string()
                } else {
                    let mut result = format!("Found {} device(s):\n", devices.len());
                    for d in devices.iter().take(20) {
                        result.push_str(&format!("  {}\n", d));
                    }
                    result.trim().to_string()
                }
            }
            Err(e) => format!("Scan failed: {e}"),
        }
    }
}

// ── Bluetooth Pair ──

pub struct BluetoothPairTool;

impl Tool for BluetoothPairTool {
    fn name(&self) -> &'static str { "bluetooth_pair" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "bluetooth" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "bluetooth_pair",
                "description": "Pair with a Bluetooth device by MAC address.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "address": {"type": "string", "description": "Device MAC address (XX:XX:XX:XX:XX:XX)"}
                    },
                    "required": ["address"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let address = args.get("address").and_then(|v| v.as_str()).unwrap_or_default();
        if !valid_mac(address) {
            return "Error: invalid MAC address format (XX:XX:XX:XX:XX:XX)".to_string();
        }

        let _ = btctl(&["power", "on"]);

        match btctl(&["pair", address]) {
            Ok(output) => {
                if output.contains("Pairing successful") || output.contains("already paired") {
                    format!("Paired with {address}")
                } else {
                    format!("Pairing response: {}", output.trim())
                }
            }
            Err(e) => format!("Pairing failed: {e}"),
        }
    }
}

// ── Bluetooth Connect ──

pub struct BluetoothConnectTool;

impl Tool for BluetoothConnectTool {
    fn name(&self) -> &'static str { "bluetooth_connect" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "bluetooth" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "bluetooth_connect",
                "description": "Connect to a paired Bluetooth device.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "address": {"type": "string", "description": "Device MAC address (XX:XX:XX:XX:XX:XX)"}
                    },
                    "required": ["address"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let address = args.get("address").and_then(|v| v.as_str()).unwrap_or_default();
        if !valid_mac(address) {
            return "Error: invalid MAC address format".to_string();
        }

        match btctl(&["connect", address]) {
            Ok(output) => {
                if output.contains("Connection successful") {
                    format!("Connected to {address}")
                } else {
                    format!("Connect response: {}", output.trim())
                }
            }
            Err(e) => format!("Connection failed: {e}"),
        }
    }
}

// ── Bluetooth Disconnect ──

pub struct BluetoothDisconnectTool;

impl Tool for BluetoothDisconnectTool {
    fn name(&self) -> &'static str { "bluetooth_disconnect" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "bluetooth" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "bluetooth_disconnect",
                "description": "Disconnect a Bluetooth device.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "address": {"type": "string", "description": "Device MAC address (XX:XX:XX:XX:XX:XX)"}
                    },
                    "required": ["address"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let address = args.get("address").and_then(|v| v.as_str()).unwrap_or_default();
        if !valid_mac(address) {
            return "Error: invalid MAC address format".to_string();
        }

        match btctl(&["disconnect", address]) {
            Ok(_) => format!("Disconnected from {address}"),
            Err(e) => format!("Disconnect failed: {e}"),
        }
    }
}

// ── Bluetooth Info ──

pub struct BluetoothInfoTool;

impl Tool for BluetoothInfoTool {
    fn name(&self) -> &'static str { "bluetooth_info" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "bluetooth" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "bluetooth_info",
                "description": "Get Bluetooth adapter status and paired/connected devices.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let mut info = Vec::new();

        // Adapter info
        if let Ok(output) = btctl(&["show"]) {
            for line in output.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("Name:") || trimmed.starts_with("Powered:")
                    || trimmed.starts_with("Discoverable:") || trimmed.starts_with("Pairable:")
                {
                    info.push(trimmed.to_string());
                }
            }
        }

        // Paired devices
        if let Ok(output) = btctl(&["paired-devices"]) {
            let devices: Vec<&str> = output.lines().filter(|l| !l.is_empty()).collect();
            if !devices.is_empty() {
                info.push(format!("\nPaired devices ({}):", devices.len()));
                for d in devices.iter().take(10) {
                    info.push(format!("  {d}"));
                }
            }
        }

        if info.is_empty() {
            "Bluetooth info unavailable (bluetoothctl not found).".to_string()
        } else {
            info.join("\n")
        }
    }
}
