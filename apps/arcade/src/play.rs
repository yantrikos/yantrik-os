//! Playing a game: hand the built file to the desktop's Browser, do not become one.
//!
//! The charter says `play` reuses the browser route, and that is what this does. The
//! shell's Browser launch already carries `--remote-debugging-port=9222`, so Arcade
//! asks the shell to open the Browser like any other caller, waits for the debugging
//! port to answer, and opens the game in a new tab over plain DevTools HTTP. No
//! second browser stack, no window of its own, and the person can see the game in the
//! same Browser they already use — with its tab bar, its zoom, its everything.
//!
//! Opening the tab is not the same as the game drawing. On a desktop whose Browser has
//! no WebGL context at all, the HUD renders over a white rectangle and nothing else
//! happens, and for a while `play` called that "done". So after the tab opens, `play`
//! attaches to it the way the verifier attaches to its headless page and reads the
//! engine's own account: frames advancing means the arena is on screen; `status: nogl`
//! means the renderer could not be made and the tab is a white box. Only the first is
//! reported as done. The second is reported as what it is, and Arcade falls back to
//! the headless screenshot it can already take, so the person still sees their game.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::verify;

const DEBUGGER: &str = "http://127.0.0.1:9222";

/// How long the opened tab gets to report its first frame. The verifier allows the
/// same for a headless boot; a desktop Browser with a GPU is well under it, and a
/// Browser with no WebGL says so in the first few hundred milliseconds.
const FIRST_FRAME_BUDGET: Duration = Duration::from_secs(20);

/// A `file://` URL for a local path, percent-encoded the way Chrome's DevTools
/// expects it in `/json/new?<url>`.
pub fn file_url(path: &Path) -> String {
    let text = path.display().to_string();
    let mut out = String::from("file://");
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Is a browser already listening on the debugging port?
fn debugger_alive(base: &str) -> bool {
    ureq::get(&format!("{base}/json/version"))
        .timeout(Duration::from_millis(700))
        .call()
        .is_ok()
}

/// The tab the Browser opened for us: its target id, and the websocket the page
/// can be watched through (Chrome names it in the same `/json/new` answer).
#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub id: String,
    pub ws_url: Option<String>,
}

/// Ask the browser to open a URL in a new tab. Newer Chromes insist this is a PUT.
pub fn open_tab(base: &str, url: &str) -> Result<Tab, String> {
    let response = ureq::put(&format!("{base}/json/new?{url}"))
        .timeout(Duration::from_secs(5))
        .call()
        .map_err(|e| format!("the Browser's debugging port refused the new tab: {e}"))?;
    let body: Value = response
        .into_json()
        .map_err(|e| format!("the Browser answered something that is not JSON: {e}"))?;
    let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
    if id.is_empty() {
        return Err("the Browser accepted the tab but named no target for it".into());
    }
    let ws_url = body
        .get("webSocketDebuggerUrl")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);
    Ok(Tab { id: id.to_string(), ws_url })
}

/// The one request `play` makes of the shell: the service it is sent to, the method, and the
/// arguments. Pinned by a test, because it is the only thing in Play that touches the person's
/// desktop, and it must be exactly what it was before the app moved to the framework.
pub(crate) fn shell_request() -> (String, &'static str, Value) {
    (
        yantrik_app::control::service_id_for("shell"),
        "app.act",
        serde_json::json!({ "action": "open_app", "args": { "name": "browser" } }),
    )
}

/// Ask the shell to launch (or focus) the Browser app through its own route.
fn ask_shell_for_browser() -> Result<(), String> {
    let (service, method, params) = shell_request();
    let client = yantrik_ipc_transport::SyncRpcClient::for_service(&service).with_timeout(Duration::from_secs(3));
    let result = client
        .call(method, params)
        .map_err(|e| {
            format!(
                "the desktop shell did not answer its control surface ({}); is the shell running?",
                e.message
            )
        })?;
    if let Some(summary) = result.get("summary").and_then(|s| s.as_str()) {
        tracing::debug!("shell answered open_app browser: {summary}");
    }
    Ok(())
}

// ── What the opened tab reports ────────────────────────────────────

