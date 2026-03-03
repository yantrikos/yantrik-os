//! Browser tools — CDP-based Chromium control for web browsing.
//!
//! 8 tools: launch_browser, browse, browser_read, browser_click,
//! browser_type, browser_screenshot, browser_tabs, browser_search.

use std::io::{Read as IoRead, Write as IoWrite};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use serde_json::{json, Value};
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

const CDP_HOST: &str = "127.0.0.1";
const CDP_PORT: u16 = 9222;
const MAX_TEXT_CHARS: usize = 3000;
const MAX_SELECTOR_LEN: usize = 200;

/// Global CDP message counter for unique IDs.
static MSG_ID: AtomicU32 = AtomicU32::new(1);

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(LaunchBrowserTool));
    reg.register(Box::new(BrowseTool));
    reg.register(Box::new(BrowserReadTool));
    reg.register(Box::new(BrowserClickTool));
    reg.register(Box::new(BrowserTypeTool));
    reg.register(Box::new(BrowserScreenshotTool));
    reg.register(Box::new(BrowserTabsTool));
    reg.register(Box::new(BrowserSearchTool));
}

// ── CDP helpers ──

/// Tab info from the CDP /json endpoint.
#[derive(Debug)]
struct CdpTab {
    id: String,
    title: String,
    url: String,
    ws_url: String,
}

/// Fetch open tabs from Chromium's CDP HTTP endpoint.
fn get_tabs() -> Result<Vec<CdpTab>, String> {
    let url = format!("http://{}:{}/json", CDP_HOST, CDP_PORT);
    let output = std::process::Command::new("curl")
        .args(["-s", "--max-time", "3", &url])
        .output()
        .map_err(|e| format!("Cannot reach browser (curl): {e}"))?;

    if !output.status.success() {
        return Err("Browser not reachable. Use launch_browser first.".to_string());
    }

    let body = String::from_utf8_lossy(&output.stdout);
    let tabs: Vec<Value> = serde_json::from_str(&body)
        .map_err(|e| format!("Bad CDP response: {e}"))?;

    Ok(tabs
        .iter()
        .filter(|t| t.get("type").and_then(|v| v.as_str()) == Some("page"))
        .map(|t| CdpTab {
            id: t["id"].as_str().unwrap_or_default().to_string(),
            title: t["title"].as_str().unwrap_or_default().to_string(),
            url: t["url"].as_str().unwrap_or_default().to_string(),
            ws_url: t["webSocketDebuggerUrl"].as_str().unwrap_or_default().to_string(),
        })
        .collect())
}

/// Connect to a tab's WebSocket and return a client.
fn connect_tab(ws_url: &str) -> Result<WebSocket<MaybeTlsStream<TcpStream>>, String> {
    let (socket, _response) = tungstenite::connect(ws_url)
        .map_err(|e| format!("WS connect failed: {e}"))?;
    Ok(socket)
}

/// Connect to the first available tab, or return an error.
fn connect_first_tab() -> Result<(WebSocket<MaybeTlsStream<TcpStream>>, CdpTab), String> {
    let tabs = get_tabs()?;
    let tab = tabs.into_iter().next()
        .ok_or_else(|| "No browser tabs open. Use launch_browser first.".to_string())?;
    if tab.ws_url.is_empty() {
        return Err("Tab has no WebSocket URL — is another debugger attached?".to_string());
    }
    let ws = connect_tab(&tab.ws_url)?;
    Ok((ws, tab))
}

/// Send a CDP command and wait for the matching response.
fn cdp_send(
    ws: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let id = MSG_ID.fetch_add(1, Ordering::Relaxed);
    let msg = json!({
        "id": id,
        "method": method,
        "params": params,
    });

    ws.send(Message::Text(msg.to_string()))
        .map_err(|e| format!("WS send error: {e}"))?;

    // Read messages until we get our response (by id)
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        if std::time::Instant::now() > deadline {
            return Err("CDP response timeout (15s)".to_string());
        }

        let msg = ws.read()
            .map_err(|e| format!("WS read error: {e}"))?;

        if let Message::Text(text) = msg {
            if let Ok(resp) = serde_json::from_str::<Value>(&text) {
                if resp.get("id").and_then(|v| v.as_u64()) == Some(id as u64) {
                    if let Some(err) = resp.get("error") {
                        return Err(format!("CDP error: {}", err));
                    }
                    return Ok(resp.get("result").cloned().unwrap_or(json!({})));
                }
                // else: event or other response, skip
            }
        }
    }
}

