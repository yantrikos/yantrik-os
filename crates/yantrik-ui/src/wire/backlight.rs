//! The brightness slider and the machine's real backlight.
//!
//! A desktop monitor or a VM has no backlight, and then there is no brightness control: the
//! row is not drawn. Where there is one, the slider starts at the level the panel is really at
//! (it used to start at an invented 80) and is read again on the system poll, so the brightness
//! keys move it too.

use std::cell::Cell;
use std::time::{Duration, Instant};

use yantrik_os::backlight;

use crate::app_context::AppContext;
use crate::App;

/// How long after the person's own slider move a reading of the panel is not applied: the
/// worker may not have reached the last value yet, and an older reading would pull the slider
/// back mid-drag.
const SETTLE: Duration = Duration::from_millis(600);

thread_local! {
    static LOCAL_CHANGE: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Put a reading on the screen. No reading means no backlight: the row goes away.
pub fn publish(ui: &App, level: Option<u8>) {
    ui.set_brightness_available(level.is_some());
    ui.set_brightness_level(level.map_or(0, i32::from));
}

pub fn wire(ui: &App, _ctx: &AppContext) {
    publish(ui, backlight::read());

    let apply = yantrik_os::latest::spawn_latest("yos-brightness-set", |pct: u8| {
        if let Err(e) = backlight::set(pct) {
            tracing::warn!(error = %e, "brightness was not set");
        }
    });
    ui.on_brightness_changed(move |level| {
        LOCAL_CHANGE.with(|c| c.set(Some(Instant::now())));
        let _ = apply.send(level.clamp(0, 100) as u8);
    });
}

/// Called on the system poll's tick: follow the brightness keys. One small file read, and only
/// on a machine that has a backlight; the property is not written when the level is the same,
/// so a still screen stays still.
pub fn refresh(ui: &App) {
    if !ui.get_brightness_available() {
        return;
    }
    if LOCAL_CHANGE.with(|c| c.get()).is_some_and(|at| at.elapsed() < SETTLE) {
        return;
    }
    if let Some(level) = backlight::read() {
        ui.set_brightness_level(i32::from(level));
    }
}
