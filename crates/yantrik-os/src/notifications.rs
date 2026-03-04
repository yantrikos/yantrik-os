//! Notification daemon entry point — delegates to [`crate::dbus_notif`].
//!
//! This module exists as a thin wrapper so that `observer.rs` can call
//! `crate::notifications::run_notification_daemon(tx)` without changing.
//! All implementation lives in the public `dbus_notif` module.

use crossbeam_channel::Sender;

use crate::events::SystemEvent;

/// Run the notification daemon. See [`crate::dbus_notif::run_notification_daemon`].
pub fn run_notification_daemon(tx: Sender<SystemEvent>) {
    crate::dbus_notif::run_notification_daemon(tx);
}
