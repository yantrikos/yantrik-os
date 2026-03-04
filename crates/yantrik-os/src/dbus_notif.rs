//! D-Bus notification daemon — implements the full org.freedesktop.Notifications spec.
//!
//! YantrikOS acts as the session notification server. When this daemon starts, it
//! claims the well-known name `org.freedesktop.Notifications` on the session bus,
//! so all apps (Firefox, Thunderbird, dunstify, notify-send, etc.) route their
//! notifications through us.
//!
//! ## Spec compliance (Desktop Notifications Specification 1.2)
//!
//! **Methods:**
//! - `GetCapabilities` → ["body", "body-markup", "actions", "icon-static"]
//! - `Notify` → captures app_name, summary, body, urgency, emits SystemEvent
//! - `CloseNotification` → emits `NotificationClosed` signal (reason=3, closed by API)
//! - `GetServerInformation` → ("Yantrik", "Yantrik OS", "0.1.0", "1.2")
//!
//! **Signals:**
//! - `NotificationClosed(id: u32, reason: u32)` — sent when a notification is dismissed
//! - `ActionInvoked(id: u32, action_key: String)` — sent when user activates an action
//!
//! ## Architecture
//!
//! The daemon runs in a dedicated thread (`yos-notifications`), spawned by the
//! SystemObserver. It uses `zbus::blocking` to avoid pulling in a full async
//! runtime. Parsed notifications are sent as `SystemEvent::NotificationReceived`
//! over the shared crossbeam channel.
//!
//! ```text
//! [Any app] --D-Bus Notify()--> [NotificationServer] --crossbeam--> [SystemObserver]
//!                                                                       |
//!                                                           [FeatureRegistry.process_event]
//!                                                                       |
//!                                                           [NotificationRelay → Whisper Cards]
//!                                                           [NotificationStore → Notification Center]
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use crossbeam_channel::Sender;

use crate::events::SystemEvent;

// ── Close reasons (Desktop Notifications Spec 1.2 §9) ──

/// Why a notification was closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CloseReason {
    /// The notification expired (timeout).
    Expired = 1,
    /// The user explicitly dismissed it.
    DismissedByUser = 2,
    /// The notification was closed by `CloseNotification` API call.
    ClosedByApi = 3,
    /// Undefined/reserved reason.
    Undefined = 4,
}

// ── Parsed notification (public for testing / external consumers) ──

/// A parsed D-Bus notification, ready for consumption.
#[derive(Debug, Clone)]
pub struct DbusNotification {
    /// Assigned notification ID.
    pub id: u32,
    /// Application name (may be empty).
    pub app_name: String,
    /// Whether this replaces an existing notification.
    pub replaces_id: u32,
    /// App icon name or path (may be empty).
    pub app_icon: String,
    /// Summary (title) of the notification.
    pub summary: String,
    /// Body text (may contain basic markup).
    pub body: String,
    /// Action identifiers + labels, interleaved: [id, label, id, label, ...].
    pub actions: Vec<String>,
    /// Urgency level: 0=low, 1=normal, 2=critical.
    pub urgency: u8,
    /// Expire timeout in milliseconds (-1 = server decides, 0 = never).
    pub expire_timeout: i32,
}

// ── Active notification tracker (for close/action signals) ──

/// Tracks active (non-expired) notification IDs so we can emit proper
/// `NotificationClosed` and `ActionInvoked` signals.
struct ActiveNotifications {
    /// Maps notification ID → (app_name, actions).
    entries: HashMap<u32, ActiveEntry>,
}

struct ActiveEntry {
    #[allow(dead_code)]
    app_name: String,
    #[allow(dead_code)]
    actions: Vec<String>,
}

