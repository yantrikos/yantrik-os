//! Network tools — download_file, http_fetch.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel, validate_path};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(DownloadFileTool));
    reg.register(Box::new(HttpFetchTool));
}

// ── Download File ──

pub struct DownloadFileTool;

impl Tool for DownloadFileTool {
    fn name(&self) -> &'static str { "download_file" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "network" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "download_file",
                "description": "Download a file from a URL to a local path.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {"type": "string", "description": "https:// URL to download"},
                        "save_path": {"type": "string", "description": "Where to save (e.g. ~/Downloads/file.pdf)"}
                    },
                    "required": ["url", "save_path"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let url = args.get("url").and_then(|v| v.as_str()).unwrap_or_default();
        let save_path = args.get("save_path").and_then(|v| v.as_str()).unwrap_or_default();

        if url.is_empty() || save_path.is_empty() {
            return "Error: url and save_path are required".to_string();
        }

        if !url.starts_with("https://") {
            return "Error: URL must start with https://".to_string();
        }

        // Validate URL has no shell metacharacters
        if url.contains(|c: char| c == '`' || c == '$' || c == ';' || c == '|' || c == '&') {
            return "Error: URL contains invalid characters".to_string();
        }

        let expanded = match validate_path(save_path) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };

        // Create parent dirs
        if let Some(parent) = std::path::Path::new(&expanded).parent() {
            if !parent.exists() {
                let _ = std::fs::create_dir_all(parent);
            }
        }

        // Download via curl with safety limits
        match std::process::Command::new("curl")
            .args([
                "-fsSL",
                "--max-time", "30",
                "--max-filesize", "104857600",  // 100MB
                "-o", &expanded,
                url,
            ])
            .output()
        {
            Ok(output) if output.status.success() => {
                // Report file size
                let size = std::fs::metadata(&expanded)
                    .map(|m| super::format_size(m.len()))
                    .unwrap_or_else(|_| "unknown size".to_string());
                format!("Downloaded {url} → {save_path} ({size})")
            }
            Ok(output) => {
                let err = String::from_utf8_lossy(&output.stderr);
                // Clean up partial download
                let _ = std::fs::remove_file(&expanded);
                format!("Download failed: {err}")
            }
            Err(e) => format!("Error (curl not available?): {e}"),
        }
    }
}

// ── HTTP Fetch ──

pub struct HttpFetchTool;

impl Tool for HttpFetchTool {
    fn name(&self) -> &'static str { "http_fetch" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "network" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "http_fetch",
                "description": "Fetch text from a URL. Returns first 3000 chars.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {"type": "string", "description": "https:// URL to fetch"},
                        "extract": {"type": "string", "enum": ["text", "headers", "json"]}
                    },
                    "required": ["url"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let url = args.get("url").and_then(|v| v.as_str()).unwrap_or_default();
        let extract = args.get("extract").and_then(|v| v.as_str()).unwrap_or("text");

        if url.is_empty() {
            return "Error: url is required".to_string();
        }

        if !url.starts_with("https://") {
            return "Error: URL must start with https://".to_string();
        }

        if url.contains(|c: char| c == '`' || c == '$' || c == ';' || c == '|' || c == '&') {
            return "Error: URL contains invalid characters".to_string();
        }

        if extract == "headers" {
            // Fetch headers only
            return match std::process::Command::new("curl")
                .args(["-fsS", "-I", "--max-time", "10", url])
                .output()
            {
                Ok(output) if output.status.success() => {
                    let headers = String::from_utf8_lossy(&output.stdout);
                    let truncated = if headers.len() > 2000 { &headers[..2000] } else { &headers };
                    truncated.to_string()
                }
                Ok(output) => {
                    let err = String::from_utf8_lossy(&output.stderr);
                    format!("Fetch failed: {err}")
                }
                Err(e) => format!("Error (curl not available?): {e}"),
            };
        }

        // Fetch body content
        match std::process::Command::new("curl")
            .args(["-fsSL", "--max-time", "10", "--max-filesize", "1048576", url])
            .output()
        {
            Ok(output) if output.status.success() => {
                let body = String::from_utf8_lossy(&output.stdout);

                let processed = if extract == "json" {
                    // Return raw JSON (truncated)
                    body.to_string()
                } else {
                    // Strip HTML tags for text mode
                    strip_html(&body)
                };

                if processed.len() > 3000 {
                    format!("{}\n... (truncated, {} total chars)", &processed[..3000], processed.len())
                } else {
                    processed
                }
            }
            Ok(output) => {
                let err = String::from_utf8_lossy(&output.stderr);
                format!("Fetch failed: {err}")
            }
            Err(e) => format!("Error (curl not available?): {e}"),
        }
    }
}

/// Strip HTML tags from text. Simple approach: remove <...> sequences.
fn strip_html(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut last_was_space = false;

    for ch in html.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
        } else if !in_tag {
            if ch.is_whitespace() {
                if !last_was_space {
                    result.push(' ');
                    last_was_space = true;
                }
            } else {
                result.push(ch);
                last_was_space = false;
            }
        }
    }

    result.trim().to_string()
}

