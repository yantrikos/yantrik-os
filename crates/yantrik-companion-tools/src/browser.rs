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
const MAX_TEXT_CHARS: usize = 6000;
const MAX_SELECTOR_LEN: usize = 200;
const MAX_ELEMENTS: usize = 250;

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
    reg.register(Box::new(BrowserSnapshotTool));
    reg.register(Box::new(BrowserClickElementTool));
    reg.register(Box::new(BrowserTypeElementTool));
    reg.register(Box::new(BrowserScrollTool));
    reg.register(Box::new(WebSearchTool));
    reg.register(Box::new(BrowserClickXYTool));
    reg.register(Box::new(BrowserTypeXYTool));
    reg.register(Box::new(BrowserLoginTool));
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

/// Inject anti-detection JS to make headless Chrome look like a real browser.
/// Call this via Page.addScriptToEvaluateOnNewDocument so it runs before page JS.
fn inject_stealth(ws: &mut WebSocket<MaybeTlsStream<TcpStream>>) {
    let stealth_js = r#"
        // Override navigator.webdriver
        Object.defineProperty(navigator, 'webdriver', { get: () => false });
        // Add realistic plugins
        Object.defineProperty(navigator, 'plugins', {
            get: () => [1, 2, 3, 4, 5].map(() => ({ length: 1 }))
        });
        // Add realistic languages
        Object.defineProperty(navigator, 'languages', {
            get: () => ['en-US', 'en']
        });
        // Hide automation-related Chrome properties
        window.chrome = { runtime: {} };
        // Override permissions query
        const origQuery = window.Notification && Notification.permission;
        if (origQuery) {
            const handler = { apply: function(target, ctx, args) {
                return args[0].name === 'notifications'
                    ? Promise.resolve({ state: Notification.permission })
                    : Reflect.apply(target, ctx, args);
            }};
        }
    "#;
    let _ = cdp_send(ws, "Page.addScriptToEvaluateOnNewDocument", json!({ "source": stealth_js }));
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
    // Reject truncated URLs (from tool trace ellipsis)
    if url.contains('\u{2026}') || url.ends_with("...") || url.ends_with("…") {
        return Err("URL appears truncated (contains '…'). Use the full URL.".to_string());
    }
    // Reject non-ASCII characters in URLs (they should be percent-encoded)
    if url.contains(|c: char| !c.is_ascii()) {
        return Err("URL contains non-ASCII characters. Percent-encode special characters.".to_string());
    }
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

/// Detect CAPTCHA or bot-protection pages and return solve instructions.
fn detect_captcha(title: &str, text: &str, elements: &str) -> Option<String> {
    let tl = title.to_lowercase();
    let xl = text.to_lowercase();
    let el = elements.to_lowercase();

    // ── Cloudflare challenges ──
    if (tl.contains("just a moment") && (tl.contains("cloudflare") || xl.contains("cloudflare")))
        || (xl.contains("checking your browser") && xl.contains("cloudflare"))
        || (xl.contains("ray id") && xl.contains("cloudflare"))
    {
        return Some(
            "CAPTCHA: Cloudflare challenge detected.\n\
             ACTION: Cloudflare challenges often auto-resolve after a few seconds. \
             Wait 5 seconds and use browser_see() to check. If a checkbox or turnstile \
             widget appears, use browser_click_xy on it. If it persists, use http_fetch \
             or try a different URL/search engine.".to_string()
        );
    }

    // ── reCAPTCHA checkbox ("I'm not a robot") ──
    if xl.contains("i'm not a robot") || xl.contains("i am not a robot") {
        return Some(
            "CAPTCHA: reCAPTCHA checkbox detected.\n\
             SOLVE IT: Use browser_see() to find the checkbox, then browser_click_xy(x, y) \
             on it. After clicking, browser_see() again — if an image grid appeared, \
             analyze the images and click matching tiles, then click Verify.".to_string()
        );
    }

    // ── Image grid CAPTCHA ("select all traffic lights") ──
    if (xl.contains("select all") && (xl.contains("images") || xl.contains("squares") || xl.contains("tiles")))
        || xl.contains("click each image")
        || xl.contains("select all matching")
    {
        return Some(
            "CAPTCHA: Image selection grid detected.\n\
             SOLVE IT: Use browser_see(question=\"Describe the CAPTCHA grid. What does it \
             ask to select? Identify the (x,y) coordinates of ALL matching tiles.\") \
             Then browser_click_xy on each matching tile. Click Verify/Submit when done. \
             browser_see() again to confirm.".to_string()
        );
    }

    // ── Text/code CAPTCHA ──
    if (xl.contains("type the") && (xl.contains("characters") || xl.contains("text") || xl.contains("letters")))
        || (xl.contains("enter the") && xl.contains("code"))
        || xl.contains("solve the captcha") || xl.contains("complete the captcha")
    {
        return Some(
            "CAPTCHA: Text/code input detected.\n\
             SOLVE IT: Use browser_see(question=\"Read the distorted text/code in the \
             CAPTCHA image. What characters or numbers does it show?\") Then \
             browser_type_xy(x, y, \"the_text\") into the input field and submit.".to_string()
        );
    }

    // ── Generic verify-human signals ──
    if xl.contains("verify you are human") || xl.contains("prove you're not a robot")
        || tl.contains("captcha") || tl.contains("are you a robot")
        || (xl.contains("recaptcha") && xl.contains("verify"))
        || xl.contains("hcaptcha")
        || tl.contains("verify you are human")
    {
        return Some(
            "CAPTCHA: Human verification detected.\n\
             SOLVE IT: Use browser_see() to identify the CAPTCHA type and location. \
             Then use browser_click_xy to interact with it. Common patterns:\n\
             - Checkbox: click it\n\
             - Image grid: identify and click matching tiles, then Verify\n\
             - Turnstile/widget: click the challenge area".to_string()
        );
    }

    // ── Hard blocks (not solvable) ──
    if (xl.contains("unusual traffic") && xl.contains("computer"))
        || tl.contains("access denied")
        || (xl.contains("blocked") && xl.contains("automated"))
    {
        return Some(
            "BOT PROTECTION: Hard block — this site rejected automated access.\n\
             Cannot solve this. Alternatives:\n\
             1. Use http_fetch with a different search engine (DuckDuckGo, Bing)\n\
             2. Try accessing via API instead of web scraping\n\
             3. Use a different URL or source".to_string()
        );
    }

    // ── Element-level detection (iframe captcha widgets) ──
    if el.contains("captcha") || el.contains("recaptcha") || el.contains("hcaptcha") {
        return Some(
            "CAPTCHA: Widget detected in page elements.\n\
             SOLVE IT: Use browser_see() to visually inspect the CAPTCHA, then \
             browser_click_xy to interact with it.".to_string()
        );
    }

    None
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

/// Which browser to drive, found rather than assumed.
///
/// Both launchers used to hardcode `chromium`, which is not installed on Debian by default and is
/// a snap-only transitional package on Ubuntu 24.04. On a clean machine the spawn failed and the
/// tools reported nothing useful — the browser simply never appeared.
///
/// Chrome first, because it is the one with the automation surface we depend on and the one most
/// likely to be current; the rest are what people actually have installed.
pub fn chrome_binary() -> Result<String, String> {
    const CANDIDATES: [&str; 6] = [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "brave-browser",
        "microsoft-edge",
    ];
    for name in CANDIDATES {
        if std::process::Command::new("which")
            .arg(name)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Ok(name.to_string());
        }
    }
    Err(format!(
        "no browser found. Looked for: {}. Install one, e.g. `apt install google-chrome-stable`.",
        CANDIDATES.join(", ")
    ))
}

/// Where a browsing identity keeps its cookies, logins and history.
///
/// One profile per identity, under the user's data directory rather than `/tmp`. The old
/// hardcoded `/tmp/chromium-profile` meant one identity for the whole machine — you could not
/// hold a work account and a personal one — and every session died with the tmpdir on reboot.
/// Sessions are the thing worth keeping: they survive where a stored password would have to be
/// replayed through a login form, 2FA and a device check.
pub fn profile_dir(identity: &str) -> std::path::PathBuf {
    let identity = sanitize_identity(identity);
    let base = std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"));
    base.join(".local/share/yantrik/browsers").join(identity)
}

/// An identity names a directory, so it may not wander out of one.
fn sanitize_identity(identity: &str) -> String {
    let cleaned: String = identity
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    if cleaned.is_empty() {
        "default".to_string()
    } else {
        cleaned
    }
}

/// The debugging port for an identity.
///
/// Derived from the name rather than fixed, because the two launchers both used 9222: whichever
/// browser started first owned the port, `is_browser_running` could not tell them apart, and the
/// headless extraction browser would happily drive the user's visible window — or report "already
/// running" when only the other one was up.
pub fn port_for(identity: &str) -> u16 {
    let identity = sanitize_identity(identity);
    if identity == "default" {
        return CDP_PORT;
    }
    // FNV-1a, so the same name always lands on the same port and a caller can find it again.
    let mut hash: u32 = 2166136261;
    for byte in identity.as_bytes() {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(16777619);
    }
    // 9223..=9322, leaving 9222 to the default identity.
    9223 + (hash % 100) as u16
}

/// Check if a browser is listening on this identity's port.
fn is_identity_running(identity: &str) -> bool {
    port_is_open(port_for(identity))
}

fn port_is_open(port: u16) -> bool {
    TcpStream::connect_timeout(
        &format!("{CDP_HOST}:{port}").parse().unwrap(),
        Duration::from_millis(500),
    )
    .is_ok()
}

/// Check if Chromium CDP is reachable.
fn is_browser_running() -> bool {
    TcpStream::connect_timeout(
        &format!("{}:{}", CDP_HOST, CDP_PORT).parse().unwrap(),
        Duration::from_millis(500),
    )
    .is_ok()
}

/// Auto-launch Chromium in headless mode for data-extraction tools (web_search, browse).
/// Returns Ok(()) if browser is available (already running or just launched),
/// Err(msg) if launch failed.
fn ensure_headless_browser() -> Result<(), String> {
    if is_browser_running() {
        return Ok(());
    }

    // Run watchdog cleanup first
    if let Some(warning) = super::browser_lifecycle::watchdog_check() {
        tracing::info!("ensure_headless pre-flight: {}", warning);
    }

    tracing::info!("Auto-launching headless Chromium for data extraction");
    let binary = match chrome_binary() {
        Ok(b) => b,
        Err(e) => return Err(e),
    };
    let profile = profile_dir("default");
    let _ = std::fs::create_dir_all(&profile);

    let result = std::process::Command::new(&binary)
        .args([
            "--headless=new",
            "--ozone-platform=wayland",
            "--remote-debugging-address=127.0.0.1",
            "--remote-debugging-port=9222",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-gpu",
            // Anti-detection: prevent navigator.webdriver=true (primary CAPTCHA trigger)
            "--disable-blink-features=AutomationControlled",
            // Anti-detection: real Chrome user-agent (not "HeadlessChrome")
            "--user-agent=Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
            // Anti-detection: realistic viewport size
            "--window-size=1920,1080",
            // Anti-detection: enable plugins/WebGL to look like real browser
            "--enable-webgl",
            "--enable-features=NetworkService,NetworkServiceInProcess",
            // Anti-detection: disable automation-related infobars
            "--disable-infobars",
            "--excludeSwitches=enable-automation",
            // Anti-detection: use real browser profile dir for persistent cookies/state
        ])
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("about:blank")
        .env("WAYLAND_DISPLAY", "wayland-0")
        .env("XDG_RUNTIME_DIR", "/run/user/1000")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();

    match result {
        Ok(_) => {
            for _ in 0..10 {
                std::thread::sleep(Duration::from_millis(500));
                if is_browser_running() {
                    // Inject stealth scripts before any page loads
                    if let Ok((mut ws, _)) = connect_first_tab() {
                        inject_stealth(&mut ws);
                    }
                    return Ok(());
                }
            }
            Err("Headless browser started but CDP not reachable".to_string())
        }
        Err(e) => Err(format!("Failed to launch headless Chromium: {e}")),
    }
}

/// Navigate current tab to about:blank to clear stale content.
/// Prevents the AI from re-reading its own previously-opened pages as "user activity".
fn cleanup_after_data_extraction(ws: &mut WebSocket<MaybeTlsStream<TcpStream>>) {
    let _ = cdp_send(ws, "Page.navigate", json!({ "url": "about:blank" }));
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
                "description": "Launch browser session for browser_* tools",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {"type": "string", "description": "Optional URL to open (default: about:blank)"},
                        "identity": {
                            "type": "string",
                            "description": "Which browsing identity to use - its own cookies, logins and history. Defaults to 'default'. Use separate identities for separate accounts."
                        },
                        "headless": {"type": "boolean", "description": "Run in headless mode (no visible window). Default: false"}
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        // Run watchdog check before launch — kill zombies from previous sessions
        if let Some(warning) = super::browser_lifecycle::watchdog_check() {
            tracing::info!("launch_browser pre-flight: {}", warning);
        }

        // Check if already running
        if is_browser_running() {
            return "Browser already running (CDP on port 9222).".to_string();
        }

        let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("about:blank");
        let headless = args.get("headless").and_then(|v| v.as_bool()).unwrap_or(false);
        if url != "about:blank" {
            if let Err(e) = validate_url(url) {
                return format!("Error: {e}");
            }
        }

        // Launch Chromium with CDP + Wayland support
        // Ensure Wayland env vars are set (worker thread may not have them)
        // Which account this window is. One profile per identity, so a work account and a
        // personal one are different sessions rather than the same cookie jar.
        let identity = args.get("identity").and_then(|v| v.as_str()).unwrap_or("default");
        let profile = profile_dir(identity);
        if let Err(e) = std::fs::create_dir_all(&profile) {
            return format!("Cannot create the profile directory {}: {e}", profile.display());
        }
        let port = port_for(identity);

        let binary = match chrome_binary() {
            Ok(b) => b,
            Err(e) => return format!("Error: {e}"),
        };

        let mut chrome_args: Vec<String> = vec![
            "--ozone-platform=wayland".into(),
            "--remote-debugging-address=127.0.0.1".into(),
            format!("--remote-debugging-port={port}"),
            // Chrome 2026 refuses a DevTools websocket whose Origin it does not know, which is
            // every connection we make. Without this every CDP call fails the handshake with 403.
            "--remote-allow-origins=*".into(),
            format!("--user-data-dir={}", profile.display()),
            "--no-first-run".into(),
            "--no-default-browser-check".into(),
            // Anti-detection: prevent navigator.webdriver=true (primary CAPTCHA trigger)
            "--disable-blink-features=AutomationControlled".into(),
            // Anti-detection: realistic viewport size
            "--window-size=1920,1080".into(),
        ];
        // Headed by default, and that is the point rather than an oversight. What makes a site
        // trust this browser is not a flag: it is a real profile with a real logged-in session, on
        // the user's own machine and their own address. A headless window on a datacentre IP is
        // what every cloud automation service is fighting, and running on someone's desktop is the
        // one advantage they cannot buy. Headless is for background work on sites that do not care.
        //
        // `--disable-gpu` is deliberately absent when headed: it forces software rendering, and a
        // SwiftShader WebGL string is itself a fingerprint.
        if headless {
            chrome_args.push("--headless=new".into());
            chrome_args.push("--disable-gpu".into());
            // The one signal `--headless=new` still leaks to any script that looks.
            chrome_args.push(
                "--user-agent=Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36".into(),
            );
        }
        chrome_args.push(url.to_string());

        let result = std::process::Command::new(&binary)
            .args(&chrome_args)
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
                        let mode = if headless { "headless" } else { "visible" };
                        return format!("Browser launched in {mode} mode (CDP on port 9222). Opening: {url}");
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
                "description": "Open URL in this controlled browser session",
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

        // Auto-launch headless browser if not running
        if let Err(e) = ensure_headless_browser() {
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
        std::thread::sleep(Duration::from_secs(3));

        // Read page title and text
        let title = eval_js(&mut ws, "document.title").unwrap_or_default();
        let text = eval_js(&mut ws, "document.body?.innerText || ''").unwrap_or_default();

        let max_text = MAX_TEXT_CHARS / 2; // Leave room for elements
        let truncated = if text.len() > max_text {
            format!("{}...\n(truncated, {} total chars)", &text[..text.floor_char_boundary(max_text)], text.len())
        } else {
            text
        };

        // Scan interactive elements
        let elements = eval_js(&mut ws, &scan_js(false)).unwrap_or_default();
        let element_count = if elements.is_empty() { 0 } else { elements.lines().count() };

        let mut out = format!("Title: {title}\nURL: {url}\n\n");

        // Detect CAPTCHA / bot protection
        if let Some(captcha_msg) = detect_captcha(&title, &truncated, &elements) {
            out.push_str(&captcha_msg);
            out.push_str("\n\n");
            out.push_str(&truncated);
            cleanup_after_data_extraction(&mut ws);
            return out;
        }

        out.push_str(&truncated);
        out.push_str("\n\n");
        out.push_str(&format!("--- Interactive Elements ({element_count}) ---\n"));
        if elements.is_empty() {
            out.push_str("(no interactive elements found)\n");
        } else {
            let limited: String = elements.lines()
                .take(MAX_ELEMENTS)
                .collect::<Vec<_>>()
                .join("\n");
            out.push_str(&limited);
            if element_count > MAX_ELEMENTS {
                out.push_str(&format!("\n... and {} more", element_count - MAX_ELEMENTS));
            }
        }
        out.push_str("\n\nUse browser_click_element(N) or browser_type_element(N, text) to interact.");

        // Clear page so stale content isn't mistaken for user activity
        cleanup_after_data_extraction(&mut ws);

        out
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
                "description": "Extract current page text only; no screenshot or clicks",
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
            format!("{}...\n(truncated, {} total chars)", &text[..text.floor_char_boundary(MAX_TEXT_CHARS)], text.len())
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
                "description": "Click page element by CSS selector; not numbered elements",
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
                "description": "Type into field by CSS selector; not numbered elements",
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
                "description": "Save screenshot of current tab as an image file",
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
                "description": "List open browser tabs with titles and URLs",
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
                "description": "Search the web in browser; opens results page",
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
        let title = eval_js(&mut ws, "document.title").unwrap_or_default();
        let text = eval_js(&mut ws, "document.body?.innerText || ''").unwrap_or_default();

        // Check for CAPTCHA/bot protection
        if let Some(captcha_msg) = detect_captcha(&title, &text, "") {
            cleanup_after_data_extraction(&mut ws);
            return format!(
                "Search for '{query}' hit bot protection:\n{captcha_msg}\n\n\
                 STOP retrying Google — use http_fetch with DuckDuckGo instead:\n\
                 http_fetch(url=\"https://html.duckduckgo.com/html/?q=YOUR+QUERY\")"
            );
        }

        let truncated = if text.len() > MAX_TEXT_CHARS {
            format!("{}...\n(truncated)", &text[..text.floor_char_boundary(MAX_TEXT_CHARS)])
        } else {
            text
        };

        cleanup_after_data_extraction(&mut ws);
        format!("Search results for: {query}\n\n{truncated}")
    }
}

// ── Web Search (lightweight core tool, uses Chromium CDP) ──

pub struct WebSearchTool;

impl Tool for WebSearchTool {
    fn name(&self) -> &'static str { "web_search" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "browser" }

    fn definition(&self) -> serde_json::Value {
        json!({
            "type": "function",
            "function": {
                "name": "web_search",
                "description": "Search the web by query; snippets only, no page fetch",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Search query (e.g. 'rust async tutorial', 'fix npm EACCES error')"
                        }
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

        // Try SearXNG first (local, fast, no rate limits)
        match searxng_search(query) {
            Ok(results) => return results,
            Err(e) => tracing::debug!("SearXNG unavailable ({e}), trying browser/DDG"),
        }

        // Auto-launch headless browser if not running — fall back to DuckDuckGo if unavailable
        if let Err(e) = ensure_headless_browser() {
            tracing::info!("Browser unavailable ({e}), using DuckDuckGo HTML fallback");
            return duckduckgo_html_search(query);
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

        let url = format!("https://www.google.com/search?q={}&num=10&hl=en", encoded);

        let (mut ws, _tab) = match connect_first_tab() {
            Ok(v) => v,
            Err(e) => return format!("Error connecting to browser: {e}"),
        };

        // Navigate to Google search
        if let Err(e) = cdp_send(&mut ws, "Page.navigate", json!({ "url": url })) {
            return format!("Navigation error: {e}");
        }

        // Wait for results to load
        std::thread::sleep(Duration::from_secs(3));

        // Extract structured search results via JS
        let js = r#"
            (function() {
                var results = [];
                // Google search result blocks
                var items = document.querySelectorAll('div.g, div[data-hveid]');
                for (var i = 0; i < items.length && results.length < 10; i++) {
                    var el = items[i];
                    var link = el.querySelector('a[href^="http"]');
                    if (!link) continue;
                    var url = link.href;
                    if (url.includes('google.com') || url.includes('accounts.google')) continue;
                    var titleEl = el.querySelector('h3');
                    var title = titleEl ? titleEl.innerText : '';
                    if (!title) continue;
                    var snippetEl = el.querySelector('div[data-sncf], span.aCOpRe, div.VwiC3b, div[style*="line"]');
                    var snippet = snippetEl ? snippetEl.innerText : '';
                    if (!snippet) {
                        // Try getting text from the result block minus the title
                        var allText = el.innerText || '';
                        var parts = allText.split('\n').filter(function(l) { return l.length > 20 && l !== title; });
                        snippet = parts.slice(0, 2).join(' ');
                    }
                    results.push(title + '\n' + url + '\n' + snippet.substring(0, 200));
                }
                if (results.length === 0) {
                    // Fallback: just get page text
                    return 'NO_STRUCTURED_RESULTS\n' + (document.body ? document.body.innerText.substring(0, 4000) : '');
                }
                return results.join('\n---\n');
            })()
        "#;

        let raw = eval_js(&mut ws, js).unwrap_or_default();

        // Check for CAPTCHA on Google search
        let page_title = eval_js(&mut ws, "document.title").unwrap_or_default();
        if let Some(captcha_msg) = detect_captcha(&page_title, &raw, "") {
            cleanup_after_data_extraction(&mut ws);
            tracing::info!("Google CAPTCHA detected, falling back to DuckDuckGo: {captcha_msg}");
            return duckduckgo_html_search(query);
        }

        if raw.starts_with("NO_STRUCTURED_RESULTS") {
            // Fallback to plain text results
            let text = raw.strip_prefix("NO_STRUCTURED_RESULTS\n").unwrap_or(&raw);
            let truncated = if text.len() > 4000 { &text[..text.floor_char_boundary(4000)] } else { text };
            cleanup_after_data_extraction(&mut ws);
            return format!("Search results for: {query}\n\n{truncated}");
        }

        // Parse structured results
        let mut output = format!("Search results for: {query}\n\n");
        for (i, block) in raw.split("\n---\n").enumerate() {
            let lines: Vec<&str> = block.lines().collect();
            if lines.len() >= 2 {
                let title = lines[0];
                let url = lines[1];
                let snippet = if lines.len() > 2 { lines[2..].join(" ") } else { String::new() };
                output.push_str(&format!("{}. {}\n   {}\n   {}\n\n", i + 1, title, url, snippet));
            }
        }

        if output.len() > 5000 {
            let boundary = output.floor_char_boundary(5000);
            output.truncate(boundary);
            output.push_str("\n... (truncated)");
        }

        // Clear page so stale content isn't mistaken for user activity
        cleanup_after_data_extraction(&mut ws);

        output
    }
}

// ── SearXNG local search (primary, fast, no rate limits) ──

/// Default SearXNG base URL. Override via SEARXNG_URL env var.
fn searxng_base_url() -> String {
    std::env::var("SEARXNG_URL").unwrap_or_else(|_| "http://localhost:8888".to_string())
}

/// Search via local SearXNG instance. Returns structured results as JSON API.
fn searxng_search(query: &str) -> Result<String, String> {
    let base = searxng_base_url();
    let encoded: String = query
        .chars()
        .map(|c| match c {
            ' ' => '+'.to_string(),
            c if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') => c.to_string(),
            c => format!("%{:02X}", c as u32),
        })
        .collect();

    let url = format!("{}/search?q={}&format=json&pageno=1", base, encoded);

    let resp = ureq::get(&url)
        .set("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(10))
        .call()
        .map_err(|e| format!("SearXNG request failed: {e}"))?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("SearXNG JSON parse failed: {e}"))?;

    let results = json
        .get("results")
        .and_then(|v| v.as_array())
        .ok_or("No results array in SearXNG response")?;

    if results.is_empty() {
        return Err("SearXNG returned zero results".to_string());
    }

    let mut output = format!("Search results for: {query}\n\n");
    for (i, r) in results.iter().take(10).enumerate() {
        let title = r.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let url = r.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let snippet = r.get("content").and_then(|v| v.as_str()).unwrap_or("");
        output.push_str(&format!("{}. {}\n   {}\n   {}\n\n", i + 1, title, url, snippet));
    }

    tracing::info!(query = %query, count = results.len().min(10), "SearXNG search");
    Ok(output)
}

// ── DuckDuckGo HTML fallback (no browser required) ──

/// Search via DuckDuckGo HTML-only endpoint. Pure HTTP — no browser, no JS, no CAPTCHAs.
fn duckduckgo_html_search(query: &str) -> String {
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

    let url = format!("https://html.duckduckgo.com/html/?q={}", encoded);

    let resp = match ureq::get(&url)
        .set("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
        .set("Accept", "text/html")
        .set("Accept-Language", "en-US,en;q=0.5")
        .call()
    {
        Ok(r) => r,
        Err(e) => return format!("Error: DuckDuckGo search failed: {e}"),
    };

    let body = match resp.into_string() {
        Ok(b) => b,
        Err(e) => return format!("Error: Failed to read DuckDuckGo response: {e}"),
    };

    // Parse DuckDuckGo HTML results
    // Result format: <a class="result__a" href="...">title</a>
    //                <a class="result__snippet" ...>snippet</a>
    let mut results: Vec<(String, String, String)> = Vec::new();
    let mut pos = 0;

    while results.len() < 10 {
        // Find result link: <a rel="nofollow" class="result__a" href="URL">TITLE</a>
        let link_marker = "class=\"result__a\"";
        let Some(marker_pos) = body[pos..].find(link_marker) else { break };
        let marker_abs = pos + marker_pos;
        pos = marker_abs + link_marker.len();

        // Extract href
        let href_start = body[..marker_abs].rfind("href=\"");
        let href = if let Some(hs) = href_start {
            let hs = hs + 6;
            let he = body[hs..].find('"').map(|i| hs + i).unwrap_or(hs);
            &body[hs..he]
        } else {
            continue;
        };

        // Extract title (text between > and </a>)
        let title_start = body[pos..].find('>').map(|i| pos + i + 1).unwrap_or(pos);
        let title_end = body[title_start..].find("</a>").map(|i| title_start + i).unwrap_or(title_start);
        let title = strip_html_basic(&body[title_start..title_end]);

        // Find snippet nearby
        let snippet_marker = "class=\"result__snippet\"";
        let snippet = if let Some(sp) = body[pos..].find(snippet_marker) {
            let sp_abs = pos + sp + snippet_marker.len();
            let snippet_start = body[sp_abs..].find('>').map(|i| sp_abs + i + 1).unwrap_or(sp_abs);
            let snippet_end = body[snippet_start..].find("</a>")
                .or_else(|| body[snippet_start..].find("</td>"))
                .map(|i| snippet_start + i)
                .unwrap_or(snippet_start);
            let raw = strip_html_basic(&body[snippet_start..snippet_end]);
            if raw.len() > 200 {
                format!("{}...", &raw[..raw.floor_char_boundary(200)])
            } else {
                raw
            }
        } else {
            String::new()
        };

        // Decode DuckDuckGo redirect URLs
        let clean_url = if href.contains("uddg=") {
            // Extract actual URL from redirect: //duckduckgo.com/l/?uddg=ENCODED_URL&...
            href.split("uddg=").nth(1)
                .and_then(|u| u.split('&').next())
                .map(|u| url_decode(u))
                .unwrap_or_else(|| href.to_string())
        } else {
            href.to_string()
        };

        if !title.is_empty() && !clean_url.is_empty() {
            results.push((title, clean_url, snippet));
        }
    }

    if results.is_empty() {
        return format!("No results found for: {query}");
    }

    let mut output = format!("Search results for: {query}\n\n");
    for (i, (title, url, snippet)) in results.iter().enumerate() {
        output.push_str(&format!("{}. {}\n   {}\n   {}\n\n", i + 1, title, url, snippet));
    }

    output
}

/// Simple HTML tag stripper for DuckDuckGo result parsing.
fn strip_html_basic(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        if ch == '<' { in_tag = true; }
        else if ch == '>' { in_tag = false; }
        else if !in_tag { result.push(ch); }
    }
    // Decode common HTML entities
    result.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .trim()
        .to_string()
}

/// Simple percent-decode for URLs.
fn url_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            } else {
                result.push('%');
                result.push_str(&hex);
            }
        } else if c == '+' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    result
}

// ── Shared: Interactive element scanning JS ──

/// JS that scans the page for interactive elements, stores them in
/// `window.__yantrik_elements`, and returns a numbered list.
///
/// Two things this does that a plain `querySelectorAll` does not, both measured against real
/// pages:
///
/// **It works out the accessible name.** A bare scan reads `textContent` and gets nothing from an
/// `<input>`, so a login form came back as `[2] input[text] ""` and `[3] input[password] ""` — the
/// two fields that are the entire task, unnamed. Resolving the name the way a screen reader does
/// — `aria-labelledby`, then `aria-label`, then the associated `<label for>`, then a wrapping
/// label — turns those into "Username or email address" and "Password". On Wikipedia 320 of 1795
/// elements had no name at all before this.
///
/// Chromium can compute this itself, over CDP's `Accessibility.getFullAXTree`, and that was the
/// obvious answer until it was timed: 1.65 seconds on a Wikipedia article, because it walks the
/// whole document. Doing that before every click is not fluency. The same names come out of ten
/// lines of JavaScript in the page, in single-digit milliseconds.
///
/// **It knows what is on screen.** A person considers what they can see; a scan of the whole
/// document returned 1795 elements and 124 KB for one Wikipedia page, of which 125 elements and
/// 9.5 KB were actually visible. Every element still gets an index — so a click on something below
/// the fold still works, and indices stay stable across looks — but the report leads with what is
/// in view and says how much is not.
const SCAN_ELEMENTS_JS: &str = r#"(() => {
    // Replaced by the caller: see `scan_js`.
    const showAll = __SHOW_ALL__;
    const sels = 'a[href], button, input, textarea, select, [role="button"], [role="link"], [role="tab"], [role="menuitem"], [contenteditable="true"], summary, details';

    // The accessible name, in the order the accname algorithm resolves it.
    const nameOf = (el) => {
        const byIds = (ids) => ids.split(/\s+/)
            .map(id => document.getElementById(id))
            .filter(Boolean)
            .map(n => (n.innerText || n.textContent || '').trim())
            .join(' ')
            .trim();

        const labelledby = el.getAttribute('aria-labelledby');
        if (labelledby) { const t = byIds(labelledby); if (t) return t; }

        const aria = el.getAttribute('aria-label');
        if (aria && aria.trim()) return aria.trim();

        if (el.id) {
            const lab = document.querySelector('label[for="' + CSS.escape(el.id) + '"]');
            if (lab) { const t = (lab.innerText || lab.textContent || '').trim(); if (t) return t; }
        }
        const wrapping = el.closest('label');
        if (wrapping) { const t = (wrapping.innerText || wrapping.textContent || '').trim(); if (t) return t; }

        const own = (el.innerText || el.textContent || '').trim();
        if (own) return own;

        return (el.placeholder || el.value || el.alt || el.title || '').trim();
    };

    const els = [];
    document.querySelectorAll(sels).forEach(el => {
        const r = el.getBoundingClientRect();
        if (r.width === 0 && r.height === 0 && el.tagName !== 'INPUT') return;
        if (el.disabled) return;
        if (el.closest('[aria-hidden="true"]') && !el.closest('[aria-modal="true"]')) return;
        els.push(el);
    });
    // Indexed before filtering, so an index means the same thing whatever is scrolled into view.
    window.__yantrik_elements = els;

    const visible = [];
    const lines = [];
    els.forEach((el, i) => {
        const r = el.getBoundingClientRect();
        const onscreen = r.bottom > 0 && r.top < innerHeight && r.right > 0 && r.left < innerWidth;

        const tag = el.tagName.toLowerCase();
        const type = el.type || '';
        const role = el.getAttribute('role') || '';
        let text = nameOf(el).replace(/\s+/g, ' ');
        if (text.length > 60) text = text.substring(0, 57) + '...';
        const name = el.name || el.id || '';
        const href = el.href || '';

        let desc = '[' + (i+1) + '] ' + tag;
        if (type && type !== 'submit') desc += '[' + type + ']';
        if (role) desc += '[' + role + ']';
        if (name) desc += ' name="' + name + '"';
        if (text) desc += ' "' + text + '"';
        if (href && tag === 'a') {
            try { desc += ' \u2192 ' + new URL(href).pathname.substring(0, 60); } catch(e) { desc += ' \u2192 ' + href.substring(0, 60); }
        }
        if (tag === 'input' || tag === 'textarea') {
            const val = el.value || '';
            if (val) desc += ' value="' + val.substring(0, 40) + '"';
        }
        if (onscreen || showAll) visible.push(desc); else lines.push(desc);
    });

    const offscreen = lines.length;
    let out = visible.join('\n');
    if (offscreen > 0) {
        // Said rather than silently dropped: an agent that cannot see something must know it is
        // there, or it will conclude the page does not have it and give up.
        out += '\n\n(' + offscreen + ' more not currently on screen \u2014 scroll, or ask for all)';
    }
    return out;
})()"#;

