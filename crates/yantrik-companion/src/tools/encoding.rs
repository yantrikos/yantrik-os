//! Encoding tools — base64_encode, base64_decode, url_encode, json_format.
//! Pure Rust implementations — no external dependencies needed.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(Base64EncodeTool));
    reg.register(Box::new(Base64DecodeTool));
    reg.register(Box::new(UrlEncodeTool));
    reg.register(Box::new(JsonFormatTool));
}

// ── Base64 Encode ──

pub struct Base64EncodeTool;

impl Tool for Base64EncodeTool {
    fn name(&self) -> &'static str { "base64_encode" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "encoding" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "base64_encode",
                "description": "Encode text to Base64.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "text": {"type": "string", "description": "Text to encode"}
                    },
                    "required": ["text"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let text = args.get("text").and_then(|v| v.as_str()).unwrap_or_default();
        if text.is_empty() {
            return "Error: text is required".to_string();
        }
        if text.len() > 100_000 {
            return "Error: text too long (max 100KB)".to_string();
        }

        // Use base64 command (available everywhere)
        let mut child = match std::process::Command::new("base64")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return format!("Error: {e}"),
        };
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(text.as_bytes());
        }
        match child.wait_with_output() {
            Ok(o) if o.status.success() => {
                String::from_utf8_lossy(&o.stdout).trim().to_string()
            }
            Ok(o) => format!("Encode failed: {}", String::from_utf8_lossy(&o.stderr)),
            Err(e) => format!("Error: {e}"),
        }
    }
}

// ── Base64 Decode ──

pub struct Base64DecodeTool;

impl Tool for Base64DecodeTool {
    fn name(&self) -> &'static str { "base64_decode" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "encoding" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "base64_decode",
                "description": "Decode Base64 text back to plain text.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "encoded": {"type": "string", "description": "Base64-encoded text"}
                    },
                    "required": ["encoded"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let encoded = args.get("encoded").and_then(|v| v.as_str()).unwrap_or_default();
        if encoded.is_empty() {
            return "Error: encoded text is required".to_string();
        }
        if encoded.len() > 200_000 {
            return "Error: input too long".to_string();
        }

        let mut child = match std::process::Command::new("base64")
            .arg("-d")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return format!("Error: {e}"),
        };
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(encoded.as_bytes());
        }
        match child.wait_with_output() {
            Ok(o) if o.status.success() => {
                let decoded = String::from_utf8_lossy(&o.stdout);
                if decoded.len() > 5000 {
                    format!("{}...\n(truncated, {} chars)", &decoded[..5000], decoded.len())
                } else {
                    decoded.to_string()
                }
            }
            Ok(o) => format!("Decode failed: {}", String::from_utf8_lossy(&o.stderr)),
            Err(e) => format!("Error: {e}"),
        }
    }
}

// ── URL Encode ──

pub struct UrlEncodeTool;

impl Tool for UrlEncodeTool {
    fn name(&self) -> &'static str { "url_encode" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "encoding" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "url_encode",
                "description": "URL-encode or decode a string.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "text": {"type": "string", "description": "Text to encode or decode"},
                        "decode": {"type": "boolean", "description": "Set true to decode instead of encode"}
                    },
                    "required": ["text"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let text = args.get("text").and_then(|v| v.as_str()).unwrap_or_default();
        let decode = args.get("decode").and_then(|v| v.as_bool()).unwrap_or(false);

        if text.is_empty() {
            return "Error: text is required".to_string();
        }

        if decode {
            url_decode(text)
        } else {
            url_encode(text)
        }
    }
}

fn url_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}

fn url_decode(s: &str) -> String {
    let mut result = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(val) = u8::from_str_radix(
                &String::from_utf8_lossy(&bytes[i + 1..i + 3]),
                16,
            ) {
                result.push(val);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            result.push(b' ');
        } else {
            result.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&result).to_string()
}

// ── JSON Format ──

pub struct JsonFormatTool;

impl Tool for JsonFormatTool {
    fn name(&self) -> &'static str { "json_format" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "encoding" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "json_format",
                "description": "Pretty-print, minify, or validate JSON text.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "json_text": {"type": "string", "description": "JSON string to format"},
                        "action": {
                            "type": "string",
                            "enum": ["pretty", "minify", "validate"],
                            "description": "What to do (default: pretty)"
                        }
                    },
                    "required": ["json_text"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let json_text = args.get("json_text").and_then(|v| v.as_str()).unwrap_or_default();
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("pretty");

        if json_text.is_empty() {
            return "Error: json_text is required".to_string();
        }

        if json_text.len() > 100_000 {
            return "Error: JSON too large (max 100KB)".to_string();
        }

        // Parse
        let parsed: serde_json::Value = match serde_json::from_str(json_text) {
            Ok(v) => v,
            Err(e) => return format!("Invalid JSON: {e}"),
        };

        match action {
            "validate" => "Valid JSON.".to_string(),
            "minify" => {
                match serde_json::to_string(&parsed) {
                    Ok(s) => s,
                    Err(e) => format!("Error: {e}"),
                }
            }
            _ => {
                // Pretty print
                match serde_json::to_string_pretty(&parsed) {
                    Ok(s) => {
                        if s.len() > 5000 {
                            format!("{}...\n(truncated)", &s[..5000])
                        } else {
                            s
                        }
                    }
                    Err(e) => format!("Error: {e}"),
                }
            }
        }
    }
}
