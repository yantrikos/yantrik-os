//! The volume, the microphone and the backlight on the shell's control surface: what `describe
//! shell` says about them, and the actions that move them, which are also what the media keys
//! run (`yos act shell ...` in config/labwc/rc.xml).
//!
//! Whatever the slider in Quick Settings can do, a key or a mind can ask for, and the answer is
//! the machine's own reading taken after the change, not the value that was asked for: a volume
//! the audio server refused, or a backlight the machine does not have, must not come back as
//! done. The same reading is what the on-screen display shows, so the number a person sees is
//! the number the machine has. The machine is reached through the small traits below so the
//! rules here are tested without one.
//!
//! One way of doing it: `set_volume` and `set_brightness` take a `level` or a `step`,
//! `set_mute` and `set_mic_mute` take `muted` or `toggle`. There are no separate "step" actions.

use serde_json::{json, Value};
use yantrik_os::audio::AudioState;

/// How far one press of a volume or brightness key moves the level, in percent.
pub const KEY_STEP: i64 = 5;

/// The speaker, as the actions see it.
pub trait Mixer {
    fn read(&self) -> Option<AudioState>;
    fn set_volume(&self, pct: u8) -> Result<(), String>;
    fn set_mute(&self, muted: bool) -> Result<(), String>;
    /// Move the volume by `step` percent in one relative call to the audio server, so two presses
    /// that overlap both count.
    fn step_volume(&self, step: i8) -> Result<(), String>;
}

/// The microphone: only a mute, because the shell has no input level to show.
pub trait Mic {
    fn muted(&self) -> Option<bool>;
    fn set_muted(&self, muted: bool) -> Result<(), String>;
}

/// The screen's backlight, if the machine has one.
pub trait Backlight {
    fn available(&self) -> bool;
    fn read(&self) -> Option<u8>;
    fn set(&self, pct: u8) -> Result<(), String>;
    /// Move the level by `step` percent in one relative call where the machine has one. `false`
    /// means it has none, and the caller reads and sets.
    fn step(&self, step: i8) -> Result<bool, String>;
}

/// The real machine: PipeWire through `wpctl`, and sysfs / `brightnessctl` / logind.
pub struct Machine;

impl Mixer for Machine {
    fn read(&self) -> Option<AudioState> {
        yantrik_os::audio::read()
    }
    fn set_volume(&self, pct: u8) -> Result<(), String> {
        yantrik_os::audio::set_volume(pct)
    }
    fn set_mute(&self, muted: bool) -> Result<(), String> {
        yantrik_os::audio::set_mute(muted)
    }
    fn step_volume(&self, step: i8) -> Result<(), String> {
        yantrik_os::audio::step_volume(step)
    }
}

impl Mic for Machine {
    fn muted(&self) -> Option<bool> {
        yantrik_os::audio::read_mic_muted()
    }
    fn set_muted(&self, muted: bool) -> Result<(), String> {
        yantrik_os::audio::set_mic_mute(muted)
    }
}

impl Backlight for Machine {
    fn available(&self) -> bool {
        yantrik_os::backlight::available()
    }
    fn read(&self) -> Option<u8> {
        yantrik_os::backlight::read()
    }
    fn set(&self, pct: u8) -> Result<(), String> {
        yantrik_os::backlight::set(pct)
    }
    fn step(&self, step: i8) -> Result<bool, String> {
        yantrik_os::backlight::step(step)
    }
}

/// What a level action was asked to do: go to a level, or move by a step from where it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Level(u8),
    Step(i8),
}

/// `level` (a whole percent, 0 to 100) or `step` (a whole number from -100 to 100, not 0), one
/// of them. Neither is clamped: a caller that asked for 150 should be told that is not a level,
/// not handed 100 and left to think it got what it asked.
pub fn parse_change(args: &Value) -> Result<Change, String> {
    match (args.get("level").filter(|v| !v.is_null()), args.get("step").filter(|v| !v.is_null())) {
        (Some(_), Some(_)) => Err("give `level` or `step`, not both".into()),
        (None, None) => Err("give `level` (0 to 100) to go to a level, or `step` (-100 to 100) to move by an amount".into()),
        (Some(_), None) => parse_level(args).map(Change::Level),
        (None, Some(step)) => {
            let step = step.as_i64().ok_or("`step` must be a whole number from -100 to 100")?;
            i8::try_from(step)
                .ok()
                .filter(|s| *s != 0 && (-100..=100).contains(s))
                .map(Change::Step)
                .ok_or_else(|| format!("`step` is {step}; it must be from -100 to 100 and not 0"))
        }
    }
}

