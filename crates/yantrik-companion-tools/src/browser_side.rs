//! The one browser the built-in companion's browser tools reach (#1085).
//!
//! Mind View's: its port, its profile, none of the person's sign-ins. These tools used to fetch
//! `http://127.0.0.1:9222/json` and drive the first tab whoever held the port, which on a desktop
//! with the person's Browser open was the person's Browser: `browser_tabs` listed their tabs and
//! `browser_*` navigated them, and `browser_cleanup` killed Chromium by name. Now a port is used
//! only after the process holding it is shown to be a browser of a mind's, run by this account,
//! on the profile that goes with the port; anything else is refused with a sentence, and nothing
//! is started on a port something else holds.

use std::path::Path;

use serde_json::Value;
use yantrik_browser_ports::{self as ports, listener, Place, Profiles};

pub const CDP_HOST: &str = "127.0.0.1";
/// The port of the default identity, which is Mind View's browser.
pub const CDP_PORT: u16 = ports::MIND_VIEW_PORT;

/// What holds a port, for a caller that wants a browser whose profile puts it at `want`.
#[derive(Debug, PartialEq, Eq)]
pub enum Held {
    Nobody,
    Ours,
    /// Something else, or something that could not be established, and why.
    Foreign(String),
}

pub fn held_in(proc: &Path, port: u16, want: Place, own: u32, profiles: &Profiles) -> Held {
    match listener::identify_in(proc, port, profiles) {
        Err(listener::Why::Nobody) => Held::Nobody,
        Err(listener::Why::Unclear(why)) => Held::Foreign(format!("what holds port {port} could not be established ({why})")),
        Ok(l) if l.uid != own => Held::Foreign(format!("port {port} is held by another account's process")),
        Ok(l) if l.place != want => Held::Foreign(format!(
            "port {port} is held by a browser that is not a mind's own (it may be the person's, with their sign-ins and tabs)"
        )),
        Ok(_) => Held::Ours,
    }
}

pub fn held(port: u16, want: Place) -> Held {
    held_in(Path::new("/proc"), port, want, listener::own_uid(), &Profiles::current())
}

/// The sentence for a port that is not ours, ending in what the mind can do about it.
fn refusal(why: &str) -> String {
    format!("Not used: {why}. The browser tools only drive Mind View's browser, so no tab was listed, opened or changed.")
}

/// Mind View's browser holds the port, or why the tools may not use it.
pub fn guard() -> Result<(), String> {
    match held(CDP_PORT, Place::MindView) {
        Held::Ours => Ok(()),
        Held::Nobody => Err("Browser not reachable. Use launch_browser first.".to_string()),
        Held::Foreign(why) => Err(refusal(&why)),
    }
}

/// Whether Mind View's browser is up. A browser that merely answers on the port does not count.
pub fn is_running() -> bool {
    held(CDP_PORT, Place::MindView) == Held::Ours
}

/// Whether a browser may be started on `port` for `want`: nothing holds it yet. A browser already
/// there is the caller's to reuse (`Ok(false)`); something else is a refusal.
pub fn may_start(port: u16, want: Place) -> Result<bool, String> {
    match held(port, want) {
        Held::Nobody => Ok(true),
        Held::Ours => Ok(false),
        Held::Foreign(why) => Err(format!("Not started: {why}, and a second browser is never started on a port that is taken.")),
    }
}

fn curl(url: &str) -> Result<String, String> {
    let output = std::process::Command::new("curl")
        .args(["-s", "--max-time", "3", url])
        .output()
        .map_err(|e| format!("Cannot reach browser (curl): {e}"))?;
    if !output.status.success() {
        return Err("Browser not reachable. Use launch_browser first.".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `/json` of Mind View's browser: its targets, after the port is shown to be Mind View's.
pub fn targets() -> Result<Vec<Value>, String> {
    guard()?;
    let body = curl(&format!("http://{CDP_HOST}:{CDP_PORT}/json"))?;
    serde_json::from_str(&body).map_err(|e| format!("Bad CDP response: {e}"))
}

/// Close one of Mind View's browser's targets by id.
pub fn close_target(id: &str) -> Result<(), String> {
    guard()?;
    curl(&format!("http://{CDP_HOST}:{CDP_PORT}/json/close/{id}")).map(|_| ())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// A fake /proc with one listener on `port` held by process 7, and the profiles it may own.
    fn fake(name: &str, port: u16, uid: u32) -> (std::path::PathBuf, Profiles) {
        let root = std::env::temp_dir().join(format!("yb-side-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("proc/net")).unwrap();
        std::fs::write(
            root.join("proc/net/tcp"),
            format!("  sl  local_address rem_address st
   0: 0100007F:{port:04X} 00000000:0000 0A 0:0 0:0 0 {uid} 0 4242 1 0
"),
        )
        .unwrap();
        std::fs::create_dir_all(root.join("proc/7/fd")).unwrap();
        symlink("socket:[4242]", root.join("proc/7/fd/3")).unwrap();
        let profiles = Profiles { mind_view: root.join("mind-view"), identities: root.join("browsers") };
        std::fs::create_dir_all(&profiles.mind_view).unwrap();
        std::fs::create_dir_all(&profiles.identities).unwrap();
        (root, profiles)
    }

    fn takes(profile: &Path, pid: u32) {
        symlink(format!("{}-{pid}", yantrik_browser_ports::this_host().unwrap_or_else(|| "h".into())), profile.join("SingletonLock")).unwrap();
    }

    #[test]
    fn mind_views_browser_is_ours_and_only_it() {
        let (root, profiles) = fake("ok", 9223, 1000);
        takes(&profiles.mind_view, 7);
        assert_eq!(held_in(&root.join("proc"), 9223, Place::MindView, 1000, &profiles), Held::Ours);
        assert_eq!(held_in(&root.join("proc"), 9999, Place::MindView, 1000, &profiles), Held::Nobody);
        assert!(matches!(held_in(&root.join("proc"), 9223, Place::MindView, 1001, &profiles), Held::Foreign(_)));
        // The person's Browser on its own port owns no mind's profile, whatever it says it runs.
        let (person, profiles) = fake("person", 9222, 1000);
        std::fs::write(person.join("proc/7/cmdline"), format!("chromium --user-data-dir={}\0", profiles.mind_view.display())).unwrap();
        assert!(matches!(held_in(&person.join("proc"), 9222, Place::MindView, 1000, &profiles), Held::Foreign(_)));
    }

    /// The old guard failed open when /proc could not be read; this fails closed.
    #[test]
    fn what_cannot_be_established_is_refused_not_assumed_ours() {
        let (root, profiles) = fake("unread", 9223, 1000);
        std::fs::remove_dir_all(root.join("proc/7/fd")).unwrap();
        assert!(matches!(held_in(&root.join("proc"), 9223, Place::MindView, 1000, &profiles), Held::Foreign(_)));
    }

    #[test]
    fn a_port_the_person_holds_is_never_started_on() {
        let (root, profiles) = fake("start", 9224, 1000);
        assert!(matches!(held_in(&root.join("proc"), 9224, Place::Identity, 1000, &profiles), Held::Foreign(_)));
        assert!(refusal("x").contains("only drive Mind View's browser"));
    }
}
