//! How the desktop starts a Chromium-family browser, for the person and for a mind (#1085).
//!
//! The person's Browser runs with a DevTools pipe (#1137: no port; `browser_pipe` hands the stream
//! to the browser service) and its own default profile, named nowhere
//! (Chrome 136 and later refuse DevTools on the default directory when `--user-data-dir` names it,
//! so naming the person's own profile would take their DevTools away). A browser started for a mind
//! runs in Mind View with Mind View's profile, whatever flags the caller passed, and with a DevTools
//! pipe handed to the browser service by the same hand-over (#1200): no port, so no job of the
//! machine can speak raw DevTools to it and go round the service's fence. The two never share a
//! profile, so a mind's web tools have nothing of the person's to reach, and the person's tools
//! nothing of a mind's. Profiles and flags are `yantrik_browser_ports`'s, which reads `browsers.conf`.

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

use super::browser_pipe::Role;
use yantrik_browser_ports::{procs, Side, PIPE_FLAG};

/// The switches that name a debugging port, its allowed origin, or a profile, by name. A caller's
/// own are dropped from a mind's launch, which has them from `launch_flags` and nowhere else.
const OWN_BY_SIDE: [&str; 3] = ["remote-debugging-", "remote-allow-origins=", "user-data-dir="];

/// Whether a caller's argument is one the launch will not carry: one of `OWN_BY_SIDE` in either
/// dash spelling (Chromium on POSIX takes `-name=` as well as `--name=`), or the `--` that ends
/// the switches and would turn the shell's own flags into addresses to open.
fn is_own_by_side(arg: &str) -> bool {
    arg == "--" || (arg.starts_with('-') && OWN_BY_SIDE.iter().any(|name| arg.trim_start_matches('-').starts_with(name)))
}

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

/// Whether a launch in Mind View is Mind View's browser: the `browser` app, which is a Chromium.
/// The app id is the shell's own routing and not an argument a caller passes.
fn is_mind_views_browser(app_id: &str, program: &Path) -> bool {
    app_id == "browser" && is_chromium(program)
}

/// The role a launch has in the DevTools hand-over, said here and in `dock::spawn_persons_browser`
/// and nowhere else: the person's Browser's own arm, or a launch in Mind View's seat of the
/// `browser` app. Anything else (a `.desktop` file whose arguments carry the pipe switch, another
/// app in Mind View) has none, so it is given no pipe.
pub fn pipe_role(app_id: &str, program: &Path, in_mind_view: bool, persons_browser: bool) -> Option<Role> {
    if persons_browser {
        Some(Role::Person)
    } else if in_mind_view && is_mind_views_browser(app_id, program) {
        Some(Role::MindView)
    } else {
        None
    }
}

/// What Chromium restores a session from, in a profile: `Sessions/` and the files older versions
/// kept beside it.
const SESSION_FILES: [&str; 5] = ["Sessions", "Current Session", "Current Tabs", "Last Session", "Last Tabs"];

/// Mind View's Chromium never restores a session: whatever an earlier run left to restore is
/// removed before the start, so what it opens is the blank page it is told to and nothing a page
/// of the last run held, which would load before the service has fenced it (#1200). A second launch
/// of a browser already running hands over to it and exits; the files are only what it has saved.
///
/// The profile is written by a browser a mind drives, so it is not trusted to be what it was: if
/// the profile or its `Default` is a symlink nothing is removed (the shell would delete wherever
/// the link leads), and `Default` is opened without following a link and the names removed
/// through that open directory, so a swap after the check leads nowhere either.
fn forget_last_session(profile: &Path) {
    use std::os::unix::fs::OpenOptionsExt as _;
    if !std::fs::symlink_metadata(profile).map(|m| m.is_dir()).unwrap_or(false) {
        return;
    }
    let default = profile.join("Default");
    let Ok(dir) = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC).open(&default) else {
        return;
    };
    // The open directory, by its descriptor: the same inode whatever the name `Default` becomes.
    let held = Path::new("/proc/self/fd").join(std::os::fd::AsRawFd::as_raw_fd(&dir).to_string());
    for name in SESSION_FILES {
        let path = held.join(name);
        let _ = match std::fs::symlink_metadata(&path) {
            Ok(m) if m.is_dir() => std::fs::remove_dir_all(&path),
            Ok(_) => std::fs::remove_file(&path),
            Err(_) => continue,
        };
    }
}

