//! Network tools — download_file, http_fetch, web_fetch.

use std::io::Read as _;
use super::{Tool, ToolContext, ToolRegistry, PermissionLevel, validate_path};

/// Register network tools. `ollama_base` and `model` are optional —
/// when provided, `web_fetch` uses LLM extraction for smart content processing.
/// When both are empty, web_fetch falls back to claude-cli extraction if available.
pub fn register(reg: &mut ToolRegistry, ollama_base: &str, model: &str) {
    reg.register(Box::new(DownloadFileTool));
    reg.register(Box::new(HttpFetchTool));
    reg.register(Box::new(WebFetchTool {
        ollama_base: ollama_base.to_string(),
        model: model.to_string(),
    }));
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
                "description": "Download a file from a URL and save it locally.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {"type": "string", "description": "URL to download from"},
                        "path": {"type": "string", "description": "Local path to save to"}
                    },
                    "required": ["url", "path"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let url = args.get("url").and_then(|v| v.as_str()).unwrap_or_default();
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();

        if url.is_empty() || path.is_empty() {
            return "Error: url and path are required".to_string();
        }

        if !url.starts_with("https://") && !url.starts_with("http://localhost") {
            return "Error: URL must start with https:// (or http://localhost)".to_string();
        }

        if let Err(e) = validate_path(path) {
            return format!("Error: {e}");
        }

        match std::process::Command::new("curl")
            .args(["-fsSL", "--max-time", "60", "--max-filesize", "10485760", "-o", path, url])
            .output()
        {
            Ok(output) if output.status.success() => {
                let size = std::fs::metadata(path)
                    .map(|m| m.len())
                    .unwrap_or(0);
                format!("Downloaded {} bytes to {path}", size)
            }
            Ok(output) => {
                let err = String::from_utf8_lossy(&output.stderr);
                format!("Download failed: {err}")
            }
            Err(e) => format!("Error: {e}"),
        }
    }
}

// ── HTTP Fetch (raw, fast, no AI) ──

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
                "description": "Fetch raw text from a URL. Returns stripped HTML content. For smarter extraction with AI processing, use web_fetch instead.",
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

        if let Err(e) = validate_fetch_url(url) {
            return format!("Error: {e}");
        }

        if extract == "headers" {
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
                    body.to_string()
                } else {
                    // Use html2text for clean readable output
                    html2text::from_read(body.as_bytes(), 100)
                };

                if processed.len() > 6000 {
                    let boundary = processed.floor_char_boundary(6000);
                    format!("{}\n... (truncated, {} total chars)", &processed[..boundary], processed.len())
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

// ── Web Fetch (AI-powered, like Claude Code's WebFetch) ──

pub struct WebFetchTool {
    ollama_base: String,
    model: String,
}

impl Tool for WebFetchTool {
    fn name(&self) -> &'static str { "web_fetch" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "network" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "web_fetch",
                "description": "Fetch a web page and extract information using AI. Converts HTML to clean markdown, then uses an AI model to process the content based on your prompt. Returns a focused, relevant answer instead of raw page text. Best for articles, documentation, search results, and any page where you need specific information extracted.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "URL to fetch (https:// only)"
                        },
                        "prompt": {
                            "type": "string",
                            "description": "What to extract or analyze from the page (e.g., 'summarize the main article', 'extract all prices', 'find the author and publication date')"
                        }
                    },
                    "required": ["url", "prompt"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let url = args.get("url").and_then(|v| v.as_str()).unwrap_or_default();
        let prompt = args.get("prompt").and_then(|v| v.as_str()).unwrap_or("Summarize the main content of this page.");

        if let Err(e) = validate_fetch_url(url) {
            return format!("Error: {e}");
        }

        if prompt.is_empty() {
            return "Error: prompt is required".to_string();
        }

        // Step 1: Fetch HTML via ureq (pure Rust, no curl dependency)
        let html = match fetch_html(url) {
            Ok(h) => h,
            Err(e) => return format!("Fetch error: {e}"),
        };

        // Step 2: Convert HTML → clean markdown via html2text
        let markdown = html2text::from_read(html.as_bytes(), 100);

        // Truncate to reasonable size for LLM context
        let max_content = 12000;
        let content = if markdown.len() > max_content {
            let boundary = markdown.floor_char_boundary(max_content);
            format!("{}\n\n[Content truncated — {} total chars]", &markdown[..boundary], markdown.len())
        } else {
            markdown
        };

        // Step 3: If LLM is available, extract with AI
        if !self.ollama_base.is_empty() {
            match llm_extract(&self.ollama_base, &self.model, &content, prompt, url) {
                Ok(result) => return result,
                Err(e) => {
                    tracing::warn!("web_fetch LLM extraction failed: {e}, trying claude-cli");
                }
            }
        }

        // Fallback: try claude-cli for extraction (works when backend is claude-cli)
        match claude_cli_extract(&content, prompt, url) {
            Ok(result) => return result,
            Err(e) => {
                tracing::debug!("web_fetch claude-cli extraction unavailable: {e}");
            }
        }

        // Final fallback: return clean markdown without AI processing
        if content.len() > 6000 {
            let boundary = content.floor_char_boundary(6000);
            format!("Page: {url}\n\n{}\n\n[Truncated — AI extraction unavailable]", &content[..boundary])
        } else {
            format!("Page: {url}\n\n{content}")
        }
    }
}

