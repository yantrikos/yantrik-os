//! The actions behind the media keys and Quick Settings' levels: `set_volume`, `set_mute`,
//! `set_brightness`, `set_mic_mute` and `show_caps_lock`. The rules (what a level or a step
//! means, what is read back) are in `control_levels`; this wires them to the machine and to the
//! on-screen display.
//!
//! All four that change something are `standard`: they move a level the person can put back
//! with the same key or slider, and nothing is stored by the shell. `show_caps_lock` is `safe`:
//! it only shows what the keyboard's LED says. None of them is held while an approval card
//! waits (`hold_windows` is for acts that put a window over the card): the pill is 240x64 above
//! the taskbar, inert, and gone in 1.2 s, and a person who presses mute with a card up has to
//! be heard.
//!
//! The machine is read and written on the socket's side of `answer_later`, not on the UI thread,
//! so a `wpctl` that hangs cannot freeze the shell. The screen is updated, and the pill shown,
//! from there through the event loop with the reading just taken.

use serde_json::{json, Value};
use yantrik_app_runtime::control::{answer_later, Action, App as ControlSurface, Param};

use crate::control_levels::{self as levels, Machine};
use crate::wire::osd::{self, Osd};
use crate::App;

/// Run `work` off the UI thread when there is a dispatch to answer later, else right here.
fn off_the_ui_thread(work: impl FnOnce() -> Result<Value, String> + Send + 'static) -> Result<Value, String> {
    answer_later(work)
        .map(|()| json!({ "answering": "off the UI thread" }))
        .or_else(|work| work())
}

/// Update what the shell shows and put the pill up, on the UI thread, with a reading just taken.
fn on_screen(weak: &slint::Weak<App>, f: impl FnOnce(&App) + Send + 'static) {
    let _ = weak.upgrade_in_event_loop(move |ui| f(&ui));
}

pub fn actions(surface: ControlSurface, ui: &App) -> ControlSurface {
    let (volume_ui, mute_ui, brightness_ui, mic_ui, caps_ui) =
        (ui.as_weak(), ui.as_weak(), ui.as_weak(), ui.as_weak(), ui.as_weak());
    surface
        .action(
            // Quick Settings' volume slider and the volume keys, for a caller. The answer is the
            // volume read back from PipeWire, which can differ from what was asked for. The
            // same pill shows for a key and for a mind, so a person sees a volume change happen.
            Action::new("set_volume", "Set the machine's output volume on the default speaker: `level` goes to 0 to 100 percent, `step` moves it by that many (the volume keys send 5 and -5). A step up unmutes. Shows the level on screen")
                .risk("standard")
                .arg(Param::integer("level").describe("Percent, 0 to 100. Give this or `step`").optional())
                .arg(Param::integer("step").describe("Percent to move by, -100 to 100, not 0. Give this or `level`").optional()),
            move |args| {
                let (args, weak) = (args.clone(), volume_ui.clone());
                off_the_ui_thread(move || {
                    let state = levels::set_volume(&args, &Machine)?;
                    on_screen(&weak, move |ui| {
                        crate::wire::audio::publish(ui, Some(state));
                        osd::show(ui, Osd::Volume(state));
                    });
                    Ok(levels::audio_answer(state))
                })
            },
        )
        .action(
            Action::new("set_mute", "Mute or unmute the machine's default speaker, keeping its volume: `muted` sets it, `toggle` flips it. Shows the result on screen")
                .risk("standard")
                .arg(Param::flag("muted").describe("true to mute, false to unmute. Give this or `toggle`").optional())
                .arg(Param::flag("toggle").describe("true to flip the mute. Give this or `muted`").optional()),
            move |args| {
                let (args, weak) = (args.clone(), mute_ui.clone());
                off_the_ui_thread(move || {
                    let state = levels::set_mute(&args, &Machine)?;
                    on_screen(&weak, move |ui| {
                        crate::wire::audio::publish(ui, Some(state));
                        osd::show(ui, Osd::Volume(state));
                    });
                    Ok(levels::audio_answer(state))
                })
            },
        )
        .action(
            // Refused, not pretended, on a machine with no backlight: a VM or a desktop monitor
            // has none, `describe` says so under `brightness.available`, and nothing is shown
            // on screen: a pill for hardware that is not there would be the lie the slider
            // avoids by not being drawn.
            Action::new("set_brightness", "Set the screen's brightness on a machine that has a backlight: `level` goes to 1 to 100 percent, `step` moves it by that many (the brightness keys send 5 and -5). Shows the level on screen")
                .risk("standard")
                .arg(Param::integer("level").describe("Percent, 0 to 100; the panel is never set below 1 so the screen stays readable. Give this or `step`").optional())
                .arg(Param::integer("step").describe("Percent to move by, -100 to 100, not 0. Give this or `level`").optional()),
            move |args| {
                let (args, weak) = (args.clone(), brightness_ui.clone());
                off_the_ui_thread(move || {
                    let level = levels::set_brightness(&args, &Machine)?;
                    on_screen(&weak, move |ui| {
                        crate::wire::backlight::publish(ui, Some(level));
                        osd::show(ui, Osd::Brightness(level));
                    });
                    Ok(json!({ "available": true, "level": level }))
                })
            },
        )
        .action(
            Action::new("set_mic_mute", "Mute or unmute the machine's default microphone: `muted` sets it, `toggle` flips it. Answers with whether it is muted now, and shows that on screen")
                .risk("standard")
                .arg(Param::flag("muted").describe("true to mute, false to unmute. Give this or `toggle`").optional())
                .arg(Param::flag("toggle").describe("true to flip the mute. Give this or `muted`").optional()),
            move |args| {
                let (args, weak) = (args.clone(), mic_ui.clone());
                off_the_ui_thread(move || {
                    let muted = levels::set_mic_mute(&args, &Machine)?;
                    on_screen(&weak, move |ui| osd::show(ui, Osd::Mic { muted }));
                    Ok(json!({ "mic_muted": muted }))
                })
            },
        )
        .action(
            // What the Caps_Lock key is bound to in rc.xml, on release: the compositor has
            // already toggled the lock by then, and the keyboard's LED is what this reads.
            Action::new("show_caps_lock", "Show on screen whether Caps Lock is on, as the keyboard's LED reports it, and answer with it. Changes nothing")
                .risk("safe"),
            move |_args| {
                let weak = caps_ui.clone();
                off_the_ui_thread(move || {
                    let on = yantrik_os::capslock::read()
                        .ok_or("this machine has no Caps Lock LED to read, so nothing is shown")?;
                    on_screen(&weak, move |ui| osd::show(ui, Osd::CapsLock { on }));
                    Ok(json!({ "caps_lock": on }))
                })
            },
        )
}