/// A launch in Mind View: the caller's arguments, and for Mind View's browser its own profile and
/// DevTools pipe in place of any the caller named, starting on a blank page: a page named on the
/// command line would load before the browser service has the pipe, and so before it can fence the
/// page's first request. A browser hands a second launch over to the one already running on its
/// profile, and a window then appears wherever that one is: the person's desktop (PR #582, S3). A
/// mind's browser has a profile of its own, so it is its own process.
pub fn mind_view_args(app_id: &str, program: &Path, args: &[&str]) -> Vec<String> {
    let name = program_name(program);
    if is_chromium(program) {
        let profile = yantrik_browser_ports::mind_view_profile();
        let _ = std::fs::create_dir_all(&profile);
        let browser = is_mind_views_browser(app_id, program);
        if browser {
            forget_last_session(&profile);
        }
        // The shell's flags first, so nothing a caller passes can come before them, and a caller's
        // port, origin, profile and `--` are dropped; for the browser its pages are too. Another
        // Chromium app in Mind View is not given a pipe: nothing would hold it (`pipe_role`).
        let mut out: Vec<String> = yantrik_browser_ports::launch_flags(Side::MindView).into_iter().filter(|f| browser || f != PIPE_FLAG).collect();
        out.extend(args.iter().filter(|a| !is_own_by_side(a) && (!browser || a.starts_with('-'))).map(|a| a.to_string()));
        if browser {
            out.push("about:blank".into());
        }
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
/// nested desktop started if it is not, the browser in it, on its profile, with its DevTools pipe
/// handed to the browser service. Runs where it may take a while (off the UI thread). Says why when
/// it cannot, in words the mind can pass on.
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
/// stopped (they would keep the person's own Browser from having it). Mind View's own has no port
/// to be taken (#1200): a browser an earlier build left running on its profile is stopped by the
/// update (`stop_mind_view.py`) before this one starts, and one that is running now was handed to
/// the service by the shell that started it. One that survived that and listens on a TCP port is
/// refused here, in words: every launch would hand over to it and exit, and its port would stay
/// open to every process of the session.
pub fn preflight(_side: Side) -> Result<(), String> {
    let stopped = procs::stop_mind_view_off_port();
    if stopped > 0 {
        tracing::info!(stopped, "Stopped a Mind View browser an earlier build left on the person's port");
    }
    match procs::mind_view_listening_now().first() {
        Some((pid, port)) => Err(format!(
            "a Mind View browser from an older build (pid {pid}) is still running with a DevTools port ({port}), which any process of this session could drive; close it and open Mind View's browser again"
        )),
        None => Ok(()),
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

    /// The bug (#1085): Mind View's browser used to be started with the person's port. Then (#1200)
    /// with a port of its own, which any job could speak raw DevTools to. Now a pipe and a profile.
    #[test]
    fn a_minds_browser_gets_a_pipe_and_mind_views_profile_whatever_the_caller_passed() {
        let persons = person_args("chromium", &["--no-first-run", "--remote-debugging-port=9999", "--remote-allow-origins=*", "--user-data-dir=/home/p/.config/chromium"]);
        let refs: Vec<&str> = persons.iter().map(String::as_str).collect();
        let args = mind_view_args("browser", Path::new("/usr/bin/chromium"), &refs);
        assert_eq!(flag(&args, "--remote-debugging-"), vec!["--remote-debugging-pipe"], "the pipe, never a port: {args:?}");
        assert!(flag(&args, "--remote-allow-origins=").is_empty(), "{args:?}");
        let profile = flag(&args, "--user-data-dir=");
        assert_eq!(profile.len(), 1, "{args:?}");
        assert!(profile[0].ends_with("yantrik/mind-view-browser/chromium"), "{profile:?}");
        assert!(!args.iter().any(|a| a.contains("9222") || a.contains("port=") || a.contains("address")), "{args:?}");
        assert!(args.contains(&"--no-first-run".to_string()));
        assert!(args.contains(&"--disable-blink-features=AutomationControlled".to_string()), "a pipe makes navigator.webdriver true otherwise");
    }

    /// A page named on the command line would load before the service holds the pipe and fences it.
    #[test]
    fn mind_views_browser_starts_on_a_blank_page_whatever_it_was_asked_to_open() {
        let args = mind_view_args("browser", Path::new("/usr/bin/chromium"), &["--no-first-run", "file:///home/p/.ssh/x.html", "https://example.com"]);
        assert_eq!(args.last().map(String::as_str), Some("about:blank"), "{args:?}");
        assert!(!args.iter().any(|a| a.contains("example.com") || a.contains(".ssh")), "{args:?}");
    }

    /// Only Mind View's browser is handed over, and by the shell's own routing: another Chromium
    /// app in Mind View is given no pipe (nothing would hold it), nor a `.desktop` entry's flag.
    #[test]
    fn only_the_browser_arm_and_mind_views_browser_have_a_role() {
        let chromium = Path::new("/usr/bin/chromium");
        assert_eq!(pipe_role("browser", chromium, false, true), Some(Role::Person));
        assert_eq!(pipe_role("browser", chromium, true, false), Some(Role::MindView));
        assert_eq!(pipe_role("browser", chromium, false, false), None, "on the person's desktop but not by the Browser's arm");
        assert_eq!(pipe_role("browser", Path::new("/usr/bin/firefox"), true, false), None);
        assert_eq!(pipe_role("chromium-app", chromium, true, false), None, "another app");
        let other = mind_view_args("chromium-app", chromium, &[]);
        assert!(!other.iter().any(|a| a.contains("remote-debugging")), "no pipe nobody holds: {other:?}");
        assert!(other.iter().any(|a| a.starts_with("--user-data-dir=")));
    }

    /// Review LOW-4: a single dash is a switch too, and `--` ends them: neither reaches the launch,
    /// and the shell's flags come before anything the caller passed.
    #[test]
    fn a_callers_single_dash_port_and_double_dash_are_dropped_and_the_shells_flags_come_first() {
        let chromium = Path::new("/usr/bin/chromium");
        for app in ["browser", "chromium-app"] {
            let args = mind_view_args(app, chromium, &["--no-first-run", "-remote-debugging-port=9999", "-remote-allow-origins=*", "-user-data-dir=/x", "--", "--remote-debugging-address=0.0.0.0"]);
            assert!(!args.iter().any(|a| a == "--" || a.contains("9999") || a.contains("address") || a.contains("origins") || a.contains("/x")), "{app}: {args:?}");
            let own = args.iter().position(|a| a.starts_with("--user-data-dir=")).unwrap();
            let theirs = args.iter().position(|a| a == "--no-first-run").unwrap();
            assert!(own < theirs, "{app}: the shell's flags first: {args:?}");
            let after_flags = args.iter().skip_while(|a| a.starts_with('-')).count();
            assert!(after_flags <= 1, "{app}: only the blank page can follow the switches: {args:?}");
        }
    }

    /// The session an earlier run left is removed before Mind View's browser starts, so it has none
    /// to restore; another profile's is not touched.
    #[test]
    fn mind_views_browser_has_no_session_to_restore() {
        let profile = std::env::temp_dir().join(format!("yb-session-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&profile);
        std::fs::create_dir_all(profile.join("Default/Sessions")).unwrap();
        std::fs::write(profile.join("Default/Sessions/Session_1"), b"tabs").unwrap();
        std::fs::write(profile.join("Default/Current Session"), b"tabs").unwrap();
        std::fs::write(profile.join("Default/Cookies"), b"kept").unwrap();
        forget_last_session(&profile);
        assert!(!profile.join("Default/Sessions").exists() && !profile.join("Default/Current Session").exists());
        assert!(profile.join("Default/Cookies").exists(), "only the session goes");
        let _ = std::fs::remove_dir_all(&profile);
    }

    /// #1200 review, LOW-5: a `Default` (or a session entry) that is a symlink is never followed.
    #[test]
    fn a_symlinked_default_is_not_followed_out_of_the_profile() {
        let base = std::env::temp_dir().join(format!("yb-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let outside = base.join("outside");
        std::fs::create_dir_all(outside.join("Sessions")).unwrap();
        std::fs::write(outside.join("Current Tabs"), b"theirs").unwrap();
        // The profile's `Default` leads elsewhere.
        let profile = base.join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        std::os::unix::fs::symlink(&outside, profile.join("Default")).unwrap();
        forget_last_session(&profile);
        assert!(outside.join("Sessions").exists() && outside.join("Current Tabs").exists(), "followed Default out of the profile");
        // The profile itself is a link.
        let linked = base.join("linked");
        std::os::unix::fs::symlink(base.join("real"), &linked).unwrap();
        std::fs::create_dir_all(base.join("real/Default")).unwrap();
        std::fs::write(base.join("real/Default/Current Tabs"), b"theirs").unwrap();
        forget_last_session(&linked);
        assert!(base.join("real/Default/Current Tabs").exists(), "followed the profile link");
        // A session entry that is a link goes, and what it led to stays.
        let own = base.join("own");
        std::fs::create_dir_all(own.join("Default")).unwrap();
        std::os::unix::fs::symlink(&outside, own.join("Default/Sessions")).unwrap();
        forget_last_session(&own);
        assert!(!own.join("Default/Sessions").exists() && outside.join("Sessions").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn firefox_in_mind_view_gets_its_own_profile_and_no_debugging_port() {
        let args = mind_view_args("browser", Path::new("/usr/bin/firefox"), &[]);
        assert_eq!(&args[..2], ["--no-remote", "--profile"]);
        assert!(args[2].ends_with("firefox"), "{args:?}");
        assert!(mind_view_args("foot", Path::new("/usr/bin/foot"), &[]).is_empty());
    }
}
