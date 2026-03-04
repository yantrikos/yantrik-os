//! Vision tools — screenshot analysis, image description, smart rename.
//!
//! Uses Ollama's native `/api/chat` endpoint with multimodal `images` field.
//! Screenshots captured via `grim` (Wayland).

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

/// Register vision tools with the Ollama base URL (e.g. "http://192.168.4.35:11434").
pub fn register(reg: &mut ToolRegistry, ollama_base: &str, model: &str) {
    let base = ollama_base.trim_end_matches('/').to_string();
    let mdl = model.to_string();
    reg.register(Box::new(AnalyzeScreenTool { ollama_base: base.clone(), model: mdl.clone() }));
    reg.register(Box::new(DescribeImageTool { ollama_base: base.clone(), model: mdl.clone() }));
    reg.register(Box::new(SmartRenameTool { ollama_base: base, model: mdl }));
}

/// Capture a screenshot via grim (Wayland) and return the file path.
fn capture_screenshot() -> Result<String, String> {
    let path = "/tmp/yantrik-vision-screenshot.png";
    let output = std::process::Command::new("grim")
        .args(["-t", "png", path])
        .output()
        .map_err(|e| format!("grim not available: {e}"))?;

    if !output.status.success() {
        return Err(format!("grim failed: {}", String::from_utf8_lossy(&output.stderr)));
    }

    Ok(path.to_string())
}

/// Base64-encode a file using the `base64` CLI tool (avoids Rust crate dep).
fn base64_encode_file(path: &str) -> Result<String, String> {
    let output = std::process::Command::new("base64")
        .args(["-w", "0", path])
        .output()
        .map_err(|e| format!("base64 command failed: {e}"))?;

    if !output.status.success() {
        return Err("base64 encoding failed".to_string());
    }

    String::from_utf8(output.stdout).map_err(|e| format!("Invalid UTF-8 from base64: {e}"))
}

/// Send an image to Ollama's vision API and get a text response.
/// Writes payload to a temp file to avoid argument length limits with large base64 data.
fn vision_request(ollama_base: &str, model: &str, prompt: &str, image_path: &str) -> Result<String, String> {
    let b64 = base64_encode_file(image_path)?;

    // Build JSON payload for Ollama native /api/chat
    let payload = serde_json::json!({
        "model": model,
        "messages": [{
            "role": "user",
            "content": prompt,
            "images": [b64]
        }],
        "stream": false
    });

    // Write payload to temp file (base64 images can be huge)
    let payload_path = "/tmp/yantrik-vision-payload.json";
    std::fs::write(payload_path, payload.to_string())
        .map_err(|e| format!("Failed to write payload: {e}"))?;

    let url = format!("{}/api/chat", ollama_base);

    // Use curl with @file to avoid argument length limits
    let output = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "--max-time", "120",
            "-H", "Content-Type: application/json",
            "-d", &format!("@{payload_path}"),
            &url,
        ])
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;

    // Clean up payload file
    let _ = std::fs::remove_file(payload_path);

    if !output.status.success() {
        return Err(format!("Ollama vision request failed: {}", String::from_utf8_lossy(&output.stderr)));
    }

    let response: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Invalid JSON from Ollama: {e}"))?;

    response["message"]["content"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "No content in Ollama response".to_string())
}

// ── analyze_screen ──

pub struct AnalyzeScreenTool {
    ollama_base: String,
    model: String,
}

impl Tool for AnalyzeScreenTool {
    fn name(&self) -> &'static str { "analyze_screen" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "vision" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "analyze_screen",
                "description": "Take a screenshot of the current screen and analyze what's visible. Can answer specific questions about on-screen content.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "question": {
                            "type": "string",
                            "description": "What to look for or analyze on screen (default: general description)"
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let question = args.get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("Describe what's currently visible on this screen. Be specific about applications, text, and content you can see.");

        let image_path = match capture_screenshot() {
            Ok(path) => path,
            Err(e) => return format!("Screenshot failed: {e}"),
        };

        match vision_request(&self.ollama_base, &self.model, question, &image_path) {
            Ok(description) => description,
            Err(e) => format!("Vision analysis failed: {e}"),
        }
    }
}

// ── describe_image ──

pub struct DescribeImageTool {
    ollama_base: String,
    model: String,
}

impl Tool for DescribeImageTool {
    fn name(&self) -> &'static str { "describe_image" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "vision" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "describe_image",
                "description": "Describe the contents of an image file. Supports PNG, JPG, WebP.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Absolute path to the image file"
                        }
                    },
                    "required": ["path"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => super::expand_home(p),
            None => return "Error: path parameter required".to_string(),
        };

        if !std::path::Path::new(&path).exists() {
            return format!("File not found: {path}");
        }

        // Limit file size to 20MB
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.len() > 20 * 1024 * 1024 {
                return "Error: image file too large (max 20MB)".to_string();
            }
        }

        let prompt = "Describe this image in detail. Include: main subject, colors, composition, text visible, and overall mood or context.";

        match vision_request(&self.ollama_base, &self.model, prompt, &path) {
            Ok(description) => description,
            Err(e) => format!("Vision analysis failed: {e}"),
        }
    }
}

// ── smart_rename ──

pub struct SmartRenameTool {
    ollama_base: String,
    model: String,
}

impl Tool for SmartRenameTool {
    fn name(&self) -> &'static str { "smart_rename" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "vision" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "smart_rename",
                "description": "Analyze an image and rename the file based on its content. Generates a descriptive, kebab-case filename.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Absolute path to the image file to rename"
                        }
                    },
                    "required": ["path"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => super::expand_home(p),
            None => return "Error: path parameter required".to_string(),
        };

        let file_path = std::path::Path::new(&path);
        if !file_path.exists() {
            return format!("File not found: {path}");
        }

        let ext = file_path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png")
            .to_lowercase();

        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.len() > 20 * 1024 * 1024 {
                return "Error: image file too large (max 20MB)".to_string();
            }
        }

        let prompt = "Generate a short, descriptive filename for this image. Rules: use kebab-case (hyphens), max 5 words, no file extension, be specific about content. Reply with ONLY the filename, nothing else.";

        let new_name = match vision_request(&self.ollama_base, &self.model, prompt, &path) {
            Ok(name) => name.trim().to_lowercase()
                .replace(' ', "-")
                .replace('_', "-")
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-')
                .collect::<String>(),
            Err(e) => return format!("Vision analysis failed: {e}"),
        };

        if new_name.is_empty() {
            return "Error: vision model returned empty filename".to_string();
        }

        let parent = file_path.parent().unwrap_or(std::path::Path::new("/tmp"));
        let new_path = parent.join(format!("{new_name}.{ext}"));

        if new_path.exists() {
            return format!("Target already exists: {}", new_path.display());
        }

        match std::fs::rename(&path, &new_path) {
            Ok(()) => format!(
                "Renamed: {} → {}",
                file_path.file_name().unwrap_or_default().to_string_lossy(),
                new_path.file_name().unwrap_or_default().to_string_lossy()
            ),
            Err(e) => format!("Rename failed: {e}"),
        }
    }
}
