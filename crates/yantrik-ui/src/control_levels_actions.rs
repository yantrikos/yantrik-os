//! The actions behind the media keys and Quick Settings' levels: `set_volume`, `set_mute`,
//! `set_brightness`, `set_mic_mute`, `mute_mic` and `show_caps_lock`. The rules (what a level or a step
//! means, what is read back) are in `control_levels`; this wires them to the machine and to the
//! on-screen display.
//!
//! The ones that move a level are `standard`: the person can put it back with the same key or
//! slider, and nothing is stored by the shell. The microphone is the exception: muting it
//! (`mute_mic`) is `standard`, but anything that can turn it ON (`set_mic_mute`, `unmute_mic`)
//! is `sensitive`, because a live microphone means the machine can hear the room, and a mind
//! asking for that is exactly what a person's grade should stop. `show_caps_lock` is `safe`: it
//! only shows what the keyboard's LED says.
//!
//! The pill is a window of its own, above app windows, so showing it is an act that puts a
//! window over the shell and goes through `card_watch::hold_windows` like the others. The level
//! is still changed while an approval card waits (a person who presses mute with a card up has
//! to be heard); only the pill is not shown then, so it is never drawn over a card.
//!
//! The machine is read and written on the socket's side of `answer_later`, not on the UI thread,
//! so a `wpctl` that hangs cannot freeze the shell. The screen is updated, and the pill shown,
//! from there through the event loop with the reading just taken.

use serde_json::{json, Value};
use slint::ComponentHandle;
use yantrik_app_runtime::control::{answer_later, Action, App as ControlSurface, Param};

use crate::control_levels::{self as levels, Machine};
use crate::wire::osd::{self, Osd};
use crate::App;

/// Run `work` off the UI thread. With no dispatch to answer later there is no worker to hand it
/// to, and running a `wpctl` right here is the blocking call the playbook forbids, so that is an
/// error and not a fallback.
fn off_the_ui_thread(work: impl FnOnce() -> Result<Value, String> + Send + 'static) -> Result<Value, String> {
    answer_later(work)
        .map(|()| json!({ "answering": "off the UI thread" }))
        .map_err(|_| "the shell has no worker free to answer this off its UI thread; ask again".to_string())
}

/// Whether the pill may be put over the shell now: `hold_windows` says no while an approval card
/// waits. Taken in the handler, where a call carrying a person's Allow is still known.
fn pill_may_cover(action: &str) -> bool {
    crate::card_watch::hold_windows(action).is_ok()
}

/// Update what the shell shows and put the pill up, on the UI thread, with a reading just taken.
fn on_screen(weak: &slint::Weak<App>, f: impl FnOnce(&App) + Send + 'static) {
    let _ = weak.upgrade_in_event_loop(move |ui| f(&ui));
}