/// The engine's own account of the tab, read through `window.__arcade.state()`.
#[derive(Debug, Clone, PartialEq)]
pub enum Seen {
    /// Frames are advancing: the arena is on screen.
    Drawing { frames: f64 },
    /// The engine could not make a WebGL renderer. `reason` is the engine's own
    /// error line ("WebGL unavailable: …"), which names what the Browser refused.
    NoWebGl { reason: String },
    /// Neither happened inside the budget. `last` is the last state the page gave,
    /// or why it could not be asked.
    Silent { last: String },
}

/// Read the engine's verdict out of one state snapshot, if it has one yet.
pub fn seen_in(state: &Value) -> Option<Seen> {
    let status = state.get("status").and_then(|v| v.as_str()).unwrap_or("");
    let frames = state.get("frames").and_then(|v| v.as_f64()).unwrap_or(0.0);
    if status == "nogl" {
        let reason = state
            .get("errors")
            .and_then(|e| e.as_array())
            .and_then(|errs| errs.iter().find_map(|e| e.as_str().filter(|s| s.starts_with("WebGL"))))
            .unwrap_or("the engine reported no WebGL context")
            .to_string();
        return Some(Seen::NoWebGl { reason });
    }
    if status == "playing" && frames > 0.0 {
        return Some(Seen::Drawing { frames });
    }
    None
}

