//! Screenshot wire module — captures screen via grim/slurp and shows a notification.
//!
//! Called from the keybind handler in `system_poll.rs`:
//! - Print         -> `take_screenshot(FullScreen)`       (save to file)
//! - Shift+Print   -> `take_screenshot(Region)`           (save to file)
//! - Ctrl+Print    -> `take_screenshot(ClipboardFull)`    (copy to clipboard)
//! - Ctrl+S+Print  -> `take_screenshot(ClipboardRegion)`  (copy to clipboard)

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
///
/// For file modes, the toast shows the saved filename.
/// For clipboard modes, the toast shows "Copied to clipboard".
pub fn take_screenshot(ui_weak: slint::Weak<App>, mode: yantrik_os::screenshot::CaptureMode) {
    std::thread::spawn(move || {
        match yantrik_os::screenshot::capture(mode) {
            Ok(msg) => {
                // Clipboard modes return "Copied to clipboard";
                // file modes return the absolute path.
                let toast_body = if msg.starts_with('/') {
                    // File path — extract just the filename for a cleaner toast
                    let filename = std::path::Path::new(&msg)
                        .file_name()
                        .map(|f| f.to_string_lossy().to_string())
                        .unwrap_or_else(|| msg.clone());
                    format!("Saved: {filename}")
                } else {
                    // Clipboard or other message — show as-is
                    msg.clone()
                };

                tracing::info!(result = %msg, "Screenshot captured");

                let _ = slint::invoke_from_event_loop(move || {
                    super::toast::push_toast(
                        &ui_weak,
                        "Screenshot",
                        &toast_body,
                        "",
                        0, // low urgency
                    );
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
                    super::toast::push_toast(
                        &ui_weak,
                        "Screenshot",
                        &format!("Failed: {e}"),
                        "",
                        2, // critical urgency
                    );
                });
            }
        }
    });
}
