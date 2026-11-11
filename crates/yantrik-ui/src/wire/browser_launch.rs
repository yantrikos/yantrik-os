//! How the desktop starts a Chromium-family browser, for the person and for a mind (#1085).
//!
//! The person's Browser runs with a DevTools pipe (#1137: no port; `browser_pipe` hands the stream
//! to the browser service) and its own default profile, named nowhere
//! (Chrome 136 and later refuse DevTools on the default directory when `--user-data-dir` names it,
//! so naming the person's own profile would take their DevTools away). A browser started for a mind runs in Mind View on Mind View's port with Mind View's
//! profile, whatever flags the caller passed. The two never share a port or a profile, so a mind's
//! web tools have nothing of the person's to reach, and the person's tools nothing of a mind's.
//! Ports and paths are `yantrik_browser_ports`'s, which reads `browsers.conf`.

use std::path::Path;

/// The web browsers the Browser pin will open, in order of preference, with what each needs.
///
/// Chromium first: it is what this OS installs, and the browser tools drive it. The flags keep
/// what the old hardcoded launch had — native Wayland, no first-run wizard, no default-browser
/// nag. The rest open as they are.
///
/// The profile is the browser's own. The visible browser used to run from
/// `--user-data-dir=/tmp/chromium-visible`, because a headless instance might hold the default
/// profile's lock; the browser tools now keep every profile under ~/.local/share/yantrik/browsers,
/// so that reason is gone, and a /tmp profile forgot every login and bookmark at each reboot.
pub const BROWSERS: &[(&str, &[&str])] = &[
    ("chromium", CHROMIUM_FLAGS),
    ("chromium-browser", CHROMIUM_FLAGS),
    ("google-chrome-stable", CHROMIUM_FLAGS),
    ("google-chrome", CHROMIUM_FLAGS),
    ("firefox", &[]),
    ("firefox-esr", &[]),
    ("epiphany-browser", &[]),
    ("epiphany", &[]),
];

/// How the desktop starts a Chromium-family browser.
///
/// The debugging port, its allowed origin and the profile are not here: they differ for the
/// person's Browser and for a mind's, and `browser_launch` adds each (#1085). The port is what
/// makes the browser one of this desktop's apps rather than a window a mind can only look at:
/// `yos web` drives the page over the DevTools protocol. Loopback only, and only the origin `yos`
/// itself presents. What it grants is what it sounds like (a process running as this user can
/// drive this browser), which such a process could already do by reading the profile on disk.
const CHROMIUM_FLAGS: &[&str] = &[
    "--ozone-platform=wayland",
    "--no-first-run",
    "--no-default-browser-check",
    // No "--disable-gpu": it also switches off SwiftShader, the software renderer, and cost
    // every machine WebGL (#133).
];

