//! Wallpaper tool — set_wallpaper.
//! Uses swaybg for Wayland compositors.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel, validate_path};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(SetWallpaperTool));
}

// ── Set Wallpaper ──

pub struct SetWallpaperTool;

impl Tool for SetWallpaperTool {
    fn name(&self) -> &'static str { "set_wallpaper" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "wallpaper" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "set_wallpaper",
                "description": "Set the desktop wallpaper to an image file",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Path to image file (png, jpg, etc.)"},
                        "mode": {
                            "type": "string",
                            "enum": ["fill", "fit", "stretch", "center", "tile"],
                            "description": "How to display the image (default: fill)"
                        },
                        "color": {
                            "type": "string",
                            "description": "Solid background color instead of image (hex, e.g. #1a1a2e)"
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("fill");
        let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("");

        // Kill any existing swaybg
        let _ = std::process::Command::new("pkill").arg("swaybg").output();

        if !color.is_empty() {
            // Solid color mode
            if !color.starts_with('#') || color.len() > 9 {
                return "Error: color must be a hex value like #1a1a2e".to_string();
            }
            match std::process::Command::new("swaybg")
                .args(["-c", color])
                .spawn()
            {
                Ok(_) => format!("Wallpaper set to solid color {color}"),
                Err(e) => format!("Error (swaybg not available?): {e}"),
            }
        } else if !path.is_empty() {
            let expanded = match validate_path(path) {
                Ok(p) => p,
                Err(e) => return format!("Error: {e}"),
            };

            if !std::path::Path::new(&expanded).exists() {
                return format!("Error: file not found: {path}");
            }

            // Check extension
            let ext = std::path::Path::new(&expanded)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if !["png", "jpg", "jpeg", "bmp", "gif", "webp"].contains(&ext.as_str()) {
                return format!("Error: unsupported image format '.{ext}'");
            }

            match std::process::Command::new("swaybg")
                .args(["-i", &expanded, "-m", mode])
                .spawn()
            {
                Ok(_) => format!("Wallpaper set to {path} (mode: {mode})"),
                Err(e) => format!("Error (swaybg not available?): {e}"),
            }
        } else {
            "Error: provide either a path to an image or a color".to_string()
        }
    }
}
