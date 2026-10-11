//! Screenshot wire module — captures screen via grim/slurp and shows a notification.
//!
//! Called from the command palette. The Print keys are bound in `config/labwc/rc.xml`.

use slint::ComponentHandle;

use crate::app_context::AppContext;
use crate::App;

/// How long Quick Settings' Screenshot waits after the panel is put away: its fade is 120 ms and
/// a capture that started inside it would photograph the panel it was pressed on.
const PANEL_FADE: std::time::Duration = std::time::Duration::from_millis(400);

/// Wire the screenshot callbacks. The palette calls `take_screenshot()` directly; Quick Settings'
/// Screenshot is a Slint callback, the whole screen like the Print key.
pub fn wire(ui: &App, _ctx: &AppContext) {
    let weak = ui.as_weak();
    ui.on_take_screenshot(move || {
        let weak = weak.clone();
        std::thread::spawn(move || {
            std::thread::sleep(PANEL_FADE);
            take_screenshot(weak, yantrik_os::screenshot::CaptureMode::FullScreen);
        });
    });
}

/// Take a screenshot and show a toast notification in the UI.
///
/// This is called from the command palette.
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

                // Through the notifications service — this thread is not the UI thread, and
                // `notify::send` does not need it to be. It used to hop back to the event loop
                // to raise a private toast that nothing kept, so "where did that screenshot
                // go" was unanswerable six seconds later.
                let _ = &ui_weak;
                yantrik_app_runtime::notify::send(
                    yantrik_app_runtime::notify::Notification::new("Screenshot", toast_body)
                        .urgency(yantrik_app_runtime::notify::Level::Low),
                );
            }
            Err(e) => {
                // Don't show notification for user-cancelled region selection
                if e.contains("cancelled") {
                    tracing::debug!("Screenshot region selection cancelled");
                    return;
                }

                tracing::warn!(error = %e, "Screenshot capture failed");

                let _ = &ui_weak;
                yantrik_app_runtime::notify::send(
                    yantrik_app_runtime::notify::Notification::new(
                        "Screenshot",
                        format!("Failed: {e}"),
                    )
                    .urgency(yantrik_app_runtime::notify::Level::Critical),
                );
            }
        }
    });
}