/// The level a change lands on, given where the level is now. Steps stop at the ends.
pub fn target(change: Change, current: u8) -> u8 {
    match change {
        Change::Level(l) => l,
        Change::Step(s) => (i16::from(current) + i16::from(s)).clamp(0, 100) as u8,
    }
}

/// `muted: true|false` or `toggle: true`, one of them, resolved against the current state.
pub fn parse_mute(args: &Value, now_muted: impl FnOnce() -> Result<bool, String>) -> Result<bool, String> {
    let muted = args.get("muted").filter(|v| !v.is_null());
    let toggle = args.get("toggle").filter(|v| !v.is_null());
    match (muted, toggle) {
        (Some(_), Some(_)) => Err("give `muted` or `toggle`, not both".into()),
        (Some(m), None) => m.as_bool().ok_or_else(|| "`muted` must be true or false".into()),
        (None, Some(t)) => match t.as_bool() {
            Some(true) => now_muted().map(|m| !m),
            _ => Err("`toggle` must be true (leave it out to use `muted`)".into()),
        },
        (None, None) => Err("`muted` must be true or false, or `toggle` true to flip it".into()),
    }
}

/// `level` as a percent: a whole number from 0 to 100. Not clamped: a caller that asked for 150
/// should be told that is not a level, not handed 100 and left to think it got what it asked.
pub fn parse_level(args: &Value) -> Result<u8, String> {
    let level = args
        .get("level")
        .and_then(Value::as_i64)
        .ok_or("`level` must be a whole number from 0 to 100")?;
    u8::try_from(level)
        .ok()
        .filter(|l| *l <= 100)
        .ok_or_else(|| format!("`level` is {level}; it must be from 0 to 100"))
}

/// What `describe shell` says about sound: the machine's volume, or `null` when it has no audio
/// server to ask, so a reader never mistakes a missing mixer for a muted one.
pub fn audio_for_describe(available: bool, volume: i32, muted: bool) -> Value {
    if available {
        json!({ "volume": volume, "muted": muted })
    } else {
        Value::Null
    }
}

/// What `describe shell` says about the backlight. `level` is `null` when there is none.
pub fn brightness_for_describe(available: bool, level: i32) -> Value {
    json!({ "available": available, "level": if available { json!(level) } else { Value::Null } })
}

pub fn audio_answer(state: AudioState) -> Value {
    json!({ "volume": state.volume_pct, "muted": state.muted })
}

/// `set_volume`: go to a level or step from the current one, then answer with what the machine
/// now reads. A step up from a muted speaker unmutes it, as a volume key does on any desktop:
/// pressing "louder" and hearing nothing is not an answer. A step down, or a level, leaves the
/// mute as it was.
pub fn set_volume(args: &Value, mixer: &impl Mixer) -> Result<AudioState, String> {
    match parse_change(args)? {
        Change::Level(level) => mixer.set_volume(level)?,
        // One relative call, not a read and a write: see `Mixer::step_volume`. Unmuting on the
        // way up is idempotent, so it needs no read of the mute first either.
        Change::Step(step) => {
            mixer.step_volume(step)?;
            if step > 0 {
                mixer.set_mute(false)?;
            }
        }
    }
    mixer.read().ok_or_else(|| "the volume was set, but the audio server did not answer when asked for it back".into())
}

/// `set_mute`: mute, unmute or toggle, then answer with what the machine now reads.
pub fn set_mute(args: &Value, mixer: &impl Mixer) -> Result<AudioState, String> {
    let muted = parse_mute(args, || {
        mixer.read().map(|s| s.muted).ok_or_else(|| "the audio server did not answer, so there is no mute to toggle".to_string())
    })?;
    mixer.set_mute(muted)?;
    mixer.read().ok_or_else(|| "the mute was set, but the audio server did not answer when asked for it back".into())
}

/// `set_mic_mute`: the same for the default microphone. Answers with whether it is muted now.
pub fn set_mic_mute(args: &Value, mic: &impl Mic) -> Result<bool, String> {
    let muted = parse_mute(args, || {
        mic.muted().ok_or_else(|| "there is no microphone to ask, so there is no mute to toggle".to_string())
    })?;
    mic.set_muted(muted)?;
    mic.muted().ok_or_else(|| "the mute was set, but the microphone could not be read back".into())
}

