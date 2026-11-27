//! The OS's own headless Chromium, for web searches (#1439).
//!
//! It is driven over `--remote-debugging-pipe`: the browser reads DevTools messages on its fd 3
//! and writes them on fd 4, and listens on no port (#1200), so no other process can speak to it.
//! Its rules, each pinned by a test below:
//!
//! * **Its own profile**, [`profile_dir`], never the person's and never Mind View's, with nothing
//!   carried in: wiped each time the browser starts.
//! * **It reads one results page and nothing else.** [`read_page`] navigates to a URL the caller
//!   built, evaluates a read-only script and closes the tab. It clicks nothing, types nothing and
//!   follows no result.
//! * **Low priority** (`nice` 10) and **one search at a time**: the lock is held for the whole
//!   search, so an unattended search never competes with itself, and the person's turn is never
//!   behind it for the CPU.
//! * **Started when a search needs it, stopped after a quiet minute** ([`QUIET`]).
//! * A search that is not done within [`BUDGET`] kills the browser and is an error.

use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// How long a search may take, from lock to answer.
pub(super) const BUDGET: Duration = Duration::from_secs(30);
/// How long the browser stays up with nothing to do.
pub(super) const QUIET: Duration = Duration::from_secs(60);
const NICE: libc::c_int = 10;
/// A DevTools message is never this long; a stream that says so is broken.
const MOST_MESSAGE: usize = 8 * 1024 * 1024;

/// The browser's own profile.
pub(super) fn profile_dir() -> std::path::PathBuf {
    yantrik_browser_ports::data_home().join("yantrik/search-browser")
}

/// The switches of the search browser. No port, and its own profile.
pub(super) fn args(profile: &std::path::Path) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--headless=new".into(),
        "--disable-gpu".into(),
        yantrik_browser_ports::PIPE_FLAG.into(),
        format!("--user-data-dir={}", profile.display()),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--disable-blink-features=AutomationControlled".into(),
        "--window-size=1280,900".into(),
        "--user-agent=Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36".into(),
    ];
    yantrik_browser_ports::append_quiet(&mut args);
    args.push("about:blank".into());
    args
}

struct Running {
    child: Child,
    /// Our end of the browser's fd 3: what we send.
    to: File,
    /// Our end of its fd 4: what it says.
    from: File,
    next_id: u64,
    last_used: Instant,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The one browser, and the lock that makes searches one at a time.
static BROWSER: Mutex<Option<Running>> = Mutex::new(None);

fn pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as RawFd; 2];
    // SAFETY: `fds` has room for the two descriptors pipe2 writes.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: pipe2 just made both, and nothing else owns them.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

fn start() -> Result<Running, String> {
    let binary = super::chrome_binary()?;
    let profile = profile_dir();
    // Nothing of an earlier run is kept: no cookie, no sign-in, no history.
    let _ = std::fs::remove_dir_all(&profile);
    std::fs::create_dir_all(&profile).map_err(|e| format!("cannot make the search browser's profile: {e}"))?;
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&profile, std::fs::Permissions::from_mode(0o700));
    }
    let (to_read, to_write) = pipe().map_err(|e| format!("no pipe for the search browser: {e}"))?;
    let (from_read, from_write) = pipe().map_err(|e| format!("no pipe for the search browser: {e}"))?;
    let (browser_in, browser_out) = (to_read.as_raw_fd(), from_write.as_raw_fd());
    let mut command = Command::new(binary);
    command.args(args(&profile)).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    for var in super::PERSON_SESSION_VARS {
        command.env_remove(var);
    }
    command.env_remove("CHROMIUM_FLAGS").env_remove("CHROMIUM_USER_FLAGS");
    // SAFETY: only async-signal-safe calls (dup2, fcntl, setpriority) between fork and exec.
    unsafe {
        command.pre_exec(move || {
            // Park both ends above 4 first, so neither source is clobbered by the other's target.
            let a = libc::fcntl(browser_in, libc::F_DUPFD, 10);
            let b = libc::fcntl(browser_out, libc::F_DUPFD, 10);
            if a < 0 || b < 0 || libc::dup2(a, 3) < 0 || libc::dup2(b, 4) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::close(a);
            libc::close(b);
            libc::setpriority(libc::PRIO_PROCESS, 0, NICE);
            Ok(())
        });
    }
    let child = command.spawn().map_err(|e| format!("the search browser did not start: {e}"))?;
    drop((to_read, from_write));
    Ok(Running { child, to: File::from(to_write), from: File::from(from_read), next_id: 1, last_used: Instant::now() })
}

/// Stops the browser after [`QUIET`] with nothing to do. One of these runs while a browser does.
fn reap_when_quiet() {
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(5));
        let Ok(mut guard) = BROWSER.try_lock() else { continue };
        match guard.as_ref() {
            None => return,
            Some(running) if running.last_used.elapsed() >= QUIET => {
                *guard = None;
                return;
            }
            Some(_) => {}
        }
    });
}

