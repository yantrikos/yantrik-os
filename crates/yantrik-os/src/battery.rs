//! Battery monitor: UPower over D-Bus, or the kernel's sysfs files when UPower is not there.
//!
//! UPower sends `PropertiesChanged` on its DisplayDevice whenever the level, the state or the
//! time estimate moves, so the monitor sleeps in between and wakes for exactly those. When
//! UPower is not on the bus (an image without the package, a minimal Alpine VM) the sysfs
//! reader takes over; see `battery_sysfs`.
//!
//! A machine with no battery sends nothing at all, so the shell draws no indicator.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use crossbeam_channel::Sender;
use zbus::zvariant::OwnedValue;

use crate::battery_sysfs;
use crate::events::{BatteryState, SystemEvent};

/// One look at the battery, whichever source it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Reading {
    pub level: u8,
    pub state: BatteryState,
    pub time_to_empty_mins: Option<u32>,
    pub time_to_full_mins: Option<u32>,
}

impl Reading {
    fn into_event(self) -> SystemEvent {
        SystemEvent::BatteryChanged {
            level: self.level,
            charging: self.state.on_charger(),
            time_to_empty_mins: self.time_to_empty_mins,
            time_to_full_mins: self.time_to_full_mins,
            state: self.state,
        }
    }
}

/// The only poll left, and the sysfs fallback's alone: the kernel's files send no signal, so
/// the only way to learn the level moved is to look. Once a minute is slow enough to cost
/// nothing and fast enough for a battery, and it runs only on a machine that has one.
const SYSFS_POLL: Duration = Duration::from_secs(60);

/// Main loop for the battery monitor thread.
pub fn run_battery_monitor(tx: Sender<SystemEvent>) {
    if let Ok(connection) = crate::power_profile::system_bus() {
        if upower_on_bus(&connection) {
            tracing::info!("Battery monitor started (UPower signals)");
            watch_upower(&connection, &tx);
            // The signal stream ended: UPower went away. The kernel's files still answer.
            tracing::warn!("UPower stopped sending; reading the battery from sysfs");
        } else {
            tracing::info!("UPower not available; reading the battery from sysfs");
        }
    } else {
        tracing::info!("No system D-Bus; reading the battery from sysfs");
    }
    watch_sysfs(&tx, Path::new(battery_sysfs::POWER_SUPPLY));
}

fn upower_on_bus(connection: &zbus::blocking::Connection) -> bool {
    let Ok(proxy) = zbus::blocking::fdo::DBusProxy::new(connection) else { return false };
    match proxy.list_names() {
        Ok(names) => names.iter().any(|n| n.as_str() == "org.freedesktop.UPower"),
        Err(_) => false,
    }
}

/// Send the battery now, then again each time UPower says something changed. Returns when the
/// signal stream ends. Sends nothing for a machine with no battery.
fn watch_upower(connection: &zbus::blocking::Connection, tx: &Sender<SystemEvent>) {
    let mut last = None;
    let mut publish = |now: Option<Reading>| {
        if now != last {
            if let Some(r) = now {
                let _ = tx.send(r.into_event());
            }
            last = now;
        }
    };
    publish(read_upower(connection));

    let props = match zbus::blocking::fdo::PropertiesProxy::builder(connection)
        .destination("org.freedesktop.UPower")
        .and_then(|b| b.path("/org/freedesktop/UPower/devices/DisplayDevice"))
        .and_then(|b| b.build())
    {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "Cannot watch UPower");
            return;
        }
    };
    let Ok(changes) = props.receive_properties_changed() else { return };
    for _ in changes {
        // UPower can signal a time estimate that moved by a second; the reading is in whole
        // minutes, so an unchanged reading is not sent on.
        publish(read_upower(connection));
    }
}

/// The sysfs fallback: look now, then once a minute, and only while a battery exists.
fn watch_sysfs(tx: &Sender<SystemEvent>, root: &Path) {
    let mut last = battery_sysfs::read_at(root);
    let Some(first) = last else {
        tracing::info!("No battery under {}; battery monitor off", root.display());
        return;
    };
    let _ = tx.send(first.into_event());
    loop {
        std::thread::sleep(SYSFS_POLL);
        match battery_sysfs::read_at(root) {
            Some(now) if Some(now) != last => {
                last = Some(now);
                let _ = tx.send(now.into_event());
            }
            _ => {}
        }
    }
}

/// Read the DisplayDevice from UPower. `None` when the machine has no battery.
fn read_upower(connection: &zbus::blocking::Connection) -> Option<Reading> {
    let msg = connection
        .call_method(
            Some("org.freedesktop.UPower"),
            "/org/freedesktop/UPower/devices/DisplayDevice",
            Some("org.freedesktop.DBus.Properties"),
            "GetAll",
            &("org.freedesktop.UPower.Device",),
        )
        .ok()?;

    let body = msg.body();
    let props: HashMap<String, OwnedValue> = body.deserialize().ok()?;

    let device_type = props.get("Type").and_then(|v| <u32>::try_from(v).ok()).unwrap_or(0);
    let present = props.get("IsPresent").and_then(|v| <bool>::try_from(v).ok()).unwrap_or(false);
    let percentage = props.get("Percentage").and_then(|v| <f64>::try_from(v).ok()).unwrap_or(100.0);
    let state = props.get("State").and_then(|v| <u32>::try_from(v).ok()).unwrap_or(0);
    let time_to_empty = props.get("TimeToEmpty").and_then(|v| <i64>::try_from(v).ok()).unwrap_or(0);
    let time_to_full = props.get("TimeToFull").and_then(|v| <i64>::try_from(v).ok()).unwrap_or(0);
    reading_from_upower(device_type, present, percentage, state, time_to_empty, time_to_full)
}

