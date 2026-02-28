//! System tools — kill_process, send_notification, system_control.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(KillProcessTool));
    reg.register(Box::new(SendNotificationTool));
    reg.register(Box::new(SystemControlTool));
}

/// Processes that must never be killed by the AI.
const PROTECTED_PROCESSES: &[&str] = &[
    "init", "labwc", "dbus-daemon", "dbus-launch",
    "pipewire", "wireplumber", "seatd",
    "yantrik-ui", "yantrik",
];

// ── Kill Process ──

pub struct KillProcessTool;

impl Tool for KillProcessTool {
    fn name(&self) -> &'static str { "kill_process" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "system" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "kill_process",
                "description": "Kill a running process by name or PID.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "target": {"type": "string", "description": "Process name or PID"}
                    },
                    "required": ["target"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let target = args.get("target").and_then(|v| v.as_str()).unwrap_or_default();
        if target.is_empty() {
            return "Error: target is required".to_string();
        }

        // Check protected list
        let target_lower = target.to_lowercase();
        for protected in PROTECTED_PROCESSES {
            if target_lower.contains(protected) {
                return format!("Error: '{target}' is a protected system process and cannot be killed");
            }
        }

        // Determine if target is a PID (numeric) or name
        if let Ok(pid) = target.parse::<u32>() {
            // Kill by PID (SIGTERM)
            match std::process::Command::new("kill")
                .arg(pid.to_string())
                .output()
            {
                Ok(output) if output.status.success() => {
                    format!("Sent SIGTERM to PID {pid}")
                }
                Ok(output) => {
                    let err = String::from_utf8_lossy(&output.stderr);
                    format!("Failed to kill PID {pid}: {err}")
                }
                Err(e) => format!("Error: {e}"),
            }
        } else {
            // Kill by name via pkill (SIGTERM)
            // Validate no metacharacters
            if target.contains(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != '.') {
                return "Error: invalid process name".to_string();
            }
            match std::process::Command::new("pkill")
                .arg(target)
                .output()
            {
                Ok(output) if output.status.success() => {
                    format!("Sent SIGTERM to process '{target}'")
                }
                Ok(_) => {
                    format!("No process matching '{target}' found")
                }
                Err(e) => format!("Error: {e}"),
            }
        }
    }
}

// ── Send Notification ──

pub struct SendNotificationTool;

impl Tool for SendNotificationTool {
    fn name(&self) -> &'static str { "send_notification" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "system" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "send_notification",
                "description": "Display a desktop notification to the user.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "title": {"type": "string", "description": "Notification title"},
                        "body": {"type": "string", "description": "Notification body"},
                        "urgency": {"type": "string", "enum": ["low", "normal", "critical"]}
                    },
                    "required": ["title", "body"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let title = args.get("title").and_then(|v| v.as_str()).unwrap_or_default();
        let body = args.get("body").and_then(|v| v.as_str()).unwrap_or_default();
        let urgency = args.get("urgency").and_then(|v| v.as_str()).unwrap_or("normal");

        if title.is_empty() || body.is_empty() {
            return "Error: title and body are required".to_string();
        }

        // Truncate to prevent abuse
        let title = if title.len() > 100 { &title[..100] } else { title };
        let body = if body.len() > 500 { &body[..500] } else { body };

        let mut cmd = std::process::Command::new("notify-send");
        cmd.arg("-u").arg(urgency);
        cmd.arg(title);
        cmd.arg(body);

        match cmd.output() {
            Ok(output) if output.status.success() => {
                format!("Notification sent: {title}")
            }
            Ok(output) => {
                let err = String::from_utf8_lossy(&output.stderr);
                format!("Notification failed: {err}")
            }
            Err(e) => format!("Error (notify-send not available?): {e}"),
        }
    }
}

// ── System Control ──

pub struct SystemControlTool;

impl Tool for SystemControlTool {
    fn name(&self) -> &'static str { "system_control" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "system" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "system_control",
                "description": "Adjust volume, brightness, or power state.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["volume_up", "volume_down", "volume_mute", "brightness_up", "brightness_down", "suspend", "shutdown"]
                        },
                        "value": {"type": "integer", "description": "Percentage for volume/brightness set"}
                    },
                    "required": ["action"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or_default();
        let value = args.get("value").and_then(|v| v.as_u64()).unwrap_or(10);

        match action {
            "volume_up" => run_wpctl(&format!("set-volume @DEFAULT_AUDIO_SINK@ {}%+", value.min(50))),
            "volume_down" => run_wpctl(&format!("set-volume @DEFAULT_AUDIO_SINK@ {}%-", value.min(50))),
            "volume_mute" => run_wpctl("set-mute @DEFAULT_AUDIO_SINK@ toggle"),
            "brightness_up" => adjust_brightness(value as i32),
            "brightness_down" => adjust_brightness(-(value as i32)),
            "suspend" => {
                "Warning: Suspend requested. Call system_control again with action=suspend to confirm.".to_string()
            }
            "shutdown" => {
                "Warning: Shutdown requested. This will power off the computer. Call system_control again with action=shutdown to confirm.".to_string()
            }
            _ => format!("Error: unknown action '{action}'"),
        }
    }
}

/// Run wpctl (PipeWire/WirePlumber volume control).
fn run_wpctl(args: &str) -> String {
    match std::process::Command::new("sh")
        .arg("-c")
        .arg(&format!("wpctl {args}"))
        .output()
    {
        Ok(output) if output.status.success() => "OK".to_string(),
        Ok(output) => {
            let err = String::from_utf8_lossy(&output.stderr);
            format!("wpctl failed: {err}")
        }
        Err(e) => format!("Error (wpctl not available?): {e}"),
    }
}

/// Adjust backlight brightness by a delta percentage.
fn adjust_brightness(delta: i32) -> String {
    // Read current brightness from sysfs
    let bl_dir = "/sys/class/backlight";
    let device = match std::fs::read_dir(bl_dir) {
        Ok(mut entries) => match entries.next() {
            Some(Ok(e)) => e.path(),
            _ => return "Error: no backlight device found".to_string(),
        },
        Err(_) => return "Error: /sys/class/backlight not accessible".to_string(),
    };

    let max_path = device.join("max_brightness");
    let cur_path = device.join("brightness");

    let max_br: i32 = match std::fs::read_to_string(&max_path) {
        Ok(s) => s.trim().parse().unwrap_or(100),
        Err(e) => return format!("Error reading max brightness: {e}"),
    };

    let cur_br: i32 = match std::fs::read_to_string(&cur_path) {
        Ok(s) => s.trim().parse().unwrap_or(50),
        Err(e) => return format!("Error reading current brightness: {e}"),
    };

    let step = max_br * delta / 100;
    let new_br = (cur_br + step).clamp(0, max_br);

    match std::fs::write(&cur_path, new_br.to_string()) {
        Ok(_) => {
            let pct = (new_br as f64 / max_br as f64 * 100.0) as i32;
            format!("Brightness: {pct}%")
        }
        Err(e) => format!("Error setting brightness: {e}"),
    }
}
