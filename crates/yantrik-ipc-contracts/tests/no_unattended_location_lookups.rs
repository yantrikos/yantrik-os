//! No IP-based location or timezone lookup is left in the tree except the one that waits for the
//! person (#1171): the desktop's ipinfo.io call, behind `location_consent`, and wttr.in, which
//! the weather tool and watch reach only once a place is named or lookups are on.

use std::path::{Path, PathBuf};

/// Hosts that answer "where is this IP". `ip-api.com` is gone for good (its free tier is plain
/// HTTP); the rest may not appear at all.
const BANNED: &[&str] = &["ip-api.com", "ipapi.co", "geojs.io", "ipify.org", "ifconfig.co", "freegeoip", "ipwho.is"];
/// Hosts that may appear, only over HTTPS and only in these files.
const GATED: &[(&str, &[&str])] = &[
    ("ipinfo.io", &["crates/yantrik-ui/src/wire/location.rs", "crates/yantrik-ipc-contracts/src/location_consent.rs"]),
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if !matches!(name.as_str(), "target" | ".git" | "node_modules" | "vendor") {
                sources(&path, out);
            }
        } else if matches!(path.extension().and_then(|x| x.to_str()), Some("rs" | "sh" | "py" | "slint" | "yaml" | "toml" | "service")) {
            out.push(path);
        }
    }
}

fn tree() -> Vec<(String, String)> {
    let mut paths = Vec::new();
    sources(&root(), &mut paths);
    paths
        .into_iter()
        .filter_map(|p| {
            let rel = p.strip_prefix(root()).ok()?.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
            let rel = rel.trim_start_matches("./").to_string();
            if rel.ends_with("tests/no_unattended_location_lookups.rs") {
                return None;
            }
            Some((rel, std::fs::read_to_string(&p).ok()?))
        })
        .collect()
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
        for host in BANNED.iter().chain(["ipinfo.io", "wttr.in"].iter()) {
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

#[test]
fn the_desktop_lookup_asks_the_consent_first() {
    let text = std::fs::read_to_string(root().join("crates/yantrik-ui/src/wire/location.rs")).unwrap();
    let gate = text.find("location_consent::allowed()").expect("detect_in_background must check the consent");
    let call = text.find("spawn(run)").expect("the lookup thread");
    assert!(gate < call, "the consent check must come before the lookup thread starts");
}