pub fn actions(surface: ControlSurface, ui: &App) -> ControlSurface {
    let (volume_ui, mute_ui, brightness_ui) = (ui.as_weak(), ui.as_weak(), ui.as_weak());
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
                let (args, weak, may_cover) = (args.clone(), volume_ui.clone(), pill_may_cover("set_volume"));
                off_the_ui_thread(move || {
                    let state = levels::set_volume(&args, &Machine)?;
                    on_screen(&weak, move |ui| {
                        crate::wire::audio::publish(ui, Some(state));
                    });
                    osd::present(Osd::Volume(state), may_cover);
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
                let (args, weak, may_cover) = (args.clone(), mute_ui.clone(), pill_may_cover("set_mute"));
                off_the_ui_thread(move || {
                    let state = levels::set_mute(&args, &Machine)?;
                    on_screen(&weak, move |ui| {
                        crate::wire::audio::publish(ui, Some(state));
                    });
                    osd::present(Osd::Volume(state), may_cover);
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
                let (args, weak, may_cover) = (args.clone(), brightness_ui.clone(), pill_may_cover("set_brightness"));
                off_the_ui_thread(move || {
                    let level = levels::set_brightness(&args, &Machine)?;
                    on_screen(&weak, move |ui| {
                        crate::wire::backlight::publish(ui, Some(level));
                    });
                    osd::present(Osd::Brightness(level), may_cover);
                    Ok(json!({ "available": true, "level": level }))
                })
            },
        )
        .action(
            // Anything that can turn the microphone on is sensitive, a toggle included: whether
            // a toggle unmutes depends on the state, and a grade is fixed per action, so the
            // whole action carries the stricter one. `mute_mic` below is the harmless half.
            Action::new("set_mic_mute", "Mute or unmute the machine's default microphone: `muted` sets it, `toggle` flips it. Unmuting makes the microphone live, so this is sensitive; `mute_mic` only ever mutes. Answers with whether it is muted now, and shows that on screen")
                .risk("sensitive")
                .arg(Param::flag("muted").describe("true to mute, false to unmute. Give this or `toggle`").optional())
                .arg(Param::flag("toggle").describe("true to flip the mute. Give this or `muted`").optional()),
            move |args| {
                let (args, may_cover) = (args.clone(), pill_may_cover("set_mic_mute"));
                off_the_ui_thread(move || {
                    let muted = levels::set_mic_mute(&args, &Machine)?;
                    osd::present(Osd::Mic { muted }, may_cover);
                    Ok(json!({ "mic_muted": muted }))
                })
            },
        )
        .action(
            Action::new("mute_mic", "Mute the machine's default microphone. It can only mute, never turn the microphone on. Answers with whether it is muted now, and shows that on screen")
                .risk("standard"),
            move |_args| {
                let may_cover = pill_may_cover("mute_mic");
                off_the_ui_thread(move || {
                    let muted = levels::set_mic_mute(&json!({ "muted": true }), &Machine)?;
                    osd::present(Osd::Mic { muted }, may_cover);
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
                let may_cover = pill_may_cover("show_caps_lock");
                off_the_ui_thread(move || {
                    // After the LED has had a moment to follow the key (see capslock::SETTLE).
                    let on = yantrik_os::capslock::read_settled()
                        .ok_or("this machine has no Caps Lock LED to read, so nothing is shown")?;
                    osd::present(Osd::CapsLock { on }, may_cover);
                    Ok(json!({ "caps_lock": on }))
                })
            },
        )
}

#[cfg(test)]
mod tests {
    const SOURCE: &str = include_str!("control_levels_actions.rs");

    /// The text of one `Action::new("name", ...)` up to the next action.
    fn action(name: &str) -> &'static str {
        let from = SOURCE.find(&format!("Action::new(\"{name}\"")).unwrap_or_else(|| panic!("no action {name}"));
        let rest = &SOURCE[from..];
        &rest[..rest.find(".action(").unwrap_or(rest.len())]
    }

    /// A mind turning the microphone on is a privacy act (review of #579).
    #[test]
    fn anything_that_can_turn_the_microphone_on_is_sensitive_and_only_muting_is_standard() {
        assert!(action("set_mic_mute").contains(".risk(\"sensitive\")"), "set_mic_mute can unmute, even as a toggle");
        let mute = action("mute_mic");
        assert!(mute.contains(".risk(\"standard\")"));
        assert!(mute.contains("\"muted\": true"), "mute_mic can only mute");
        assert!(!mute.contains(".arg("), "mute_mic takes nothing that could say unmute");
    }

    /// The pill is a window over the shell: every action that shows it asks `hold_windows`
    /// first, and none runs the work on the calling thread.
    #[test]
    fn every_action_that_shows_the_pill_asks_hold_windows_first() {
        for name in ["set_volume", "set_mute", "set_brightness", "set_mic_mute", "mute_mic", "show_caps_lock"] {
            let a = action(name);
            assert!(a.contains("pill_may_cover(\"") && a.contains(name), "{name} asks hold_windows");
            assert!(a.find("pill_may_cover").unwrap() < a.find("off_the_ui_thread").unwrap(), "{name} asks before it hands off");
            assert!(a.contains("osd::present(") && !a.contains("osd::show("), "{name} shows the pill through present()");
        }
        assert!(SOURCE.contains("card_watch::hold_windows("));
    }

    #[test]
    fn caps_lock_reads_the_led_after_the_settle() {
        assert!(action("show_caps_lock").contains("capslock::read_settled()"));
        assert!(!action("show_caps_lock").contains("capslock::read()"));
    }

    #[test]
    fn there_is_no_inline_fallback_that_blocks_the_ui_thread() {
        assert!(!SOURCE.contains(concat!(".or_else(|work| ", "work())")));
    }
}
