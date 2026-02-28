//! Battery monitor — reads UPower via D-Bus.
//!
//! On systems without UPower (like a minimal Alpine VM), this gracefully
//! logs a warning and exits. The mock observer provides fake battery events.

use crossbeam_channel::Sender;

use crate::events::SystemEvent;

/// Main loop for the battery monitor thread.
/// Polls UPower over D-Bus every 30 seconds.
pub fn run_battery_monitor(tx: Sender<SystemEvent>) {
    // Try to connect to the system D-Bus
    let connection = match zbus::blocking::Connection::system() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "No system D-Bus — battery monitor disabled");
            return;
        }
    };

    let proxy = match zbus::blocking::fdo::DBusProxy::new(&connection) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to create D-Bus proxy");
            return;
        }
    };

    // Check if UPower is available
    let names = match proxy.list_names() {
        Ok(n) => n,
        Err(_) => {
            tracing::warn!("Cannot list D-Bus names — battery monitor disabled");
            return;
        }
    };

    let has_upower = names
        .iter()
        .any(|n| n.as_str() == "org.freedesktop.UPower");

    if !has_upower {
        tracing::info!("UPower not available — battery monitor disabled");
        return;
    }

    tracing::info!("Battery monitor started (UPower)");

    // Poll battery state every 30 seconds
    loop {
        if let Some(event) = read_battery(&connection) {
            let _ = tx.send(event);
        }
        std::thread::sleep(std::time::Duration::from_secs(30));
    }
}

/// Read battery state from UPower via D-Bus.
fn read_battery(connection: &zbus::blocking::Connection) -> Option<SystemEvent> {
    // Call org.freedesktop.UPower.GetDisplayDevice() or enumerate devices
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
    let props: std::collections::HashMap<String, zbus::zvariant::OwnedValue> =
        body.deserialize().ok()?;

    let percentage = props
        .get("Percentage")
        .and_then(|v| <f64>::try_from(v).ok())
        .unwrap_or(100.0);

    // UPower State: 1=Charging, 2=Discharging, 3=Empty, 4=Full, 5=PendingCharge, 6=PendingDischarge
    let state = props
        .get("State")
        .and_then(|v| <u32>::try_from(v).ok())
        .unwrap_or(0);

    let charging = state == 1 || state == 5;

    let time_to_empty = props
        .get("TimeToEmpty")
        .and_then(|v| <i64>::try_from(v).ok())
        .and_then(|secs| {
            if secs > 0 {
                Some((secs / 60) as u32)
            } else {
                None
            }
        });

    Some(SystemEvent::BatteryChanged {
        level: percentage as u8,
        charging,
        time_to_empty_mins: time_to_empty,
    })
}
