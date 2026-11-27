//! Network tools — download_file, http_fetch, web_fetch.

use std::io::Read as _;
use super::{Tool, ToolContext, ToolRegistry, PermissionLevel, validate_write_path};

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
                "description": "Download URL to a local file; do not open or extract",
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

        // A download is a write: never into a dotfile or hidden folder in the home, where
        // programs read their settings (a downloaded ~/.gitconfig runs code as the person).
        let path = match validate_write_path(path) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };
        let path = path.as_str();

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
                "description": "Fetch raw URL text; no AI extraction",
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
                    let truncated = if headers.len() > 2000 { &headers[..headers.floor_char_boundary(2000)] } else { &headers };
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
                "description": "Fetch webpage and extract answers with AI",
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
                        },
                        "plain": {
                            "type": "boolean",
                            "description": "Return the page's own text, with no model reading it: no redirects, and no loopback, private or link-local address. Used by background jobs."
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

        // A background job's fetch: the page's text and nothing else. No model reads it here (the
        // job's own writer does, through the gateway), nothing is followed to another address, and
        // no address on this machine or its network is asked.
        if args.get("plain").and_then(|v| v.as_bool()).unwrap_or(false) {
            return match fetch_plain(url) {
                Ok(html) => {
                    let text = html2text::from_read(html.as_bytes(), 100);
                    let end = text.floor_char_boundary(PLAIN_MAX);
                    format!("Page: {url}\n\n{}", &text[..end])
                }
                Err(e) => format!("Fetch error: {e}"),
            };
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

/// The most of a page's text a plain fetch returns.
pub const PLAIN_MAX: usize = 12_000;

/// Whether `ip` is an address a background fetch must never ask: this machine, a private network,
/// link-local (the metadata address is in it), unique-local, multicast, broadcast or unspecified.
pub fn forbidden_address(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || o[0] == 0
                || (o[0] == 100 && (64..128).contains(&o[1])) // shared address space (CGNAT)
                || (o[0] == 192 && o[1] == 0 && o[2] == 0) // IETF protocol assignments
                || (o[0] == 198 && (o[1] == 18 || o[1] == 19)) // benchmarking 198.18.0.0/15
                || o[0] >= 240
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return forbidden_address(IpAddr::V4(v4));
            }
            let seg = v6.segments();
            let first = seg[0];
            // v4-compatible ::a.b.c.d (deprecated) carries a v4 in its last 32 bits.
            if seg[..6].iter().all(|s| *s == 0) && !(v6.is_loopback() || v6.is_unspecified()) {
                return forbidden_address(IpAddr::V4(std::net::Ipv4Addr::new((seg[6] >> 8) as u8, seg[6] as u8, (seg[7] >> 8) as u8, seg[7] as u8)));
            }
            // 6to4 2002::/16 carries a v4 in its next 32 bits.
            if first == 0x2002 && forbidden_address(IpAddr::V4(std::net::Ipv4Addr::new((seg[1] >> 8) as u8, seg[1] as u8, (seg[2] >> 8) as u8, seg[2] as u8))) {
                return true;
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00 // unique-local fc00::/7
                || (first & 0xffc0) == 0xfe80 // link-local fe80::/10
                || (first & 0xffc0) == 0xfec0 // deprecated site-local fec0::/10
                || (first == 0x64 && seg[1] == 0xff9b) // NAT64 64:ff9b::/96 and local-use 64:ff9b:1::/48 can carry any v4
        }
    }
}

/// Resolves a name for a plain fetch and drops every forbidden address, at the moment of the
/// connection (so a name that changes its answer between a check and the connect gains nothing).
/// A name with only forbidden answers fails.
struct PublicOnly;