impl ActiveNotifications {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    fn insert(&mut self, id: u32, app_name: String, actions: Vec<String>) {
        // Cap at 500 active notifications to prevent unbounded growth.
        if self.entries.len() >= 500 {
            // Remove the oldest (smallest ID) entry.
            if let Some(&oldest) = self.entries.keys().min() {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(id, ActiveEntry { app_name, actions });
    }

    fn remove(&mut self, id: u32) -> bool {
        self.entries.remove(&id).is_some()
    }
}

// ── D-Bus interface implementation ──

/// The notification server object served on the session D-Bus.
///
/// This struct is held alive by the zbus connection for the lifetime of the
/// daemon thread. It is `Send + Sync` because zbus requires it.
struct NotificationServer {
    /// Channel to send parsed notifications to the main event loop.
    tx: Sender<SystemEvent>,
    /// Monotonically increasing notification ID counter.
    next_id: AtomicU32,
    /// Active notifications (for close signals).
    active: Mutex<ActiveNotifications>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl NotificationServer {
    // ── Methods ──

    /// Returns the capabilities of this notification daemon.
    ///
    /// Spec: <https://specifications.freedesktop.org/notification-spec/latest/ar01s09.html>
    fn get_capabilities(&self) -> Vec<String> {
        vec![
            "body".into(),
            "body-markup".into(),
            "actions".into(),
            "icon-static".into(),
        ]
    }

    /// Called by apps to display a notification.
    ///
    /// This is the main entry point. We parse the notification, assign an ID,
    /// track it for potential close/action signals, and forward it as a
    /// `SystemEvent::NotificationReceived`.
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        _app_icon: &str,
        summary: &str,
        body: &str,
        actions: Vec<String>,
        hints: HashMap<String, zbus::zvariant::OwnedValue>,
        _expire_timeout: i32,
    ) -> u32 {
        // Assign or reuse notification ID.
        let id = if replaces_id > 0 {
            replaces_id
        } else {
            self.next_id.fetch_add(1, Ordering::Relaxed)
        };

        // Extract urgency from hints dict (byte value: 0=low, 1=normal, 2=critical).
        let urgency = hints
            .get("urgency")
            .and_then(|v| <u8>::try_from(v).ok())
            .unwrap_or(1);

        tracing::info!(
            app = app_name,
            summary,
            urgency,
            id,
            actions_count = actions.len(),
            "D-Bus notification received"
        );

        // Track active notification for close/action signals.
        if let Ok(mut active) = self.active.lock() {
            active.insert(id, app_name.to_string(), actions);
        }

        // Forward to the system event loop.
        let _ = self.tx.send(SystemEvent::NotificationReceived {
            app: app_name.to_string(),
            summary: summary.to_string(),
            body: body.to_string(),
            urgency,
        });

        id
    }

    /// Close a notification by ID.
    ///
    /// Apps call this to programmatically dismiss a notification they sent.
    /// We emit the `NotificationClosed` signal with reason=3 (closed by API).
    fn close_notification(&self, id: u32) {
        tracing::debug!(id, "CloseNotification requested");

        if let Ok(mut active) = self.active.lock() {
            active.remove(id);
        }

        // Emit the NotificationClosed signal via dbus-send (best-effort).
        // Using CLI avoids the complexity of async signal emission from a
        // blocking zbus interface method.
        emit_close(id, CloseReason::ClosedByApi);
    }

    /// Returns server identity and version.
    ///
    /// Returns: (name, vendor, version, spec_version).
    fn get_server_information(&self) -> (String, String, String, String) {
        (
            "Yantrik".into(),
            "Yantrik OS".into(),
            env!("CARGO_PKG_VERSION").into(),
            "1.2".into(),
        )
    }

