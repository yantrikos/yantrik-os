//! Where this machine is, and what time it thinks it is.
//!
//! A desktop that does not know its own timezone is wrong about everything a person reads on it.
//! This one ran at `Etc/UTC` on hardware in Arkansas, which is five hours out: the clock in the
//! status bar, the date beside it, "today" in the calendar, the greeting on the home screen and
//! every timestamp in memory were all shifted by most of a working day. It said "Good night" at
//! seven in the evening and nobody could tell whether that was a bug in the greeting or in the
//! clock — it was the clock, and the greeting was telling the truth about a lie.
//!
//! Nothing in the OS had ever asked where it was. The weather instinct got away with it because
//! wttr.in geolocates the caller's IP server-side, so weather looked right while the machine
//! itself knew nothing.
//!
//! # Why this asks the network, once
//!
//! There is no offline way to learn a timezone. A fresh install has no user, no configuration and
//! no GPS; the only signals are the IP address and asking the person. Installers ask, and this OS
//! should too — but until its first-run wizard covers it, a machine that silently runs five hours
//! out is the worse failure. So: one lookup, only when nothing is known, written somewhere a
//! person can read and correct.
//!
//! What that costs is honest and bounded: one request that reveals this machine's public IP to
//! one service, which every weather fetch already did implicitly. It does not repeat, it does not
//! run when a location is already recorded, and `YANTRIK_NO_GEOLOCATE=1` turns it off entirely.
//!
//! # Why it does not keep re-checking
//!
//! A laptop that travels would drift, and a machine that silently changed its own clock while
//! somebody was working would be worse than one that was wrong in a fixed direction. Detection is
//! for a machine that knows nothing. Changing it afterwards is the person's call, and the value
//! sits in `settings.yaml` under `place` where they can make it.

use std::time::Duration;

// One definition of a plausible zone name, shared with the installer, which checks the timezone
// it is about to write into the installed system against the same shape.
use crate::installer_rules::is_plausible_timezone;

/// How long the lookup may take before it is abandoned. It runs on a worker thread and nothing
/// waits for it, so this only bounds how long that thread lives.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(6);

/// What a machine's timezone says when nobody has ever set it.
///
/// Debian's default. Treated as "unknown" rather than as a choice, because it is what an
/// unattended install leaves behind — a machine deliberately running UTC is rare enough, and
/// setting `place.timezone` by hand is how you say so.
const UNSET_TIMEZONE: &str = "Etc/UTC";

/// Learn where this machine is, if it does not already know. Returns immediately.
pub fn detect_in_background() {
    if std::env::var("YANTRIK_NO_GEOLOCATE")
        .map(|v| matches!(v.trim(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
    {
        tracing::info!("Location lookup disabled (YANTRIK_NO_GEOLOCATE)");
        return;
    }

    // A worker, because this makes a network call and the caller is the thread that draws the
    // screen. A desktop that stalls for six seconds at login to ask the internet where it is
    // would be a worse bug than the one this fixes.
    std::thread::Builder::new()
        .name("locate".into())
        .spawn(run)
        .ok();
}

fn run() {
    let known = super::settings::place();

    // Already located. The timezone may still need applying — a place can be recorded before
    // the system clock has been told about it, which is exactly the state a failed `timedatectl`
    // leaves behind.
    if !known.city.is_empty() || !known.timezone.is_empty() {
        apply_timezone(&known.timezone);
        return;
    }

    let Some(place) = look_up() else {
        // Said once, at info: a machine with no network at login is ordinary, and it will be
        // asked again next start.
        tracing::info!("Could not determine this machine's location; leaving it unset");
        return;
    };

    tracing::info!(
        city = %place.city,
        region = %place.region,
        country = %place.country,
        timezone = %place.timezone,
        "Located this machine"
    );
    super::settings::set_place(place.clone());
    apply_timezone(&place.timezone);
}

/// Ask where we are.
fn look_up() -> Option<super::settings::Place> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(LOOKUP_TIMEOUT)
        .timeout_read(LOOKUP_TIMEOUT)
        .build();
    let body: serde_json::Value = agent.get("https://ipinfo.io/json").call().ok()?.into_json().ok()?;

    let text = |key: &str| body[key].as_str().unwrap_or_default().trim().to_string();
    // "loc" is "36.3728,-94.2088".
    let (lat, lon) = body["loc"]
        .as_str()
        .and_then(|s| s.split_once(','))
        .and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?)))
        .unwrap_or((0.0, 0.0));

    let place = super::settings::Place {
        city: text("city"),
        region: text("region"),
        country: text("country"),
        lat,
        lon,
        timezone: text("timezone"),
        source: "detected".to_string(),
    };
    // A reply that carried nothing useful is not worth recording — it would look like a located
    // machine and stop this from ever asking again.
    if place.city.is_empty() && place.timezone.is_empty() {
        return None;
    }
    Some(place)
}