/// UPower's DisplayDevice as a reading, or `None` when it is not a battery.
///
/// UPower always answers for /DisplayDevice, whether or not the machine has one. On a desktop
/// or a VM it comes back Type=0 (Unknown), IsPresent=false, Percentage=0, State=0, and read
/// without this check that is indistinguishable from a laptop about to die. The desktop duly
/// announced "Battery critical: Battery at 0%. Plug in now." on a virtual machine with an
/// empty /sys/class/power_supply, which is where it was photographed.
///
/// Type 2 is Battery in the UPower enumeration. A machine without one reports no battery
/// rather than an empty one, which is also why no desktop PC running Ubuntu shows a battery
/// icon.
pub(crate) fn reading_from_upower(
    device_type: u32,
    present: bool,
    percentage: f64,
    state: u32,
    time_to_empty_secs: i64,
    time_to_full_secs: i64,
) -> Option<Reading> {
    if device_type != 2 || !present {
        return None;
    }
    // UPower State: 1 Charging, 2 Discharging, 3 Empty, 4 Fully charged, 5 Pending charge,
    // 6 Pending discharge. "Pending charge" is the charge-limit case: on the charger, taking
    // none. "Pending discharge" is the battery about to be used.
    let state = match state {
        1 => BatteryState::Charging,
        2 | 3 | 6 => BatteryState::Discharging,
        4 => BatteryState::Full,
        5 => BatteryState::PluggedNotCharging,
        _ => BatteryState::Unknown,
    };
    // A time only means something for the direction the battery is going, and UPower reports 0
    // for "do not know". Neither is shown as a number.
    let mins = |secs: i64| if secs > 0 { Some((secs / 60) as u32) } else { None };
    Some(Reading {
        level: percentage.round().clamp(0.0, 100.0) as u8,
        state,
        time_to_empty_mins: if state == BatteryState::Discharging { mins(time_to_empty_secs) } else { None },
        time_to_full_mins: if state == BatteryState::Charging { mins(time_to_full_secs) } else { None },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The photographed bug: a VM answers for /DisplayDevice with nothing in it.
    #[test]
    fn a_machine_with_no_battery_reads_as_none_not_as_zero_percent() {
        assert_eq!(reading_from_upower(0, false, 0.0, 0, 0, 0), None);
        assert_eq!(reading_from_upower(2, false, 0.0, 0, 0, 0), None, "type battery but not present");
        assert_eq!(reading_from_upower(1, true, 80.0, 2, 0, 0), None, "a line power device");
    }

    #[test]
    fn discharging_carries_time_to_empty_only() {
        let r = reading_from_upower(2, true, 64.4, 2, 8100, 999).unwrap();
        assert_eq!(r.level, 64);
        assert_eq!(r.state, BatteryState::Discharging);
        assert_eq!(r.time_to_empty_mins, Some(135));
        assert_eq!(r.time_to_full_mins, None);
    }

    #[test]
    fn charging_carries_time_to_full_only() {
        let r = reading_from_upower(2, true, 60.0, 1, 999, 2400).unwrap();
        assert_eq!(r.state, BatteryState::Charging);
        assert_eq!((r.time_to_empty_mins, r.time_to_full_mins), (None, Some(40)));
    }

    /// Zero seconds is UPower's "no estimate yet", right after unplugging.
    #[test]
    fn an_unknown_estimate_is_none_not_zero_minutes() {
        let r = reading_from_upower(2, true, 50.0, 2, 0, 0).unwrap();
        assert_eq!(r.time_to_empty_mins, None);
    }

    #[test]
    fn upower_states_map_to_the_four_a_person_sees() {
        let state = |s| reading_from_upower(2, true, 80.0, s, 0, 0).unwrap().state;
        assert_eq!(state(4), BatteryState::Full);
        assert_eq!(state(5), BatteryState::PluggedNotCharging);
        assert_eq!(state(6), BatteryState::Discharging);
        assert_eq!(state(3), BatteryState::Discharging);
        assert_eq!(state(0), BatteryState::Unknown);
    }

    /// The old alerts read `charging` to mean "do not ask the person to plug in".
    #[test]
    fn a_battery_held_at_its_limit_is_on_a_charger_for_the_alerts() {
        let event = reading_from_upower(2, true, 80.0, 5, 0, 0).unwrap().into_event();
        match event {
            SystemEvent::BatteryChanged { charging, state, .. } => {
                assert!(charging);
                assert_eq!(state, BatteryState::PluggedNotCharging);
            }
            other => panic!("{other:?}"),
        }
    }
}