/// `set_brightness`: refuses plainly when the machine has no backlight (a VM, a desktop
/// monitor), otherwise moves it and answers with the level the panel now reads.
pub fn set_brightness(args: &Value, panel: &impl Backlight) -> Result<u8, String> {
    if !panel.available() {
        return Err("this machine has no backlight, so there is no brightness to set (`describe shell` shows brightness.available: false)".into());
    }
    match parse_change(args)? {
        Change::Level(level) => panel.set(level)?,
        Change::Step(step) => {
            // One relative call where the machine has one. Without (logind only), a read and a
            // write is all there is, and a lost step on a held key is the cost.
            if !panel.step(step)? {
                let current = panel.read().ok_or("the backlight could not be read, so there is no level to step from")?;
                panel.set(target(Change::Step(step), current))?;
            }
        }
    }
    panel.read().ok_or_else(|| "the brightness was set, but the backlight could not be read back".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    /// A speaker that remembers what it was told, and can be made to cap or fail.
    struct FakeMixer {
        state: Cell<Option<AudioState>>,
        cap: Option<u8>,
        fail: bool,
        calls: RefCell<Vec<String>>,
    }
    impl FakeMixer {
        fn at(volume_pct: u8, muted: bool) -> Self {
            Self { state: Cell::new(Some(AudioState { volume_pct, muted })), cap: None, fail: false, calls: RefCell::default() }
        }
        fn gone() -> Self {
            Self { state: Cell::new(None), cap: None, fail: false, calls: RefCell::default() }
        }
    }
    impl Mixer for FakeMixer {
        fn read(&self) -> Option<AudioState> {
            self.state.get()
        }
        fn set_volume(&self, pct: u8) -> Result<(), String> {
            if self.fail {
                return Err("wpctl could not be run".into());
            }
            self.calls.borrow_mut().push(format!("volume {pct}"));
            let muted = self.state.get().is_some_and(|s| s.muted);
            self.state.set(Some(AudioState { volume_pct: self.cap.map_or(pct, |c| pct.min(c)), muted }));
            Ok(())
        }
        fn step_volume(&self, step: i8) -> Result<(), String> {
            if self.fail {
                return Err("wpctl could not be run".into());
            }
            let s = self.state.get().ok_or("the audio server did not answer")?;
            self.calls.borrow_mut().push(format!("step {step}"));
            let volume_pct = (i16::from(s.volume_pct) + i16::from(step)).clamp(0, 100) as u8;
            self.state.set(Some(AudioState { volume_pct, muted: s.muted }));
            Ok(())
        }
        fn set_mute(&self, muted: bool) -> Result<(), String> {
            self.calls.borrow_mut().push(format!("mute {muted}"));
            let volume_pct = self.state.get().map_or(0, |s| s.volume_pct);
            self.state.set(Some(AudioState { volume_pct, muted }));
            Ok(())
        }
    }

    struct FakeMic(Cell<Option<bool>>);
    impl Mic for FakeMic {
        fn muted(&self) -> Option<bool> {
            self.0.get()
        }
        fn set_muted(&self, muted: bool) -> Result<(), String> {
            self.0.set(Some(muted));
            Ok(())
        }
    }

    struct FakePanel {
        level: Cell<Option<u8>>,
        has: bool,
        touched: Cell<bool>,
        relative: bool,
        steps: RefCell<Vec<i8>>,
    }
    impl FakePanel {
        fn at(level: u8) -> Self {
            Self { level: Cell::new(Some(level)), has: true, touched: Cell::new(false), relative: true, steps: RefCell::default() }
        }
        fn none() -> Self {
            Self { level: Cell::new(None), has: false, touched: Cell::new(false), relative: true, steps: RefCell::default() }
        }
    }
    impl Backlight for FakePanel {
        fn available(&self) -> bool {
            self.has
        }
        fn read(&self) -> Option<u8> {
            self.level.get()
        }
        fn set(&self, pct: u8) -> Result<(), String> {
            self.touched.set(true);
            self.level.set(Some(pct.max(1)));
            Ok(())
        }
        fn step(&self, step: i8) -> Result<bool, String> {
            if !self.relative {
                return Ok(false);
            }
            self.touched.set(true);
            self.steps.borrow_mut().push(step);
            let now = i16::from(self.level.get().unwrap_or(0)) + i16::from(step);
            self.level.set(Some(now.clamp(1, 100) as u8));
            Ok(true)
        }
    }

    #[test]
    fn a_level_is_a_whole_percent_and_never_quietly_clamped() {
        assert_eq!(parse_level(&json!({ "level": 0 })), Ok(0));
        assert_eq!(parse_level(&json!({ "level": 100 })), Ok(100));
        for bad in [json!({ "level": 101 }), json!({ "level": -1 }), json!({ "level": 45.5 }), json!({ "level": "loud" }), json!({})] {
            assert!(parse_level(&bad).is_err(), "{bad}");
        }
        assert!(parse_level(&json!({ "level": 150 })).unwrap_err().contains("0 to 100"));
    }

    #[test]
    fn a_change_is_a_level_or_a_step_and_never_both_or_neither() {
        assert_eq!(parse_change(&json!({ "level": 30 })), Ok(Change::Level(30)));
        assert_eq!(parse_change(&json!({ "step": 5 })), Ok(Change::Step(5)));
        assert_eq!(parse_change(&json!({ "step": -5 })), Ok(Change::Step(-5)));
        assert_eq!(parse_change(&json!({ "step": 5, "level": null })), Ok(Change::Step(5)), "null is left out");
        for bad in [json!({}), json!({ "level": 30, "step": 5 }), json!({ "step": 0 }), json!({ "step": 101 }), json!({ "step": -101 }), json!({ "step": 2.5 }), json!({ "step": "up" })] {
            assert!(parse_change(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_step_stops_at_the_ends_of_the_range() {
        assert_eq!(target(Change::Step(5), 97), 100);
        assert_eq!(target(Change::Step(-5), 3), 0);
        assert_eq!(target(Change::Step(5), 40), 45);
        assert_eq!(target(Change::Level(80), 10), 80);
    }

    #[test]
    fn set_volume_answers_with_what_the_machine_reads_not_what_was_asked() {
        // The audio server capped it: the answer says 80, and does not echo the 95 asked for.
        let mixer = FakeMixer { cap: Some(80), ..FakeMixer::at(10, false) };
        let state = set_volume(&json!({ "level": 95 }), &mixer).unwrap();
        assert_eq!(audio_answer(state), json!({ "volume": 80, "muted": false }));
    }

    #[test]
    fn a_volume_key_steps_from_where_the_machine_is() {
        let mixer = FakeMixer::at(42, false);
        assert_eq!(set_volume(&json!({ "step": 5 }), &mixer).unwrap().volume_pct, 47);
        assert_eq!(set_volume(&json!({ "step": -5 }), &mixer).unwrap().volume_pct, 42);
        // A step is a relative call, never a write of a target computed from an earlier read:
        // two presses that overlapped used to read the same level and lose one (review #579).
        assert_eq!(*mixer.calls.borrow(), ["step 5", "mute false", "step -5"]);
    }

    #[test]
    fn a_step_never_reads_the_level_it_would_have_stepped_from() {
        // The server holds a level the caller has no reading of: an atomic step still lands.
        let mixer = FakeMixer::at(42, false);
        mixer.state.set(Some(AudioState { volume_pct: 42, muted: false }));
        set_volume(&json!({ "step": -5 }), &mixer).unwrap();
        assert!(mixer.calls.borrow().iter().all(|c| !c.starts_with("volume ")), "no absolute write for a step");
    }

    #[test]
    fn a_brightness_step_is_relative_where_the_machine_can_and_read_then_set_where_it_cannot() {
        let panel = FakePanel::at(50);
        assert_eq!(set_brightness(&json!({ "step": 5 }), &panel), Ok(55));
        assert_eq!(*panel.steps.borrow(), [5]);
        let panel = FakePanel { relative: false, ..FakePanel::at(50) };
        assert_eq!(set_brightness(&json!({ "step": -5 }), &panel), Ok(45));
        assert!(panel.steps.borrow().is_empty());
    }

    #[test]
    fn louder_on_a_muted_speaker_unmutes_but_quieter_and_a_level_do_not() {
        let mixer = FakeMixer::at(30, true);
        let state = set_volume(&json!({ "step": 5 }), &mixer).unwrap();
        assert_eq!((state.volume_pct, state.muted), (35, false), "the key made a sound possible");
        let mixer = FakeMixer::at(30, true);
        assert!(set_volume(&json!({ "step": -5 }), &mixer).unwrap().muted);
        let mixer = FakeMixer::at(30, true);
        assert!(set_volume(&json!({ "level": 50 }), &mixer).unwrap().muted, "a caller that set a level did not ask to unmute");
    }

    #[test]
    fn set_volume_reports_the_machines_refusal_and_never_hands_it_a_bad_level() {
        let mixer = FakeMixer { fail: true, ..FakeMixer::at(30, false) };
        assert!(set_volume(&json!({ "level": 30 }), &mixer).unwrap_err().contains("wpctl"));
        let mixer = FakeMixer::at(30, false);
        assert!(set_volume(&json!({ "level": 200 }), &mixer).is_err());
        assert!(mixer.calls.borrow().is_empty(), "a bad level never reached the machine");
    }

    #[test]
    fn a_volume_that_cannot_be_read_is_not_reported_as_done_or_stepped_from_nothing() {
        let mixer = FakeMixer::gone();
        let err = set_volume(&json!({ "step": 5 }), &mixer).unwrap_err();
        assert!(err.contains("did not answer"), "{err}");
        assert!(mixer.calls.borrow().is_empty(), "no volume was invented to step from");
    }

    #[test]
    fn set_mute_mutes_unmutes_and_toggles_and_reads_the_result_back() {
        let mixer = FakeMixer::at(45, false);
        assert_eq!(audio_answer(set_mute(&json!({ "muted": true }), &mixer).unwrap()), json!({ "volume": 45, "muted": true }));
        assert!(!set_mute(&json!({ "toggle": true }), &mixer).unwrap().muted, "toggle flips what the machine says now");
        assert!(set_mute(&json!({ "toggle": true }), &mixer).unwrap().muted);
        for bad in [json!({ "muted": "yes" }), json!({}), json!({ "muted": true, "toggle": true }), json!({ "toggle": false })] {
            assert!(set_mute(&bad, &mixer).is_err(), "{bad}");
        }
        assert!(set_mute(&json!({ "toggle": true }), &FakeMixer::gone()).is_err(), "nothing to toggle");
    }

    #[test]
    fn the_microphone_toggles_from_its_own_state() {
        let mic = FakeMic(Cell::new(Some(false)));
        assert_eq!(set_mic_mute(&json!({ "toggle": true }), &mic), Ok(true));
        assert_eq!(set_mic_mute(&json!({ "toggle": true }), &mic), Ok(false));
        assert_eq!(set_mic_mute(&json!({ "muted": true }), &mic), Ok(true));
        assert!(set_mic_mute(&json!({ "toggle": true }), &FakeMic(Cell::new(None))).is_err(), "no microphone is not an unmuted one");
        assert!(set_mic_mute(&json!({}), &mic).is_err());
    }

    #[test]
    fn set_brightness_refuses_clearly_without_a_backlight() {
        let panel = FakePanel::none();
        let err = set_brightness(&json!({ "step": 5 }), &panel).unwrap_err();
        assert!(err.contains("no backlight"), "{err}");
        assert!(!panel.touched.get(), "nothing was run against a machine with no backlight");
    }

    #[test]
    fn set_brightness_answers_with_the_panels_own_reading() {
        let panel = FakePanel::at(50);
        assert_eq!(set_brightness(&json!({ "level": 1 }), &panel), Ok(1));
        assert_eq!(set_brightness(&json!({ "step": 5 }), &panel), Ok(6));
        assert_eq!(set_brightness(&json!({ "step": -5 }), &panel), Ok(1), "the panel's own floor, read back");
        assert!(set_brightness(&json!({ "level": 101 }), &FakePanel::at(50)).is_err());
    }

    #[test]
    fn describe_shows_no_number_for_hardware_that_is_not_there() {
        assert_eq!(audio_for_describe(false, 0, false), Value::Null, "no mixer is not a muted mixer");
        assert_eq!(audio_for_describe(true, 45, true), json!({ "volume": 45, "muted": true }));
        assert_eq!(brightness_for_describe(false, 0), json!({ "available": false, "level": null }));
        assert_eq!(brightness_for_describe(true, 70), json!({ "available": true, "level": 70 }));
    }
}