/// Attach to the tab and wait for the engine to either draw or give up. Same
/// session type, same polling, same state hook as the verifier — a second opinion
/// would be a second thing to be wrong.
pub fn watch_tab(ws_url: &str, budget: Duration) -> Seen {
    let mut session = match verify::Session::connect(ws_url) {
        Ok(s) => s,
        Err(e) => return Seen::Silent { last: format!("could not attach to the tab: {e}") },
    };
    if let Err(e) = session.call("Runtime.enable", json!({})) {
        return Seen::Silent { last: format!("the tab refused Runtime.enable: {e}") };
    }
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        if let Ok(v) = session.evaluate("window.__arcade ? JSON.stringify(window.__arcade.state()) : 'null'") {
            if let Some(state) = v.as_str().and_then(|t| serde_json::from_str::<Value>(t).ok()) {
                if let Some(seen) = seen_in(&state) {
                    return seen;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    let last = session
        .evaluate("window.__arcade ? JSON.stringify(window.__arcade.state()) : 'no engine on the page'")
        .map(|v| v.as_str().map(String::from).unwrap_or_else(|| v.to_string()))
        .unwrap_or_else(|e| e);
    Seen::Silent { last }
}

/// One sentence for what `play` did, from what the tab reported. `fallback` is the
/// outcome of the headless screenshot taken when the Browser could not draw, so the
/// sentence can say where the person's game can actually be seen. A tab that drew
/// nothing is never "done": that was the white box the issue is about.
pub fn verdict(name: &str, seen: &Seen, fallback: Option<&Result<PathBuf, String>>) -> Result<String, String> {
    match seen {
        Seen::Drawing { frames } => Ok(format!(
            "opened {name} in the desktop Browser; the game is drawing ({frames:.0} frames so far)"
        )),
        Seen::NoWebGl { reason } => {
            let shown = match fallback {
                Some(Ok(path)) => format!(
                    "The game itself is fine: a headless screenshot of it is at {}",
                    path.display()
                ),
                Some(Err(e)) => format!("The headless screenshot failed too: {e}"),
                None => "`screenshot` renders it in a headless browser instead".to_string(),
            };
            Err(format!(
                "opened {name} in the desktop Browser, but it drew nothing: the Browser has no WebGL context ({reason}). {shown}"
            ))
        }
        Seen::Silent { last } => Err(format!(
            "opened {name} in the desktop Browser, but the game never reported a frame within {} s (last state: {last})",
            FIRST_FRAME_BUDGET.as_secs()
        )),
    }
}

/// Open a built game in the desktop Browser and watch it start. The whole sequence
/// has one honest refusal per thing that can go wrong, because every one of them is
/// something a person needs to hear: the shell is not running, the browser did not
/// come up, its debugging port is not the one the route promises, or the tab opened
/// and drew nothing. `fallback_shot` is where the headless screenshot goes in that
/// last case.
pub fn play(html: &Path, fallback_shot: &Path) -> Result<String, String> {
    let abs = std::fs::canonicalize(html)
        .map_err(|e| format!("cannot read {}: {e}", html.display()))?;
    if !debugger_alive(DEBUGGER) {
        ask_shell_for_browser()?;
    }
    // The Browser may already have been up (port alive immediately) or may be
    // starting now; either way, wait for the port to answer.
    let deadline = Instant::now() + Duration::from_secs(12);
    while !debugger_alive(DEBUGGER) {
        if Instant::now() > deadline {
            return Err(
                "the Browser did not open its debugging port (127.0.0.1:9222) within 12 s; \
                 the desktop's Browser route is supposed to carry --remote-debugging-port=9222"
                    .into(),
            );
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let url = file_url(&abs);
    let tab = open_tab(DEBUGGER, &url)?;
    let name = abs.file_name().and_then(|n| n.to_str()).unwrap_or("the game");
    let Some(ws_url) = tab.ws_url else {
        // Every Chromium names the socket in its /json/new answer; one that does not
        // has opened the tab, and that is all Arcade can truthfully say about it.
        return Ok(format!(
            "opened {name} in the desktop Browser (tab {}); the Browser named no debugger socket for the tab, so whether it draws was not checked",
            tab.id
        ));
    };
    let seen = watch_tab(&ws_url, FIRST_FRAME_BUDGET);
    let fallback = match &seen {
        Seen::NoWebGl { .. } => Some(verify::screenshot_game(&abs, fallback_shot).map(|()| fallback_shot.to_path_buf())),
        _ => None,
    };
    verdict(name, &seen, fallback.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use tungstenite::protocol::Message;

    #[test]
    fn file_urls_encode_what_a_url_cannot_carry() {
        let url = file_url(Path::new("/home/me/My Games/pip's run/index.html"));
        assert_eq!(url, "file:///home/me/My%20Games/pip%27s%20run/index.html");
    }

    #[test]
    fn file_urls_leave_safe_characters_alone() {
        let url = file_url(Path::new("/tmp/a-b_c.d~e/f/index.html"));
        assert_eq!(url, "file:///tmp/a-b_c.d~e/f/index.html");
    }

    /// One canned HTTP exchange: enough to prove open_tab sends a PUT to the
    /// right path with the URL in the query, and reads the target back.
    #[test]
    fn open_tab_puts_a_new_tab_and_reads_the_target_back() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            // Consume the headers so the client sees a well-formed exchange.
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" || line == "\n" {
                    break;
                }
            }
            let body = r#"{"id":"TARGET42","type":"page","url":"file:///x","webSocketDebuggerUrl":"ws://127.0.0.1:9222/devtools/page/TARGET42"}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
            request_line
        });
        let tab = open_tab(&format!("http://127.0.0.1:{port}"), "file:///tmp/game/index.html").unwrap();
        assert_eq!(tab.id, "TARGET42");
        assert_eq!(tab.ws_url.as_deref(), Some("ws://127.0.0.1:9222/devtools/page/TARGET42"));
        let request_line = server.join().unwrap();
        assert!(request_line.starts_with("PUT "), "newer Chromes require PUT: {request_line}");
        assert!(request_line.contains("/json/new?file:///tmp/game/index.html"), "{request_line}");
    }

    #[test]
    fn a_dead_debugger_port_is_not_alive() {
        // Port 1 on loopback: nothing listens there.
        assert!(!debugger_alive("http://127.0.0.1:1"));
    }

    // ── What the tab reports ──────────────────────────────────────

    #[test]
    fn a_nogl_state_is_seen_as_no_webgl_with_the_engines_reason() {
        let state = json!({
            "status": "nogl", "frames": 0, "webgl": false,
            "errors": ["WebGL unavailable: Error creating WebGL context."]
        });
        assert_eq!(
            seen_in(&state),
            Some(Seen::NoWebGl { reason: "WebGL unavailable: Error creating WebGL context.".into() })
        );
    }

    #[test]
    fn frames_advancing_is_seen_as_drawing_and_booting_is_not_yet_anything() {
        assert_eq!(seen_in(&json!({"status": "playing", "frames": 12})), Some(Seen::Drawing { frames: 12.0 }));
        assert_eq!(seen_in(&json!({"status": "playing", "frames": 0})), None);
        assert_eq!(seen_in(&json!({"status": "booting", "frames": 0})), None);
    }

    /// The white box: the tab opened, the HUD drew, the arena did not. That is never
    /// "done", and the sentence names the reason and where the game can be seen.
    #[test]
    fn a_tab_that_drew_nothing_is_never_done() {
        let seen = Seen::NoWebGl { reason: "WebGL unavailable: Error creating WebGL context.".into() };
        let shot = PathBuf::from("/lib/games/bramble/screenshot.png");
        let err = verdict("index.html", &seen, Some(&Ok(shot))).unwrap_err();
        assert!(err.contains("drew nothing"), "{err}");
        assert!(err.contains("no WebGL context"), "{err}");
        assert!(err.contains("Error creating WebGL context"), "{err}");
        assert!(err.contains("/lib/games/bramble/screenshot.png"), "{err}");

        let err = verdict("index.html", &seen, Some(&Err("no Chromium-class browser found".into()))).unwrap_err();
        assert!(err.contains("screenshot failed too"), "{err}");
        assert!(err.contains("no Chromium-class browser found"), "{err}");

        let err = verdict("index.html", &Seen::Silent { last: "{\"status\":\"booting\"}".into() }, None).unwrap_err();
        assert!(err.contains("never reported a frame"), "{err}");
    }

    #[test]
    fn a_drawing_tab_is_done_and_says_so() {
        let ok = verdict("index.html", &Seen::Drawing { frames: 40.0 }, None).unwrap();
        assert_eq!(ok, "opened index.html in the desktop Browser; the game is drawing (40 frames so far)");
    }

    // ── A fake tab over a real websocket ──────────────────────────
    // The same CDP shape the verifier's fake speaks, reduced to what watch_tab
    // asks: Runtime.enable, then state polls answered from a script.

    fn fake_tab(state: Value) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let Ok((stream, _)) = listener.accept() else { return };
            stream.set_read_timeout(Some(Duration::from_millis(300))).ok();
            let Ok(mut ws) = tungstenite::accept(stream) else { return };
            loop {
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
                let Ok(req) = serde_json::from_str::<Value>(&text) else { continue };
                let id = req.get("id").cloned().unwrap_or(Value::Null);
                let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
                let result = if method == "Runtime.evaluate" {
                    json!({ "result": { "type": "string", "value": state.to_string() } })
                } else {
                    json!({})
                };
                if ws.send(Message::Text(json!({ "id": id, "result": result }).to_string())).is_err() {
                    return;
                }
            }
        });
        format!("ws://127.0.0.1:{port}/devtools/page/FAKE")
    }

    #[test]
    fn watching_a_tab_with_no_webgl_reads_the_engines_own_verdict() {
        let ws = fake_tab(json!({
            "status": "nogl", "frames": 0, "webgl": false,
            "errors": ["WebGL unavailable: Error creating WebGL context."]
        }));
        let seen = watch_tab(&ws, Duration::from_secs(5));
        assert_eq!(seen, Seen::NoWebGl { reason: "WebGL unavailable: Error creating WebGL context.".into() });
    }

    #[test]
    fn watching_a_tab_that_draws_sees_frames() {
        let ws = fake_tab(json!({ "status": "playing", "frames": 33, "webgl": true, "errors": [] }));
        assert_eq!(watch_tab(&ws, Duration::from_secs(5)), Seen::Drawing { frames: 33.0 });
    }

    #[test]
    fn a_tab_that_never_boots_is_silent_with_its_last_state() {
        let ws = fake_tab(json!({ "status": "booting", "frames": 0 }));
        match watch_tab(&ws, Duration::from_millis(700)) {
            Seen::Silent { last } => assert!(last.contains("booting"), "{last}"),
            other => panic!("expected Silent, got {other:?}"),
        }
    }

    /// Play's request to the shell is the old app's, byte for byte: `app.act` on `app-shell` with
    /// `open_app` of `browser`. The old code built the same service name through
    /// `yantrik_app_runtime::control::service_id_for`, which is `yantrik_surface::service_id_for`.
    #[test]
    fn the_request_to_the_shell_is_the_one_the_old_app_made() {
        let (service, method, params) = shell_request();
        assert_eq!(service, "app-shell");
        assert_eq!(method, "app.act");
        assert_eq!(params.to_string(), r#"{"action":"open_app","args":{"name":"browser"}}"#);
    }
}