/// The scan, told whether to report everything or only what is in view.
///
/// A substitution rather than two constants: the two versions differed by one boolean and keeping
/// them as separate strings is how they drift apart.
fn scan_js(show_all: bool) -> String {
    SCAN_ELEMENTS_JS.replace("__SHOW_ALL__", if show_all { "true" } else { "false" })
}

// ── Browser Snapshot ──

pub struct BrowserSnapshotTool;

impl Tool for BrowserSnapshotTool {
    fn name(&self) -> &'static str { "browser_snapshot" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "browser" }

    fn definition(&self) -> serde_json::Value {
        json!({
            "type": "function",
            "function": {
                "name": "browser_snapshot",
                "description": "List numbered interactive elements; for element actions",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "all": {
                            "type": "boolean",
                            "description": "Include elements that are not currently on screen. Off by default: the snapshot reports what is visible and says how many more there are."
                        },
                        "include_text": {"type": "boolean", "description": "Also include page text content (default: true)"}
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let include_text = args.get("include_text").and_then(|v| v.as_bool()).unwrap_or(true);
        // What is on screen, unless asked otherwise — a person considers what they can see, and
        // the whole document is thirteen times the bytes for a page like a Wikipedia article.
        let show_all = args.get("all").and_then(|v| v.as_bool()).unwrap_or(false);

        let (mut ws, tab) = match connect_first_tab() {
            Ok(v) => v,
            Err(e) => return format!("Error: {e}"),
        };

        let title = eval_js(&mut ws, "document.title").unwrap_or_default();
        let current_url = eval_js(&mut ws, "window.location.href").unwrap_or_default();

        // Scan interactive elements
        let elements = match eval_js(&mut ws, &scan_js(show_all)) {
            Ok(e) => e,
            Err(e) => return format!("Error scanning elements: {e}"),
        };

        let element_count = if elements.is_empty() { 0 } else { elements.lines().count() };

        let mut out = format!("Page: {title}\nURL: {current_url}\n\n");

        if include_text {
            let text = eval_js(&mut ws, "document.body?.innerText || ''").unwrap_or_default();
            let max = MAX_TEXT_CHARS / 2; // Leave room for elements
            let truncated = if text.len() > max {
                format!("{}...\n(truncated, {} total chars)", &text[..text.floor_char_boundary(max)], text.len())
            } else {
                text.clone()
            };

            // Check for CAPTCHA
            if let Some(captcha_msg) = detect_captcha(&title, &text, &elements) {
                out.push_str(&captcha_msg);
                out.push_str("\n\n");
                out.push_str(&truncated);
                return out;
            }

            out.push_str("--- Page Text ---\n");
            out.push_str(&truncated);
            out.push_str("\n\n");
        }

        out.push_str(&format!("--- Interactive Elements ({element_count}) ---\n"));
        if elements.is_empty() {
            out.push_str("(no interactive elements found)\n");
        } else {
            // Limit to MAX_ELEMENTS to avoid token waste
            let limited: String = elements.lines()
                .take(MAX_ELEMENTS)
                .collect::<Vec<_>>()
                .join("\n");
            out.push_str(&limited);
            if element_count > MAX_ELEMENTS {
                out.push_str(&format!("\n... and {} more (scroll down to see more)", element_count - MAX_ELEMENTS));
            }
        }
        out.push_str("\n\nUse browser_click_element or browser_type_element with the [N] number to interact.");

        out
    }
}

// ── Browser Click Element ──

pub struct BrowserClickElementTool;

impl Tool for BrowserClickElementTool {
    fn name(&self) -> &'static str { "browser_click_element" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "browser" }

    fn definition(&self) -> serde_json::Value {
        json!({
            "type": "function",
            "function": {
                "name": "browser_click_element",
                "description": "Click numbered page element from browser_snapshot",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "element": {"type": "integer", "description": "Element number from browser_snapshot (e.g. 5)"}
                    },
                    "required": ["element"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let num = args.get("element").and_then(|v| v.as_u64()).unwrap_or(0);
        if num == 0 {
            return "Error: element number is required (from browser_snapshot)".to_string();
        }

        let (mut ws, _tab) = match connect_first_tab() {
            Ok(v) => v,
            Err(e) => return format!("Error: {e}"),
        };

        let js = format!(
            r#"(() => {{
                const els = window.__yantrik_elements;
                if (!els) return 'Error: No snapshot. Call browser_snapshot first.';
                const el = els[{idx}];
                if (!el) return 'Error: Element {num} not found (max: ' + els.length + ')';
                el.scrollIntoView({{ block: 'center' }});
                el.focus();
                el.click();
                const tag = el.tagName.toLowerCase();
                const text = (el.textContent || el.value || '').trim().substring(0, 100);
                return 'Clicked [' + {num} + '] ' + tag + ': ' + text;
            }})()"#,
            idx = num - 1,
            num = num,
        );

        match eval_js(&mut ws, &js) {
            Ok(result) => {
                // Wait briefly for any navigation or DOM changes
                std::thread::sleep(Duration::from_millis(500));
                result
            }
            Err(e) => format!("Click error: {e}"),
        }
    }
}

// ── Browser Type Element ──

pub struct BrowserTypeElementTool;

impl Tool for BrowserTypeElementTool {
    fn name(&self) -> &'static str { "browser_type_element" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "browser" }

    fn definition(&self) -> serde_json::Value {
        json!({
            "type": "function",
            "function": {
                "name": "browser_type_element",
                "description": "Type into numbered field from browser_snapshot",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "element": {"type": "integer", "description": "Element number from browser_snapshot"},
                        "text": {"type": "string", "description": "Text to type"},
                        "clear_first": {"type": "boolean", "description": "Clear existing content before typing (default: true)"},
                        "submit": {"type": "boolean", "description": "Press Enter after typing (default: false)"}
                    },
                    "required": ["element", "text"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let num = args.get("element").and_then(|v| v.as_u64()).unwrap_or(0);
        let text = args.get("text").and_then(|v| v.as_str()).unwrap_or_default();
        let clear_first = args.get("clear_first").and_then(|v| v.as_bool()).unwrap_or(true);
        let submit = args.get("submit").and_then(|v| v.as_bool()).unwrap_or(false);

        if num == 0 || text.is_empty() {
            return "Error: element number and text are required".to_string();
        }

        if text.len() > 5000 {
            return "Error: text too long (max 5000 chars)".to_string();
        }

        // Sanitize text for JS string
        let safe_text = text
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('\n', "\\n")
            .replace('\r', "");

        let (mut ws, _tab) = match connect_first_tab() {
            Ok(v) => v,
            Err(e) => return format!("Error: {e}"),
        };

        let clear_js = if clear_first { "el.value = '';" } else { "" };
        let submit_js = if submit {
            "el.dispatchEvent(new KeyboardEvent('keydown', {key:'Enter',keyCode:13,bubbles:true}));"
        } else {
            ""
        };

        let js = format!(
            r#"(() => {{
                const els = window.__yantrik_elements;
                if (!els) return 'Error: No snapshot. Call browser_snapshot first.';
                const el = els[{idx}];
                if (!el) return 'Error: Element {num} not found (max: ' + els.length + ')';
                el.scrollIntoView({{ block: 'center' }});
                el.focus();
                {clear_js}
                // Use native input setter for React/SPA compatibility
                const nativeSet = Object.getOwnPropertyDescriptor(
                    window.HTMLTextAreaElement.prototype, 'value'
                )?.set || Object.getOwnPropertyDescriptor(
                    window.HTMLInputElement.prototype, 'value'
                )?.set;
                if (nativeSet) {{
                    nativeSet.call(el, {clear_val}'{safe_text}');
                }} else {{
                    el.value = {clear_val}'{safe_text}';
                }}
                el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                {submit_js}
                const tag = el.tagName.toLowerCase();
                const name = el.name || el.id || tag;
                return 'Typed into [' + {num} + '] ' + name + ': "' + '{safe_text}'.substring(0, 50) + '"';
            }})()"#,
            idx = num - 1,
            num = num,
            clear_js = clear_js,
            clear_val = if clear_first { "" } else { "el.value + " },
            safe_text = safe_text,
            submit_js = submit_js,
        );

        match eval_js(&mut ws, &js) {
            Ok(result) => result,
            Err(e) => format!("Type error: {e}"),
        }
    }
}

// ── Browser Scroll ──

pub struct BrowserScrollTool;

impl Tool for BrowserScrollTool {
    fn name(&self) -> &'static str { "browser_scroll" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "browser" }

    fn definition(&self) -> serde_json::Value {
        json!({
            "type": "function",
            "function": {
                "name": "browser_scroll",
                "description": "Scroll current page viewport up or down",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "direction": {"type": "string", "enum": ["down", "up", "top", "bottom"], "description": "Scroll direction (default: down)"}
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let direction = args.get("direction").and_then(|v| v.as_str()).unwrap_or("down");

        let (mut ws, _tab) = match connect_first_tab() {
            Ok(v) => v,
            Err(e) => return format!("Error: {e}"),
        };

        let js = match direction {
            "up" => "window.scrollBy(0, -600); 'Scrolled up'",
            "top" => "window.scrollTo(0, 0); 'Scrolled to top'",
            "bottom" => "window.scrollTo(0, document.body.scrollHeight); 'Scrolled to bottom'",
            _ => "window.scrollBy(0, 600); 'Scrolled down'",
        };

        match eval_js(&mut ws, js) {
            Ok(result) => result,
            Err(e) => format!("Scroll error: {e}"),
        }
    }
}

// ── Browser See (Vision + CDP) ──

/// Combined browser screenshot + vision analysis.
/// Takes a screenshot via CDP and sends it to the vision model for analysis.
/// Optionally includes DOM element scan for actionable targets.
pub struct BrowserSeeTool {
    pub ollama_base: String,
    pub model: String,
}

/// Registration for vision-enabled browser tool (called from mod.rs).
pub fn register_vision(reg: &mut ToolRegistry, ollama_base: &str, model: &str) {
    reg.register(Box::new(BrowserSeeTool {
        ollama_base: ollama_base.to_string(),
        model: model.to_string(),
    }));
}

impl Tool for BrowserSeeTool {
    fn name(&self) -> &'static str { "browser_see" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "browser" }

    fn definition(&self) -> serde_json::Value {
        json!({
            "type": "function",
            "function": {
                "name": "browser_see",
                "description": "View page image for vision; use before x,y actions",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "question": {
                            "type": "string",
                            "description": "What to look for or analyze (e.g. 'Where is the sidebar edit field?', 'What buttons are visible?'). Default: describe the page layout and interactive elements."
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let user_question = args.get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("Describe this webpage layout and all visible interactive elements.");

        // Append coordinate instructions so vision model outputs pixel positions.
        let question = format!(
            "{user_question}\n\
             List each interactive element as: description at (x, y)"
        );

        // 1. Take CDP screenshot
        let (mut ws, tab) = match connect_first_tab() {
            Ok(v) => v,
            Err(e) => return format!("Error: {e}"),
        };

        let screenshot = match cdp_send(&mut ws, "Page.captureScreenshot", json!({ "format": "png" })) {
            Ok(r) => r,
            Err(e) => return format!("Screenshot error: {e}"),
        };

        let b64_data = match screenshot.get("data").and_then(|v| v.as_str()) {
            Some(d) => d,
            None => return "Error: no screenshot data".to_string(),
        };

        // 2. Get viewport dimensions (for coordinate reference)
        let viewport = cdp_send(&mut ws, "Runtime.evaluate", json!({
            "expression": "JSON.stringify({w: window.innerWidth, h: window.innerHeight})"
        })).ok()
            .and_then(|r| r["result"]["value"].as_str().map(String::from))
            .and_then(|s| serde_json::from_str::<Value>(&s).ok());
        let (vp_w, vp_h) = viewport.as_ref()
            .map(|v| (v["w"].as_u64().unwrap_or(1280), v["h"].as_u64().unwrap_or(720)))
            .unwrap_or((1280, 720));

        // 3. Save screenshot (for reference)
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let img_path = format!("/tmp/yantrik-see-{ts}.png");
        if let Ok(bytes) = base64_decode(b64_data) {
            let _ = std::fs::write(&img_path, &bytes);
        }

        // Append viewport size so coordinates are grounded
        let question_with_dims = format!(
            "{question}\nViewport: {vp_w}x{vp_h} pixels."
        );

        // 4. Send to vision model
        let payload = serde_json::json!({
            "model": self.model,
            "messages": [{
                "role": "user",
                "content": question_with_dims,
                "images": [b64_data]
            }],
            "stream": false
        });

        let payload_path = "/tmp/yantrik-see-payload.json";
        if let Err(e) = std::fs::write(payload_path, payload.to_string()) {
            return format!("Error writing payload: {e}");
        }

        let url = format!("{}/api/chat", self.ollama_base);
        let output = match std::process::Command::new("curl")
            .args([
                "-fsSL",
                "--max-time", "120",
                "-H", "Content-Type: application/json",
                "-d", &format!("@{payload_path}"),
                &url,
            ])
            .output()
        {
            Ok(o) => o,
            Err(e) => {
                let _ = std::fs::remove_file(payload_path);
                return format!("Vision request failed: {e}");
            }
        };

        let _ = std::fs::remove_file(payload_path);

        if !output.status.success() {
            return format!("Vision model error: {}", String::from_utf8_lossy(&output.stderr));
        }

        let response: serde_json::Value = match serde_json::from_slice(&output.stdout) {
            Ok(v) => v,
            Err(e) => return format!("Invalid vision response: {e}"),
        };

        let vision_text = response["message"]["content"]
            .as_str()
            .unwrap_or("(no vision response)");

        // 4. Also get DOM element scan for actionable targets
        let elements = eval_js(&mut ws, &scan_js(false)).unwrap_or_default();
        let element_count = if elements.is_empty() { 0 } else { elements.lines().count() };

        // 5. Combine vision analysis + element list
        let mut out = format!("Page: {}\nURL: {}\nViewport: {}x{} pixels\n\n", tab.title, tab.url, vp_w, vp_h);
        out.push_str("--- Vision Analysis (with coordinates) ---\n");
        out.push_str(vision_text);
        out.push_str("\n\n");
        out.push_str(&format!("--- Interactive Elements ({element_count}) ---\n"));
        if elements.is_empty() {
            out.push_str("(no interactive elements found)\n");
        } else {
            let limited: String = elements.lines()
                .take(MAX_ELEMENTS)
                .collect::<Vec<_>>()
                .join("\n");
            out.push_str(&limited);
        }
        out.push_str(&format!(
            "\n\nTO INTERACT: Use browser_click_xy(x, y) and browser_type_xy(x, y, text) \
             with pixel coordinates from the vision analysis above. \
             Viewport is {vp_w}x{vp_h}. These work on ANY website.\n\
             Alternatively, use browser_click_element(N) / browser_type_element(N, text) with the [N] numbers."
        ));

        out
    }
}

// ── Browser Click at Coordinates ──
//
// Uses CDP Input.dispatchMouseEvent for real native-level mouse clicks.
// Works with ANY website regardless of framework (React, Angular, Shadow DOM).
// Coordinates come from browser_see screenshots.

pub struct BrowserClickXYTool;

impl Tool for BrowserClickXYTool {
    fn name(&self) -> &'static str { "browser_click_xy" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "browser" }

    fn definition(&self) -> serde_json::Value {
        json!({
            "type": "function",
            "function": {
                "name": "browser_click_xy",
                "description": "Click page at screen coordinates x,y; use visual layout",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "x": {"type": "integer", "description": "X pixel coordinate (from left edge of viewport)"},
                        "y": {"type": "integer", "description": "Y pixel coordinate (from top edge of viewport)"},
                        "double_click": {"type": "boolean", "description": "Double-click instead of single click (default: false)"}
                    },
                    "required": ["x", "y"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let x = args.get("x").and_then(|v| v.as_f64()).unwrap_or(-1.0);
        let y = args.get("y").and_then(|v| v.as_f64()).unwrap_or(-1.0);
        let double_click = args.get("double_click").and_then(|v| v.as_bool()).unwrap_or(false);

        if x < 0.0 || y < 0.0 {
            return "Error: x and y coordinates are required (positive integers)".to_string();
        }

        let (mut ws, _tab) = match connect_first_tab() {
            Ok(v) => v,
            Err(e) => return format!("Error: {e}"),
        };

        let click_count = if double_click { 2 } else { 1 };

        // CDP Input.dispatchMouseEvent: move → press → release
        // This triggers real browser-level mouse events that work with any framework.
        if let Err(e) = cdp_send(&mut ws, "Input.dispatchMouseEvent", json!({
            "type": "mouseMoved",
            "x": x,
            "y": y,
        })) {
            return format!("Mouse move error: {e}");
        }

        if let Err(e) = cdp_send(&mut ws, "Input.dispatchMouseEvent", json!({
            "type": "mousePressed",
            "x": x,
            "y": y,
            "button": "left",
            "clickCount": click_count,
        })) {
            return format!("Mouse press error: {e}");
        }

        if let Err(e) = cdp_send(&mut ws, "Input.dispatchMouseEvent", json!({
            "type": "mouseReleased",
            "x": x,
            "y": y,
            "button": "left",
            "clickCount": click_count,
        })) {
            return format!("Mouse release error: {e}");
        }

        // Brief wait for DOM changes
        std::thread::sleep(Duration::from_millis(500));

        // Try to identify what was clicked using JS
        let what = eval_js(&mut ws, &format!(
            "(() => {{ const el = document.elementFromPoint({x}, {y}); \
             if (!el) return 'empty area'; \
             const tag = el.tagName.toLowerCase(); \
             const text = (el.textContent || el.value || '').trim().substring(0, 80); \
             return tag + (text ? ': ' + text : ''); \
            }})()"
        )).unwrap_or_else(|_| "unknown".to_string());

        format!("Clicked at ({}, {}) — {}", x as i64, y as i64, what)
    }
}

// ── Browser Type at Coordinates ──
//
// Click at coordinates to focus, then use CDP Input for typing.
// Works with shadow DOM, contenteditable, React inputs, etc.

pub struct BrowserTypeXYTool;

impl Tool for BrowserTypeXYTool {
    fn name(&self) -> &'static str { "browser_type_xy" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "browser" }

    fn definition(&self) -> serde_json::Value {
        json!({
            "type": "function",
            "function": {
                "name": "browser_type_xy",
                "description": "Type at screen coordinates x,y; use after browser_see",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "x": {"type": "integer", "description": "X pixel coordinate of the input field"},
                        "y": {"type": "integer", "description": "Y pixel coordinate of the input field"},
                        "text": {"type": "string", "description": "Text to type"},
                        "clear_first": {"type": "boolean", "description": "Select all and delete before typing (default: true)"},
                        "submit": {"type": "boolean", "description": "Press Enter after typing (default: false)"}
                    },
                    "required": ["x", "y", "text"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let x = args.get("x").and_then(|v| v.as_f64()).unwrap_or(-1.0);
        let y = args.get("y").and_then(|v| v.as_f64()).unwrap_or(-1.0);
        let text = args.get("text").and_then(|v| v.as_str()).unwrap_or_default();
        let clear_first = args.get("clear_first").and_then(|v| v.as_bool()).unwrap_or(true);
        let submit = args.get("submit").and_then(|v| v.as_bool()).unwrap_or(false);

        if x < 0.0 || y < 0.0 || text.is_empty() {
            return "Error: x, y coordinates and text are required".to_string();
        }

        if text.len() > 5000 {
            return "Error: text too long (max 5000 chars)".to_string();
        }

        let (mut ws, _tab) = match connect_first_tab() {
            Ok(v) => v,
            Err(e) => return format!("Error: {e}"),
        };

        // 1. Click at coordinates to focus the field
        for evt in &["mouseMoved", "mousePressed", "mouseReleased"] {
            let mut params = json!({ "type": evt, "x": x, "y": y });
            if *evt != "mouseMoved" {
                params["button"] = json!("left");
                params["clickCount"] = json!(1);
            }
            if let Err(e) = cdp_send(&mut ws, "Input.dispatchMouseEvent", params) {
                return format!("Click error: {e}");
            }
        }
        std::thread::sleep(Duration::from_millis(200));

        // 2. Clear existing content if requested (Ctrl+A, then Delete)
        if clear_first {
            let _ = cdp_send(&mut ws, "Input.dispatchKeyEvent", json!({
                "type": "keyDown",
                "key": "a",
                "code": "KeyA",
                "modifiers": 2,
            }));
            let _ = cdp_send(&mut ws, "Input.dispatchKeyEvent", json!({
                "type": "keyUp",
                "key": "a",
                "code": "KeyA",
                "modifiers": 2,
            }));
            let _ = cdp_send(&mut ws, "Input.dispatchKeyEvent", json!({
                "type": "keyDown",
                "key": "Backspace",
                "code": "Backspace",
            }));
            let _ = cdp_send(&mut ws, "Input.dispatchKeyEvent", json!({
                "type": "keyUp",
                "key": "Backspace",
                "code": "Backspace",
            }));
            std::thread::sleep(Duration::from_millis(100));
        }

        // 3. Type text using CDP insertText (handles all frameworks)
        if let Err(e) = cdp_send(&mut ws, "Input.insertText", json!({
            "text": text,
        })) {
            return format!("Type error: {e}");
        }

        // 4. Submit if requested
        if submit {
            let _ = cdp_send(&mut ws, "Input.dispatchKeyEvent", json!({
                "type": "keyDown",
                "key": "Enter",
                "code": "Enter",
            }));
            let _ = cdp_send(&mut ws, "Input.dispatchKeyEvent", json!({
                "type": "keyUp",
                "key": "Enter",
                "code": "Enter",
            }));
            std::thread::sleep(Duration::from_millis(500));
        }

        let preview = if text.len() > 50 { &text[..text.floor_char_boundary(50)] } else { text };
        format!("Typed at ({}, {}): \"{}\"", x as i64, y as i64, preview)
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn an_identity_cannot_wander_out_of_its_directory() {
        // The name reaches us from a tool call, which reaches us from a model, which may have read
        // it off a web page. It names a directory, so it gets to be alphanumeric and nothing else.
        // The property, not the exact spelling: nothing that can traverse a path survives.
        for hostile in ["../../etc", "..", "a/../../b", "/etc/shadow", "x\0y"] {
            let safe = sanitize_identity(hostile);
            assert!(!safe.contains('/'), "{hostile:?} -> {safe:?}");
            assert!(!safe.contains('.'), "{hostile:?} -> {safe:?}");
            assert!(!safe.contains('\0'), "{hostile:?} -> {safe:?}");
        }
        assert_eq!(sanitize_identity("work/personal"), "work-personal");
        assert_eq!(sanitize_identity(""), "default");
        assert_eq!(sanitize_identity("   "), "default");
        assert_eq!(sanitize_identity("company-x_2"), "company-x_2");
    }

    #[test]
    fn each_identity_gets_its_own_port() {
        // The bug this exists for: both launchers hardcoded 9222, so the headless extraction
        // browser and the user's visible window fought over one port and `is_browser_running`
        // could not tell which had won.
        assert_eq!(port_for("default"), CDP_PORT);
        let work = port_for("work");
        let personal = port_for("personal");
        assert_ne!(work, personal);
        assert_ne!(work, CDP_PORT);
        assert!((9223..=9322).contains(&work));
    }

    #[test]
    fn the_same_identity_always_finds_its_own_browser_again() {
        // Derived, not allocated: a second process must land on the same port without a registry.
        assert_eq!(port_for("marketing"), port_for("marketing"));
        assert_eq!(port_for("marketing"), port_for("  marketing  "));
    }

    #[test]
    fn profiles_live_where_they_survive_a_reboot() {
        let dir = profile_dir("work");
        let shown = dir.display().to_string();
        assert!(shown.ends_with("/browsers/work"), "{shown}");
        // The old hardcoded /tmp/chromium-profile lost every session on reboot, and a session is
        // the thing worth keeping.
        assert!(!shown.starts_with("/tmp/chromium"), "{shown}");
    }

    #[test]
    fn the_scan_is_told_which_mode_it_is_in() {
        assert!(scan_js(true).contains("const showAll = true;"));
        assert!(scan_js(false).contains("const showAll = false;"));
        assert!(!scan_js(false).contains("__SHOW_ALL__"), "the placeholder must be substituted");
    }
}

// ── Signing in without showing anyone the password ──

/// JS that finds a login form and reports what it found, naming nothing secret.
///
/// Password field first, because it is the only unambiguous anchor on a page: `input[type=
/// password]`. The username is then whatever visible text field sits nearest *above* it in the
/// document, which is how every login form on the web is laid out. Guessing from names — `user`,
/// `login`, `email`, `identifier` — works until a site calls it `session[login]`, and the
/// positional rule does not care what it is called.
const FIND_LOGIN_FORM_JS: &str = r#"(() => {
    const visible = (el) => {
        const r = el.getBoundingClientRect();
        return r.width > 0 && r.height > 0;
    };

    const passwords = [...document.querySelectorAll('input[type="password"]')].filter(visible);
    if (passwords.length === 0) return JSON.stringify({ found: false, why: 'no password field on this page' });
    const password = passwords[0];

    const all = [...document.querySelectorAll('input')].filter(visible);
    const before = all.slice(0, all.indexOf(password));
    const user = before.reverse().find(el =>
        ['text', 'email', 'tel', ''].includes((el.type || '').toLowerCase())
    ) || null;

    // The submit control, if the form has an obvious one. Not required: many forms submit on
    // Enter, and pressing Enter is what a person does anyway.
    const form = password.closest('form');
    const submit = form
        ? form.querySelector('button[type="submit"], input[type="submit"], button:not([type])')
        : null;

    window.__yantrik_login = { user, password, submit };
    return JSON.stringify({
        found: true,
        origin: location.origin,
        has_user_field: !!user,
        has_submit: !!submit,
        // Named, never valued: enough for a caller to see the right form was found, and nothing
        // that could carry what is typed into it.
        user_field: user ? (user.getAttribute('aria-label') || user.name || user.id || user.type) : null,
    });
})()"#;

/// Focus one of the fields found above, so `Input.insertText` goes to the right place.
fn focus_login_field(
    ws: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    which: &str,
) -> Result<(), String> {
    let js = format!(
        "(() => {{ const f = window.__yantrik_login && window.__yantrik_login.{which}; \
          if (!f) return 'missing'; f.focus(); f.value = ''; return 'ok'; }})()"
    );
    match eval_js(ws, &js)?.as_str() {
        "ok" => Ok(()),
        _ => Err(format!("the {which} field went away before it could be filled")),
    }
}

/// Whether a stored credential belongs to the page currently open.
///
/// The check that makes this tool safe to offer at all. Without it, a page can navigate itself
/// anywhere and be handed the password for somewhere else — which is the entire attack, performed
/// by the defence. Compared on registrable host rather than full URL, because a credential saved
/// for `https://reddit.com` must work on `https://www.reddit.com/login`.
fn origin_matches(stored_url: &str, page_origin: &str) -> bool {
    let host_of = |s: &str| -> String {
        let s = s.trim().trim_end_matches('/');
        let s = s.split("://").last().unwrap_or(s);
        let host = s.split('/').next().unwrap_or(s);
        let host = host.split('@').last().unwrap_or(host);
        let host = host.split(':').next().unwrap_or(host);
        host.trim_start_matches("www.").to_lowercase()
    };
    let stored = host_of(stored_url);
    let page = host_of(page_origin);
    if stored.is_empty() || page.is_empty() {
        return false;
    }
    // Exact host, or the page is a subdomain of the stored host: `accounts.google.com` for a
    // credential saved against `google.com`. Never the reverse, and never a suffix match that
    // would let `notreddit.com` pass for `reddit.com`.
    page == stored || page.ends_with(&format!(".{stored}"))
}

pub struct BrowserLoginTool;

impl Tool for BrowserLoginTool {
    fn name(&self) -> &'static str { "browser_login" }
    /// Sensitive rather than Standard: it reads a credential, even though it never shows one.
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "browser" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "browser_login",
                "description": "Sign in to the page currently open, using a credential from the \
                                vault. The password is typed straight into the form and is never \
                                returned, logged, or shown to you — you get only whether it \
                                worked. Navigate to the site's login page first. Refuses if the \
                                open page is not the site the credential belongs to.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "service": {
                            "type": "string",
                            "description": "The vault entry to use, e.g. 'reddit.com'"
                        },
                        "pin": {
                            "type": "string",
                            "description": "Vault PIN, if the vault is protected. Ask the user."
                        },
                        "submit": {
                            "type": "boolean",
                            "description": "Press the sign-in button afterwards. Default true."
                        }
                    },
                    "required": ["service"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let service = args.get("service").and_then(|v| v.as_str()).unwrap_or("").trim();
        if service.is_empty() {
            return "Error: `service` is required — which vault entry to sign in with.".to_string();
        }
        let pin = args.get("pin").and_then(|v| v.as_str());
        let should_submit = args.get("submit").and_then(|v| v.as_bool()).unwrap_or(true);

        // The vault, before the browser: a locked vault should say so without having touched the
        // page.
        if yantrikdb_core::vault::has_pin(&ctx.db.conn()) {
            match pin {
                None => {
                    return "VAULT_PIN_REQUIRED: the vault is protected. Ask the user for their \
                            vault PIN and pass it as `pin`."
                        .to_string()
                }
                Some(p) => {
                    if !yantrikdb_core::vault::verify_pin(&ctx.db.conn(), p) {
                        return "VAULT_PIN_INVALID: Incorrect PIN. Access denied.".to_string();
                    }
                }
            }
        }

        let enc = match yantrikdb_core::vault::vault_encryption(&ctx.db.conn()) {
            Ok(e) => e,
            Err(e) => return format!("Error: {e}"),
        };
        let entries = match yantrikdb_core::vault::get(&ctx.db.conn(), &enc, service) {
            Ok(e) => e,
            Err(e) => return format!("Error reading the vault: {e}"),
        };
        let Some(entry) = entries.into_iter().next() else {
            return format!("No vault entry for '{service}'. Store one first with vault_store.");
        };

        let (mut ws, _tab) = match connect_first_tab() {
            Ok(v) => v,
            Err(e) => return format!("Error: {e}"),
        };

        let found = match eval_js(&mut ws, FIND_LOGIN_FORM_JS) {
            Ok(v) => v,
            Err(e) => return format!("Error reading the page: {e}"),
        };
        let form: serde_json::Value = match serde_json::from_str(&found) {
            Ok(v) => v,
            Err(e) => return format!("Error: could not read the page's form ({e})"),
        };
        if !form["found"].as_bool().unwrap_or(false) {
            return format!(
                "No login form here: {}. Navigate to the site's sign-in page first.",
                form["why"].as_str().unwrap_or("no password field")
            );
        }

        // The check this tool exists to have. A page can navigate itself anywhere; it must not be
        // able to navigate somewhere and be handed a credential for somewhere else.
        let page_origin = form["origin"].as_str().unwrap_or("");
        let stored_url = entry.url.clone().unwrap_or_else(|| service.to_string());
        if !origin_matches(&stored_url, page_origin) {
            return format!(
                "Refused: the open page is {page_origin}, and the credential for '{service}' \
                 belongs to {stored_url}. A password is only ever typed into the site it was \
                 saved for."
            );
        }

        if form["has_user_field"].as_bool().unwrap_or(false) {
            if let Err(e) = focus_login_field(&mut ws, "user") {
                return format!("Error: {e}");
            }
            // `Input.insertText` rather than assigning `value` in JavaScript: the username never
            // becomes part of a script string, and the page sees the same events it would from a
            // keyboard, which is what frameworks like React actually listen for.
            if let Err(e) = cdp_send(&mut ws, "Input.insertText", json!({ "text": entry.username })) {
                return format!("Error filling the username: {e}");
            }
        }

        if let Err(e) = focus_login_field(&mut ws, "password") {
            return format!("Error: {e}");
        }
        if let Err(e) = cdp_send(&mut ws, "Input.insertText", json!({ "text": entry.password })) {
            // Deliberately does not include the underlying error verbatim in case a transport
            // failure echoes what was being sent.
            let _ = e;
            return "Error: could not fill the password field.".to_string();
        }

        if should_submit {
            let clicked = eval_js(
                &mut ws,
                "(() => { const s = window.__yantrik_login && window.__yantrik_login.submit; \
                  if (s) { s.click(); return 'clicked'; } return 'no-button'; })()",
            )
            .unwrap_or_default();
            if clicked != "clicked" {
                // Enter, which is what a person does on a form with no visible button.
                let _ = cdp_send(&mut ws, "Input.dispatchKeyEvent", json!({
                    "type": "keyDown", "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13
                }));
                let _ = cdp_send(&mut ws, "Input.dispatchKeyEvent", json!({
                    "type": "keyUp", "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13
                }));
            }
            std::thread::sleep(Duration::from_millis(1500));
        }

        // Everything this returns is safe to put in front of a model and to write to the audit
        // log. The username is shown because knowing which account was used is the point; the
        // password appears nowhere in this function's output, by construction.
        let now = eval_js(&mut ws, "location.href").unwrap_or_default();
        format!(
            "Signed in to {service} as {} on {page_origin}.{} Now at {now}. \
             Check the page to confirm — a wrong password looks like a page that did not change.",
            entry.username,
            if should_submit { "" } else { " Form filled, not submitted." }
        )
    }
}

#[cfg(test)]
mod login_tests {
    //! `origin_matches` is the whole safety of `browser_login`. Without it the tool is the attack
    //! it exists to prevent: a page can navigate itself anywhere, and would then be handed the
    //! password for somewhere else.

    use super::*;

    #[test]
    fn a_credential_only_goes_to_its_own_site() {
        assert!(origin_matches("https://reddit.com", "https://reddit.com"));
        assert!(origin_matches("https://reddit.com", "https://www.reddit.com"));
        assert!(origin_matches("https://www.reddit.com/login", "https://reddit.com"));
        // Saved against the bare domain, used on the sign-in subdomain: the ordinary case.
        assert!(origin_matches("https://google.com", "https://accounts.google.com"));
    }

    #[test]
    fn a_lookalike_domain_is_refused() {
        // The attack this is for. Every one of these reads as "reddit.com" to a hurried human and
        // must not to this function.
        for hostile in [
            "https://notreddit.com",
            "https://reddit.com.evil.test",
            "https://evil.test/reddit.com",
            "https://reddit-com.evil.test",
            "https://xreddit.com",
        ] {
            assert!(
                !origin_matches("https://reddit.com", hostile),
                "{hostile} was accepted for reddit.com"
            );
        }
    }

    #[test]
    fn a_parent_domain_does_not_inherit_a_subdomains_credential() {
        // The relationship only runs one way. A credential for an internal host must not be typed
        // into the company's public site.
        assert!(origin_matches("https://example.com", "https://mail.example.com"));
        assert!(!origin_matches("https://mail.example.com", "https://example.com"));
    }

    #[test]
    fn nothing_matches_nothing() {
        // An entry saved with no URL must not silently match every page. `service` is used as the
        // fallback, and if that is empty too the answer is no.
        assert!(!origin_matches("", "https://reddit.com"));
        assert!(!origin_matches("https://reddit.com", ""));
        assert!(!origin_matches("", ""));
    }

    #[test]
    fn ports_and_credentials_in_the_url_do_not_confuse_it() {
        assert!(origin_matches("http://localhost:3000", "http://localhost:8080"));
        assert!(origin_matches("https://user@example.com", "https://example.com"));
        assert!(!origin_matches("https://user@example.com", "https://user@evil.test"));
    }

    #[test]
    fn the_form_scan_never_reports_a_value() {
        // What the page returns about its own form is put in front of a model. It may name the
        // username field; it must never carry what is typed into either field.
        assert!(!FIND_LOGIN_FORM_JS.contains(".value"), "the scan must not read field values");
        assert!(FIND_LOGIN_FORM_JS.contains("has_user_field"));
    }
}
