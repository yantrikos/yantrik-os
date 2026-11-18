//! Playing a game: hand the built file to the desktop's Browser, do not become one.
//!
//! The person's own Arcade opens the game in the person's Browser, through the browser service
//! (`persons_browser`). That Browser has no DevTools port: it is driven over a pipe the shell hands
//! to the service (#1137).
//!
//! A play a mind asked for is not opened here (#1085, #1200). A game is a page a mind may have
//! written, and a `file://` page in the person's signed-in profile can navigate itself to their
//! signed-in sites, so it never goes there. It used to open in Mind View's browser over that
//! browser's DevTools port; Mind View's browser has no port any more, because a port is reachable
//! by every process of the machine and a job speaking raw DevTools to it goes round the fence the
//! browser service puts on a browser's tabs. The mind opens its own game with the `browser` tool
//! (`go file://<path>`), which does go through the service.
//!
//! The person's play reports what the page draws: whether frames advance, or the engine could
//! not make a WebGL renderer (`persons_browser`).

use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use yantrik_browser_ports::{listener, Side};

/// Whose Browser a play opens in: the asker's, never the Arcade's. A mind's `play` reaches the
/// person's running Arcade (a copy drawn for a mind is a different process, and `open_app arcade`
/// is refused), so who runs the Arcade says nothing; who asked does.
fn side(by_mind: bool) -> Side {
    if by_mind || yantrik_app::for_mind::from_env() { Side::MindView } else { Side::Person }
}

/// Whether the call being handled is a mind's, from where it is known (inside the handler): a
/// token, the mind account, no caller at all, another account, or a caller the shell does not call
/// the person. Fails towards a mind: the person's Browser is for a caller shown to be the person.
pub fn caller_is_mind() -> bool {
    use yantrik_app::control::{agent_token, caller};
    let peer = caller().map(|c| (c.pid, c.uid));
    mind_by(agent_token().is_some(), peer, listener::own_uid(), yantrik_ipc_transport::mind_door::is_mind, |pid| {
        let client = yantrik_ipc_transport::SyncRpcClient::for_service(&yantrik_app::control::service_id_for("shell"))
            .with_timeout(Duration::from_secs(3));
        let params = serde_json::json!({ "action": "caller_class", "args": { "pid": pid } });
        let reply = client.call("app.act", params).ok()?;
        reply["result"]["class"].as_str().map(str::to_string)
    })
}

/// [`caller_is_mind`] with the facts given.
pub(crate) fn mind_by(
    token: bool,
    peer: Option<(i32, u32)>,
    own: u32,
    is_mind_account: impl Fn(u32) -> bool,
    ask_shell: impl FnOnce(i32) -> Option<String>,
) -> bool {
    match peer {
        _ if token => true,
        None => true,
        Some((_, uid)) if uid != own || is_mind_account(uid) => true,
        Some((pid, _)) => ask_shell(pid).as_deref() != Some("person"),
    }
}

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
pub(crate) fn ask_shell_for_browser() -> Result<(), String> {
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

/// What a mind is told when it asks to play a game: its own tool is the road.
pub(crate) const MIND_PLAYS_THROUGH_ITS_BROWSER: &str = "A game a mind built is not opened by Arcade: Mind View's browser has no DevTools port, \
and the person's Browser is not a mind's to use. Open it with the `browser` tool: go file://<the game's index.html>.";

/// Open a built game in the person's Browser. Whose Browser is decided by who asked, and a mind's
/// play is refused with where to go instead (`MIND_PLAYS_THROUGH_ITS_BROWSER`).
pub fn play(html: &Path, _fallback_shot: &Path, by_mind: bool) -> Result<String, String> {
    let abs = std::fs::canonicalize(html)
        .map_err(|e| format!("cannot read {}: {e}", html.display()))?;
    match side(by_mind) {
        Side::Person => crate::persons_browser::play(&abs),
        Side::MindView => Err(MIND_PLAYS_THROUGH_ITS_BROWSER.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    /// The 520 failure: a mind's play reached the person's running Arcade and opened in the person's
    /// Browser. Who asked decides, not whose Arcade it is.
    #[test]
    fn a_play_is_opened_in_the_askers_browser_not_the_arcades() {
        assert_eq!(side(true), Side::MindView, "a mind's play, in the person's own Arcade");
        assert_eq!(side(false), Side::Person);
        let no = |_: u32| false;
        let person = |_: i32| Some("person".to_string());
        assert!(!mind_by(false, Some((7, 1000)), 1000, no, person), "the shell calls it the person");
        assert!(mind_by(true, Some((7, 1000)), 1000, no, person), "a token");
        assert!(mind_by(false, None, 1000, no, person), "no caller at all");
        assert!(mind_by(false, Some((7, 1001)), 1000, no, person), "another account");
        assert!(mind_by(false, Some((7, 1000)), 1000, |u| u == 1000, person), "the mind account");
        for answer in [Some("mind".to_string()), None, Some("".to_string()), Some("Person".to_string())] {
            assert!(mind_by(false, Some((7, 1000)), 1000, no, |_| answer.clone()), "{answer:?}: Hermes, or a shell that cannot say");
        }
    }

    /// #1200: a mind's play is refused, and the sentence says where its own road is.
    #[test]
    fn a_minds_play_is_refused_and_told_to_use_its_browser_tool() {
        let dir = std::env::temp_dir().join(format!("arcade-mind-play-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let html = dir.join("index.html");
        std::fs::write(&html, "<p>game</p>").unwrap();
        let why = play(&html, &dir.join("shot.png"), true).unwrap_err();
        assert!(why.contains("`browser` tool") && why.contains("no DevTools port"), "{why}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// No port is spelled or asked for here: both browsers are held over pipes.
    #[test]
    fn nothing_here_reaches_a_browser_by_port() {
        let source = include_str!("play.rs");
        let code: String = source.split("#[cfg(test)]").next().unwrap().lines().filter(|l| !l.trim_start().starts_with("//")).collect();
        for gone in ["/json/", "remote-debugging-port", "debugger_alive", "side.port()", "listener::identify"] {
            assert!(!code.contains(gone), "a way back to a port: {gone}");
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