/// Set the system timezone, if this machine has never had one set.
///
/// Deliberately not "set it to whatever we detected": a machine whose timezone somebody chose
/// keeps that choice. Only the Debian default is treated as an empty slot.
fn apply_timezone(timezone: &str) {
    if timezone.is_empty() {
        return;
    }
    // A name from the network, about to be handed to a system command. Nothing here builds a
    // shell string — the argument goes straight to `timedatectl` — but the value is still
    // checked against the shape of an IANA name and against the zone actually existing, because
    // the cheapest place to stop a bad value is before it becomes an argument at all.
    if !is_plausible_timezone(timezone) {
        tracing::warn!(timezone, "Ignoring an implausible timezone");
        return;
    }
    if !std::path::Path::new("/usr/share/zoneinfo").join(timezone).exists() {
        tracing::warn!(timezone, "This machine has no zoneinfo for that timezone");
        return;
    }

    match current_timezone() {
        Some(current) if current != UNSET_TIMEZONE => {
            if current != timezone {
                // Not overridden. Somebody — a person or an installer — already decided, and a
                // desktop that quietly moves its own clock is a worse citizen than one that is
                // out of date.
                tracing::info!(
                    current = %current,
                    detected = %timezone,
                    "Timezone already set; leaving it alone"
                );
            }
            return;
        }
        _ => {}
    }

    let done = std::process::Command::new("sudo")
        .args(["-n", "timedatectl", "set-timezone", timezone])
        .output();
    match done {
        Ok(out) if out.status.success() => {
            // Said loudly, because the clock a person is looking at will not agree with this
            // until the shell restarts: `chrono` reads the zone once per process.
            tracing::info!(
                timezone,
                "Set the system timezone — the desktop clock picks it up on the next start"
            );
        }
        Ok(out) => tracing::warn!(
            timezone,
            error = %String::from_utf8_lossy(&out.stderr).trim(),
            "Could not set the system timezone"
        ),
        Err(e) => tracing::warn!(timezone, error = %e, "Could not run timedatectl"),
    }
}

/// What the system says its timezone is.
fn current_timezone() -> Option<String> {
    // The symlink is the source of truth and needs no process; `timedatectl` is a fallback for
    // systems where /etc/localtime is a copy rather than a link.
    if let Ok(target) = std::fs::read_link("/etc/localtime") {
        let path = target.to_string_lossy();
        if let Some(name) = path.split("zoneinfo/").nth(1) {
            return Some(name.to_string());
        }
    }
    let out = std::process::Command::new("timedatectl").arg("show").output().ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("Timezone=").map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::is_plausible_timezone;

    #[test]
    fn real_zone_names_pass() {
        for name in ["America/Chicago", "Etc/UTC", "Asia/Kolkata", "America/Argentina/Salta"] {
            assert!(is_plausible_timezone(name), "{name} should be accepted");
        }
    }

    /// The value arrives over the network and ends up as an argument to a system command. It is
    /// passed as an argv entry rather than through a shell, so none of these could have become a
    /// second command — but a name that escapes the zoneinfo directory, or one long enough to be
    /// someone experimenting, is rejected before it gets that far.
    #[test]
    fn anything_that_is_not_a_zone_name_is_refused() {
        for bad in [
            "",
            "/etc/passwd",
            "../../etc/shadow",
            "America/Chicago; rm -rf /",
            "America/Chicago\nEtc/UTC",
            "$(whoami)",
        ] {
            assert!(!is_plausible_timezone(bad), "{bad:?} should be refused");
        }
    }
}
