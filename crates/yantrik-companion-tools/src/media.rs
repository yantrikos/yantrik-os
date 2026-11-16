//! Media tools — screenshot, audio_control, audio_info.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(ScreenshotTool));
    reg.register(Box::new(AudioControlTool));
    reg.register(Box::new(AudioInfoTool));
}

// ── Screenshot ──

pub struct ScreenshotTool;

impl ScreenshotTool {
    /// `execute` with the desk to ask on, which a test points at its own sockets.
    pub(crate) fn run(&self, desk: &crate::app_ui::Desk, args: &serde_json::Value) -> String {
        crate::screen_read::take_picture(desk, args.get("region").and_then(|v| v.as_str()).unwrap_or_default())
    }
}

impl Tool for ScreenshotTool {
    fn name(&self) -> &'static str { "screenshot" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "media" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "screenshot",
                "description": "Take a picture of the whole screen. The person is asked first on every call; the shell takes it and saves it in ~/Pictures/Screenshots, and the reply is only its path. The picture is not shown to you and its text is not read: to read the screen use analyze_screen",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "region": {"type": "string", "description": "Optional rectangle of the screen to take, as the shell's screenshot_screen reads it"}
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        self.run(&crate::app_ui::Desk::session(), args)
    }
}

// ── Audio Control ──

pub struct AudioControlTool;

impl Tool for AudioControlTool {
    fn name(&self) -> &'static str { "audio_control" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "media" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "audio_control",
                "description": "Set volume level or toggle mute",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "action": {"type": "string", "enum": ["set_volume", "mute", "unmute"]},
                        "volume": {"type": "integer", "description": "Volume percentage (0-100)"}
                    },
                    "required": ["action"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or_default();
        let volume = args.get("volume").and_then(|v| v.as_u64()).unwrap_or(50);

        match action {
            "set_volume" => {
                let vol = volume.min(100);
                match std::process::Command::new("wpctl")
                    .args(["set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{}%", vol)])
                    .output()
                {
                    Ok(o) if o.status.success() => format!("Volume set to {vol}%"),
                    Ok(o) => format!("Failed: {}", String::from_utf8_lossy(&o.stderr)),
                    Err(e) => format!("Error: {e}"),
                }
            }
            "mute" => {
                match std::process::Command::new("wpctl")
                    .args(["set-mute", "@DEFAULT_AUDIO_SINK@", "1"])
                    .output()
                {
                    Ok(o) if o.status.success() => "Audio muted".to_string(),
                    Ok(o) => format!("Failed: {}", String::from_utf8_lossy(&o.stderr)),
                    Err(e) => format!("Error: {e}"),
                }
            }
            "unmute" => {
                match std::process::Command::new("wpctl")
                    .args(["set-mute", "@DEFAULT_AUDIO_SINK@", "0"])
                    .output()
                {
                    Ok(o) if o.status.success() => "Audio unmuted".to_string(),
                    Ok(o) => format!("Failed: {}", String::from_utf8_lossy(&o.stderr)),
                    Err(e) => format!("Error: {e}"),
                }
            }
            _ => format!("Error: unknown action '{action}'. Use set_volume, mute, or unmute."),
        }
    }
}

// ── Audio Info ──

pub struct AudioInfoTool;

impl Tool for AudioInfoTool {
    fn name(&self) -> &'static str { "audio_info" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "media" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "audio_info",
                "description": "Get current audio volume and device info",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let mut info = String::new();

        // Get default sink volume
        match std::process::Command::new("wpctl")
            .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
            .output()
        {
            Ok(o) if o.status.success() => {
                let vol = String::from_utf8_lossy(&o.stdout);
                info.push_str(&format!("Output: {}", vol.trim()));
            }
            _ => info.push_str("Output: unknown"),
        }

        // Get default source volume
        match std::process::Command::new("wpctl")
            .args(["get-volume", "@DEFAULT_AUDIO_SOURCE@"])
            .output()
        {
            Ok(o) if o.status.success() => {
                let vol = String::from_utf8_lossy(&o.stdout);
                info.push_str(&format!("\nInput: {}", vol.trim()));
            }
            _ => {}
        }

        if info.is_empty() {
            "Error: could not get audio info (wpctl not available?)".to_string()
        } else {
            info
        }
    }
}