/// Validate a URL for browser navigation.
fn validate_url(url: &str) -> Result<(), String> {
    if url.starts_with("https://") || url.starts_with("http://localhost") || url.starts_with("http://127.0.0.1") {
        // Block shell metacharacters
        if url.contains(|c: char| c == '`' || c == '$' || c == ';' || c == '|' || c == '&') {
            return Err("URL contains invalid characters".to_string());
        }
        Ok(())
    } else if url.starts_with("http://") {
        Err("Only https:// URLs or localhost are allowed".to_string())
    } else {
        Err("URL must start with https://".to_string())
    }
}

/// Validate and sanitize a CSS selector.
fn validate_selector(sel: &str) -> Result<String, String> {
    if sel.is_empty() {
        return Err("Selector is required".to_string());
    }
    if sel.len() > MAX_SELECTOR_LEN {
        return Err(format!("Selector too long (max {} chars)", MAX_SELECTOR_LEN));
    }
    // Strip characters that could break JS string injection
    let clean: String = sel.chars()
        .filter(|c| !matches!(c, '`' | '$' | '\\' | '\n' | '\r'))
        .collect();
    if clean.is_empty() {
        return Err("Selector contains only invalid characters".to_string());
    }
    Ok(clean)
}

/// Evaluate a JS expression in the browser and return the string result.
fn eval_js(
    ws: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    expression: &str,
) -> Result<String, String> {
    let result = cdp_send(ws, "Runtime.evaluate", json!({
        "expression": expression,
        "returnByValue": true,
    }))?;

    if let Some(exc) = result.get("exceptionDetails") {
        let text = exc.get("text").and_then(|v| v.as_str()).unwrap_or("JS error");
        return Err(format!("JS exception: {text}"));
    }

    let value = result
        .get("result")
        .and_then(|r| r.get("value"));

    match value {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(v) => Ok(v.to_string()),
        None => Ok(String::new()),
    }
}

/// Check if Chromium CDP is reachable.
fn is_browser_running() -> bool {
    TcpStream::connect_timeout(
        &format!("{}:{}", CDP_HOST, CDP_PORT).parse().unwrap(),
        Duration::from_millis(500),
    )
    .is_ok()
}

// ── Launch Browser ──

pub struct LaunchBrowserTool;

impl Tool for LaunchBrowserTool {
    fn name(&self) -> &'static str { "launch_browser" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "browser" }

