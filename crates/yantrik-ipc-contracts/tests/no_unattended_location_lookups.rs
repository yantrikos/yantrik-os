//! No IP-based location or timezone lookup is left in the tree except the ones that wait for the
//! person (#1171): the desktop's ipinfo.io call, behind `location_consent`, and wttr.in, which
//! the weather tool and watch reach only once a place is named or lookups are on.
//!
//! The scan reads every file git tracks (or would track): scripts without an extension, unit
//! files, JSON, HTML and the rest, not a list of extensions. The behaviour itself is pinned by
//! unit tests where the code is (`weather::resolve`, `weather_watch::alerts`,
//! `wire::location::settle`); this keeps a new caller from going around them.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Hosts that answer "where is this IP" (or "what zone is this IP in"). None may appear at all.
/// `ip-api.com` is gone for good (its free tier is plain HTTP).
const BANNED: &[&str] = &[
    "ip-api.com", "ipapi.co", "geojs.io", "ipify.org", "ifconfig.co", "freegeoip", "ipwho.is",
    "ifconfig.me", "icanhazip", "ipgeolocation.io", "ipdata.co", "ipstack", "ipregistry", "db-ip",
    "geoplugin", "worldtimeapi.org", "timeapi.io", "checkip.amazonaws.com", "ident.me",
];

/// Hosts that may appear, only over HTTPS and only in these files.
const GATED: &[(&str, &[&str])] = &[
    (
        "ipinfo.io",
        &[
            "crates/yantrik-ui/src/wire/location.rs",
            "crates/yantrik-ipc-contracts/src/location_consent.rs",
            "crates/yantrik-ui/src/control_location.rs",
        ],
    ),
    (
        "wttr.in",
        &[
            "crates/yantrik-companion-tools/src/weather.rs",
            "crates/yantrik-companion-instincts/src/weather_watch.rs",
            "crates/yantrik-ipc-contracts/src/location_consent.rs",
            "crates/yantrik-ui/src/control_location.rs",
            "crates/yantrik-ui/src/lens.rs",
            "crates/yantrik-ui/src/wire/location.rs",
            "skills/weather.yaml",
        ],
    ),
];

const WEATHER_TOOL: &str = "crates/yantrik-companion-tools/src/weather.rs";
const WEATHER_WATCH: &str = "crates/yantrik-companion-instincts/src/weather_watch.rs";
const SHELL_CARD: &str = "crates/yantrik-ui/src/control_location.rs";
const THIS_TEST: &str = "crates/yantrik-ipc-contracts/tests/no_unattended_location_lookups.rs";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every tracked or untracked-but-not-ignored path, from git; without a `.git` (a source export),
/// every file under the root except build output.
fn paths() -> Vec<String> {
    let listed = Command::new("git")
        .args(["ls-files", "--cached", "--others", "--exclude-standard"])
        .current_dir(root())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(str::to_string).collect::<Vec<_>>());
    listed.unwrap_or_else(|| {
        let mut out = Vec::new();
        walk(&root(), &mut out);
        out
    })
}

fn walk(dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            if !matches!(e.file_name().to_string_lossy().as_ref(), "target" | ".git" | "node_modules" | "vendor") {
                walk(&path, out);
            }
        } else if let Ok(rel) = path.strip_prefix(root()) {
            out.push(rel.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"));
        }
    }
}

/// Every text file in the tree, by path, with its text. A file that is not UTF-8 is not text.
fn tree() -> Vec<(String, String)> {
    paths()
        .into_iter()
        .filter(|rel| rel != THIS_TEST)
        .filter_map(|rel| {
            let text = std::fs::read_to_string(root().join(&rel)).ok()?;
            Some((rel, text))
        })
        .collect()
}

fn text_of(files: &[(String, String)], rel: &str) -> String {
    files.iter().find(|(f, _)| f == rel).unwrap_or_else(|| panic!("{rel} is not in the tree")).1.clone()
}

fn code_of(text: &str) -> &str {
    text.split("#[cfg(test)]").next().unwrap()
}

#[test]
fn the_scan_reads_extensionless_scripts_and_every_kind_of_text() {
    let files = tree();
    for must in [
        "deploy/yantrik-os/yantrik-update",
        "deploy/yantrik-os/yos-mcp",
        "crates/yantrik-ui/src/wire/location.rs",
        "harnesses/pi/yantrik-pi.service",
    ] {
        assert!(files.iter().any(|(f, _)| f == must), "{must} is not scanned");
    }
}

#[test]
fn no_banned_geolocation_host_appears_anywhere() {
    for (file, text) in tree() {
        for host in BANNED {
            assert!(!text.contains(host), "{file} names {host}: IP lookups must not return");
        }
    }
}

#[test]
fn no_location_lookup_url_is_plain_http() {
    for (file, text) in tree() {
        for host in BANNED.iter().chain(GATED.iter().map(|(h, _)| h)) {
            assert!(!text.contains(&format!("http://{host}")), "{file}: plain-HTTP lookup of {host}");
        }
    }
}

#[test]
fn the_gated_hosts_stay_in_their_gated_files() {
    for (file, text) in tree() {
        for (host, allowed) in GATED {
            assert!(!text.contains(host) || allowed.contains(&file.as_str()), "{file} names {host}");
        }
    }
}

/// wttr.in's no-place form (`wttr.in/?`) finds the caller by address. It is built in the weather
/// tool only, after the tool has decided (`resolve`), and the watch's fetch is called from one
/// place, behind its own check.
#[test]
fn the_no_place_url_is_reached_only_behind_the_consent() {
    let files = tree();
    for (file, text) in &files {
        assert!(!text.contains("wttr.in/?") || file == WEATHER_TOOL, "{file} builds the no-place wttr.in URL");
    }
    let tool = text_of(&files, WEATHER_TOOL);
    let code = code_of(&tool);
    let execute = &code[code.find("fn execute").expect("the tool's execute")..];
    let decided = execute.find("resolve(").expect("execute must decide the place first");
    let first_url = execute.find("wttr.in/?").expect("the no-place URL");
    assert!(decided < first_url, "the no-place URL is built before the place is decided");
    assert!(code.contains("location_consent::allowed()"), "the tool must read the switch");

    let watch = text_of(&files, WEATHER_WATCH);
    let code = code_of(&watch);
    assert_eq!(code.matches("fetch_weather").count(), 2, "fetch_weather is the definition and one call, inside the check");
    assert!(code.contains("self.alerts(location_consent::allowed()"), "the watch must pass the switch to its check");
}

/// Only the shell's card writes the switch, and no tool offers a way to say yes.
#[test]
fn only_the_shells_card_writes_the_switch() {
    for (file, text) in tree() {
        let writes = text.contains("location_consent::turn_on");
        assert!(!writes || file == SHELL_CARD, "{file} writes the location switch; only the shell's card may");
    }
}

/// The telemetry opt-out is set for every harness the desktop launches.
#[test]
fn every_harness_is_asked_not_to_report_usage() {
    let files = tree();
    for rel in [
        "harnesses/openclaw/yantrik-openclaw.service",
        "harnesses/pi/yantrik-pi.service",
        "harnesses/deepseek/yantrik-deepseek.service",
        "harnesses/lib/install/hermes.sh",
    ] {
        assert!(text_of(&files, rel).contains("DO_NOT_TRACK"), "{rel} must set DO_NOT_TRACK");
    }
}
