//! Display tools — display_info, set_resolution.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(DisplayInfoTool));
    reg.register(Box::new(SetResolutionTool));
}

// ── Display Info ──

pub struct DisplayInfoTool;

impl Tool for DisplayInfoTool {
    fn name(&self) -> &'static str { "display_info" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "display" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "display_info",
                "description": "Get display resolution, refresh rate, and output name.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        match std::process::Command::new("wlr-randr").output() {
            Ok(output) if output.status.success() => {
                let text = String::from_utf8_lossy(&output.stdout);
                // Parse wlr-randr output — extract output name, resolution, refresh
                let mut result = String::new();
                let mut current_output = String::new();

                for line in text.lines() {
                    let trimmed = line.trim();
                    if !line.starts_with(' ') && !trimmed.is_empty() {
                        // Output name line (e.g. "Virtual-1")
                        current_output = trimmed.to_string();
                    } else if trimmed.contains("current") {
                        // Mode line with "current" marker
                        result.push_str(&format!("{}: {}\n", current_output, trimmed));
                    }
                }

                if result.is_empty() {
                    // Fallback: return raw (truncated)
                    let trunc = if text.len() > 1000 { &text[..1000] } else { &text };
                    trunc.to_string()
                } else {
                    result.trim().to_string()
                }
            }
            Ok(output) => {
                let err = String::from_utf8_lossy(&output.stderr);
                format!("wlr-randr failed: {err}")
            }
            Err(e) => format!("Error (wlr-randr not available?): {e}"),
        }
    }
}

// ── Set Resolution ──

pub struct SetResolutionTool;

impl Tool for SetResolutionTool {
    fn name(&self) -> &'static str { "set_resolution" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "display" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "set_resolution",
                "description": "Change display resolution.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "output": {"type": "string", "description": "Output name (e.g. Virtual-1)"},
                        "width": {"type": "integer"},
                        "height": {"type": "integer"}
                    },
                    "required": ["output", "width", "height"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let output = args.get("output").and_then(|v| v.as_str()).unwrap_or_default();
        let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(0);
        let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(0);

        if output.is_empty() || width == 0 || height == 0 {
            return "Error: output, width, and height are required".to_string();
        }

        // Validate output name (alphanumeric + hyphen only)
        if output.contains(|c: char| !c.is_alphanumeric() && c != '-' && c != '_') {
            return "Error: invalid output name".to_string();
        }

        let mode = format!("{}x{}", width, height);
        match std::process::Command::new("wlr-randr")
            .args(["--output", output, "--mode", &mode])
            .output()
        {
            Ok(o) if o.status.success() => format!("Resolution set to {mode} on {output}"),
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                format!("Failed to set resolution: {err}")
            }
            Err(e) => format!("Error: {e}"),
        }
    }
}
