//! Open a web page for the person: one place, so every caller is held to the same rules, and the
//! page always ends up in front of them.
//!
//! Two ways:
//! - [`open`]: an ordinary link (the Lens), in the person's own browser.
//! - [`open_apart`]: a page where a credential is made or shown (a provider's sign-up and API-key
//!   pages, from the free AI card), in a browser of its own: its own profile, and no
//!   remote-debugging port. The desktop's own Chromium listens on 127.0.0.1:9222 so the browser
//!   tools can drive it, which means anything driving it could read a key off the page the moment
//!   the provider shows it (security review of #544, #545). Nothing can attach to this one.
//!
//! Either way the page comes to the front the same way: the window list is read before, and the
//! window that appears, or whose title changes (a running browser takes the page as a tab and
//! its title becomes the page's), is brought forward once the compositor has it. Read, not timed
//! (Pranab on VM 520, 1 Oct 2026: "After clicking sites are not opening up": they had opened
//! behind Settings). The opener is then waited for, so no press leaves a zombie.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long a page has to show up before the desktop stops looking for it: a cold browser start
/// on a small VM takes a few seconds.
const ARRIVAL: Duration = Duration::from_secs(12);
const POLL: Duration = Duration::from_millis(300);

/// Open an ordinary link in the person's browser (`xdg-open`), and bring it to the front.
pub fn open(url: &str) -> Result<(), String> {
    let url = web_address(url)?;
    let before = foreign_windows();
    let child = Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("the browser could not be opened: {e}"))?;
    bring_forward(before, child);
    Ok(())
}

/// Open a page where a credential is made or shown, in a browser apart from the desktop's own:
/// a private profile of its own and no remote-debugging port, so nothing on this machine can
/// drive it or read its pages. It comes to the front like any page.
pub fn open_apart(url: &str) -> Result<(), String> {
    let url = web_address(url)?;
    let (browser, _) = crate::wire::dock::find_browser().ok_or("there is no browser on this machine")?;
    let profile = yantrik_ml::private_dir::state_dir("provider-pages")
        .map_err(|e| format!("no private profile for the browser: {e}"))?;
    let program = crate::wire::dock::find_program(browser).ok_or("there is no browser on this machine")?;
    let before = foreign_windows();
    let child = Command::new(program)
        .args(apart_args(browser, &profile, url))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("the browser could not be opened: {e}"))?;
    bring_forward(before, child);
    Ok(())
}

/// The browser's arguments for a page apart: its own profile, a window of its own, and no
/// remote debugging (none of the desktop's `CHROMIUM_FLAGS` debugging switches is here).
fn apart_args(browser: &str, profile: &Path, url: &str) -> Vec<String> {
    let profile = profile.display().to_string();
    if browser.contains("chrom") {
        vec![
            "--ozone-platform=wayland".into(),
            "--no-first-run".into(),
            "--no-default-browser-check".into(),
            format!("--user-data-dir={profile}"),
            "--new-window".into(),
            url.into(),
        ]
    } else if browser.starts_with("firefox") {
        vec!["--no-remote".into(), "--profile".into(), profile, "--new-window".into(), url.into()]
    } else {
        vec![format!("--profile={profile}"), "--new-window".into(), url.into()]
    }
}

/// Only `https://` and `http://` addresses, whole: a `file:` or custom scheme handed to a browser
/// or the opener could start anything registered for it.
fn web_address(url: &str) -> Result<&str, String> {
    let url = url.trim();
    if (url.starts_with("https://") || url.starts_with("http://")) && !url.contains(|c: char| c.is_whitespace() || c.is_control()) {
        Ok(url)
    } else {
        Err("only a web address is opened".to_string())
    }
}

/// The windows that are not the shell's own (ours declare no app id), as (app id, title).
fn foreign_windows() -> Vec<(String, String)> {
    crate::windows::refresh_compositor_windows();
    crate::windows::shell_windows()
        .into_iter()
        .filter(|w| !w.wayland_app_id.is_empty())
        .map(|w| (w.wayland_app_id, w.title))
        .collect()
}

/// The title of a window in `now` that was not in `before`: one that appeared, or one whose
/// title changed (a browser that took the page as a tab).
fn arrived(before: &[(String, String)], now: &[(String, String)]) -> Option<String> {
    let mut left: Vec<&(String, String)> = before.iter().collect();
    for w in now {
        match left.iter().position(|b| *b == w) {
            Some(i) => {
                left.swap_remove(i);
            }
            None => return Some(w.1.clone()),
        }
    }
    None
}

/// Wait, off the caller's thread, for the page's window, bring it forward, then reap the opener.
fn bring_forward(before: Vec<(String, String)>, mut child: std::process::Child) {
    let _ = std::thread::Builder::new().name("open-url".into()).spawn(move || {
        let deadline = Instant::now() + ARRIVAL;
        let mut presented = false;
        while Instant::now() < deadline {
            std::thread::sleep(POLL);
            if let Some(title) = arrived(&before, &foreign_windows()) {
                presented = crate::windows::present(&title);
                break;
            }
        }
        if !presented {
            tracing::info!("a page was opened, and no window for it appeared to bring forward");
        }
        // xdg-open, or a browser this started, may stay until the browser exits: waiting costs a
        // thread here, never a zombie.
        let _ = child.wait();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(app: &str, title: &str) -> (String, String) {
        (app.to_string(), title.to_string())
    }

    #[test]
    fn only_a_web_address_is_handed_on() {
        for refused in ["file:///etc/passwd", "javascript:alert(1)", "ssh://host", "https://a b", "", "-https://x", "https://x\u{7}y"] {
            assert!(web_address(refused).is_err(), "{refused:?}");
        }
        assert_eq!(web_address(" https://console.groq.com/keys\n").unwrap(), "https://console.groq.com/keys");
    }

    /// The window the page arrived in: a new one, or the browser whose title became the page's.
    #[test]
    fn the_window_a_page_arrived_in_is_the_one_that_changed() {
        let before = vec![w("chromium", "Liveperf - Chromium"), w("blender", "(Unsaved) - Blender 4.3.2")];
        let tab = vec![w("chromium", "API Keys - GroqCloud - Chromium"), w("blender", "(Unsaved) - Blender 4.3.2")];
        assert_eq!(arrived(&before, &tab).as_deref(), Some("API Keys - GroqCloud - Chromium"));
        let mut new_window = before.clone();
        new_window.push(w("chromium", "Sign up - Cloudflare - Chromium"));
        assert_eq!(arrived(&before, &new_window).as_deref(), Some("Sign up - Cloudflare - Chromium"));
        assert_eq!(arrived(&before, &before), None, "nothing changed, nothing is brought forward");
        // Two windows of the same title: only an extra one counts as arrived.
        let two = vec![w("foot", "Terminal"), w("foot", "Terminal")];
        assert_eq!(arrived(&two[..1], &two).as_deref(), Some("Terminal"));
        assert_eq!(arrived(&two, &two), None);
    }

    /// A page apart gets its own profile and never a debugging port.
    #[test]
    fn a_page_apart_has_its_own_profile_and_no_debugging_port() {
        let profile = Path::new("/home/p/.local/state/yantrik/provider-pages");
        for browser in ["chromium", "google-chrome", "firefox", "epiphany"] {
            let args = apart_args(browser, profile, "https://console.groq.com/keys");
            let joined = args.join(" ");
            assert!(joined.contains("provider-pages"), "{browser}: {joined}");
            assert!(!joined.contains("remote-debugging") && !joined.contains("9222"), "{browser}: {joined}");
            assert_eq!(args.last().unwrap(), "https://console.groq.com/keys");
        }
    }
}
