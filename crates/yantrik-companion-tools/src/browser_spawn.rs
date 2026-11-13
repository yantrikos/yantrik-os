//! How the companion's own Chromium is started (#1124, round 8, finding 1): always through
//! `workshops::browser_command`, so the browser a Mind drives, and whose DevTools port a job can
//! reach on loopback, has no view of the workshops (`file://` included). The two spawns in
//! `browser.rs` build their command here and nowhere else. That no other file starts a Chromium
//! is held by `xtask check-architecture` (rule `mind-spawn`), not by a test of one file.

use std::path::Path;
use std::process::Command;

use yantrik_browser_ports::{self as ports};
use yantrik_ipc_contracts::home_paths::workshops;

/// The Optimization Guide and the other features that call Google on an idle profile, by the names
/// Chromium 153 (and the Debian trixie package) knows them. `--disable-background-networking` does
/// not stop them: the hints, model and page-annotation services fetch from
/// `optimizationguide-pa.googleapis.com` on their own (#1175, gate 9 bar 3). A name a build does not
/// know is ignored, so the list may name more than one version's.
pub const DISABLED_FEATURES: &[&str] = &[
    "OptimizationHints",
    "OptimizationHintsFetching",
    "OptimizationGuideModelDownloading",
    "OptimizationTargetPrediction",
    "OptimizationGuideOnDeviceModel",
    "OptimizationGuidePushNotifications",
    "PageContentAnnotations",
    "PageVisibilityPageContentAnnotations",
    "AutofillServerCommunication",
    "SegmentationPlatform",
    "CertificateTransparencyComponentUpdater",
    "Translate",
    "MediaRouter",
];

/// Switches that stop Chromium's own calls home, for EVERY Chromium the companion starts (the
/// headed one in `browser.rs` and the headless one here): the one list, so no start can differ.
pub const QUIET_SWITCHES: &[&str] = &[
    // Google's component updater, field trials, safe-browsing pings are not the person's search.
    "--disable-background-networking",
    "--disable-component-update",
    "--disable-sync",
    "--no-pings",
    "--disable-domain-reliability",
    "--disable-default-apps",
    "--disable-breakpad",
    "--metrics-recording-only",
    "--disable-client-side-phishing-detection",
    "--safebrowsing-disable-auto-update",
    "--disable-component-extensions-with-background-pages",
];

/// [`QUIET_SWITCHES`] and `--disable-features=` with [`DISABLED_FEATURES`].
pub fn quiet_args() -> Vec<String> {
    let mut args: Vec<String> = QUIET_SWITCHES.iter().map(|s| s.to_string()).collect();
    args.push(format!("--disable-features={}", DISABLED_FEATURES.join(",")));
    args
}

/// `binary args` as a command that runs in the browser view (the workshops base hidden), or the
/// reason it must not run: nothing is started unwrapped on a machine that holds workshops.
pub fn walled(home: &Path, binary: &str, args: &[String]) -> Result<Command, String> {
    let (program, argv) = workshops::browser_command(home, binary, args)?;
    let mut command = Command::new(program);
    command.args(argv);
    Ok(command)
}

/// The command of the headless browser `web_search` and `browse` start on `port`.
pub fn headless(home: &Path, binary: &str, profile: &Path, port: u16) -> Result<Command, String> {
    let mut args: Vec<String> = [
        "--headless=new",
        "--ozone-platform=headless",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-gpu",
        // Anti-detection: no navigator.webdriver, a real user agent and viewport, WebGL on.
        "--disable-blink-features=AutomationControlled",
        "--user-agent=Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
        "--window-size=1920,1080",
        "--enable-webgl",
        "--enable-features=NetworkService,NetworkServiceInProcess",
        "--disable-infobars",
        "--excludeSwitches=enable-automation",
    ]
    .map(String::from)
    .to_vec();
    args.extend(quiet_args());
    args.extend(ports::debugging_flags(port));
    args.push(format!("--user-data-dir={}", profile.display()));
    args.push("about:blank".to_string());
    walled(home, binary, &args)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    /// Every switch and every feature name is pinned, and the headless launch carries them all.
    #[test]
    fn the_headless_start_carries_every_quiet_switch_and_feature() {
        let want: Vec<&str> = vec![
            "--disable-background-networking", "--disable-component-update", "--disable-sync", "--no-pings",
            "--disable-domain-reliability", "--disable-default-apps", "--disable-breakpad", "--metrics-recording-only",
            "--disable-client-side-phishing-detection", "--safebrowsing-disable-auto-update",
            "--disable-component-extensions-with-background-pages",
        ];
        assert_eq!(QUIET_SWITCHES, want.as_slice());
        let features = quiet_args().into_iter().find_map(|a| a.strip_prefix("--disable-features=").map(str::to_string)).unwrap();
        for f in ["OptimizationHints", "OptimizationHintsFetching", "OptimizationGuideModelDownloading", "OptimizationTargetPrediction",
            "OptimizationGuideOnDeviceModel", "PageContentAnnotations"] {
            assert!(features.split(',').any(|x| x == f), "{f} in {features}");
        }
        let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        if let Ok(c) = headless(&home, "/usr/bin/chromium", Path::new("/tmp/p"), 9223) {
            let a = argv(&c);
            for arg in quiet_args() {
                assert!(a.contains(&arg), "{arg} in {a:?}");
            }
        }
    }

    fn argv(c: &Command) -> Vec<String> {
        c.get_args().map(OsStr::to_string_lossy).map(|s| s.into_owned()).collect()
    }

    /// Round 8, finding 1: the headless browser starts in bwrap's view. Where the base exists, its
    /// argv begins with `--tmpfs /var/lib/yantrik/workshops`; where bwrap is missing it is refused
    /// if workshops are held, and run as it was if none are.
    #[test]
    fn the_headless_browser_starts_behind_a_tmpfs_over_the_base() {
        let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        let base = std::path::Path::new(workshops::BASE);
        match headless(&home, "/usr/bin/chromium", Path::new("/tmp/p"), 9223) {
            Ok(c) => {
                let a = argv(&c);
                assert!(a.iter().any(|x| x == "--remote-debugging-port=9223") && (c.get_program() == "/usr/bin/chromium" || a.iter().any(|x| x == "/usr/bin/chromium")), "{a:?}");
                if workshops::find_bwrap().is_some() {
                    assert!(c.get_program().to_string_lossy().ends_with("bwrap"));
                    assert_eq!(a[..3], ["--dev-bind", "/", "/"]);
                    if base.is_dir() {
                        assert_eq!(a[3..5], ["--tmpfs", workshops::BASE]);
                    }
                } else {
                    assert_eq!(c.get_program(), "/usr/bin/chromium");
                }
            }
            Err(e) => assert!(e.contains("bwrap") && e.contains("nothing was run"), "{e}"),
        }
    }

    /// The command is exactly `browser_command`'s, not a parallel construction.
    #[test]
    fn walled_is_browser_commands_answer() {
        let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        let args = vec!["--x".to_string()];
        match (walled(&home, "chromium", &args), workshops::browser_command(&home, "chromium", &args)) {
            (Ok(c), Ok((program, want))) => {
                assert_eq!(c.get_program().to_string_lossy(), program);
                assert_eq!(argv(&c), want);
            }
            (Err(e), Err(w)) => assert_eq!(e, w),
            other => panic!("{other:?}"),
        }
    }
}
