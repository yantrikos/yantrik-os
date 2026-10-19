//! The screenshot the command palette and Quick Settings take: the whole screen, saved and
//! copied, with the same notification the Print keys give. It is `wire::capture_bar`'s person
//! screenshot, not a second implementation — this file used to run its own grim through
//! `yantrik_os::screenshot`, with no deadline and a UTC file name.

use std::time::Duration;

use crate::app_context::AppContext;
use crate::capture::shot::{Area, Keep};
use crate::App;

/// How long Quick Settings' Screenshot waits after the panel is put away: its fade is 120 ms and
/// a capture that started inside it would photograph the panel it was pressed on.
const PANEL_FADE: Duration = Duration::from_millis(400);

/// Quick Settings' Screenshot is a Slint callback, the whole screen like the Print key used to be.
pub fn wire(ui: &App, _ctx: &AppContext) {
    ui.on_take_screenshot(|| {
        crate::wire::capture_bar::person_shot(Area::Screen, PANEL_FADE, false, Keep::FileAndClipboard);
    });
}

/// The command palette's "Take a screenshot".
pub fn take_screenshot() {
    crate::wire::capture_bar::person_shot(Area::Screen, PANEL_FADE, false, Keep::FileAndClipboard);
}