impl ureq::Resolver for PublicOnly {
    fn resolve(&self, netloc: &str) -> std::io::Result<Vec<std::net::SocketAddr>> {
        use std::net::ToSocketAddrs;
        let all: Vec<_> = netloc.to_socket_addrs()?.collect();
        let public: Vec<_> = all.iter().copied().filter(|a| !forbidden_address(a.ip())).collect();
        if public.is_empty() || public.len() != all.len() {
            return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "that name points at an address on this machine or a private network"));
        }
        Ok(public)
    }
}

/// [`fetch_html`] for a background job: `https` only, **no redirect is followed** (a 3xx is an
/// error), and the name may resolve only to public addresses.
fn fetch_plain(url: &str) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .resolver(PublicOnly)
        .timeout(std::time::Duration::from_secs(20))
        .build();
    let resp = match agent
        .get(url)
        .set("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
        .set("Accept", "text/html,text/plain;q=0.9")
        .call()
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code, _)) => return Err(format!("HTTP {code}")),
        Err(e) => return Err(format!("HTTP request failed: {}", yantrik_shown_address::without_sign_in(&e.to_string()))),
    };
    let status = resp.status();
    if status >= 300 {
        return Err(format!("HTTP {status} (a job does not follow redirects)"));
    }
    let mut body = String::new();
    resp.into_reader().take(1_048_576).read_to_string(&mut body).map_err(|e| format!("Read error: {e}"))?;
    Ok(body)
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
        // ureq 2's error text starts with the whole address, sign-in details included.
        .map_err(|e| format!("LLM request failed: {}", yantrik_shown_address::without_sign_in(&e.to_string())))?
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

    let words = ["-p", "--output-format", "text"].map(String::from);
    let mut command = crate::spawn::mind_command(&crate::spawn::account_home()?, &crate::spawn::here(), "claude", &words, &[])?;
    let output = command
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chat_model_that_cannot_be_reached_is_named_without_its_sign_in() {
        // Nothing listens on port 9, so ureq 2 fails in transport, and its text starts with the URL.
        let err = llm_extract("http://user:pw@127.0.0.1:9", "m", "page", "what?", "https://page.example").unwrap_err();
        assert!(err.starts_with("LLM request failed: http://127.0.0.1:9/api/chat"), "{err}");
        assert!(!err.contains("user") && !err.contains("pw") && !err.contains('@'), "{err}");
    }

    #[test]
    fn a_plain_fetch_refuses_loopback_private_link_local_unique_local_and_metadata_addresses() {
        for bad in ["127.0.0.1", "127.8.8.8", "10.1.2.3", "172.16.0.1", "172.31.255.255", "192.168.1.1", "169.254.169.254", "0.0.0.0", "100.64.0.1", "224.0.0.1", "255.255.255.255", "::1", "::", "fc00::1", "fd12:3456::1", "fe80::1", "::ffff:127.0.0.1", "::ffff:169.254.169.254", "::ffff:10.0.0.1", "64:ff9b::7f00:1", "ff02::1", "198.18.0.1", "198.19.255.254", "2002:a00:1::1", "2002:7f00:1::1", "64:ff9b:1::a00:1", "fec0::1", "::a00:1", "::7f00:1"] {
            assert!(forbidden_address(bad.parse().unwrap()), "{bad} must be refused");
        }
        for good in ["93.184.216.34", "8.8.8.8", "172.32.0.1", "1.1.1.1", "2606:4700:4700::1111", "2a00:1450:4001::200e"] {
            assert!(!forbidden_address(good.parse().unwrap()), "{good} is public");
        }
    }

    #[test]
    fn the_resolver_of_a_plain_fetch_will_not_answer_with_a_local_address() {
        use ureq::Resolver;
        // A name that is really a loopback (a rebinding name behaves the same), and the numbers.
        for netloc in ["localhost:443", "127.0.0.1:443", "[::1]:443", "169.254.169.254:80", "10.0.0.5:443"] {
            assert!(PublicOnly.resolve(netloc).is_err(), "{netloc}");
        }
        assert!(fetch_plain("https://localhost/").is_err());
        assert!(fetch_plain("https://169.254.169.254/latest/meta-data/").is_err());
    }
}
