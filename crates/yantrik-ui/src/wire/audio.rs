//! The volume slider and the machine's real volume.
//!
//! What the shell shows is read from PipeWire (`yantrik_os::audio`) when it starts and again
//! whenever the audio server says the default output changed, so the volume keys, another app
//! and this slider all end in the same number. The slider used to start at an invented 50 and
//! move an ALSA mixer the session's audio server does not use.

use std::cell::Cell;
use std::time::{Duration, Instant};

use slint::ComponentHandle;
use yantrik_os::audio::{self, AudioState};
use yantrik_os::SystemEvent;

use crate::app_context::AppContext;
use crate::App;

/// How long after the person's own slider move an `AudioChanged` is taken to be its echo. The
/// echo is read after the drag has already moved on, so applying it would pull the slider back
/// to a level a few hundred milliseconds old.
const ECHO_WINDOW: Duration = Duration::from_millis(400);

thread_local! {
    /// When the person last moved the slider or the mute. Both the callbacks and the event
    /// drain run on the thread that draws the screen.
    static LOCAL_CHANGE: Cell<Option<Instant>> = const { Cell::new(None) };
}

fn changed_locally() {
    LOCAL_CHANGE.with(|c| c.set(Some(Instant::now())));
}

fn is_echo() -> bool {
    LOCAL_CHANGE.with(|c| c.get()).is_some_and(|at| at.elapsed() < ECHO_WINDOW)
}

/// Put a reading on the screen. No reading means no audio server to ask, so the volume row is
/// not shown at all rather than showing a level nobody measured.
pub fn publish(ui: &App, state: Option<AudioState>) {
    match state {
        Some(s) => {
            ui.set_volume_available(true);
            ui.set_volume_level(i32::from(s.volume_pct));
            ui.set_volume_muted(s.muted);
        }
        None => {
            ui.set_volume_available(false);
            ui.set_volume_level(0);
            ui.set_volume_muted(false);
        }
    }
}

/// Name the default output under the slider, as the audio server names it, or name nothing.
/// Read off the UI thread (it runs `wpctl`), when the shell starts and each time Quick Settings
/// opens: the default output can change without its volume changing, so no volume event says so.
pub(super) fn refresh_output_name(ui: &App) {
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let name = audio::read_output_name().unwrap_or_default();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if ui.get_volume_device().as_str() != name {
                ui.set_volume_device(name.into());
            }
        });
    });
}

/// Read the machine and show it.
pub fn wire(ui: &App, _ctx: &AppContext) {
    publish(ui, audio::read());
    refresh_output_name(ui);

    // The slider already shows where the person put it; the worker makes the machine agree.
    // Moving it while muted is asking to hear something, so it unmutes, as the other desktops do.
    let apply = yantrik_os::latest::spawn_latest("yos-volume-set", |pct: u8| {
        if let Err(e) = audio::set_volume(pct) {
            tracing::warn!(error = %e, "volume was not set");
            return;
        }
        if audio::read().is_some_and(|s| s.muted) {
            let _ = audio::set_mute(false);
        }
    });
    let weak = ui.as_weak();
    ui.on_volume_changed(move |level| {
        changed_locally();
        if let Some(ui) = weak.upgrade() {
            ui.set_volume_muted(false);
        }
        let _ = apply.send(level.clamp(0, 100) as u8);
    });

    let weak = ui.as_weak();
    ui.on_volume_mute_toggled(move || {
        let Some(ui) = weak.upgrade() else { return };
        changed_locally();
        let want = !ui.get_volume_muted();
        if let Err(e) = audio::set_mute(want) {
            tracing::warn!(error = %e, "mute was not changed");
        }
        // The machine's answer, not the one that was asked for.
        publish(&ui, audio::read());
    });
}

/// Follow what the system poll drained: the latest `AudioChanged` is the machine's volume now.
pub fn apply_events(ui: &App, events: &[SystemEvent]) {
    let latest = events.iter().rev().find_map(|e| match e {
        SystemEvent::AudioChanged { volume_pct, muted } => Some(AudioState { volume_pct: *volume_pct, muted: *muted }),
        _ => None,
    });
    if let Some(state) = latest {
        if !is_echo() {
            publish(ui, Some(state));
        }
    }
}
