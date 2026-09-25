//! The headless verifier: does the built game actually play?
//!
//! Playwright's Python package does not install cleanly in this WSL image and its
//! browser download is exactly the kind of network dependency a self-contained kit
//! should not grow, so the verifier drives a Chromium directly over the DevTools
//! protocol instead: HTTP to discover the page target (`ureq`), one websocket for
//! everything else (`tungstenite`), both already workspace dependencies. Any
//! Chromium-class browser will do; the discovery list matches the one the desktop's
//! Browser route uses.
//!
//! The gates are the charter's, in the charter's order:
//!
//!  1. boots — the page loads, the engine starts, frames advance
//!  2. no_console_errors — across the whole session, CDP events and the engine's
//!     own window.onerror collection
//!  3. frame_renders — a real screenshot decodes and is not one flat colour
//!  4. input_moves_player — a dispatched arrow key moves the player's position
//!  5. bot_reaches_win — the greedy bot, steering through the ordinary input
//!     vector, collects every item
//!  6. bot_reaches_lose — the suicidal bot exhausts the lives
//!  7. frame_budget — average frame time under software rendering stays under a
//!     generous budget; clearing it is a pass on any machine, and missing it on
//!     a machine whose own floor is above it says nothing about the game (#97)
//!
//! A gate answers one of two questions, and the report keeps them apart (#97):
//! the first six are about the game (`correctness`), the seventh is about speed
//! on this machine (`performance`). Each gate ends passed, failed, or
//! inconclusive. Inconclusive means the machine could not settle the question —
//! too slow to run a bot to the end of its budget, or too slow for the frame
//! budget to be about the game at all — and it never counts as a pass, but it
//! is also not the game's failure, and every summary line says which of the two
//! it was. The bots are budgeted in the
//! engine's simulated seconds, not the wall clock, because on a machine with no
//! GPU the wall clock is a measure of the renderer (#111).
//!
//! The session logic is split from Chrome discovery and launching
//! (`verify_session` takes a websocket URL), so the unit test runs the entire
//! gate sequence against a fake CDP server — no browser, no display, no luck.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tungstenite::protocol::Message;
use tungstenite::WebSocket;

/// Average frame-time ceiling in milliseconds, measured by the engine over its
/// last 90 frames. Clearing this under software rendering is a real pass — any
/// faster machine clears it too. Missing it is not a real failure: the number
/// is one machine's floor, and "software rendering" is not one speed. On the
/// GPU-less VM 520 (#97) the smallest game this grammar can express averaged
/// 361.3 ms and a fuller one 354.2 — past a slow machine's floor the number
/// measures the renderer, not the game. So this is a pass line, not a fail
/// line: only a runner with a reference frame time measured on its own machine
/// could blame a game for missing it, and no such runner exists yet.
pub const FRAME_BUDGET_MS: f64 = 250.0;

/// The bots' budgets. Simulated seconds are the engine's own clock and mean the
/// same on every machine; the wall clock is only there so a machine that cannot
/// run the simulation at a useful pace ends the check with a sentence about the
/// machine instead of hanging. When the wall ceiling arrives first the gate is
/// inconclusive, never a failure of the game.
#[derive(Debug, Clone, PartialEq)]
pub struct Limits {
    /// Simulated seconds the win bot gets: a base, plus this much per item.
    pub win_sim_secs: (f64, f64),
    /// Simulated seconds the lose bot gets.
    pub lose_sim_secs: f64,
    /// Wall-clock seconds allowed per simulated second of budget. The engine catches
    /// up in fixed steps while a bot drives, so on anything above 2 fps wall and
    /// simulated time run level and this is headroom; below that the check is given up.
    pub wall_per_sim_sec: f64,
    /// How often the engine's state is polled.
    pub poll: Duration,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits { win_sim_secs: (25.0, 5.0), lose_sim_secs: 60.0, wall_per_sim_sec: 3.0, poll: Duration::from_millis(300) }
    }
}

impl Limits {
    fn win_budget(&self, target: usize) -> f64 {
        self.win_sim_secs.0 + self.win_sim_secs.1 * target as f64
    }

    fn wall_ceiling(&self, sim_secs: f64) -> Duration {
        Duration::from_secs_f64(sim_secs * self.wall_per_sim_sec)
    }
}

/// Which question a gate answers. Correctness is about the game: it boots, keeps a
/// clean console, draws, answers input, can be won and lost. Performance is about
/// speed on this machine, which is the renderer's and the hardware's as much as the
/// game's — on a box with no GPU it is mostly theirs.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Correctness,
    Performance,
}

/// What a gate concluded. `Inconclusive` is the machine failing to settle the
/// question — too slow to run a bot to the end of its simulated budget, or a
/// build whose engine predates the clock the check needs. It never satisfies a
/// gate: a report with one is not `passed`. It is also not the game's failure, and
/// the summary lines say so, because a person who reads "the bot never won" goes
/// and softens a game that was fine.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Passed,
    Failed,
    Inconclusive,
}

impl Verdict {
    /// The word serde writes, read back off a verify.json.
    pub fn parse(text: &str) -> Option<Verdict> {
        match text {
            "passed" => Some(Verdict::Passed),
            "failed" => Some(Verdict::Failed),
            "inconclusive" => Some(Verdict::Inconclusive),
            _ => None,
        }
    }

