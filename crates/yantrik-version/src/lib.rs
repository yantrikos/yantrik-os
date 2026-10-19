//! One version, for every surface that has to say what is running here.
//!
//! ── What this replaces ──
//!
//! A machine was asked three times what it was and gave three answers:
//!
//! ```text
//! /opt/yantrik/.version   0.3.0
//! /opt/yantrik/BUILD      version=v0.1.0-179-g6fc8b13
//! the terminal banner     Yantrik Terminal v0.1.0
//! ```
//!
//! None of the three was lying on purpose. `.version` was written once, by the ISO that
//! installed the machine, back when that script had `0.3.0` typed into it, and nothing has
//! rewritten it since — not the installer, not `yantrik-update`. `BUILD` is written from
//! `git describe` by whatever produced the build, and was the only one of the three that
//! was current. The banner, and the About screen, and the Settings screen, each showed the
//! `version` field of a `Cargo.toml` — `0.3.0` for the shell, `0.1.0` for the apps — numbers
//! nobody has moved in months and which say nothing about which build is installed.
//!
//! The answer to "which of them is right" cannot be a convention people remember. So there is
//! one function, and every surface calls it.
//!
//! ── Where the answer comes from ──
//!
//! In order, first non-empty wins:
//!
//! 1. `$YANTRIK_VERSION` — a caller that already knows (a test, a launcher, a one-off run of a
//!    binary out of a build directory against an installed tree).
//! 2. `version=` in the installed BUILD marker, `$YANTRIK_PREFIX/BUILD`, default
//!    `/opt/yantrik/BUILD`. This is the machine's own record of what was installed: written by
//!    `build-release.sh` into the bundle, copied in by the ISO build, and rewritten by
//!    `yantrik-update` on every apply and rollback. It is also the field `yantrik-update`
//!    compares to decide whether an update exists, so agreeing with it is the point.
//! 3. `YANTRIK_BUILD_VERSION`, baked by this crate's `build.rs` from the same
//!    `git describe --tags --match "v[0-9]*" --always --dirty` the release scripts run. This is the answer during
//!    development, where there is no installed tree to read.
//! 4. This crate's own package version, as a last resort.
//!
//! The marker comes before the baked string on purpose. A binary can outlive the tree it was
//! built in — `deploy-to-vm.sh` copies binaries onto a machine, `yantrik-update` swaps them —
//! and the question every one of these surfaces is really answering is "what is installed on
//! this machine", which is what the marker records.
//!
//! Step 4 is a single constant rather than each crate's own `CARGO_PKG_VERSION` because
//! per-crate versions are what produced the disagreement above: the shell says `0.3.0`, the
//! apps say `0.1.0`, and neither tracks anything.

use std::path::PathBuf;
use std::sync::OnceLock;

/// The default location of the installed build marker.
pub const DEFAULT_PREFIX: &str = "/opt/yantrik";

/// The build marker this machine records its install in.
pub fn build_marker_path() -> PathBuf {
    marker_path(std::env::var("YANTRIK_PREFIX").ok().as_deref())
}

/// `$YANTRIK_PREFIX/BUILD` — the same variable and the same default `yantrik-update` uses, so a
/// staged tree or a test prefix moves the script and the desktop together.
fn marker_path(prefix: Option<&str>) -> PathBuf {
    let prefix = prefix.map(str::trim).filter(|p| !p.is_empty()).unwrap_or(DEFAULT_PREFIX);
    PathBuf::from(prefix).join("BUILD")
}

/// What is running here. Resolved once per process.
pub fn version() -> &'static str {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(|| {
        let from_env = std::env::var("YANTRIK_VERSION").ok();
        let marker = std::fs::read_to_string(build_marker_path()).ok();
        resolve(
            from_env.as_deref(),
            marker.as_deref().and_then(parse_build_version),
            option_env!("YANTRIK_BUILD_VERSION"),
            env!("CARGO_PKG_VERSION"),
        )
        .to_string()
    })
}

/// The `version=` line of a BUILD marker.
///
/// Anchored to the start of a line, because the marker also carries `installed=` and has carried
/// other `*version*` keys in the past, and a substring match would pick up whichever came first.
/// The first `version=` wins if somehow there are two: the same rule `yantrik-update`'s
/// `build_field` follows (`sed -n 's/^version=//p' | head -1`), so the script and the desktop
/// cannot read one file differently.
pub fn parse_build_version(contents: &str) -> Option<&str> {
    contents
        .lines()
        .filter_map(|line| line.strip_prefix("version="))
        // A marker written on a machine with CRLF line endings, and stray padding.
        .map(|v| v.trim())
        .find(|v| !v.is_empty())
}

/// The precedence, with nothing to read from — the whole decision, separated from the I/O so it
/// can be tested. Empty and whitespace-only values count as absent: a marker with a bare
/// `version=` line is a marker that does not know, not a machine whose version is "".
fn resolve<'a>(
    from_env: Option<&'a str>,
    from_marker: Option<&'a str>,
    from_build: Option<&'a str>,
    cargo: &'a str,
) -> &'a str {
    [from_env, from_marker, from_build, Some(cargo)]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|v| !v.is_empty())
        .unwrap_or(cargo)
}