impl Running {
    fn send(&mut self, method: &str, params: Value, session: Option<&str>) -> Result<u64, String> {
        use std::io::Write;
        let id = self.next_id;
        self.next_id += 1;
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        let mut bytes = message.to_string().into_bytes();
        bytes.push(0);
        self.to.write_all(&bytes).map_err(|e| format!("the search browser went away ({e})"))?;
        Ok(id)
    }

    /// The reply to `id`, skipping every event and other reply, within `deadline`.
    fn reply(&mut self, id: u64, deadline: Instant, pending: &mut Vec<u8>) -> Result<Value, String> {
        loop {
            while let Some(message) = next_message(pending)? {
                let value: Value = serde_json::from_slice(&message).map_err(|_| "the search browser sent something unreadable".to_string())?;
                if value.get("id").and_then(Value::as_u64) == Some(id) {
                    return match value.get("error") {
                        Some(e) => Err(format!("the search browser refused ({})", e.get("message").and_then(Value::as_str).unwrap_or("no reason"))),
                        None => Ok(value.get("result").cloned().unwrap_or(Value::Null)),
                    };
                }
            }
            let left = deadline.checked_duration_since(Instant::now()).ok_or("the search took too long")?;
            if !wait_readable(self.from.as_raw_fd(), left) {
                return Err("the search took too long".into());
            }
            let mut chunk = [0u8; 16 * 1024];
            match self.from.read(&mut chunk) {
                Ok(0) | Err(_) => return Err("the search browser went away".into()),
                Ok(n) => pending.extend_from_slice(&chunk[..n]),
            }
        }
    }

    fn call(&mut self, method: &str, params: Value, session: Option<&str>, deadline: Instant, pending: &mut Vec<u8>) -> Result<Value, String> {
        let id = self.send(method, params, session)?;
        self.reply(id, deadline, pending)
    }
}

/// The next whole message in `pending` (they end in a NUL byte), removed from it.
pub(super) fn next_message(pending: &mut Vec<u8>) -> Result<Option<Vec<u8>>, String> {
    match pending.iter().position(|b| *b == 0) {
        Some(end) => {
            let message: Vec<u8> = pending.drain(..=end).take(end).collect();
            Ok(Some(message))
        }
        None if pending.len() > MOST_MESSAGE => Err("the search browser sent more than a page can be".into()),
        None => Ok(None),
    }
}

fn wait_readable(fd: RawFd, within: Duration) -> bool {
    let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
    let ms = within.as_millis().min(i32::MAX as u128) as i32;
    // SAFETY: one valid pollfd.
    let n = unsafe { libc::poll(&mut p, 1, ms) };
    n > 0
}

/// Load `url` in a fresh tab, run `script` (a read-only expression that evaluates to a string),
/// close the tab and hand back the string. One at a time; killed and an error past [`BUDGET`].
pub(super) fn read_page(url: &str, script: &str) -> Result<String, String> {
    let mut guard = BROWSER.lock().unwrap_or_else(|e| e.into_inner());
    let deadline = Instant::now() + BUDGET;
    if guard.as_mut().is_some_and(|r| r.child.try_wait().ok().flatten().is_some()) {
        *guard = None;
    }
    if guard.is_none() {
        *guard = Some(start()?);
        reap_when_quiet();
    }
    let running = guard.as_mut().expect("just started");
    let outcome = read_in_tab(running, url, script, deadline);
    match &outcome {
        Ok(_) => running.last_used = Instant::now(),
        // A browser that stalled or died is not kept.
        Err(_) => *guard = None,
    }
    outcome
}

