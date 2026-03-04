//! Toast notification banner wiring.
//!
//! Manages the stacking glassmorphic toast system. When a NotificationReceived
//! event fires (via system_poll.rs), `push_toast()` adds a ToastData entry to
//! the UI queue. Toasts auto-dismiss based on urgency (3s low, 5s normal, 10s
//! critical). Click navigates to the notification center (screen 9).

use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::App;

/// Wire toast notification callbacks.
pub fn wire(ui: &App, _ctx: &AppContext) {
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

/// Push a new toast notification. Called from system_poll.rs when
/// NotificationReceived fires, and from other modules (screenshot, focus, bridge).
pub fn push_toast(
    ui_weak: &slint::Weak<App>,
    app: &str,
    summary: &str,
    body: &str,
    urgency: u8,
) {
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