/// A version as a person reads it: `v0.1.0-1572-gf063fcef` (git describe: tag, commits since,
/// commit) → "0.1.0, build 1572"; `v0.2.0` → "0.2.0". Anything else is said as it is. The exact
/// string stays where it is needed to report a problem (About, `--version`, `describe`); Settings
/// showed it as the OS's version and the sweep of 17 October read it as a code.
pub fn plain(version: &str) -> String {
    let v = version.trim();
    let bare = v.strip_prefix('v').unwrap_or(v);
    let parts: Vec<&str> = bare.split('-').collect();
    let semver = |s: &str| !s.is_empty() && s.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    match parts.as_slice() {
        [tag] if semver(tag) => tag.to_string(),
        [tag, count, commit] if semver(tag) && count.chars().all(|c| c.is_ascii_digit()) && commit.starts_with('g') => {
            format!("{tag}, build {count}")
        }
        _ => v.to_string(),
    }
}

/// Answer `--version` on the command line and exit, or return and let the program start.
///
/// Called from `init_tracing` in the app runtime, which is the first line of every app binary's
/// `main`, so every app in this OS answers the same string without sixteen copies of this check.
pub fn handle_version_flag(program: &str) {
    if std::env::args().skip(1).any(|a| a == "--version" || a == "-V") {
        println!("{program} {}", version());
        std::process::exit(0);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_version_is_said_without_the_commit() {
        assert_eq!(super::plain("v0.1.0-1572-gf063fcef"), "0.1.0, build 1572");
        assert_eq!(super::plain("v0.2.0"), "0.2.0");
        assert_eq!(super::plain("f063fcef-90-g7be5de4"), "f063fcef-90-g7be5de4", "not a release: said as it is");
    }

    use super::*;

    /// A real marker, copied off the live machine on 2026-09-21.
    const LIVE_MARKER: &str = "name=yantrik-os-v0.1.0-258-g64165ad-20260921-64165ad-linux-amd64\n\
                               version=v0.1.0-258-g64165ad\n\
                               git=64165ad\n\
                               channel=nightly\n\
                               installed=2026-09-21T23:33:25Z\n\
                               binaries=32\n";

    #[test]
    fn reads_the_version_out_of_a_real_marker() {
        assert_eq!(parse_build_version(LIVE_MARKER), Some("v0.1.0-258-g64165ad"));
    }

    #[test]
    fn a_marker_written_with_crlf_still_parses() {
        // The release scripts write LF, but the marker is a plain text file that has been
        // hand-edited on machines before now, and a trailing \r would have become part of the
        // version string and then part of a URL.
        let crlf = LIVE_MARKER.replace('\n', "\r\n");
        assert_eq!(parse_build_version(&crlf), Some("v0.1.0-258-g64165ad"));
    }

    #[test]
    fn only_a_line_that_starts_with_version_counts() {
        // `yantrik-update --porcelain` prints `installed_version=`, and someone pasting its
        // output into a marker is exactly the accident worth not honouring.
        assert_eq!(parse_build_version("installed_version=9.9.9\ngit=abc\n"), None);
        assert_eq!(
            parse_build_version("installed_version=9.9.9\nversion=v1.2.3\n"),
            Some("v1.2.3")
        );
    }

    #[test]
    fn a_marker_that_does_not_know_is_not_a_version() {
        assert_eq!(parse_build_version("version=\ngit=abc\n"), None);
        assert_eq!(parse_build_version("version=   \n"), None);
        assert_eq!(parse_build_version(""), None);
        assert_eq!(parse_build_version("name=only\n"), None);
    }

    #[test]
    fn the_first_version_line_wins() {
        assert_eq!(parse_build_version("version=v1\nversion=v2\n"), Some("v1"));
    }

    #[test]
    fn the_marker_beats_the_baked_in_build() {
        // The case this ordering exists for: binaries swapped onto a machine by an update, or a
        // binary built here and copied there. The marker is what the machine installed.
        assert_eq!(resolve(None, Some("v0.1.0-258-g64165ad"), Some("v0.1.0-12-gdeadbee"), "0.1.0"), "v0.1.0-258-g64165ad");
    }

    #[test]
    fn an_explicit_version_beats_everything() {
        assert_eq!(resolve(Some("v9.9.9"), Some("v1"), Some("v2"), "0.1.0"), "v9.9.9");
    }

    #[test]
    fn falls_back_through_to_the_cargo_version() {
        assert_eq!(resolve(None, None, Some("v0.1.0-12-gdeadbee"), "0.1.0"), "v0.1.0-12-gdeadbee");
        assert_eq!(resolve(None, None, None, "0.1.0"), "0.1.0");
    }

    #[test]
    fn blank_values_do_not_win_over_the_next_source() {
        // An exported but empty YANTRIK_VERSION is the shape a shell script leaves behind when
        // the variable it meant to set was itself empty. It must not blank the About screen.
        assert_eq!(resolve(Some(""), Some("v1"), None, "0.1.0"), "v1");
        assert_eq!(resolve(Some("  "), None, Some("v2"), "0.1.0"), "v2");
        assert_eq!(resolve(Some(""), Some(" "), Some(""), "0.1.0"), "0.1.0");
    }

    #[test]
    fn the_marker_is_where_yantrik_update_keeps_it() {
        assert_eq!(marker_path(None), PathBuf::from("/opt/yantrik/BUILD"));
        assert_eq!(marker_path(Some("")), PathBuf::from("/opt/yantrik/BUILD"));
        assert_eq!(marker_path(Some("/tmp/stage")), PathBuf::from("/tmp/stage/BUILD"));
    }
}
