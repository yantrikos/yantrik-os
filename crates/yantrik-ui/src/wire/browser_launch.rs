//! How the desktop starts a Chromium-family browser, for the person and for a mind (#1085).
//!
//! The person's Browser runs on the person's port with its own default profile, named nowhere
//! (Chrome 136 and later refuse DevTools on the default directory when `--user-data-dir` names it,
//! so naming the person's own profile would take their port away). A browser started for a mind runs in Mind View on Mind View's port with Mind View's
//! profile, whatever flags the caller passed. The two never share a port or a profile, so a mind's
//! web tools have nothing of the person's to reach, and the person's tools nothing of a mind's.
//! Ports and paths are `yantrik_browser_ports`'s, which reads `browsers.conf`.

use std::path::{Path, PathBuf};

use yantrik_ipc_contracts::home_paths::workshops;

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

/// The Browser pin's arguments for the person: the program's own, then its port. A Chromium only:
/// Firefox has no DevTools port to name and would refuse the switches.
pub fn person_args(program: &str, own: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = own.iter().map(|a| a.to_string()).collect();
    if is_chromium(Path::new(program)) {
        args.extend(yantrik_browser_ports::launch_flags(Side::Person));
    }
    args
}

/// Whether `program` is a browser the desktop starts as the person's Browser or in Mind View.
pub fn is_browser(program: &Path) -> bool {
    is_chromium(program) || matches!(program_name(program).as_str(), "firefox" | "firefox-esr")
}

/// Blender, which a Mind drives through `run_python` wherever it was started.
pub fn is_blender(program: &Path) -> bool {
    program_name(program) == "blender"
}

/// Does this launch get the workshops hidden? Every browser, every app opened in Mind View (a
/// Mind drives it) and Blender anywhere (its surface is Mind-drivable on the person's desk too).
pub fn is_walled(program: &Path, mind_view: bool) -> bool {
    mind_view || is_browser(program) || is_blender(program)
}

/// What an app is started with: for Mind View the arguments of [`mind_view_args`], and for a
/// browser, a Mind View app or Blender the outside view with only the workshops hidden (a browser
/// needs its sockets in `/run/user`), so a program a Mind drives cannot read them. Refuses (with
/// the reason) when the machine holds workshops and cannot hide them from it.
pub fn start_line(program: &Path, args: &[&str], mind_view: bool) -> Result<(PathBuf, Vec<String>), String> {
    let args = match mind_view {
        true => mind_view_args(program, args),
        false => args.iter().map(|a| a.to_string()).collect(),
    };
    match is_walled(program, mind_view) {
        true => walled(program, args),
        false => Ok((program.to_path_buf(), args)),
    }
}