// ── Shared helpers ──

/// Validate a URL for fetch operations.
fn validate_fetch_url(url: &str) -> Result<(), String> {
    if url.is_empty() {
        return Err("url is required".to_string());
    }
    if !url.starts_with("https://") {
        return Err("URL must start with https://".to_string());
    }
    // Reject truncated URLs (from tool trace ellipsis in memory)
    if url.contains('\u{2026}') || url.ends_with("...") {
        return Err("URL appears truncated (contains '...'). Construct the full URL instead of copying from memory traces.".to_string());
    }
    // Reject non-ASCII characters
    if url.contains(|c: char| !c.is_ascii()) {
        return Err("URL contains non-ASCII characters. Use percent-encoding for special characters.".to_string());
    }
    if url.contains(|c: char| c == '`' || c == '$' || c == ';' || c == '|' || c == '&') {
        return Err("URL contains invalid characters".to_string());
    }
    Ok(())
}

/// Fetch HTML content from a URL using ureq.
fn fetch_html(url: &str) -> Result<String, String> {
    let resp = ureq::get(url)
        .set("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
        .set("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .set("Accept-Language", "en-US,en;q=0.5")
        .call()
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    let status = resp.status();
    if status >= 400 {
        return Err(format!("HTTP {status}"));
    }

    // Read body with size limit (1MB)
    let mut body = String::new();
    resp.into_reader()
        .take(1_048_576)
        .read_to_string(&mut body)
        .map_err(|e| format!("Read error: {e}"))?;

    Ok(body)
}

/// Call LLM to extract/summarize content from a web page.
fn llm_extract(ollama_base: &str, model: &str, content: &str, prompt: &str, url: &str) -> Result<String, String> {
    let system = format!(
        "You are a web content extraction assistant. The user has fetched a web page and wants specific information from it.\n\
         Page URL: {url}\n\n\
         RULES:\n\
         - Answer based ONLY on the page content provided\n\
         - Be concise and direct\n\
         - If the requested information is not in the page, say so clearly\n\
         - Format your response in clean markdown"
    );

    let user_msg = format!(
        "PAGE CONTENT:\n{content}\n\n---\n\nEXTRACT: {prompt}"
    );

    let payload = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user_msg}
        ],
        "stream": false,
        "options": {
            "temperature": 0.1,
            "num_predict": 1024
        }
    });

    let api_url = format!("{}/api/chat", ollama_base);

    let resp: serde_json::Value = ureq::post(&api_url)
        .set("Content-Type", "application/json")
        .send_string(&payload.to_string())
        .map_err(|e| format!("LLM request failed: {e}"))?
        .into_json()
        .map_err(|e| format!("LLM response parse error: {e}"))?;

    let answer = resp["message"]["content"]
        .as_str()
        .ok_or("No content in LLM response")?
        .trim()
        .to_string();

    if answer.is_empty() {
        return Err("Empty LLM response".to_string());
    }

    Ok(answer)
}

/// Use claude-cli for web content extraction (when Ollama is not available).
fn claude_cli_extract(content: &str, prompt: &str, url: &str) -> Result<String, String> {
    // Check if claude CLI is available
    let which = std::process::Command::new("which")
        .arg("claude")
        .output();
    match which {
        Ok(o) if o.status.success() => {}
        _ => return Err("claude CLI not available".to_string()),
    }

    // Truncate content to avoid overwhelming the CLI
    let max_chars = 8000;
    let content = if content.len() > max_chars {
        &content[..content.floor_char_boundary(max_chars)]
    } else {
        content
    };

    let input = format!(
        "Extract information from this web page. Page URL: {url}\n\n\
         PAGE CONTENT:\n{content}\n\n---\n\n\
         EXTRACT: {prompt}\n\n\
         Be concise and direct. Answer based ONLY on the page content."
    );

    let output = std::process::Command::new("claude")
        .args(["-p", "--output-format", "text"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .and_then(|mut child| {
            if let Some(stdin) = child.stdin.as_mut() {
                use std::io::Write;
                let _ = stdin.write_all(input.as_bytes());
            }
            child.wait_with_output()
        })
        .map_err(|e| format!("claude-cli error: {e}"))?;

    if !output.status.success() {
        return Err("claude-cli returned non-zero".to_string());
    }

    let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if result.is_empty() {
        return Err("Empty claude-cli response".to_string());
    }

    Ok(result)
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