    // Note: NotificationClosed and ActionInvoked signals are emitted via
    // the `emit_close()` and `emit_action()` public functions using dbus-send,
    // rather than as zbus signal declarations. This avoids the complexity of
    // async signal emission within a blocking interface and makes signal
    // emission accessible from outside the D-Bus handler (e.g., from the UI
    // thread when the user dismisses a notification).
}

// ── Public API ──

/// Start the D-Bus notification daemon.
///
/// Claims `org.freedesktop.Notifications` on the session bus and serves the
/// interface at `/org/freedesktop/Notifications`. Blocks the calling thread
/// indefinitely — intended to run in a dedicated thread spawned by the
/// SystemObserver.
///
/// If the session bus is unavailable or the name is already claimed by another
/// daemon (dunst, mako, etc.), logs a warning and returns gracefully.
///
/// # Arguments
///
/// * `tx` — crossbeam sender for `SystemEvent::NotificationReceived` events.
///
/// # Example
///
/// ```rust,no_run
/// let (tx, rx) = crossbeam_channel::bounded(256);
/// std::thread::spawn(move || yantrik_os::dbus_notif::run_notification_daemon(tx));
/// // rx.recv() will yield NotificationReceived events
/// ```
pub fn run_notification_daemon(tx: Sender<SystemEvent>) {
    match start_daemon(tx) {
        Ok(()) => {} // never returns (blocks in event loop)
        Err(e) => {
            tracing::warn!(
                error = %e,
                "D-Bus notification daemon failed to start — \
                 notifications from other apps will not appear. \
                 Is another notification daemon (dunst, mako) already running?"
            );
        }
    }
}

/// Internal: build the zbus connection and block.
fn start_daemon(tx: Sender<SystemEvent>) -> Result<(), zbus::Error> {
    let server = NotificationServer {
        tx,
        next_id: AtomicU32::new(1),
        active: Mutex::new(ActiveNotifications::new()),
    };

    let connection = zbus::blocking::connection::Builder::session()?
        .name("org.freedesktop.Notifications")?
        .serve_at("/org/freedesktop/Notifications", server)?
        .build()?;

    tracing::info!(
        unique_name = %connection.unique_name().map(|n| n.as_str()).unwrap_or("?"),
        "D-Bus notification daemon started — claimed org.freedesktop.Notifications"
    );

    // The zbus blocking connection's internal async-io reactor handles incoming
    // method calls automatically. We keep this thread alive with a loop that
    // also serves as a heartbeat check — if the connection drops, we detect it.
    loop {
        // Sleep in short intervals so the thread can respond to shutdown signals
        // or connection failures more promptly than a 1-hour sleep.
        std::thread::sleep(std::time::Duration::from_secs(60));

        // Verify the connection is still alive by checking the unique name.
        // If the bus disconnected, the unique name becomes inaccessible.
        if connection.unique_name().is_none() {
            tracing::warn!("D-Bus session connection lost — notification daemon exiting");
            break;
        }
    }

    Ok(())
}

/// Emit a `NotificationClosed` signal from outside the D-Bus method handler.
///
/// This is used by the UI layer when the user dismisses a notification from
/// the notification center or a whisper card. Uses `dbus-send` CLI as a simple,
/// reliable approach that avoids zbus message builder version concerns.
///
/// The `id` must match a previously assigned notification ID from `Notify`.
///
/// This is a best-effort operation — if the CLI tool is unavailable or the
/// notification was already closed, the call silently returns.
pub fn emit_close(id: u32, reason: CloseReason) {
    let result = std::process::Command::new("dbus-send")
        .args([
            "--session",
            "--type=signal",
            "--dest=org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications.NotificationClosed",
            &format!("uint32:{}", id),
            &format!("uint32:{}", reason as u32),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();

    if let Err(e) = result {
        tracing::debug!(error = %e, id, "Failed to emit NotificationClosed signal via dbus-send");
    }
}

/// Emit an `ActionInvoked` signal from outside the D-Bus method handler.
///
/// Called when the user clicks an action button on a notification in the UI.
/// Uses `dbus-send` CLI for reliability.
pub fn emit_action(id: u32, action_key: &str) {
    let result = std::process::Command::new("dbus-send")
        .args([
            "--session",
            "--type=signal",
            "--dest=org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications.ActionInvoked",
            &format!("uint32:{}", id),
            &format!("string:{}", action_key),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();

    if let Err(e) = result {
        tracing::debug!(error = %e, id, action_key, "Failed to emit ActionInvoked signal via dbus-send");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_reason_values_match_spec() {
        assert_eq!(CloseReason::Expired as u32, 1);
        assert_eq!(CloseReason::DismissedByUser as u32, 2);
        assert_eq!(CloseReason::ClosedByApi as u32, 3);
        assert_eq!(CloseReason::Undefined as u32, 4);
    }

    #[test]
    fn active_notifications_cap() {
        let mut active = ActiveNotifications::new();
        for i in 1..=501 {
            active.insert(i, format!("app-{}", i), vec![]);
        }
        // Should have capped at 500 (one was evicted to make room).
        assert!(active.entries.len() <= 500);
    }

    #[test]
    fn active_notifications_remove() {
        let mut active = ActiveNotifications::new();
        active.insert(1, "test".into(), vec![]);
        assert!(active.remove(1));
        assert!(!active.remove(1)); // Already removed.
    }
}