    /// Fold gates into one answer: any failure is a failure, else any open
    /// question leaves the whole question open.
    fn fold<'a>(verdicts: impl Iterator<Item = &'a Verdict>) -> Verdict {
        let mut out = Verdict::Passed;
        for v in verdicts {
            match v {
                Verdict::Failed => return Verdict::Failed,
                Verdict::Inconclusive => out = Verdict::Inconclusive,
                Verdict::Passed => {}
            }
        }
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Gate {
    pub gate: String,
    pub kind: Kind,
    pub verdict: Verdict,
    /// `verdict == Passed`, kept as a plain bool because the CLI's exit code, the
    /// games list and reports written before verdicts existed all read it.
    pub passed: bool,
    pub detail: String,
}

impl Gate {
    /// A gate out of a verify.json written by any version of this verifier: one
    /// from before verdicts existed has only `passed`, which says the same thing
    /// in fewer words.
    pub fn from_value(v: &Value) -> Option<Gate> {
        let gate = v.get("gate")?.as_str()?.to_string();
        let passed = v.get("passed").and_then(|p| p.as_bool()).unwrap_or(false);
        let verdict = v
            .get("verdict")
            .and_then(|s| s.as_str())
            .and_then(Verdict::parse)
            .unwrap_or(if passed { Verdict::Passed } else { Verdict::Failed });
        let detail = v.get("detail").and_then(|d| d.as_str()).unwrap_or("no detail").to_string();
        Some(Gate { kind: gate_kind(&gate), gate, verdict, passed: verdict == Verdict::Passed, detail })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Report {
    pub passed: bool,
    /// The game's verdict: every correctness gate folded together.
    pub correctness: Verdict,
    /// This machine's verdict on the game's speed: the performance gates folded.
    pub performance: Verdict,
    pub when: String,
    pub browser: String,
    pub frame_budget_ms: f64,
    pub gates: Vec<Gate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<String>,
}

impl Report {
    fn new(browser: String) -> Report {
        Report {
            passed: false,
            correctness: Verdict::Inconclusive,
            performance: Verdict::Inconclusive,
            when: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
            browser,
            frame_budget_ms: FRAME_BUDGET_MS,
            gates: Vec::new(),
            screenshot: None,
        }
    }

    fn gate(&mut self, name: &str, verdict: Verdict, detail: impl Into<String>) {
        self.gates.push(Gate {
            gate: name.into(),
            kind: gate_kind(name),
            verdict,
            passed: verdict == Verdict::Passed,
            detail: detail.into(),
        });
    }

    /// Mark every remaining gate as not run, so a report always shows the full
    /// charter list and an early failure explains what never happened.
    fn abandon(&mut self, names: &[&str], reason: &str) {
        for n in names {
            self.gate(n, Verdict::Failed, format!("not run: {reason}"));
        }
    }

    fn finish(mut self) -> Report {
        self.correctness = Verdict::fold(self.gates.iter().filter(|g| g.kind == Kind::Correctness).map(|g| &g.verdict));
        self.performance = Verdict::fold(self.gates.iter().filter(|g| g.kind == Kind::Performance).map(|g| &g.verdict));
        self.passed = self.gates.iter().all(|g| g.verdict == Verdict::Passed);
        self
    }

    /// One line for the job list and the window's banner.
    pub fn summary_line(&self) -> String {
        summary_line(&self.gates)
    }
}

const ALL_GATES: [&str; 7] = [
    "boots",
    "no_console_errors",
    "frame_renders",
    "input_moves_player",
    "bot_reaches_win",
    "bot_reaches_lose",
    "frame_budget",
];

/// Which question each gate answers; only the frame budget is about speed.
pub fn gate_kind(name: &str) -> Kind {
    if name == "frame_budget" {
        Kind::Performance
    } else {
        Kind::Correctness
    }
}

/// The clause every reader gets when a gate is inconclusive.
pub const MACHINE_NOT_GAME: &str = "the machine, not the game";

/// The gate that settles a report: none when every gate passed, otherwise the
/// first failed gate, or — when nothing failed — the first inconclusive one. A
/// failure outranks an inconclusive because a failure is the game's and answers
/// the question; an inconclusive only says this machine could not.
pub fn deciding_gate(gates: &[Gate]) -> Option<&Gate> {
    gates
        .iter()
        .find(|g| g.verdict == Verdict::Failed)
        .or_else(|| gates.iter().find(|g| g.verdict == Verdict::Inconclusive))
}

/// The one sentence for a whole report, shared by the job list, the games list
/// and the banner so a person and a mind read the same words. It names the gate
/// that settled it and says whether that was the game or the machine.
pub fn summary_line(gates: &[Gate]) -> String {
    match deciding_gate(gates) {
        None => format!("passed all {} gates", gates.len()),
        Some(g) if g.verdict == Verdict::Inconclusive => {
            format!("inconclusive at {} ({MACHINE_NOT_GAME}): {}", g.gate, g.detail)
        }
        Some(g) => format!("failed at {}: {}", g.gate, g.detail),
    }
}

// ── Chrome discovery ───────────────────────────────────────────────

/// The same candidate list the desktop's Browser route uses.
pub fn chrome_binary() -> Option<PathBuf> {
    ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser", "brave-browser", "microsoft-edge"]
        .iter()
        .map(PathBuf::from)
        .find(|b| {
            std::process::Command::new(b)
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok()
        })
}

// ── Headless launch ────────────────────────────────────────────────

pub struct Headless {
    child: Child,
    pub ws_url: String,
    pub browser: String,
    profile: PathBuf,
}

impl Drop for Headless {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}

/// Launch a headless Chromium showing the given file, and find its page target.
pub fn launch_headless(html: &Path) -> Result<Headless, String> {
    let binary = chrome_binary().ok_or_else(|| {
        "no Chromium-class browser found (looked for google-chrome, chromium, chromium-browser, brave-browser, microsoft-edge); install one to verify".to_string()
    })?;
    let abs = std::fs::canonicalize(html)
        .map_err(|e| format!("cannot read {}: {e}", html.display()))?;
    let url = format!("file://{}", abs.display());

    let profile = std::env::temp_dir().join(format!("arcade-verify-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&profile);
    std::fs::create_dir_all(&profile).map_err(|e| format!("cannot create a profile directory: {e}"))?;

    let child = Command::new(&binary)
        // Software rendering: WSL here has no GPU path for headless Chromium.
        .args([
            "--headless=new",
            "--use-gl=angle",
            "--use-angle=swiftshader",
            "--enable-unsafe-swiftshader",
            "--no-sandbox",
            "--disable-dev-shm-usage",
            "--mute-audio",
            "--hide-scrollbars",
            "--window-size=1280,720",
            "--remote-debugging-port=0",
        ])
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(&url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", binary.display()))?;

    let mut headless = Headless { child, ws_url: String::new(), browser: binary.display().to_string(), profile: profile.clone() };

    // The port lands in DevToolsActivePort once the debugger is up.
    let port_file = profile.join("DevToolsActivePort");
    let deadline = Instant::now() + Duration::from_secs(30);
    let port = loop {
        if Instant::now() > deadline {
            return Err("the browser did not open its debugging port within 30 s".into());
        }
        if let Ok(text) = std::fs::read_to_string(&port_file) {
            if let Some(first) = text.lines().next() {
                if let Ok(p) = first.trim().parse::<u16>() {
                    break p;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    };

    let base = format!("http://127.0.0.1:{port}");
    let ws_url = page_ws_url(&base)?;
    headless.ws_url = ws_url;
    Ok(headless)
}

/// GET /json/list and take the first page target's debugger URL.
pub fn page_ws_url(http_base: &str) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut last = "no targets yet".to_string();
    while Instant::now() < deadline {
        match ureq::get(&format!("{http_base}/json/list")).call() {
            Ok(resp) => match resp.into_json::<Value>() {
                Ok(list) => {
                    if let Some(targets) = list.as_array() {
                        for t in targets {
                            if t.get("type").and_then(|v| v.as_str()) == Some("page") {
                                if let Some(ws) = t.get("webSocketDebuggerUrl").and_then(|v| v.as_str()) {
                                    return Ok(ws.to_string());
                                }
                            }
                        }
                        last = "the browser listed targets but no page among them".into();
                    }
                }
                Err(e) => last = format!("/json/list answered something that is not JSON: {e}"),
            },
            Err(e) => last = format!("/json/list: {e}"),
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(format!("no page target from the browser ({last})"))
}

// ── The CDP session ────────────────────────────────────────────────

pub struct Session {
    conn: WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>,
    next_id: AtomicU64,
    events: Vec<Value>,
}

impl Session {
    pub fn connect(ws_url: &str) -> Result<Session, String> {
        let (conn, _) = tungstenite::connect(ws_url).map_err(|e| format!("cannot reach the browser's websocket: {e}"))?;
        // Reads must not block forever: the gate loop polls between them. The
        // stream is plain ws:// (the browser listens on 127.0.0.1), and tungstenite
        // only hands out the TcpStream through the enum, not a getter.
        if let tungstenite::stream::MaybeTlsStream::Plain(tcp) = conn.get_ref() {
            let _ = tcp.set_read_timeout(Some(Duration::from_millis(150)));
        }
        let session = Session { conn, next_id: AtomicU64::new(1), events: Vec::new() };
        Ok(session)
    }

    /// Send a command and wait for its answer, parking events on the way.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let msg = json!({ "id": id, "method": method, "params": params });
        self.conn
            .send(Message::Text(msg.to_string()))
            .map_err(|e| format!("the browser websocket refused a {method} command: {e}"))?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if Instant::now() > deadline {
                return Err(format!("{method} did not answer within 30 s"));
            }
            match self.conn.read() {
                Ok(Message::Text(text)) => {
                    if let Ok(v) = serde_json::from_str::<Value>(&text) {
                        if v.get("id").and_then(|i| i.as_u64()) == Some(id) {
                            if let Some(err) = v.get("error") {
                                return Err(format!("{method}: {}", err.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error")));
                            }
                            return Ok(v.get("result").cloned().unwrap_or(json!({})));
                        }
                        if v.get("method").is_some() {
                            self.events.push(v);
                        }
                    }
                }
                Ok(Message::Ping(data)) => {
                    let _ = self.conn.send(Message::Pong(data));
                }
                Ok(Message::Close(_)) => return Err("the browser closed the websocket".into()),
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::TimedOut => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return Err(format!("the browser websocket failed: {e}")),
            }
        }
    }

    /// Collect events that arrived since the last drain, without waiting.
    pub fn drain_events(&mut self) -> Vec<Value> {
        // One non-blocking sweep: read whatever is already buffered.
        loop {
            match self.conn.read() {
                Ok(Message::Text(text)) => {
                    if let Ok(v) = serde_json::from_str::<Value>(&text) {
                        if v.get("method").is_some() {
                            self.events.push(v);
                        }
                    }
                }
                Ok(Message::Ping(data)) => {
                    let _ = self.conn.send(Message::Pong(data));
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) =>
                {
                    break
                }
                Err(_) => break,
            }
        }
        std::mem::take(&mut self.events)
    }

    /// Runtime.evaluate with returnByValue: the engine's hooks all return plain data.
    pub fn evaluate(&mut self, expression: &str) -> Result<Value, String> {
        let result = self.call(
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true }),
        )?;
        if let Some(exc) = result.get("exceptionDetails") {
            let text = exc
                .get("exception")
                .and_then(|e| e.get("description"))
                .and_then(|d| d.as_str())
                .unwrap_or("an exception with no description");
            return Err(format!("the page threw: {text}"));
        }
        Ok(result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }
}

/// Errors a page reports on its own: console.error calls, uncaught exceptions,
/// and browser log entries at error level.
fn event_errors(events: &[Value]) -> Vec<String> {
    let mut out = Vec::new();
    for e in events {
        match e.get("method").and_then(|m| m.as_str()) {
            Some("Runtime.consoleAPICalled") => {
                if e.pointer("/params/type").and_then(|t| t.as_str()) == Some("error") {
                    let text: Vec<String> = e
                        .pointer("/params/args")
                        .and_then(|a| a.as_array())
                        .map(|args| {
                            args.iter()
                                .map(|a| {
                                    a.get("value")
                                        .and_then(|v| v.as_str())
                                        .map(String::from)
                                        .unwrap_or_else(|| a.get("description").and_then(|d| d.as_str()).unwrap_or("?").to_string())
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    out.push(format!("console.error: {}", text.join(" ")));
                }
            }
            Some("Runtime.exceptionThrown") => {
                let text = e
                    .pointer("/params/exceptionDetails/exception/description")
                    .and_then(|d| d.as_str())
                    .or_else(|| e.pointer("/params/exceptionDetails/text").and_then(|d| d.as_str()))
                    .unwrap_or("an uncaught exception");
                out.push(format!("page exception: {text}"));
            }
            Some("Log.entryAdded") => {
                if e.pointer("/params/entry/level").and_then(|l| l.as_str()) == Some("error") {
                    let text = e.pointer("/params/entry/text").and_then(|t| t.as_str()).unwrap_or("?");
                    out.push(format!("browser log: {text}"));
                }
            }
            _ => {}
        }
    }
    out
}

// ── The gate sequence ──────────────────────────────────────────────

/// Run the full verification against an already-discovered page target. This is
/// the half the unit test exercises against a fake CDP server.
pub fn verify_session(ws_url: &str, browser: &str, screenshot_out: Option<&Path>, limits: &Limits) -> Result<Report, String> {
    let mut report = Report::new(browser.to_string());
    let mut session = Session::connect(ws_url)?;
    let mut page_events: Vec<Value> = Vec::new();

    session.call("Runtime.enable", json!({}))?;
    session.call("Log.enable", json!({}))?;
    session.call("Page.enable", json!({}))?;

    // Gate 1: boots. The engine flips status to "playing" only after its first
    // rendered frame, so frames > 0 and status playing means the whole stack —
    // Three.js, WebGL, spec parsing, arena construction — came up.
    let boot = wait_for_state(&mut session, Duration::from_secs(20), &|s| {
        s.get("status").and_then(|v| v.as_str()) == Some("playing")
            && s.get("frames").and_then(|v| v.as_f64()).unwrap_or(0.0) > 0.0
    });
    let (boot_ok, boot_detail, target) = match &boot {
        Some(state) => {
            let frames = state.get("frames").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let target = state.get("target").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            (true, format!("engine up, {frames:.0} frames rendered"), target)
        }
        None => {
            let state = session
                .evaluate("window.__arcade ? JSON.stringify(window.__arcade.state()) : 'no engine'")
                .unwrap_or(Value::Null);
            (false, format!("the engine never reported playing (last state: {state})"), 10usize)
        }
    };
    if !boot_ok {
        report.gate("boots", Verdict::Failed, boot_detail.clone());
        report.abandon(&ALL_GATES[1..], "the game did not boot");
        return Ok(report.finish());
    }

    // Gate 3 runs here (the report keeps the charter's order regardless): a real
    // screenshot of a real frame.
    let shot = capture_screenshot(&mut session);
    let (shot_ok, shot_detail, png_bytes) = match shot {
        Ok(bytes) => match png_distinct_colours(&bytes) {
            Ok(n) if n >= 8 => (true, format!("frame has {n} distinct colours"), Some(bytes)),
            Ok(n) => (false, format!("the frame is nearly blank: only {n} distinct colours"), Some(bytes)),
            Err(e) => (false, format!("the screenshot is not a readable PNG: {e}"), None),
        },
        Err(e) => (false, format!("no screenshot from the browser: {e}"), None),
    };
    if let (Some(bytes), Some(out)) = (&png_bytes, screenshot_out) {
        if let Some(parent) = out.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(out, bytes).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
        report.screenshot = out.file_name().and_then(|n| n.to_str()).map(String::from);
    }

    // From here the verifier drives, so the engine keeps its simulated clock level
    // with the wall clock whatever the frame rate. A build older than the hook has
    // no setPaced; the guard keeps that from being a thrown exception, and the
    // gates below then say the build predates the clock they need.
    session.evaluate("window.__arcade.setPaced && window.__arcade.setPaced(true); 0").ok();

    // Gate 4: input moves the player. A real dispatched key event, through the
    // engine's real handler, into the real movement code. Judged in simulated
    // seconds: 1.4 s of wall clock is a few frames under software rendering, and
    // this gate used to fail a fine game with "moved only 0.36 m" on a slow host.
    let before = poll_state(&mut session);
    let key_result = dispatch_key(&mut session, "KeyW", "w", 87);
    let hold_started = Instant::now();
    std::thread::sleep(Duration::from_millis(1400));
    let after = key_result.ok().and_then(|()| poll_state(&mut session));
    let (input_verdict, input_detail) = judge_input(before.as_ref(), after.as_ref(), hold_started.elapsed());

    // Gate 5: the greedy bot reaches WIN through the ordinary input path. Budgeted
    // in the engine's simulated seconds: the wall clock on a GPU-less machine
    // measures swiftshader, and this gate used to fail fine games with it (#111).
    session.evaluate("window.__arcade.reset(); window.__arcade.setBot('win'); 0").ok();
    let win_budget = limits.win_budget(target);
    let win = run_bot(&mut session, win_budget, limits, &|s| {
        s.get("status").and_then(|v| v.as_str()) == Some("win")
    });
    let (win_verdict, win_detail) = judge_win(&win, target, win_budget);

    // Gate 6: the suicidal bot reaches LOSE, on the same clock.
    session.evaluate("window.__arcade.reset(); window.__arcade.setBot('lose'); 0").ok();
    let lose = run_bot(&mut session, limits.lose_sim_secs, limits, &|s| {
        s.get("status").and_then(|v| v.as_str()) == Some("lose")
    });
    let (lose_verdict, lose_detail) = judge_lose(&lose, limits.lose_sim_secs);

    // Gate 7: frame budget, read after the bots have exercised everything, with the
    // engine back on a person's clock so the measurement is of the game as played.
    session
        .evaluate("window.__arcade.setBot(null); window.__arcade.setPaced && window.__arcade.setPaced(false); window.__arcade.reset(); 0")
        .ok();
    std::thread::sleep(Duration::from_millis(2500));
    let final_state = session.evaluate("JSON.stringify(window.__arcade.state())").unwrap_or(Value::Null);
    let (budget_verdict, budget_detail) = judge_frame_budget(state_num(&final_state, "frameMs"));

    // Gate 2 is judged last so it covers the whole session: CDP events plus the
    // engine's own window.onerror collection.
    page_events.extend(session.drain_events());
    let mut errors = event_errors(&page_events);
    if let Some(arr) = state_num_or_array(&final_state, "errors") {
        errors.extend(arr);
    }
    let engine_errors = engine_error_list(&mut session);
    errors.extend(engine_errors);
    errors.dedup();
    let (console_ok, console_detail) = if errors.is_empty() {
        (true, "the console stayed clean for the whole session".to_string())
    } else {
        let shown: Vec<&str> = errors.iter().map(|s| s.as_str()).take(4).collect();
        (false, format!("{} error(s): {}", errors.len(), shown.join(" | ")))
    };

    // Assemble in the charter's order regardless of execution order.
    let as_verdict = |ok: bool| if ok { Verdict::Passed } else { Verdict::Failed };
    let executed: Vec<(&str, Verdict, String)> = vec![
        ("boots", as_verdict(boot_ok), boot_detail),
        ("no_console_errors", as_verdict(console_ok), console_detail),
        ("frame_renders", as_verdict(shot_ok), shot_detail),
        ("input_moves_player", input_verdict, input_detail),
        ("bot_reaches_win", win_verdict, win_detail),
        ("bot_reaches_lose", lose_verdict, lose_detail),
        ("frame_budget", budget_verdict, budget_detail),
    ];
    for (name, verdict, detail) in executed {
        report.gate(name, verdict, detail);
    }
    report.gates.sort_by_key(|g| ALL_GATES.iter().position(|n| *n == g.gate).unwrap_or(99));

    Ok(report.finish())
}

// ── The bots, on the engine's clock ────────────────────────────────

/// How a bot's run ended.
#[derive(Debug, Clone, PartialEq)]
enum BotRun {
    /// The engine reached the state the bot was sent for.
    Reached(Value),
    /// The engine's clock ran the whole simulated budget and it never did: the game.
    Exhausted(Value),
    /// The wall ceiling arrived while the simulation was still short of its
    /// budget: this machine could not run the check to its end.
    TooSlow { last: Value, wall: Duration },
    /// The engine reports no simulated clock — a build from before it had one —
    /// and the wall ceiling arrived without the state the bot was sent for.
    NoClock { last: Value },
    /// The engine stopped answering state queries.
    Silent,
}

/// Drive one bot until the predicate holds, the simulated budget is spent, or the
/// wall ceiling arrives, whichever comes first. The engine's `elapsed` is
/// simulated seconds of play since the last reset — the bot's own clock, which
/// runs at the same rate on every machine that can keep 2 fps.
fn run_bot(session: &mut Session, sim_secs: f64, limits: &Limits, pred: &dyn Fn(&Value) -> bool) -> BotRun {
    let start = Instant::now();
    let ceiling = limits.wall_ceiling(sim_secs);
    let mut last: Option<Value> = None;
    let mut has_clock = false;
    loop {
        if let Some(state) = poll_state(session) {
            if pred(&state) {
                return BotRun::Reached(state);
            }
            match state.get("elapsed").and_then(|v| v.as_f64()) {
                Some(elapsed) if elapsed >= sim_secs => return BotRun::Exhausted(state),
                Some(_) => has_clock = true,
                None => {}
            }
            last = Some(state);
        }
        let wall = start.elapsed();
        if wall >= ceiling {
            return match last {
                None => BotRun::Silent,
                Some(last) if has_clock => BotRun::TooSlow { last, wall },
                Some(last) => BotRun::NoClock { last },
            };
        }
        std::thread::sleep(limits.poll);
    }
}

/// The input gate's verdict and sentence, from the engine's state before and
/// after the held key. Half a metre is the bar; the slowest legal player (4 m/s)
/// clears it in a fraction of a simulated second, so a player that did not move
/// it in a whole one is the game's fault. When the simulation itself did not get
/// a whole second in the wall time it had, the machine could not settle it.
fn judge_input(before: Option<&Value>, after: Option<&Value>, wall: Duration) -> (Verdict, String) {
    let (Some(before), Some(after)) = (before, after) else {
        return (Verdict::Failed, "the engine stopped answering state queries".to_string());
    };
    let pos = |s: &Value| Some((s.get("x")?.as_f64()?, s.get("z")?.as_f64()?));
    let (Some((x0, z0)), Some((x1, z1))) = (pos(before), pos(after)) else {
        return (Verdict::Failed, "the engine's state carries no player position".to_string());
    };
    let d = ((x1 - x0).powi(2) + (z1 - z0).powi(2)).sqrt();
    let clock = |s: &Value| s.get("elapsed").and_then(|v| v.as_f64());
    let sim = match (clock(before), clock(after)) {
        (Some(t0), Some(t1)) => Some(t1 - t0),
        _ => None,
    };
    match sim {
        _ if d > 0.5 => (
            Verdict::Passed,
            match sim {
                Some(sim) => format!("holding W for {sim:.1} simulated seconds moved the player {d:.2} m"),
                None => format!("holding W moved the player {d:.2} m"),
            },
        ),
        Some(sim) if sim >= 1.0 => (
            Verdict::Failed,
            format!("holding W for {sim:.1} simulated seconds moved the player only {d:.2} m"),
        ),
        Some(sim) => {
            let frame_ms = after.get("frameMs").and_then(|v| v.as_f64()).unwrap_or(0.0);
            (
                Verdict::Inconclusive,
                format!(
                    "in {:.1} s of wall clock this machine simulated only {sim:.1} s of the held key (average frame {frame_ms:.1} ms under software rendering); the player moved {d:.2} m",
                    wall.as_secs_f64()
                ),
            )
        }
        None => (Verdict::Inconclusive, NO_CLOCK.to_string()),
    }
}

/// The win gate's verdict and sentence. Pure, so the sentences can be tested
/// without a browser: the sentence is what a person acts on, and the wrong one
/// sends them off to soften a game that was fine.
fn judge_win(run: &BotRun, target: usize, budget: f64) -> (Verdict, String) {
    let collected = |s: &Value| s.get("collected").and_then(|v| v.as_u64()).unwrap_or(0);
    let elapsed = |s: &Value| s.get("elapsed").and_then(|v| v.as_f64()).unwrap_or(0.0);
    match run {
        BotRun::Reached(s) => (
            Verdict::Passed,
            format!("the win bot collected all {} items in {:.0} simulated seconds", collected(s), elapsed(s)),
        ),
        BotRun::Exhausted(s) => (
            Verdict::Failed,
            format!(
                "the win bot never won: {} of {target} collected in its whole budget of {budget:.0} simulated seconds of play",
                collected(s)
            ),
        ),
        BotRun::TooSlow { last, wall } => (Verdict::Inconclusive, too_slow(last, wall, budget, &format!("the win bot had collected {} of {target}", collected(last)))),
        BotRun::NoClock { .. } => (Verdict::Inconclusive, NO_CLOCK.to_string()),
        BotRun::Silent => (Verdict::Failed, "the engine stopped answering state queries".to_string()),
    }
}

fn judge_lose(run: &BotRun, budget: f64) -> (Verdict, String) {
    let lives = |s: &Value| s.get("lives").and_then(|v| v.as_i64()).unwrap_or(-1);
    let elapsed = |s: &Value| s.get("elapsed").and_then(|v| v.as_f64()).unwrap_or(0.0);
    match run {
        BotRun::Reached(s) => (
            Verdict::Passed,
            format!("the lose bot burned every life in {:.0} simulated seconds (lives now {})", elapsed(s), lives(s)),
        ),
        BotRun::Exhausted(s) => (
            Verdict::Failed,
            format!(
                "the lose bot never lost: {} lives left after its whole budget of {budget:.0} simulated seconds of play",
                lives(s)
            ),
        ),
        BotRun::TooSlow { last, wall } => (Verdict::Inconclusive, too_slow(last, wall, budget, &format!("the lose bot had {} lives left", lives(last)))),
        BotRun::NoClock { .. } => (Verdict::Inconclusive, NO_CLOCK.to_string()),
        BotRun::Silent => (Verdict::Failed, "the engine stopped answering state queries".to_string()),
    }
}

const NO_CLOCK: &str = "this build's engine reports no simulated clock (it was built before the bots were judged by one); run `build` again and verify the new file";

/// The frame-budget gate's verdict and sentence, pure so both machines are
/// testable without a browser. This gate has no reference frame time of its
/// own to compare against (FRAME_BUDGET_MS documents what that costs it), so it
/// can only clear a game or decline to judge it: under the budget is a pass on
/// any machine, and over it the measurement is either a slow game or a slow
/// machine — indistinguishable on a machine whose floor is above the budget,
/// which is the machine #97 was filed from. It is never the game's failure on
/// this evidence.
fn judge_frame_budget(frame_ms: Option<f64>) -> (Verdict, String) {
    match frame_ms {
        None => (
            Verdict::Inconclusive,
            "the engine stopped reporting frame times, so this machine could not measure the gate".to_string(),
        ),
        Some(ms) if ms <= FRAME_BUDGET_MS => (
            Verdict::Passed,
            format!("average frame {ms:.1} ms under software rendering, budget {FRAME_BUDGET_MS:.0} ms"),
        ),
        Some(ms) => (
            Verdict::Inconclusive,
            format!(
                "average frame {ms:.1} ms under software rendering, budget {FRAME_BUDGET_MS:.0} ms; this machine sits below the budget's own floor, where the number measures the renderer and not the game"
            ),
        ),
    }
}

/// The sentence for a machine that could not run a bot to the end of its budget:
/// the wall time it had, how little of the simulation that bought, and the
/// frame time that explains it.
fn too_slow(last: &Value, wall: &Duration, budget: f64, progress: &str) -> String {
    let elapsed = last.get("elapsed").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let frame_ms = last.get("frameMs").and_then(|v| v.as_f64()).unwrap_or(0.0);
    format!(
        "in {:.0} s of wall clock this machine simulated only {elapsed:.0} of the {budget:.0} s budget (average frame {frame_ms:.1} ms under software rendering); {progress}",
        wall.as_secs_f64()
    )
}

/// One poll of the engine's state, parsed.
fn poll_state(session: &mut Session) -> Option<Value> {
    let v = session.evaluate("window.__arcade ? JSON.stringify(window.__arcade.state()) : 'null'").ok()?;
    serde_json::from_str::<Value>(v.as_str()?).ok().filter(|s| s.is_object())
}

fn engine_error_list(session: &mut Session) -> Vec<String> {
    match session.evaluate("window.__arcade ? JSON.stringify(window.__arcade.state().errors) : '[]'") {
        Ok(v) => {
            if let Some(text) = v.as_str().map(String::from).or_else(|| Some(v.to_string())) {
                if let Ok(list) = serde_json::from_str::<Vec<String>>(&text) {
                    return list.into_iter().map(|e| format!("window.onerror: {e}")).collect();
                }
            }
            Vec::new()
        }
        Err(_) => Vec::new(),
    }
}

/// Poll the engine's state until the predicate holds. The hook returns an object;
/// asking for its JSON keeps the websocket traffic to plain strings.
fn wait_for_state(session: &mut Session, budget: Duration, pred: &dyn Fn(&Value) -> bool) -> Option<Value> {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        if let Ok(v) = session.evaluate("window.__arcade ? JSON.stringify(window.__arcade.state()) : 'null'") {
            if let Some(text) = v.as_str() {
                if let Ok(state) = serde_json::from_str::<Value>(text) {
                    if pred(&state) {
                        return Some(state);
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    None
}

/// The evaluate results come back as JSON text; pull a field back out.
fn state_num(v: &Value, field: &str) -> Option<f64> {
    let state = serde_json::from_str::<Value>(v.as_str()?).ok()?;
    state.get(field)?.as_f64()
}

fn state_num_or_array(v: &Value, field: &str) -> Option<Vec<String>> {
    let state = serde_json::from_str::<Value>(v.as_str()?).ok()?;
    let arr = state.get(field)?.as_array()?;
    Some(arr.iter().map(|e| format!("window.onerror: {}", e.as_str().unwrap_or("?"))).collect())
}

/// A real key event: keyDown with the same code/key/virtual-key numbers a
/// keyboard would produce, a hold, then keyUp.
fn dispatch_key(session: &mut Session, code: &str, key: &str, vk: u32) -> Result<(), String> {
    session.call(
        "Input.dispatchKeyEvent",
        json!({ "type": "keyDown", "code": code, "key": key, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk }),
    )?;
    std::thread::sleep(Duration::from_millis(120));
    session.call(
        "Input.dispatchKeyEvent",
        json!({ "type": "keyUp", "code": code, "key": key, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk }),
    )?;
    // The engine listens on window, and a held key repeats: hold it by re-sending
    // keyDown a few times so one dispatch is not one 16 ms nudge.
    for _ in 0..8 {
        std::thread::sleep(Duration::from_millis(120));
        session.call(
            "Input.dispatchKeyEvent",
            json!({ "type": "keyDown", "code": code, "key": key, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk }),
        )?;
    }
    session.call(
        "Input.dispatchKeyEvent",
        json!({ "type": "keyUp", "code": code, "key": key, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk }),
    )?;
    Ok(())
}

fn capture_screenshot(session: &mut Session) -> Result<Vec<u8>, String> {
    let result = session.call("Page.captureScreenshot", json!({ "format": "png" }))?;
    let data = result
        .get("data")
        .and_then(|d| d.as_str())
        .ok_or_else(|| "the screenshot answer carried no data".to_string())?;
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|e| format!("the screenshot is not valid base64: {e}"))
}

// ── The non-blank check, pure ──────────────────────────────────────

/// Count distinct colours in a PNG. A WebGL context that failed, a canvas that
/// never drew, or a black-screen crash all come back as one or two colours; a
/// rendered arena comes back with sky, ground, walls, creature, items and shadow
/// at minimum. Eight is a floor anything real clears with room to spare.
pub fn png_distinct_colours(bytes: &[u8]) -> Result<usize, String> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().map_err(|e| format!("{e}"))?;
    // png 0.18 answers None when the frame size is not knowable up front (it is
    // for every screenshot we get); the fallback is a generous RGBA bound so
    // next_frame still has room and the count degrades to "unreadable", not panic.
    let upper_bound = {
        let info = reader.info();
        info.width as usize * info.height as usize * 4 + info.height as usize * 8
    };
    let mut buf = vec![0u8; reader.output_buffer_size().unwrap_or(upper_bound)];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("{e}"))?;
    let channels = info.color_type.samples();
    let mut seen = std::collections::HashSet::new();
    // Sample every fourth pixel: a 1280x720 frame has nearly a million, and
    // distinct-colour counting does not need all of them.
    let mut px = 0;
    while px + channels <= buf.len() {
        seen.insert([buf[px], buf[px + 1], buf[px + 2]]);
        if seen.len() > 64 {
            break; // clearly not blank; stop counting
        }
        px += channels * 4;
    }
    Ok(seen.len())
}

// ── The whole job, start to finish ─────────────────────────────────

/// Verify a built game: find a browser, launch it headless on the file, run the
/// gates, and leave a screenshot beside the game if asked.
pub fn verify_game(html: &Path, screenshot_out: Option<&Path>) -> Result<Report, String> {
    let headless = launch_headless(html)?;
    verify_session(&headless.ws_url, &headless.browser, screenshot_out, &Limits::default())
}

/// Just take the screenshot — the `screenshot` action and the CLI's one-file mode.
pub fn screenshot_game(html: &Path, png_out: &Path) -> Result<(), String> {
    let headless = launch_headless(html)?;
    let mut session = Session::connect(&headless.ws_url)?;
    session.call("Runtime.enable", json!({}))?;
    // Give the engine a moment to render something worth photographing.
    wait_for_state(&mut session, Duration::from_secs(15), &|s| {
        s.get("frames").and_then(|v| v.as_f64()).unwrap_or(0.0) > 5.0
    });
    let bytes = capture_screenshot(&mut session)?;
    if let Some(parent) = png_out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    std::fs::write(png_out, &bytes).map_err(|e| format!("cannot write {}: {e}", png_out.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    // ── Pure pieces ───────────────────────────────────────────────

    fn solid_png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, w, h);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            // write_image_data wants the whole frame in one call, rows back to back.
            let data = rgb.repeat((w * h) as usize);
            writer.write_image_data(&data).unwrap();
        }
        out
    }

    /// An image where every pixel is its own colour — the stand-in for a rendered
    /// frame, which the distinct-colour gate must wave through.
    fn gradient_png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, w, h);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            let mut data = Vec::with_capacity((w * h * 3) as usize);
            for i in 0..(w * h) {
                data.push((i * 7 % 251) as u8);
                data.push((i * 13 % 251) as u8);
                data.push((i * 29 % 251) as u8);
            }
            writer.write_image_data(&data).unwrap();
        }
        out
    }

    #[test]
    fn a_flat_frame_is_blank() {
        let png_bytes = solid_png(64, 64, [12, 12, 12]);
        assert_eq!(png_distinct_colours(&png_bytes).unwrap(), 1);
    }

    #[test]
    fn a_rendered_frame_is_not_blank() {
        // The counter samples every fourth pixel: 32 pixels, 8 samples, 8 colours.
        let png_bytes = gradient_png(8, 4);
        assert_eq!(png_distinct_colours(&png_bytes).unwrap(), 8);
    }

    #[test]
    fn junk_is_refused_as_a_sentence() {
        let err = png_distinct_colours(b"not a png at all").unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn event_errors_pick_out_console_exceptions_and_logs() {
        let events = vec![
            json!({"method": "Runtime.consoleAPICalled", "params": {"type": "error", "args": [{"value": "THREE says no"}]}}),
            json!({"method": "Runtime.consoleAPICalled", "params": {"type": "log", "args": [{"value": "harmless"}]}}),
            json!({"method": "Runtime.exceptionThrown", "params": {"exceptionDetails": {"text": "Uncaught", "exception": {"description": "TypeError: x is not a function"}}}}),
            json!({"method": "Log.entryAdded", "params": {"entry": {"level": "error", "text": "GL failure"}}}),
            json!({"method": "Log.entryAdded", "params": {"entry": {"level": "info", "text": "fine"}}}),
        ];
        let errors = event_errors(&events);
        assert_eq!(errors.len(), 3);
        assert!(errors[0].contains("THREE says no"));
        assert!(errors[1].contains("TypeError"));
        assert!(errors[2].contains("GL failure"));
    }

    #[test]
    fn slug_of_a_report_lists_every_gate_in_order() {
        let mut report = Report::new("test".into());
        report.gate("boots", Verdict::Passed, "up");
        report.abandon(&ALL_GATES[1..], "the game did not boot");
        let report = report.finish();
        assert!(!report.passed);
        assert_eq!(report.correctness, Verdict::Failed);
        let names: Vec<&str> = report.gates.iter().map(|g| g.gate.as_str()).collect();
        assert_eq!(names, ALL_GATES);
        assert!(report.gates[1].detail.contains("did not boot"));
    }

    #[test]
    fn an_inconclusive_gate_never_passes_the_report_and_names_the_machine() {
        let mut report = Report::new("test".into());
        for name in &ALL_GATES[..4] {
            report.gate(name, Verdict::Passed, "fine");
        }
        report.gate("bot_reaches_win", Verdict::Inconclusive, "only 12 of 95 s simulated");
        report.gate("bot_reaches_lose", Verdict::Passed, "fine");
        report.gate("frame_budget", Verdict::Passed, "fine");
        let report = report.finish();
        assert!(!report.passed, "inconclusive must never satisfy a required check");
        assert_eq!(report.correctness, Verdict::Inconclusive);
        assert_eq!(report.performance, Verdict::Passed);
        let line = report.summary_line();
        assert!(line.starts_with("inconclusive at bot_reaches_win"), "{line}");
        assert!(line.contains(MACHINE_NOT_GAME), "{line}");
        assert!(line.contains("12 of 95"), "{line}");
    }

    #[test]
    fn a_failure_outranks_an_inconclusive_in_the_summary() {
        let mut report = Report::new("test".into());
        report.gate("boots", Verdict::Passed, "up");
        report.gate("bot_reaches_win", Verdict::Inconclusive, "too slow");
        report.gate("frame_budget", Verdict::Failed, "average frame 354.2 ms");
        let report = report.finish();
        assert_eq!(report.correctness, Verdict::Inconclusive);
        assert_eq!(report.performance, Verdict::Failed);
        assert_eq!(report.summary_line(), "failed at frame_budget: average frame 354.2 ms");
    }

    #[test]
    fn a_gate_from_an_old_report_gets_its_verdict_from_passed() {
        let old = Gate::from_value(&json!({ "gate": "bot_reaches_win", "passed": false, "detail": "never won" })).unwrap();
        assert_eq!(old.verdict, Verdict::Failed);
        assert_eq!(old.kind, Kind::Correctness);
        let new = Gate::from_value(&json!({ "gate": "frame_budget", "passed": false, "verdict": "inconclusive", "detail": "x" })).unwrap();
        assert_eq!(new.verdict, Verdict::Inconclusive);
        assert_eq!(new.kind, Kind::Performance);
        assert!(Gate::from_value(&json!({ "passed": true })).is_none(), "a gate without a name is not a gate");
    }

    #[test]
    fn the_win_judge_blames_the_game_only_when_the_simulation_ran_its_budget() {
        let state = json!({ "collected": 5, "target": 14, "elapsed": 95.0, "frameMs": 16.0 });
        let (v, detail) = judge_win(&BotRun::Exhausted(state.clone()), 14, 95.0);
        assert_eq!(v, Verdict::Failed);
        assert!(detail.contains("5 of 14"), "{detail}");
        assert!(detail.contains("95 simulated seconds"), "{detail}");

        let stalled = json!({ "collected": 5, "target": 14, "elapsed": 19.0, "frameMs": 308.5 });
        let (v, detail) = judge_win(&BotRun::TooSlow { last: stalled, wall: Duration::from_secs(95) }, 14, 95.0);
        assert_eq!(v, Verdict::Inconclusive);
        assert!(detail.contains("this machine"), "{detail}");
        assert!(detail.contains("19 of the 95 s"), "{detail}");
        assert!(detail.contains("308.5 ms under software rendering"), "{detail}");
        assert!(detail.contains("5 of 14"), "{detail}");

        let (v, detail) = judge_win(&BotRun::NoClock { last: json!({}) }, 14, 95.0);
        assert_eq!(v, Verdict::Inconclusive);
        assert!(detail.contains("`build`"), "{detail}");

        let (v, _) = judge_win(&BotRun::Reached(json!({ "collected": 14, "elapsed": 40.0 })), 14, 95.0);
        assert_eq!(v, Verdict::Passed);
    }

    #[test]
    fn the_frame_budget_judge_clears_a_game_but_never_blames_a_slow_machine_for_its_floor() {
        // The dev box's numbers (#97's own table): clears the bar, a real pass.
        let (v, detail) = judge_frame_budget(Some(48.0));
        assert_eq!(v, Verdict::Passed);
        assert!(detail.contains("48.0 ms"), "{detail}");
        assert!(detail.contains("250 ms"), "{detail}");
        // VM 520's numbers: the seven-berries game at 354.2 ms, and the floor
        // probe — the smallest game the grammar can express — at 361.3. Both
        // over budget, indistinguishable from each other; the machine cannot
        // settle the question, so it must not be reported as the game's.
        let (v, detail) = judge_frame_budget(Some(354.2));
        assert_eq!(v, Verdict::Inconclusive, "over budget on an uncalibrated machine is not a game failure: {detail}");
        assert!(detail.contains("354.2"), "{detail}");
        assert!(detail.contains("250"), "{detail}");
        assert!(detail.contains("renderer"), "{detail}");
        let (v, _) = judge_frame_budget(Some(361.3));
        assert_eq!(v, Verdict::Inconclusive);
        // Exactly on the budget still passes, as it always did.
        assert_eq!(judge_frame_budget(Some(FRAME_BUDGET_MS)).0, Verdict::Passed);
        // No frame times at all: the machine could not measure the gate either.
        let (v, detail) = judge_frame_budget(None);
        assert_eq!(v, Verdict::Inconclusive);
        assert!(detail.contains("stopped reporting"), "{detail}");
    }

    // ── The fake CDP server ───────────────────────────────────────
    // A whole verification run, browser-free: the test server answers the same
    // methods Chrome would, and walks the engine state through boots → input
    // move → win → lose on a script. If the gate sequence, the polling, the
    // event collection or the report shape breaks, this fails.

    struct Fake {
        port: u16,
        stop: Arc<AtomicBool>,
    }

    /// What the fake engine does once the win bot is set.
    #[derive(Clone, Copy, PartialEq)]
    enum Script {
        /// The bot wins; nothing else happens.
        Clean,
        /// The bot wins, and one console.error is pushed mid-session, so both
        /// verdicts of the console gate are covered.
        PlantError,
        /// The bot never wins and the engine's clock runs past the whole budget:
        /// the game's failure.
        WinExhausted,
        /// The bot never wins and the engine's clock barely moves per poll: a
        /// machine too slow to run the check.
        TooSlow,
        /// The bot never wins and the engine reports no `elapsed` at all: a build
        /// from before the clock existed.
        NoClock,
        /// The held key barely moves the player because the machine simulated a
        /// fraction of a second in the time the key was held; the bots then win
        /// and lose as normal.
        InputTooSlow,
        /// Everything is clean and the game plays, but the final frame time is
        /// VM 520's from #97: over the absolute budget on a machine whose own
        /// floor is above it, which the gate must not report as the game's.
        SlowFloor,
    }

    fn state_json(status: &str, x: f64, z: f64, collected: u32, lives: u32, frames: u32, frame_ms: f64, elapsed: Option<f64>) -> Value {
        let mut s = json!({
            "status": status, "collected": collected, "target": 5, "lives": lives,
            "x": x, "z": z, "frameMs": frame_ms, "frames": frames, "bot": null, "errors": []
        });
        if let Some(e) = elapsed {
            s["elapsed"] = json!(e);
        }
        s
    }

    impl Fake {
        fn start(script: Script) -> Fake {
            let plant_error = script == Script::PlantError;
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let stop = Arc::new(AtomicBool::new(false));
            let stop2 = stop.clone();
            std::thread::spawn(move || {
                let (stream, _) = match listener.accept() {
                    Ok(s) => s,
                    Err(_) => return,
                };
                stream.set_read_timeout(Some(Duration::from_millis(300))).ok();
                let mut ws = match tungstenite::accept(stream) {
                    Ok(ws) => ws,
                    Err(_) => return,
                };
                // Scripted engine: the fake advances one phase per interesting call.
                let mut phase = 0u32; // 0 boot, 1 input-before, 2 input-after, 3 win, 4 lose, 5+ final
                let mut planted = false;
                let mut win_polls = 0u32;
                loop {
                    if stop2.load(Ordering::SeqCst) {
                        return;
                    }
                    let msg = match ws.read() {
                        Ok(m) => m,
                        Err(tungstenite::Error::Io(ref e))
                            if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) =>
                        {
                            continue
                        }
                        Err(_) => return,
                    };
                    let text = match msg {
                        Message::Text(t) => t,
                        Message::Close(_) => return,
                        _ => continue,
                    };
                    let req: Value = match serde_json::from_str(&text) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let id = req.get("id").cloned().unwrap_or(Value::Null);
                    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
                    let result: Value = match method {
                        "Runtime.enable" | "Log.enable" | "Page.enable" => json!({}),
                        "Page.captureScreenshot" => {
                            let b64 = base64::engine::general_purpose::STANDARD.encode(gradient_png(16, 16));
                            json!({ "data": b64 })
                        }
                        "Input.dispatchKeyEvent" => {
                            if phase < 2 {
                                phase = 2; // a key arrived: the player is "moving"
                            }
                            json!({})
                        }
                        "Runtime.evaluate" => {
                            let expr = req.pointer("/params/expression").and_then(|e| e.as_str()).unwrap_or("");
                            if expr.contains("setBot('win')") {
                                phase = phase.max(3);
                                json!({ "result": { "type": "number", "value": 0 } })
                            } else if expr.contains("setBot('lose')") {
                                phase = phase.max(4);
                                json!({ "result": { "type": "number", "value": 0 } })
                            } else if expr.contains("setBot(null)") {
                                phase = phase.max(5);
                                json!({ "result": { "type": "number", "value": 0 } })
                            } else if expr.contains("__arcade.state()") {
                                // Advance: boot completes on the first poll; the
                                // input gate sees movement only after a real key
                                // dispatch moved the fake to phase 2.
                                if phase == 0 {
                                    phase = 1;
                                }
                                if phase == 3 {
                                    win_polls += 1;
                                }
                                let s = match phase {
                                    1 => state_json("playing", 0.0, 0.0, 0, 3, 40, 16.0, Some(0.0)),
                                    2 if script == Script::InputTooSlow => state_json("playing", 0.0, -0.36, 0, 3, 43, 512.8, Some(0.15)),
                                    2 => state_json("playing", 0.0, -1.8, 0, 3, 90, 16.0, Some(1.4)),
                                    // The win phase is where the scripts differ: a
                                    // win, a game that runs its whole simulated
                                    // budget without one, a clock that crawls, or
                                    // no clock at all.
                                    3 => match script {
                                        Script::WinExhausted => state_json("playing", 2.0, -3.0, 3, 3, 400, 17.0, Some(999.0)),
                                        Script::TooSlow => state_json("playing", 2.0, -3.0, 3, 3, 400, 308.5, Some(win_polls as f64 * 0.5)),
                                        Script::NoClock => state_json("playing", 2.0, -3.0, 3, 3, 400, 17.0, None),
                                        _ => state_json("win", 2.0, -3.0, 5, 3, 400, 17.0, Some(31.2)),
                                    },
                                    4 => state_json("lose", 1.0, 1.0, 2, 0, 700, 18.0, Some(12.0)),
                                    // The SlowFloor machine plays everything cleanly;
                                    // only the final frame time differs — VM 520's.
                                    _ => state_json("playing", 0.0, 0.0, 0, 3, 900, if script == Script::SlowFloor { 354.2 } else { 16.5 }, Some(2.5)),
                                };
                                json!({ "result": { "type": "string", "value": s.to_string() } })
                            } else {
                                json!({ "result": { "type": "string", "value": "null" } })
                            }
                        }
                        _ => json!({}),
                    };
                    let answer = json!({ "id": id, "result": result });
                    if ws.send(Message::Text(answer.to_string())).is_err() {
                        return;
                    }
                    // One console error in the middle of the session must sink the
                    // console gate: push it unsolicited once the run is under way.
                    if plant_error && !planted && phase >= 3 {
                        planted = true;
                        let event = json!({
                            "method": "Runtime.consoleAPICalled",
                            "params": { "type": "error", "args": [{ "value": "planted failure" }] }
                        });
                        if ws.send(Message::Text(event.to_string())).is_err() {
                            return;
                        }
                    }
                }
            });
            Fake { port, stop }
        }

        fn ws_url(&self) -> String {
            format!("ws://127.0.0.1:{}/devtools/page/FAKE", self.port)
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
        }
    }

    /// Budgets a test can afford to wait out: the fake's target is 5, so the win
    /// budget is 50 simulated seconds, and the wall ceiling is one second.
    fn quick_limits() -> Limits {
        Limits { wall_per_sim_sec: 0.02, poll: Duration::from_millis(40), ..Limits::default() }
    }

    #[test]
    fn a_clean_session_passes_every_gate() {
        let fake = Fake::start(Script::Clean);
        let shot_dir = std::env::temp_dir().join(format!("arcade-fake-shot-clean-{}", std::process::id()));
        let shot = shot_dir.join("screenshot.png");
        let report = verify_session(&fake.ws_url(), "fake-chrome", Some(&shot), &quick_limits()).unwrap();

        let names: Vec<&str> = report.gates.iter().map(|g| g.gate.as_str()).collect();
        assert_eq!(names, ALL_GATES, "the report must list the charter's gates in the charter's order");
        for g in &report.gates {
            assert!(g.passed, "gate {} should pass against a clean fake: {}", g.gate, g.detail);
            assert_eq!(g.verdict, Verdict::Passed);
        }
        assert!(report.passed);
        assert_eq!(report.correctness, Verdict::Passed);
        assert_eq!(report.performance, Verdict::Passed);
        assert_eq!(report.summary_line(), "passed all 7 gates");
        assert_eq!(report.browser, "fake-chrome");
        let win = report.gates.iter().find(|g| g.gate == "bot_reaches_win").unwrap();
        assert!(win.detail.contains("31 simulated seconds"), "{}", win.detail);

        // The screenshot was written where the caller asked, and it decodes.
        assert!(shot.exists());
        assert_eq!(report.screenshot.as_deref(), Some("screenshot.png"));
        assert!(png_distinct_colours(&std::fs::read(&shot).unwrap()).unwrap() >= 4);
        let _ = std::fs::remove_dir_all(&shot_dir);
    }

    #[test]
    fn a_planted_console_error_fails_exactly_the_console_gate() {
        let fake = Fake::start(Script::PlantError);
        let report = verify_session(&fake.ws_url(), "fake-chrome", None, &quick_limits()).unwrap();
        assert_eq!(report.screenshot, None, "no path asked for, no screenshot recorded");
        for g in &report.gates {
            if g.gate == "no_console_errors" {
                assert!(!g.passed, "the planted error must sink the console gate");
                assert_eq!(g.verdict, Verdict::Failed);
                assert!(g.detail.contains("planted failure"), "{}", g.detail);
            } else {
                assert!(g.passed, "gate {} should be untouched by the planted error: {}", g.gate, g.detail);
            }
        }
        assert!(!report.passed, "one failed gate fails the run");
        assert_eq!(report.correctness, Verdict::Failed);
    }

    /// Every gate but one, checked to have passed, so each script proves it touches
    /// exactly the gate it is about.
    fn all_but(report: &Report, gate: &str) {
        for g in &report.gates {
            if g.gate != gate {
                assert_eq!(g.verdict, Verdict::Passed, "gate {} should be untouched: {}", g.gate, g.detail);
            }
        }
    }

    fn all_but_win_passed(report: &Report) {
        all_but(report, "bot_reaches_win");
    }

    #[test]
    fn the_input_judge_blames_the_game_only_when_the_simulation_ran_a_whole_second() {
        let at = |z: f64, elapsed: Option<f64>, frame_ms: f64| {
            let mut s = json!({ "x": 0.0, "z": z, "frameMs": frame_ms });
            if let Some(e) = elapsed {
                s["elapsed"] = json!(e);
            }
            s
        };
        let wall = Duration::from_millis(2500);
        // Moved: passed, on any clock.
        let (v, d) = judge_input(Some(&at(0.0, Some(0.0), 16.0)), Some(&at(-1.8, Some(1.4), 16.0)), wall);
        assert_eq!(v, Verdict::Passed);
        assert!(d.contains("1.4 simulated seconds"), "{d}");
        // A whole simulated second and still nothing: the game.
        let (v, d) = judge_input(Some(&at(0.0, Some(0.0), 16.0)), Some(&at(-0.1, Some(2.4), 16.0)), wall);
        assert_eq!(v, Verdict::Failed);
        assert!(d.contains("only 0.10 m"), "{d}");
        // A fraction of a simulated second: this machine (the real numbers from a
        // 512 ms/frame software renderer, which the old gate called the game's).
        let (v, d) = judge_input(Some(&at(0.0, Some(0.0), 512.8)), Some(&at(-0.36, Some(0.15), 512.8)), wall);
        assert_eq!(v, Verdict::Inconclusive);
        assert!(d.contains("this machine"), "{d}");
        assert!(d.contains("only 0.1 s"), "{d}");
        assert!(d.contains("512.8 ms under software rendering"), "{d}");
        // No clock at all and no movement: rebuild.
        let (v, d) = judge_input(Some(&at(0.0, None, 16.0)), Some(&at(-0.2, None, 16.0)), wall);
        assert_eq!(v, Verdict::Inconclusive);
        assert!(d.contains("`build`"), "{d}");
        // No clock but it moved: passed, in the old words.
        let (v, d) = judge_input(Some(&at(0.0, None, 16.0)), Some(&at(-1.2, None, 16.0)), wall);
        assert_eq!(v, Verdict::Passed);
        assert_eq!(d, "holding W moved the player 1.20 m");
        // The engine went quiet.
        assert_eq!(judge_input(Some(&at(0.0, None, 16.0)), None, wall).0, Verdict::Failed);
    }

    #[test]
    fn a_machine_too_slow_to_hold_a_key_is_inconclusive_not_a_bad_game() {
        let fake = Fake::start(Script::InputTooSlow);
        let report = verify_session(&fake.ws_url(), "fake-chrome", None, &quick_limits()).unwrap();
        all_but(&report, "input_moves_player");
        let input = report.gates.iter().find(|g| g.gate == "input_moves_player").unwrap();
        assert_eq!(input.verdict, Verdict::Inconclusive);
        assert!(input.detail.contains("this machine"), "{}", input.detail);
        assert!(input.detail.contains("0.36 m"), "{}", input.detail);
        assert!(!report.passed);
        assert_eq!(report.correctness, Verdict::Inconclusive);
        assert!(report.summary_line().starts_with("inconclusive at input_moves_player"), "{}", report.summary_line());
    }

    #[test]
    fn a_bot_that_runs_its_simulated_budget_without_winning_fails_the_game() {
        let fake = Fake::start(Script::WinExhausted);
        let report = verify_session(&fake.ws_url(), "fake-chrome", None, &quick_limits()).unwrap();
        all_but_win_passed(&report);
        let win = report.gates.iter().find(|g| g.gate == "bot_reaches_win").unwrap();
        assert_eq!(win.verdict, Verdict::Failed);
        assert!(win.detail.contains("3 of 5"), "{}", win.detail);
        assert!(win.detail.contains("50 simulated seconds"), "{}", win.detail);
        assert!(!report.passed);
        assert_eq!(report.correctness, Verdict::Failed);
        assert_eq!(report.performance, Verdict::Passed);
        assert!(report.summary_line().starts_with("failed at bot_reaches_win"), "{}", report.summary_line());
    }

    #[test]
    fn a_machine_too_slow_to_run_the_bot_is_inconclusive_not_a_bad_game() {
        // This is #111: the engine's clock creeps because every frame is a
        // software-rendered 300 ms, the wall ceiling arrives first, and the report
        // must say the machine could not settle it — not that the bot never won.
        let fake = Fake::start(Script::TooSlow);
        let report = verify_session(&fake.ws_url(), "fake-chrome", None, &quick_limits()).unwrap();
        all_but_win_passed(&report);
        let win = report.gates.iter().find(|g| g.gate == "bot_reaches_win").unwrap();
        assert_eq!(win.verdict, Verdict::Inconclusive);
        assert!(!win.passed);
        assert!(win.detail.contains("this machine"), "{}", win.detail);
        assert!(win.detail.contains("of the 50 s budget"), "{}", win.detail);
        assert!(win.detail.contains("308.5 ms under software rendering"), "{}", win.detail);
        assert!(!win.detail.contains("never won"), "{}", win.detail);
        assert!(!report.passed, "inconclusive must never pass the report");
        assert_eq!(report.correctness, Verdict::Inconclusive);
        assert_eq!(report.performance, Verdict::Passed);
        let line = report.summary_line();
        assert!(line.contains(MACHINE_NOT_GAME), "{line}");
    }

    #[test]
    fn a_build_with_no_simulated_clock_is_told_to_rebuild() {
        let fake = Fake::start(Script::NoClock);
        let report = verify_session(&fake.ws_url(), "fake-chrome", None, &quick_limits()).unwrap();
        all_but_win_passed(&report);
        let win = report.gates.iter().find(|g| g.gate == "bot_reaches_win").unwrap();
        assert_eq!(win.verdict, Verdict::Inconclusive);
        assert!(win.detail.contains("`build`"), "{}", win.detail);
        assert!(!report.passed);
    }

    #[test]
    fn a_machine_whose_floor_is_over_the_frame_budget_is_inconclusive_not_a_bad_game() {
        // This is #97: VM 520 has no GPU, its software renderer runs even the
        // smallest expressible game at ~360 ms, and the whole report went red at
        // `frame_budget` as if the game were broken. The correctness of a game
        // that plays perfectly on such a machine must stay `passed`, the
        // performance question must stay open with the measurement attached,
        // and the report must still not pass overall.
        let fake = Fake::start(Script::SlowFloor);
        let report = verify_session(&fake.ws_url(), "fake-chrome", None, &quick_limits()).unwrap();
        all_but(&report, "frame_budget");
        let budget = report.gates.iter().find(|g| g.gate == "frame_budget").unwrap();
        assert_eq!(budget.verdict, Verdict::Inconclusive);
        assert!(!budget.passed);
        assert!(budget.detail.contains("354.2 ms"), "{}", budget.detail);
        assert!(!budget.detail.contains("failed"), "{}", budget.detail);
        assert_eq!(report.correctness, Verdict::Passed);
        assert_eq!(report.performance, Verdict::Inconclusive);
        assert!(!report.passed, "inconclusive must never satisfy a required check");
        let line = report.summary_line();
        assert!(line.starts_with("inconclusive at frame_budget"), "{line}");
        assert!(line.contains(MACHINE_NOT_GAME), "{line}");
    }
}
