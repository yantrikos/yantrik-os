//! Network monitor — reads NetworkManager via D-Bus.
//!
//! On systems without NetworkManager, gracefully disables.
//! The mock observer provides fake network events.

use crossbeam_channel::Sender;

use crate::events::SystemEvent;

/// Main loop for the network monitor thread.
/// Polls NetworkManager over D-Bus every 15 seconds.
pub fn run_network_monitor(tx: Sender<SystemEvent>) {
    let connection = match zbus::blocking::Connection::system() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "No system D-Bus — network monitor disabled");
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

    let names = match proxy.list_names() {
        Ok(n) => n,
        Err(_) => {
            tracing::warn!("Cannot list D-Bus names — network monitor disabled");
            return;
        }
    };

    let has_nm = names
        .iter()
        .any(|n| n.as_str() == "org.freedesktop.NetworkManager");

    if !has_nm {
        tracing::info!("NetworkManager not available — using /sys/class/net fallback");
        run_fallback_monitor(tx);
        return;
    }

    tracing::info!("Network monitor started (NetworkManager)");

    loop {
        if let Some(event) = read_network_state(&connection) {
            let _ = tx.send(event);
        }
        std::thread::sleep(std::time::Duration::from_secs(15));
    }
}

/// Fallback network monitor for systems without NetworkManager.
/// Checks /sys/class/net for carrier state.
fn run_fallback_monitor(tx: Sender<SystemEvent>) {
    // Send initial state immediately
    let mut last_connected = is_any_interface_up();
    let _ = tx.send(SystemEvent::NetworkChanged {
        connected: last_connected,
        ssid: None,
        signal: None,
    });
    loop {
        std::thread::sleep(std::time::Duration::from_secs(15));
        let connected = is_any_interface_up();
        if connected != last_connected {
            let _ = tx.send(SystemEvent::NetworkChanged {
                connected,
                ssid: None,
                signal: None,
            });
            last_connected = connected;
        }
    }
}

/// Check if any non-loopback interface is up with a carrier.
fn is_any_interface_up() -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/class/net") else { return false };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "lo" { continue; }
        let operstate = entry.path().join("operstate");
        if let Ok(state) = std::fs::read_to_string(&operstate) {
            if state.trim() == "up" {
                return true;
            }
        }
    }
    false
}

/// Read network state from NetworkManager via D-Bus.
fn read_network_state(connection: &zbus::blocking::Connection) -> Option<SystemEvent> {
    // Get overall connectivity state
    let msg = connection
        .call_method(
            Some("org.freedesktop.NetworkManager"),
            "/org/freedesktop/NetworkManager",
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(
                "org.freedesktop.NetworkManager",
                "State",
            ),
        )
        .ok()?;

    let body = msg.body();
    let variant: zbus::zvariant::OwnedValue = body.deserialize().ok()?;
    let state: u32 = variant.try_into().ok()?;

    // NM State: 70 = connected global, 60 = connected site, 50 = connected local
    let connected = state >= 60;

    // Try to get active connection SSID
    let ssid = read_active_ssid(connection);

    Some(SystemEvent::NetworkChanged {
        connected,
        ssid,
        signal: None, // Could read from wireless device properties
    })
}

/// Try to read the SSID of the active wireless connection.
fn read_active_ssid(connection: &zbus::blocking::Connection) -> Option<String> {
    // Get primary active connection path
    let msg = connection
        .call_method(
            Some("org.freedesktop.NetworkManager"),
            "/org/freedesktop/NetworkManager",
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(
                "org.freedesktop.NetworkManager",
                "PrimaryConnection",
            ),
        )
        .ok()?;

    let body = msg.body();
    let variant: zbus::zvariant::OwnedValue = body.deserialize().ok()?;
    let conn_path: zbus::zvariant::OwnedObjectPath = variant.try_into().ok()?;

    if conn_path.as_str() == "/" {
        return None;
    }

    // Get the connection's ID (which is usually the SSID for WiFi)
    let msg = connection
        .call_method(
            Some("org.freedesktop.NetworkManager"),
            conn_path.as_str(),
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(
                "org.freedesktop.NetworkManager.Connection.Active",
                "Id",
            ),
        )
        .ok()?;

    let body = msg.body();
    let variant: zbus::zvariant::OwnedValue = body.deserialize().ok()?;
    let id: String = variant.try_into().ok()?;

    Some(id)
}
