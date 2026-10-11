//! Power profiles: Power Saver / Balanced / Performance through power-profiles-daemon.
//!
//! The daemon has two bus names. Current releases own `org.freedesktop.UPower.PowerProfiles`;
//! older ones `net.hadess.PowerProfiles`. Both are tried, the new one first. A machine with
//! neither reports no profiles at all, and the shell then draws no choice.
//!
//! `performance` is not on every machine (the daemon only offers it where the platform has such
//! a mode), so what a person may pick is the daemon's own `Profiles` list, never the three names.

use std::collections::HashMap;
use std::time::Duration;

use crossbeam_channel::Sender;
use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

use crate::events::SystemEvent;

/// Every profile the daemon can name, in the order a person reads them.
pub const KNOWN: [&str; 3] = ["power-saver", "balanced", "performance"];

/// (bus name, object path, interface), newest first.
const NAMES: [(&str, &str, &str); 2] = [
    ("org.freedesktop.UPower.PowerProfiles", "/org/freedesktop/UPower/PowerProfiles", "org.freedesktop.UPower.PowerProfiles"),
    ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles", "net.hadess.PowerProfiles"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PowerProfiles {
    pub active: String,
    pub offered: Vec<String>,
}

impl PowerProfiles {
    fn into_event(self) -> SystemEvent {
        SystemEvent::PowerProfileChanged { active: self.active, offered: self.offered }
    }
}

/// How long any one call to the bus may take. zbus waits forever without a timeout, and a hung
/// daemon, or a polkit prompt nobody answers, would otherwise hold whatever thread asked.
const METHOD_TIMEOUT: Duration = Duration::from_secs(2);

/// A system-bus connection whose calls give up after [`METHOD_TIMEOUT`]. Every D-Bus call the
/// power code makes goes through one of these, battery.rs's too.
pub(crate) fn system_bus() -> zbus::Result<Connection> {
    zbus::blocking::connection::Builder::system()?.method_timeout(METHOD_TIMEOUT).build()
}

/// What the daemon says now, or `None` when there is no daemon.
pub fn read() -> Option<PowerProfiles> {
    let conn = system_bus().ok()?;
    NAMES.iter().find_map(|n| read_from(&conn, n))
}

fn read_from(conn: &Connection, (dest, path, iface): &(&str, &str, &str)) -> Option<PowerProfiles> {
    let msg = conn
        .call_method(Some(*dest), *path, Some("org.freedesktop.DBus.Properties"), "GetAll", &(*iface,))
        .ok()?;
    let body = msg.body();
    let props: HashMap<String, OwnedValue> = body.deserialize().ok()?;
    let active = props.get("ActiveProfile").and_then(|v| String::try_from(v.clone()).ok())?;
    let profiles: Vec<HashMap<String, OwnedValue>> = props
        .get("Profiles")
        .and_then(|v| Vec::<HashMap<String, OwnedValue>>::try_from(v.clone()).ok())
        .unwrap_or_default();
    let listed: Vec<String> = profiles
        .iter()
        .filter_map(|p| p.get("Profile").and_then(|v| String::try_from(v.clone()).ok()))
        .collect();
    Some(PowerProfiles { offered: offered_in_order(&listed, &active), active })
}

/// The daemon's profiles in the order a person reads them, whatever order it listed them in.
/// The active one is always included: a daemon that lists nothing still has one in effect.
pub(crate) fn offered_in_order(listed: &[String], active: &str) -> Vec<String> {
    KNOWN
        .iter()
        .filter(|k| listed.iter().any(|l| l == **k) || **k == active)
        .map(|k| k.to_string())
        .collect()
}

/// Switch profile and answer with what is in effect afterwards, read back from the daemon.
/// Refuses a name the daemon does not offer, so "performance" on a machine without it is an
/// error and not a silent no-op that reports success.
pub fn set(profile: &str) -> Result<PowerProfiles, String> {
    if !KNOWN.contains(&profile) {
        return Err(format!("`{profile}` is not a power profile; use power-saver, balanced or performance"));
    }
    let conn = system_bus().map_err(|e| format!("no system bus: {e}"))?;
    let (name, current) = NAMES
        .iter()
        .find_map(|n| read_from(&conn, n).map(|c| (n, c)))
        .ok_or("there is no power-profiles-daemon on this machine, so there is no profile to set")?;
    if !current.offered.iter().any(|o| o == profile) {
        return Err(format!("this machine does not offer `{profile}`; it offers: {}", current.offered.join(", ")));
    }
    conn.call_method(
        Some(name.0),
        name.1,
        Some("org.freedesktop.DBus.Properties"),
        "Set",
        &(name.2, "ActiveProfile", zbus::zvariant::Value::from(profile)),
    )
    .map_err(|e| format!("the daemon did not take it (it refused, or did not answer within {} s): {e}", METHOD_TIMEOUT.as_secs()))?;
    read_from(&conn, name).ok_or_else(|| "the daemon stopped answering after the change".to_string())
}

/// Tell the shell what the daemon says, once at the start and again when it changes. Returns
/// at once when there is no daemon: nothing is sent, so nothing is drawn.
pub fn run_power_profile_monitor(tx: Sender<SystemEvent>) {
    let Ok(conn) = system_bus() else { return };
    let Some((name, mut last)) = NAMES.iter().find_map(|n| read_from(&conn, n).map(|c| (n, c))) else {
        tracing::info!("power-profiles-daemon not available: no power profile choice");
        return;
    };
    tracing::info!(active = %last.active, "Power profile monitor started");
    let _ = tx.send(last.clone().into_event());

    // PropertiesChanged, not a poll: the daemon also changes when another tool sets a profile,
    // or when it drops to power-saver by itself on a low battery.
    let props = match zbus::blocking::fdo::PropertiesProxy::builder(&conn)
        .destination(name.0)
        .and_then(|b| b.path(name.1))
        .and_then(|b| b.build())
    {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "Cannot watch power profile changes");
            return;
        }
    };
    let Ok(changes) = props.receive_properties_changed() else { return };
    for _ in changes {
        if let Some(now) = read_from(&conn, name) {
            if now != last {
                last = now.clone();
                let _ = tx.send(now.into_event());
            }
        }
        // A burst of signals (the daemon sets several properties at once) is one read.
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

    /// The daemon lists performance first on some builds; a person reads saver, balanced,
    /// performance.
    #[test]
    fn offered_profiles_come_in_reading_order() {
        assert_eq!(
            offered_in_order(&names(&["performance", "balanced", "power-saver"]), "balanced"),
            names(&["power-saver", "balanced", "performance"])
        );
    }

    /// Hardware with no performance mode: it is not offered, so it is not drawn.
    #[test]
    fn performance_is_absent_when_the_daemon_does_not_list_it() {
        assert_eq!(
            offered_in_order(&names(&["balanced", "power-saver"]), "balanced"),
            names(&["power-saver", "balanced"])
        );
    }

    #[test]
    fn the_active_profile_is_always_offered_and_unknown_names_are_dropped() {
        assert_eq!(offered_in_order(&names(&["turbo"]), "balanced"), names(&["balanced"]));
    }

    #[test]
    fn a_name_that_is_not_a_profile_is_refused_before_the_bus_is_touched() {
        assert!(set("turbo").unwrap_err().contains("not a power profile"));
    }
}