    fn definition(&self) -> serde_json::Value {
        json!({
            "type": "function",
            "function": {
                "name": "launch_browser",
                "description": "Launch Chromium browser with remote debugging. Call this before using other browser tools.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {"type": "string", "description": "Optional URL to open (default: about:blank)"}
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        // Check if already running
        if is_browser_running() {
            return "Browser already running (CDP on port 9222).".to_string();
        }

        let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("about:blank");
        if url != "about:blank" {
            if let Err(e) = validate_url(url) {
                return format!("Error: {e}");
            }
        }

        // Launch Chromium with CDP + Wayland support
        // Ensure Wayland env vars are set (worker thread may not have them)
        let result = std::process::Command::new("chromium")
            .args([
                "--ozone-platform=wayland",
                "--remote-debugging-address=127.0.0.1",
                "--remote-debugging-port=9222",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-gpu",
                url,
            ])
            .env("WAYLAND_DISPLAY", "wayland-0")
            .env("XDG_RUNTIME_DIR", "/run/user/1000")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();

        match result {
            Ok(_) => {
                // Wait a moment for CDP to become available
                for _ in 0..10 {
                    std::thread::sleep(Duration::from_millis(500));
                    if is_browser_running() {
                        return format!("Browser launched (CDP on port 9222). Opening: {url}");
                    }
                }
                "Browser process started but CDP not yet reachable. Try again in a few seconds.".to_string()
            }
            Err(e) => format!("Failed to launch Chromium: {e}. Is chromium installed? (apk add chromium)"),
        }
    }
}

// ── Browse (navigate + read) ──

pub struct BrowseTool;

impl Tool for BrowseTool {
    fn name(&self) -> &'static str { "browse" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "browser" }

    fn definition(&self) -> serde_json::Value {
        json!({
            "type": "function",
            "function": {
                "name": "browse",
                "description": "Navigate the browser to a URL and return the page title + text content (first 3000 chars). Uses Chromium with the user's logged-in sessions.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {"type": "string", "description": "URL to navigate to (https:// only)"}
                    },
                    "required": ["url"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let url = args.get("url").and_then(|v| v.as_str()).unwrap_or_default();
        if url.is_empty() {
            return "Error: url is required".to_string();
        }
        if let Err(e) = validate_url(url) {
            return format!("Error: {e}");
        }

        let (mut ws, _tab) = match connect_first_tab() {
            Ok(v) => v,
            Err(e) => return format!("Error: {e}"),
        };

        // Navigate
        if let Err(e) = cdp_send(&mut ws, "Page.navigate", json!({ "url": url })) {
            return format!("Navigation error: {e}");
        }

        // Wait for page to load
        std::thread::sleep(Duration::from_secs(2));

        // Read page title and text
        let title = eval_js(&mut ws, "document.title").unwrap_or_default();
        let text = eval_js(&mut ws, "document.body?.innerText || ''").unwrap_or_default();

        let truncated = if text.len() > MAX_TEXT_CHARS {
            format!("{}...\n(truncated, {} total chars)", &text[..MAX_TEXT_CHARS], text.len())
        } else {
            text
        };

        format!("Title: {title}\nURL: {url}\n\n{truncated}")
    }
}

// ── Browser Read ──

pub struct BrowserReadTool;

impl Tool for BrowserReadTool {
    fn name(&self) -> &'static str { "browser_read" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "browser" }

    fn definition(&self) -> serde_json::Value {
        json!({
            "type": "function",
            "function": {
                "name": "browser_read",
                "description": "Read the current page's text content from the browser (first 3000 chars).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "selector": {"type": "string", "description": "Optional CSS selector to read specific element (default: whole page)"}
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let selector = args.get("selector").and_then(|v| v.as_str()).unwrap_or("");

        let (mut ws, _tab) = match connect_first_tab() {
            Ok(v) => v,
            Err(e) => return format!("Error: {e}"),
        };

        let title = eval_js(&mut ws, "document.title").unwrap_or_default();
        let current_url = eval_js(&mut ws, "window.location.href").unwrap_or_default();

        let js = if selector.is_empty() {
            "document.body?.innerText || ''".to_string()
        } else {
            let sel = match validate_selector(selector) {
                Ok(s) => s,
                Err(e) => return format!("Error: {e}"),
            };
            format!("(document.querySelector('{}')?.innerText || 'Element not found')", sel.replace('\'', "\\'"))
        };

        let text = match eval_js(&mut ws, &js) {
            Ok(t) => t,
            Err(e) => return format!("Error reading page: {e}"),
        };

        let truncated = if text.len() > MAX_TEXT_CHARS {
            format!("{}...\n(truncated, {} total chars)", &text[..MAX_TEXT_CHARS], text.len())
        } else {
            text
        };

        format!("Title: {title}\nURL: {current_url}\n\n{truncated}")
    }
}

// ── Browser Click ──

pub struct BrowserClickTool;

impl Tool for BrowserClickTool {
    fn name(&self) -> &'static str { "browser_click" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "browser" }

    fn definition(&self) -> serde_json::Value {
        json!({
            "type": "function",
            "function": {
                "name": "browser_click",
                "description": "Click an element on the current page by CSS selector.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "selector": {"type": "string", "description": "CSS selector for the element to click (e.g. '#submit-btn', 'a.login')"}
                    },
                    "required": ["selector"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let selector = args.get("selector").and_then(|v| v.as_str()).unwrap_or_default();
        if selector.is_empty() {
            return "Error: selector is required".to_string();
        }

        let sel = match validate_selector(selector) {
            Ok(s) => s,
            Err(e) => return format!("Error: {e}"),
        };

        let (mut ws, _tab) = match connect_first_tab() {
            Ok(v) => v,
            Err(e) => return format!("Error: {e}"),
        };

        let js = format!(
            r#"(() => {{
                const el = document.querySelector('{}');
                if (!el) return 'Element not found: {}';
                el.click();
                return 'Clicked: ' + (el.textContent || el.tagName).substring(0, 100);
            }})()"#,
            sel.replace('\'', "\\'"),
            sel.replace('\'', "\\'"),
        );

        match eval_js(&mut ws, &js) {
            Ok(result) => result,
            Err(e) => format!("Click error: {e}"),
        }
    }
}

// ── Browser Type ──

pub struct BrowserTypeTool;

impl Tool for BrowserTypeTool {
    fn name(&self) -> &'static str { "browser_type" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "browser" }

    fn definition(&self) -> serde_json::Value {
        json!({
            "type": "function",
            "function": {
                "name": "browser_type",
                "description": "Type text into an input element on the current page by CSS selector.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "selector": {"type": "string", "description": "CSS selector for the input (e.g. '#search', 'input[name=q]')"},
                        "text": {"type": "string", "description": "Text to type into the input"},
                        "submit": {"type": "boolean", "description": "Whether to submit the form after typing (default: false)"}
                    },
                    "required": ["selector", "text"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let selector = args.get("selector").and_then(|v| v.as_str()).unwrap_or_default();
        let text = args.get("text").and_then(|v| v.as_str()).unwrap_or_default();
        let submit = args.get("submit").and_then(|v| v.as_bool()).unwrap_or(false);

        if selector.is_empty() || text.is_empty() {
            return "Error: selector and text are required".to_string();
        }

        let sel = match validate_selector(selector) {
            Ok(s) => s,
            Err(e) => return format!("Error: {e}"),
        };

        // Sanitize text for JS string injection
        let safe_text = text
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('\n', "\\n")
            .replace('\r', "");

        if safe_text.len() > 2000 {
            return "Error: text too long (max 2000 chars)".to_string();
        }

        let (mut ws, _tab) = match connect_first_tab() {
            Ok(v) => v,
            Err(e) => return format!("Error: {e}"),
        };

        let submit_js = if submit {
            "if (el.form) el.form.submit(); else el.dispatchEvent(new KeyboardEvent('keydown', {key: 'Enter', keyCode: 13, bubbles: true}));"
        } else {
            ""
        };

        let js = format!(
            r#"(() => {{
                const el = document.querySelector('{}');
                if (!el) return 'Element not found: {}';
                el.focus();
                el.value = '{}';
                el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                {}
                return 'Typed into: ' + (el.name || el.id || el.tagName);
            }})()"#,
            sel.replace('\'', "\\'"),
            sel.replace('\'', "\\'"),
            safe_text,
            submit_js,
        );

        match eval_js(&mut ws, &js) {
            Ok(result) => {
                if submit {
                    // Wait a moment for form submission
                    std::thread::sleep(Duration::from_secs(1));
                }
                result
            }
            Err(e) => format!("Type error: {e}"),
        }
    }
}

// ── Browser Screenshot ──

pub struct BrowserScreenshotTool;

impl Tool for BrowserScreenshotTool {
    fn name(&self) -> &'static str { "browser_screenshot" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "browser" }

    fn definition(&self) -> serde_json::Value {
        json!({
            "type": "function",
            "function": {
                "name": "browser_screenshot",
                "description": "Take a screenshot of the current browser tab. Saves as PNG to /tmp.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let (mut ws, tab) = match connect_first_tab() {
            Ok(v) => v,
            Err(e) => return format!("Error: {e}"),
        };

        let result = match cdp_send(&mut ws, "Page.captureScreenshot", json!({ "format": "png" })) {
            Ok(r) => r,
            Err(e) => return format!("Screenshot error: {e}"),
        };

        let b64_data = match result.get("data").and_then(|v| v.as_str()) {
            Some(d) => d,
            None => return "Error: no screenshot data returned".to_string(),
        };

        // Decode base64 and save to /tmp
        let bytes = match base64_decode(b64_data) {
            Ok(b) => b,
            Err(e) => return format!("Error decoding screenshot: {e}"),
        };

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let path = format!("/tmp/yantrik-screenshot-{ts}.png");

        match std::fs::write(&path, &bytes) {
            Ok(_) => format!(
                "Screenshot saved: {path} ({} bytes)\nPage: {} — {}",
                bytes.len(),
                tab.title,
                tab.url,
            ),
            Err(e) => format!("Error saving screenshot: {e}"),
        }
    }
}

/// Simple base64 decoder (no external dep needed — standard alphabet).
fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [255u8; 256];
    for (i, &b) in TABLE.iter().enumerate() {
        lookup[b as usize] = i as u8;
    }

    let input: Vec<u8> = input.bytes().filter(|b| *b != b'\n' && *b != b'\r' && *b != b' ').collect();
    let mut output = Vec::with_capacity(input.len() * 3 / 4);

    for chunk in input.chunks(4) {
        let mut buf = [0u8; 4];
        let mut count = 0;
        for (i, &b) in chunk.iter().enumerate() {
            if b == b'=' {
                break;
            }
            let val = lookup[b as usize];
            if val == 255 {
                return Err(format!("Invalid base64 character: {}", b as char));
            }
            buf[i] = val;
            count += 1;
        }

        if count >= 2 {
            output.push((buf[0] << 2) | (buf[1] >> 4));
        }
        if count >= 3 {
            output.push((buf[1] << 4) | (buf[2] >> 2));
        }
        if count >= 4 {
            output.push((buf[2] << 6) | buf[3]);
        }
    }

    Ok(output)
}

// ── Browser Tabs ──

pub struct BrowserTabsTool;

impl Tool for BrowserTabsTool {
    fn name(&self) -> &'static str { "browser_tabs" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "browser" }

    fn definition(&self) -> serde_json::Value {
        json!({
            "type": "function",
            "function": {
                "name": "browser_tabs",
                "description": "List all open browser tabs with their titles and URLs.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let tabs = match get_tabs() {
            Ok(t) => t,
            Err(e) => return format!("Error: {e}"),
        };

        if tabs.is_empty() {
            return "No tabs open.".to_string();
        }

        let mut out = format!("{} tab(s) open:\n", tabs.len());
        for (i, tab) in tabs.iter().enumerate() {
            out.push_str(&format!("{}. {} — {}\n", i + 1, tab.title, tab.url));
        }
        out
    }
}

// ── Browser Search ──

pub struct BrowserSearchTool;

impl Tool for BrowserSearchTool {
    fn name(&self) -> &'static str { "browser_search" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "browser" }

    fn definition(&self) -> serde_json::Value {
        json!({
            "type": "function",
            "function": {
                "name": "browser_search",
                "description": "Search the web using the browser. Navigates to Google and returns result text.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "Search query"}
                    },
                    "required": ["query"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or_default();
        if query.is_empty() {
            return "Error: query is required".to_string();
        }

        if query.len() > 500 {
            return "Error: query too long (max 500 chars)".to_string();
        }

        // URL-encode the query
        let encoded: String = query
            .chars()
            .map(|c| match c {
                ' ' => '+'.to_string(),
                c if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') => {
                    c.to_string()
                }
                c => format!("%{:02X}", c as u32),
            })
            .collect();

        let url = format!("https://www.google.com/search?q={}", encoded);

        let (mut ws, _tab) = match connect_first_tab() {
            Ok(v) => v,
            Err(e) => return format!("Error: {e}"),
        };

        // Navigate to search
        if let Err(e) = cdp_send(&mut ws, "Page.navigate", json!({ "url": url })) {
            return format!("Navigation error: {e}");
        }

        // Wait for results to load
        std::thread::sleep(Duration::from_secs(3));

        // Read search results
        let text = eval_js(&mut ws, "document.body?.innerText || ''").unwrap_or_default();

        let truncated = if text.len() > MAX_TEXT_CHARS {
            format!("{}...\n(truncated)", &text[..MAX_TEXT_CHARS])
        } else {
            text
        };

        format!("Search results for: {query}\n\n{truncated}")
    }
}