fn walled(program: &Path, args: Vec<String>) -> Result<(PathBuf, Vec<String>), String> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let (program, args) = workshops::browser_command(&home, &program.to_string_lossy(), &args)?;
    Ok((PathBuf::from(program), args))
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
    let (bin, args) = super::dock::find_browser().ok_or("there is no web browser on this machine")?;
    let seat = crate::mind_view::ensure().map_err(|why| format!("Mind View is not available ({why})"))?;
    preflight(Side::MindView)?;
    super::dock::launch("browser", bin, args, None, None, Some(&seat), false);
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

    fn flag(args: &[String], name: &str) -> Vec<String> {
        args.iter().filter(|a| a.starts_with(name)).cloned().collect()
    }

    #[test]
    fn the_persons_browser_names_their_port_and_no_profile() {
        let args = person_args("chromium", &["--no-first-run"]);
        assert_eq!(flag(&args, "--remote-debugging-port="), vec!["--remote-debugging-port=9222"]);
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
        assert!(!args.iter().any(|a| a.contains("9222")), "{args:?}");
        assert!(args.contains(&"--no-first-run".to_string()));
    }

    #[test]
    fn firefox_in_mind_view_gets_its_own_profile_and_no_debugging_port() {
        let args = mind_view_args(Path::new("/usr/bin/firefox"), &[]);
        assert_eq!(&args[..2], ["--no-remote", "--profile"]);
        assert!(args[2].ends_with("firefox"), "{args:?}");
        assert!(mind_view_args(Path::new("/usr/bin/foot"), &[]).is_empty());
    }

    #[test]
    fn a_browser_is_walled_and_other_programs_are_not_browsers() {
        assert!(is_browser(Path::new("/usr/bin/chromium")) && is_browser(Path::new("/usr/bin/firefox")));
        assert!(!is_browser(Path::new("/usr/bin/blender")));
        match walled(Path::new("/usr/bin/chromium"), vec!["--x".into()]) {
            Ok((program, args)) if program.ends_with("bwrap") => {
                assert_eq!(&args[args.len() - 2..], ["/usr/bin/chromium", "--x"]);
                assert!(args.iter().any(|a| a == "--unshare-pid"));
            }
            Ok((program, args)) => assert_eq!((program, args), (PathBuf::from("/usr/bin/chromium"), vec!["--x".to_string()])),
            Err(why) => assert!(why.contains("nothing was run"), "{why}"),
        }
    }

    /// Round 7, finding 3: the argv of a launch is the base-only view in front of the program, or
    /// (no bwrap, nothing held) the program as it was, or a refusal.
    fn assert_walled_or_plain(launch: Result<(PathBuf, Vec<String>), String>, program: &str, rest: &[&str]) {
        match launch {
            Ok((bin, argv)) if bin.ends_with("bwrap") => {
                let tail = [program].iter().chain(rest).map(|s| s.to_string()).collect::<Vec<_>>();
                assert_eq!(argv[argv.len() - tail.len()..], tail[..], "{argv:?}");
                let tmpfs: Vec<&String> = argv.windows(2).filter(|w| w[0] == "--tmpfs").map(|w| &w[1]).collect();
                // The base and nothing else, and only where it exists (`workshops::wrapped`). Which
                // base is the process's own: another test in this binary moves it under a test home.
                let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
                let bases = [PathBuf::from(workshops::BASE), home.join(".test-workshops")];
                assert!(tmpfs.len() <= 1 && tmpfs.iter().all(|t| bases.iter().any(|b| Path::new(t.as_str()) == b)), "the base only: {argv:?}");
                assert!(argv.iter().any(|a| a == "--unshare-pid") && !argv.iter().any(|a| a == "--die-with-parent"), "{argv:?}");
            }
            Ok((bin, argv)) => {
                assert_eq!(bin, PathBuf::from(program));
                assert_eq!(argv, rest.iter().map(|a| a.to_string()).collect::<Vec<_>>());
            }
            Err(why) => assert!(why.contains("nothing was run"), "{why}"),
        }
    }

    #[test]
    fn every_mind_view_launch_and_every_blender_launch_is_walled() {
        assert!(is_walled(Path::new("/usr/bin/foot"), true), "a Mind View app is driven by a Mind");
        assert!(is_walled(Path::new("/usr/bin/blender"), false), "Blender on the desk is Mind-drivable too");
        assert!(is_walled(Path::new("/usr/bin/blender"), true) && is_walled(Path::new("/usr/bin/chromium"), false));
        assert!(!is_walled(Path::new("/usr/bin/foot"), false), "the person's own apps are not");
        assert_walled_or_plain(start_line(Path::new("/usr/bin/foot"), &["--x"], true), "/usr/bin/foot", &["--x"]);
        assert_walled_or_plain(
            start_line(Path::new("/usr/bin/blender"), &["--python", "/opt/b.py"], false),
            "/usr/bin/blender",
            &["--python", "/opt/b.py"],
        );
        // The person's other apps are started as they were.
        assert_eq!(start_line(Path::new("/usr/bin/foot"), &["-e", "x"], false).unwrap(), (PathBuf::from("/usr/bin/foot"), vec!["-e".to_string(), "x".to_string()]));
    }
}
