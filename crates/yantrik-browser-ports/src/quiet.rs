//! What stops a Chromium the companion starts from calling Google on an idle profile (#1175, gate 9
//! bar 3). One list, here, so the headed and headless launches in `yantrik-companion-tools` and the
//! two the desktop shell starts (Mind View's browser, the provider-pages window) cannot differ. The
//! person's own Browser (`Side::Person`) is never given these: it keeps its own settings.
//!
//! Debian's `/usr/bin/chromium` wrapper sources `/etc/chromium.d/*` (and honours the
//! `CHROMIUM_USER_FLAGS` variable, Debian's per-user hook; `~/.config/chromium-flags.conf` is Arch's,
//! not Debian's) and puts the resulting `CHROMIUM_FLAGS` BEFORE the arguments the caller passes:
//! `exec chromium $CHROMIUM_FLAGS "$@"`. Chromium keeps the last value of a repeated switch, so our
//! switches come last and win. [`append_quiet`] merges the caller's own `--disable-features=` (in the
//! same argument list) into the one we pass. A `--disable-features` set by `/etc/chromium.d/*` or
//! `CHROMIUM_USER_FLAGS` is NOT in that list: it is replaced by ours, not merged. Code cannot see it;
//! VM 520 checks the shipped wrapper and its `/etc/chromium.d/` for one.

/// The Optimization Guide and the other features that call Google on an idle profile, by the names
/// Chromium 153 (and the Debian trixie package) knows them. `--disable-background-networking` does
/// not stop them: the hints, model and page-annotation services fetch from
/// `optimizationguide-pa.googleapis.com` on their own. A name a build does not know is ignored, so
/// the list may name more than one version's.
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

/// Switches that stop Chromium's own calls home, for EVERY Chromium the companion or a mind starts.
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

const FEATURES_PREFIX: &str = "--disable-features=";

/// [`QUIET_SWITCHES`] and exactly one `--disable-features=` with [`DISABLED_FEATURES`].
pub fn quiet_args() -> Vec<String> {
    let mut args: Vec<String> = QUIET_SWITCHES.iter().map(|s| s.to_string()).collect();
    args.push(format!("{FEATURES_PREFIX}{}", DISABLED_FEATURES.join(",")));
    args
}

/// Appends [`quiet_args`] to `args`. A `--disable-features=` already in `args` is removed and its
/// names joined to ours, so the launch has exactly one and nothing the caller disabled is lost.
pub fn append_quiet(args: &mut Vec<String>) {
    let mut features: Vec<String> = DISABLED_FEATURES.iter().map(|s| s.to_string()).collect();
    args.retain(|a| match a.strip_prefix(FEATURES_PREFIX) {
        Some(list) => {
            for f in list.split(',').map(str::trim).filter(|f| !f.is_empty()) {
                if !features.iter().any(|x| x == f) {
                    features.push(f.to_string());
                }
            }
            false
        }
        None => true,
    });
    args.extend(QUIET_SWITCHES.iter().map(|s| s.to_string()));
    args.push(format!("{FEATURES_PREFIX}{}", features.join(",")));
}

/// Whether `args` carries every quiet switch and exactly one `--disable-features=` holding every
/// name in [`DISABLED_FEATURES`]. What the launch tests assert, so no test restates the lists.
pub fn carries_quiet(args: &[String]) -> Result<(), String> {
    for s in QUIET_SWITCHES {
        if !args.iter().any(|a| a == s) {
            return Err(format!("{s} missing from {args:?}"));
        }
    }
    let lists: Vec<&str> = args.iter().filter_map(|a| a.strip_prefix(FEATURES_PREFIX)).collect();
    if lists.len() != 1 {
        return Err(format!("{} --disable-features= in {args:?}", lists.len()));
    }
    for f in DISABLED_FEATURES {
        if !lists[0].split(',').any(|x| x == *f) {
            return Err(format!("{f} missing from {}", lists[0]));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switches_and_features_are_pinned() {
        let want = [
            "--disable-background-networking", "--disable-component-update", "--disable-sync", "--no-pings",
            "--disable-domain-reliability", "--disable-default-apps", "--disable-breakpad", "--metrics-recording-only",
            "--disable-client-side-phishing-detection", "--safebrowsing-disable-auto-update",
            "--disable-component-extensions-with-background-pages",
        ];
        assert_eq!(QUIET_SWITCHES, want.as_slice());
        for f in ["OptimizationHints", "OptimizationHintsFetching", "OptimizationGuideModelDownloading", "OptimizationTargetPrediction",
            "OptimizationGuideOnDeviceModel", "PageContentAnnotations"] {
            assert!(DISABLED_FEATURES.contains(&f), "{f}");
        }
        assert_eq!(carries_quiet(&quiet_args()), Ok(()));
    }

    #[test]
    fn a_features_switch_already_in_the_list_is_merged_into_the_one() {
        let mut args = vec!["--x".to_string(), "--disable-features=Foo,Translate".to_string(), "--y".to_string()];
        append_quiet(&mut args);
        assert_eq!(carries_quiet(&args), Ok(()));
        let list = args.iter().find_map(|a| a.strip_prefix("--disable-features=")).unwrap();
        assert!(list.split(',').any(|f| f == "Foo") && list.split(',').filter(|f| *f == "Translate").count() == 1, "{list}");
        assert!(args.contains(&"--x".to_string()) && args.contains(&"--y".to_string()));
    }

    #[test]
    fn a_missing_switch_or_a_second_features_switch_is_caught() {
        let mut args = quiet_args();
        args.push("--disable-features=Other".into());
        assert!(carries_quiet(&args).is_err());
        assert!(carries_quiet(&[]).is_err());
    }
}