fn read_in_tab(r: &mut Running, url: &str, script: &str, deadline: Instant) -> Result<String, String> {
    let mut pending = Vec::new();
    let target = r.call("Target.createTarget", json!({ "url": "about:blank" }), None, deadline, &mut pending)?;
    let target_id = target["targetId"].as_str().ok_or("no tab")?.to_string();
    let result = (|| {
        let attached = r.call("Target.attachToTarget", json!({ "targetId": target_id, "flatten": true }), None, deadline, &mut pending)?;
        let session = attached["sessionId"].as_str().ok_or("no tab session")?.to_string();
        let nav = r.call("Page.navigate", json!({ "url": url }), Some(&session), deadline, &mut pending)?;
        if let Some(why) = nav.get("errorText").and_then(Value::as_str) {
            return Err(format!("the page did not load ({why})"));
        }
        loop {
            let state = r.call("Runtime.evaluate", json!({ "expression": "document.readyState", "returnByValue": true }), Some(&session), deadline, &mut pending)?;
            if state["result"]["value"].as_str() == Some("complete") {
                break;
            }
            if Instant::now() + Duration::from_millis(250) >= deadline {
                return Err("the page did not finish loading".into());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        // Results drawn by script settle just after `complete`.
        std::thread::sleep(Duration::from_millis(500));
        let read = r.call("Runtime.evaluate", json!({ "expression": script, "returnByValue": true }), Some(&session), deadline, &mut pending)?;
        read["result"]["value"].as_str().map(str::to_string).ok_or_else(|| "the page could not be read".to_string())
    })();
    // Whatever happened, the tab goes, so no page stays open for anything to look at.
    let _ = r.call("Target.closeTarget", json!({ "targetId": target_id }), None, Instant::now() + Duration::from_secs(3), &mut pending);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_browser_has_a_pipe_a_profile_of_its_own_and_no_port() {
        let profile = std::path::Path::new("/data/yantrik/search-browser");
        let a = args(profile);
        assert!(a.iter().any(|x| x == "--remote-debugging-pipe"));
        assert!(!a.iter().any(|x| x.contains("remote-debugging-port") || x.contains("remote-allow-origins")), "{a:?}");
        assert!(a.contains(&format!("--user-data-dir={}", profile.display())));
        assert!(a.contains(&"--headless=new".to_string()));
        assert_eq!(a.iter().filter(|x| x.starts_with("--disable-features=")).count(), 1, "the quiet list, once");
        let theirs = [yantrik_browser_ports::mind_view_profile(), yantrik_browser_ports::identities_root()];
        assert!(theirs.iter().all(|p| !profile_dir().starts_with(p) && !p.starts_with(profile_dir())), "its profile is nobody else's");
        assert!(profile_dir().ends_with("yantrik/search-browser"));
    }

    #[test]
    fn it_runs_at_low_priority_one_search_at_a_time_and_stops_when_quiet() {
        let src = include_str!("headless.rs").split("#[cfg(test)]").next().unwrap();
        assert!(src.contains("libc::setpriority(libc::PRIO_PROCESS, 0, NICE)") && NICE_IS_LOW);
        assert!(src.contains("let mut guard = BROWSER.lock()"), "the lock is taken before anything starts");
        assert!(QUIET <= Duration::from_secs(120) && BUDGET <= Duration::from_secs(60));
        // It never clicks, types or follows: the only methods it ever sends.
        let methods: std::collections::BTreeSet<&str> = src
            .split("r.call(\"")
            .skip(1)
            .map(|s| s.split('"').next().unwrap())
            .collect();
        let expected: std::collections::BTreeSet<&str> =
            ["Target.createTarget", "Target.attachToTarget", "Page.navigate", "Runtime.evaluate", "Target.closeTarget"].into();
        assert_eq!(methods, expected, "a new method needs a decision");
        assert!(!src.contains("Input.") && !src.contains("DOM.") && !src.contains("Network."));
    }

    const NICE_IS_LOW: bool = NICE >= 5;

    #[test]
    fn messages_are_nul_ended_and_come_out_whole() {
        let mut pending = b"{\"id\":1}\0{\"id\":2".to_vec();
        assert_eq!(next_message(&mut pending).unwrap().unwrap(), b"{\"id\":1}");
        assert_eq!(next_message(&mut pending).unwrap(), None, "half a message waits");
        pending.extend_from_slice(b"}\0");
        assert_eq!(next_message(&mut pending).unwrap().unwrap(), b"{\"id\":2}");
        assert!(pending.is_empty());
        let mut huge = vec![b'x'; MOST_MESSAGE + 1];
        assert!(next_message(&mut huge).is_err());
    }

    /// A stand-in for the browser on two pipes: a reply, an event, and the reply that counts.
    #[test]
    fn a_reply_is_found_among_events_and_other_replies() {
        let (to_read, to_write) = pipe().unwrap();
        let (from_read, from_write) = pipe().unwrap();
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        let mut r = Running { child, to: File::from(to_write), from: File::from(from_read), next_id: 7, last_used: Instant::now() };
        let mut feed = File::from(from_write);
        std::io::Write::write_all(
            &mut feed,
            b"{\"method\":\"Page.loadEventFired\",\"params\":{}}\0{\"id\":3,\"result\":{}}\0{\"id\":7,\"result\":{\"v\":1}}\0",
        )
        .unwrap();
        let id = r.send("Target.createTarget", json!({}), None).unwrap();
        assert_eq!(id, 7);
        let mut sent = [0u8; 256];
        let n = File::from(to_read).read(&mut sent).unwrap();
        assert!(String::from_utf8_lossy(&sent[..n]).contains("Target.createTarget"));
        assert_eq!(r.reply(7, Instant::now() + Duration::from_secs(2), &mut Vec::new()).unwrap()["v"], 1);
        // Nothing more is said: the wait ends at the deadline instead of hanging.
        let late = r.reply(8, Instant::now() + Duration::from_millis(100), &mut Vec::new()).unwrap_err();
        assert!(late.contains("too long"), "{late}");
    }

    #[test]
    fn a_missing_browser_is_an_error_and_not_a_hang() {
        // `start` needs a Chromium; where there is none it says so and keeps nothing.
        if super::super::chrome_binary().is_err() {
            assert!(start().is_err());
        }
    }
}
