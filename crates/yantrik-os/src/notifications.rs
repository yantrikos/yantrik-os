//! D-Bus notification daemon — implements org.freedesktop.Notifications.
//!
//! Registers as the session notification daemon so all app notifications flow
//! through Yantrik instead of a separate daemon (dunst, mako, etc.).
//! Notifications are emitted as SystemEvent::NotificationReceived, flowing
//! through the event system → features → Whisper Cards → AI memory.

use std::sync::atomic::{AtomicU32, Ordering};

use crossbeam_channel::Sender;

use crate::events::SystemEvent;

/// Notification server object served on the session D-Bus.
struct NotificationServer {
    tx: Sender<SystemEvent>,
    next_id: AtomicU32,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl NotificationServer {
    /// Returns the capabilities of this notification daemon.
    fn get_capabilities(&self) -> Vec<String> {
        vec![
            "body".into(),
            "body-markup".into(),
            "actions".into(),
        ]
    }

    /// Called by apps to display a notification.
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        _app_icon: &str,
        summary: &str,
        body: &str,
        _actions: Vec<String>,
        hints: std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
        _expire_timeout: i32,
    ) -> u32 {
        let id = if replaces_id > 0 {
            replaces_id
        } else {
            self.next_id.fetch_add(1, Ordering::Relaxed)
        };

        // Extract urgency from hints (0=low, 1=normal, 2=critical)
        let urgency = hints
            .get("urgency")
            .and_then(|v| <u8>::try_from(v).ok())
            .unwrap_or(1);

        tracing::info!(
            app = app_name,
            summary,
            urgency,
            id,
            "Notification received via D-Bus"
        );

        let _ = self.tx.send(SystemEvent::NotificationReceived {
            app: app_name.to_string(),
            summary: summary.to_string(),
            body: body.to_string(),
            urgency,
        });

        id
    }

    /// Close a notification by ID.
    fn close_notification(&self, id: u32) {
        tracing::debug!(id, "Notification close requested");
    }

    /// Returns server identity.
    fn get_server_information(&self) -> (String, String, String, String) {
        (
            "Yantrik".into(),
            "Yantrik OS".into(),
            "0.1.0".into(),
            "1.2".into(),
        )
    }
}

/// Run the notification daemon on the session D-Bus.
/// Claims `org.freedesktop.Notifications` and serves the interface.
/// Blocks forever (intended to run in a dedicated thread).
pub fn run_notification_daemon(tx: Sender<SystemEvent>) {
    match start_daemon(tx) {
        Ok(()) => {} // never returns
        Err(e) => {
            tracing::warn!(error = %e, "Notification daemon failed to start — notifications disabled");
        }
    }
}

fn start_daemon(tx: Sender<SystemEvent>) -> Result<(), zbus::Error> {
    let server = NotificationServer {
        tx,
        next_id: AtomicU32::new(1),
    };

    let _connection = zbus::blocking::connection::Builder::session()?
        .name("org.freedesktop.Notifications")?
        .serve_at("/org/freedesktop/Notifications", server)?
        .build()?;

    tracing::info!("Notification daemon started on session D-Bus");

    // Keep the connection alive. The internal async-io runtime handles incoming
    // method calls automatically.
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
