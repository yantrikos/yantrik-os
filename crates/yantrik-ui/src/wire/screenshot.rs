//! Screenshot wire module — captures screen via grim/slurp and shows a notification.
//!
//! Called from the keybind handler in `system_poll.rs`:
//! - Print       -> `take_screenshot(FullScreen)`
//! - Shift+Print -> `take_screenshot(Region)`

use crate::app_context::AppContext;
use crate::App;

/// Wire screenshot callbacks. Currently no Slint callbacks to register,
/// but follows the wire module pattern for future UI integration
/// (e.g. a screenshot button in quick settings).
pub fn wire(_ui: &App, _ctx: &AppContext) {
    // Keybind-driven — capture is triggered from handle_keybind() in system_poll.rs
    // via the public `take_screenshot()` function below.
    //
    // If a Slint callback is added later (e.g. `on_take_screenshot`), wire it here.
}

/// Take a screenshot and show a toast notification in the UI.
///
/// This is called from the keybind handler (`system_poll::handle_keybind`).
/// Runs grim/slurp on a background thread to avoid blocking the UI event loop.
pub fn take_screenshot(ui_weak: slint::Weak<App>, mode: yantrik_os::screenshot::CaptureMode) {
    std::thread::spawn(move || {
        match yantrik_os::screenshot::capture(mode) {
            Ok(path) => {
                // Extract just the filename for the notification text
                let filename = std::path::Path::new(&path)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.clone());

                tracing::info!(path = %path, "Screenshot captured");

                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_notification_text(
                            format!("Screenshot saved: {filename}").into(),
                        );
                        ui.set_show_notification(true);
                    }
                });
            }
            Err(e) => {
                // Don't show notification for user-cancelled region selection
                if e.contains("cancelled") {
                    tracing::debug!("Screenshot region selection cancelled");
                    return;
                }

                tracing::warn!(error = %e, "Screenshot capture failed");

                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_notification_text(
                            format!("Screenshot failed: {e}").into(),
                        );
                        ui.set_show_notification(true);
                    }
                });
            }
        }
    });
}