/// The first browser from `BROWSERS` this machine has.
pub fn find_browser() -> Option<(&'static str, &'static [&'static str])> {
    BROWSERS.iter().copied().find(|(name, _)| super::dock::find_program(name).is_some())
}

/// The names looked for, for a message that says none was found.
pub fn browser_names() -> String {
    BROWSERS.iter().map(|(name, _)| *name).collect::<Vec<_>>().join(", ")
}

use yantrik_browser_ports::{listener, procs, Place, Side};

/// The switches that name a debugging port, its allowed origin, or a profile. A caller's own are
/// dropped from a mind's launch, which has them from `launch_flags` and nowhere else.
const OWN_BY_SIDE: [&str; 3] = ["--remote-debugging-", "--remote-allow-origins=", "--user-data-dir="];

fn program_name(program: &Path) -> String {
    program.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_lowercase()
}

pub fn is_chromium(program: &Path) -> bool {
    matches!(program_name(program).as_str(), "chromium" | "chromium-browser" | "google-chrome" | "google-chrome-stable" | "chrome")
}

/// The Browser pin's arguments for the person: the program's own, then its DevTools pipe. A Chromium only:
/// Firefox has no DevTools port to name and would refuse the switches.
pub fn person_args(program: &str, own: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = own.iter().map(|a| a.to_string()).collect();
    if is_chromium(Path::new(program)) {
        args.extend(yantrik_browser_ports::launch_flags(Side::Person));
    }
    args
}

/// A launch in Mind View: the caller's arguments, and for a browser the mind's own port and
/// profile in place of any the caller named. A browser hands a second launch over to the one
/// already running on its profile, and a window then appears wherever that one is: the person's
/// desktop (PR #582, S3). A mind's browser has a profile of its own, so it is its own process.
pub fn mind_view_args(program: &Path, args: &[&str]) -> Vec<String> {
    let name = program_name(program);
    if is_chromium(program) {
        let _ = std::fs::create_dir_all(yantrik_browser_ports::mind_view_profile());
        let mut out: Vec<String> =
            args.iter().filter(|a| !OWN_BY_SIDE.iter().any(|p| a.starts_with(p))).map(|a| a.to_string()).collect();
        out.extend(yantrik_browser_ports::launch_flags(Side::MindView));
        return out;
    }
    let mut out: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    if matches!(name.as_str(), "firefox" | "firefox-esr") {
        let profile = yantrik_browser_ports::mind_view_profile().with_file_name("firefox");
        let _ = std::fs::create_dir_all(&profile);
        out.extend(["--no-remote".to_string(), "--profile".to_string(), profile.display().to_string()]);
    }
    out
}

/// Open Mind View's browser for a mind that needs one (its first web call, with none open): the
/// nested desktop started if it is not, the browser in it, on its port and profile. Runs where it
/// may take a while (off the UI thread). Says why when it cannot, in words the mind can pass on.
pub fn open_in_mind_view() -> Result<(), String> {
    if !crate::wire::settings::minds_open_in_mind_view() {
        return Err("the setting \"minds open apps in Mind View\" is off, so a mind has no browser of its own".into());
    }
    let (bin, args) = find_browser().ok_or("there is no web browser on this machine")?;
    let seat = crate::mind_view::ensure().map_err(|why| format!("Mind View is not available ({why})"))?;
    preflight(Side::MindView)?;
    super::dock::launch("browser", bin, args, None, None, Some(&seat), false, false);
    Ok(())
}

/// Before a browser starts: Mind View's browsers an earlier build left on the person's port are
/// stopped (they would keep the person's own Browser from having it), and a browser for Mind View
/// is not started on a port something else holds. No second Chromium on an occupied port.
pub fn preflight(side: Side) -> Result<(), String> {
    let stopped = procs::stop_mind_view_off_port();
    if stopped > 0 {
        tracing::info!(stopped, "Stopped a Mind View browser an earlier build left on the person's port");
    }
    if side != Side::MindView {
        return Ok(());
    }
    match listener::identify(Side::MindView.port()) {
        Err(listener::Why::Nobody) => Ok(()),
        // Mind View's own browser: the launch hands over to it and starts nothing.
        Ok(l) if l.place == Place::MindView && l.uid == listener::own_uid() => Ok(()),
        Ok(_) | Err(listener::Why::Unclear(_)) => Err(format!(
            "port {} is held by something that is not Mind View's browser, so none was started",
            Side::MindView.port()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Browser pin's list, whichever browser answers it.
    #[test]
    fn the_browser_list_has_chromium_first_and_flags_only_for_chromium() {
        assert!(BROWSERS.iter().any(|(name, _)| *name == "chromium"));
        assert!(BROWSERS.iter().any(|(name, _)| name.starts_with("firefox")));
        // Only the Chromium family gets Chromium's flags; Firefox would refuse to start on them.
        for (name, flags) in BROWSERS {
            let chromium = name.contains("chrom");
            assert_eq!(!flags.is_empty(), chromium, "{name}");
            assert!(!flags.iter().any(|f| f.contains("/tmp")), "{name}: a /tmp profile forgets everything");
        }
        assert!(browser_names().starts_with("chromium, "));
    }

    fn flag(args: &[String], name: &str) -> Vec<String> {
        args.iter().filter(|a| a.starts_with(name)).cloned().collect()
    }

    #[test]
    fn the_persons_browser_names_their_port_and_no_profile() {
        let args = person_args("chromium", &["--no-first-run"]);
        assert_eq!(flag(&args, "--remote-debugging-"), vec!["--remote-debugging-pipe"], "the pipe, never a port");
        assert!(!args.iter().any(|a| a.contains("9222") || a.contains("remote-allow-origins")), "{args:?}");
        assert!(flag(&args, "--user-data-dir").is_empty(), "their own profile is the default, named nowhere: {args:?}");
        assert!(args.contains(&"--no-first-run".to_string()));
        // A program the desktop does not start as the Browser gets nothing it cannot honour.
        assert_eq!(person_args("firefox", &[]), Vec::<String>::new());
    }

    /// The bug: Mind View's browser used to be started with the person's port (the flags came
    /// from the Browser pin) and nothing but a profile of its own.
    #[test]
    fn a_minds_browser_gets_mind_views_port_and_profile_whatever_the_caller_passed() {
        let persons = person_args("chromium", &["--no-first-run"]);
        let refs: Vec<&str> = persons.iter().map(String::as_str).collect();
        let args = mind_view_args(Path::new("/usr/bin/chromium"), &refs);
        assert_eq!(flag(&args, "--remote-debugging-port="), vec!["--remote-debugging-port=9223"]);
        assert_eq!(flag(&args, "--remote-allow-origins="), vec!["--remote-allow-origins=http://127.0.0.1:9223"]);
        let profile = flag(&args, "--user-data-dir=");
        assert_eq!(profile.len(), 1, "{args:?}");
        assert!(profile[0].ends_with("yantrik/mind-view-browser/chromium"), "{profile:?}");
        assert!(!args.iter().any(|a| a.contains("9222") || a.ends_with("-pipe")), "a mind never gets the person's pipe: {args:?}");
        assert!(args.contains(&"--no-first-run".to_string()));
    }

    #[test]
    fn firefox_in_mind_view_gets_its_own_profile_and_no_debugging_port() {
        let args = mind_view_args(Path::new("/usr/bin/firefox"), &[]);
        assert_eq!(&args[..2], ["--no-remote", "--profile"]);
        assert!(args[2].ends_with("firefox"), "{args:?}");
        assert!(mind_view_args(Path::new("/usr/bin/foot"), &[]).is_empty());
    }
}
