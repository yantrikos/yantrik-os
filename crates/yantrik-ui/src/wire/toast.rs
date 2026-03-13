//! Toast notification banner wiring.
//!
//! Manages the stacking glassmorphic toast system. When a NotificationReceived
//! event fires (via system_poll.rs), `push_toast()` adds a ToastData entry to
//! the UI queue. Toasts auto-dismiss based on urgency (3s low, 5s normal, 10s
//! critical). Click navigates to the notification center (screen 9).
//!
//! All toasts are also persisted to the notification store so they appear
//! in the Notification Center (screen 9). Uses a thread-local reference
//! since all toast pushes run on the main (UI) thread.

use std::cell::RefCell;
use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::notifications;
use crate::App;

thread_local! {
    static NOTIFICATION_STORE: RefCell<Option<notifications::SharedStore>> = RefCell::new(None);
}

/// Wire toast notification callbacks and set up the thread-local store.
pub fn wire(ui: &App, ctx: &AppContext) {
    // Install the notification store in thread-local so push_toast can use it
    NOTIFICATION_STORE.with(|cell| {
        *cell.borrow_mut() = Some(ctx.notification_store.clone());
    });

    let ui_weak = ui.as_weak();

    // Click toast -> open notification center
    ui.on_toast_clicked(move |_id| {
        if let Some(ui) = ui_weak.upgrade() {
            // Clear toasts
            ui.set_toast_queue(ModelRc::new(VecModel::from(Vec::<crate::ToastData>::new())));
            // Navigate to notification center
            ui.set_current_screen(9);
            ui.invoke_navigate(9);
        }
    });

    let ui_weak2 = ui.as_weak();
    ui.on_toast_dismissed(move |id| {
        if let Some(ui) = ui_weak2.upgrade() {
            remove_toast(&ui, id);
        }
    });
}

/// Push a new toast notification. Also persists to the notification store
/// if called on the main thread (where the store is initialized).
/// Set `skip_store` to true if the caller already stored the notification
/// (e.g. the D-Bus path in system_poll.rs).
pub fn push_toast(
    ui_weak: &slint::Weak<App>,
    app: &str,
    summary: &str,
    body: &str,
    urgency: u8,
) {
    push_toast_ex(ui_weak, app, summary, body, urgency, false);
}

/// Push a toast without storing to the notification center.
/// Use this when the caller already stores the notification separately.
pub fn push_toast_no_store(
    ui_weak: &slint::Weak<App>,
    app: &str,
    summary: &str,
    body: &str,
    urgency: u8,
) {
    push_toast_ex(ui_weak, app, summary, body, urgency, true);
}

fn push_toast_ex(
    ui_weak: &slint::Weak<App>,
    app: &str,
    summary: &str,
    body: &str,
    urgency: u8,
    skip_store: bool,
) {
    // Persist to notification store (thread-local, only works on main thread)
    if !skip_store {
        NOTIFICATION_STORE.with(|cell| {
            if let Some(ref store) = *cell.borrow() {
                store.borrow_mut().push(
                    app.to_string(),
                    summary.to_string(),
                    body.to_string(),
                    urgency,
                );
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_notification_unread_count(store.borrow().unread_count() as i32);
                }
            }
        });
    }

    if let Some(ui) = ui_weak.upgrade() {
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i32;

        let icon_char = app
            .chars()
            .next()
            .unwrap_or('N')
            .to_uppercase()
            .to_string();

        let toast = crate::ToastData {
            id,
            app_name: app.into(),
            summary: summary.into(),
            body: body.chars().take(80).collect::<String>().into(),
            urgency: urgency as i32,
            icon_char: icon_char.into(),
        };

        // Get current toasts, keep max 3
        let current = ui.get_toast_queue();
        let mut toasts: Vec<crate::ToastData> = (0..current.row_count())
            .filter_map(|i| current.row_data(i))
            .collect();

        // Remove oldest if at 3
        while toasts.len() >= 3 {
            toasts.remove(0);
        }
        toasts.push(toast);
        ui.set_toast_queue(ModelRc::new(VecModel::from(toasts)));

        // Auto-dismiss timer
        let dismiss_ms = match urgency {
            0 => 3000,  // low
            2 => 10000, // critical
            _ => 5000,  // normal
        };

        let ui_weak = ui.as_weak();
        let timer = Timer::default();
        timer.start(
            TimerMode::SingleShot,
            Duration::from_millis(dismiss_ms),
            move || {
                if let Some(ui) = ui_weak.upgrade() {
                    remove_toast(&ui, id);
                }
            },
        );
        std::mem::forget(timer);
    }
}

fn remove_toast(ui: &App, id: i32) {
    let current = ui.get_toast_queue();
    let toasts: Vec<crate::ToastData> = (0..current.row_count())
        .filter_map(|i| current.row_data(i))
        .filter(|t| t.id != id)
        .collect();
    ui.set_toast_queue(ModelRc::new(VecModel::from(toasts)));
}
