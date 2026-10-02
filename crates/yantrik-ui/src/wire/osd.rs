//! The on-screen display: what the volume, brightness, mic and Caps Lock keys answer with.
//!
//! The pill itself is the kit's `YOsd` (y_osd.slint). This decides what it says. It is always
//! given a reading taken from the machine after the change, never the value that was asked for,
//! so what a person sees is what the machine has. It is shown only by the control-surface
//! actions that make a change, which is what both the media keys and a mind go through: the
//! audio watcher's echo of a change does not show it, so another app moving the volume in the
//! background does not put a pill over what the person is reading.

use yantrik_os::audio::AudioState;

use crate::App;

/// What there is to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Osd {
    Volume(AudioState),
    Brightness(u8),
    Mic { muted: bool },
    CapsLock { on: bool },
}

/// What the pill draws, in the shape `App.show-osd` takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub kind: &'static str,
    pub bar: bool,
    pub level: i32,
    pub words: String,
    pub dimmed: bool,
}

pub fn view(osd: Osd) -> View {
    match osd {
        // A muted speaker keeps its level in the bar, quiet, and says "Muted" in place of a
        // percent nobody is hearing. A speaker at 0% is as silent as a muted one, so it gets
        // the crossed speaker too.
        Osd::Volume(AudioState { volume_pct, muted }) => View {
            kind: if muted || volume_pct == 0 { "volume-off" } else { "volume" },
            bar: true,
            level: i32::from(volume_pct),
            words: if muted { "Muted".into() } else { format!("{volume_pct}%") },
            dimmed: muted,
        },
        Osd::Brightness(pct) => View { kind: "brightness", bar: true, level: i32::from(pct), words: format!("{pct}%"), dimmed: false },
        Osd::Mic { muted } => View {
            kind: if muted { "mic-off" } else { "mic" },
            bar: false,
            level: 0,
            words: if muted { "Microphone muted".into() } else { "Microphone on".into() },
            dimmed: false,
        },
        Osd::CapsLock { on } => View {
            kind: "caps-lock",
            bar: false,
            level: 0,
            words: if on { "Caps Lock on".into() } else { "Caps Lock off".into() },
            dimmed: false,
        },
    }
}

/// Show it. UI thread only.
pub fn show(ui: &App, osd: Osd) {
    let v = view(osd);
    ui.invoke_show_osd(v.kind.into(), v.bar, v.level, v.words.into(), v.dimmed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_shows_the_level_the_machine_has() {
        let v = view(Osd::Volume(AudioState { volume_pct: 45, muted: false }));
        assert_eq!((v.kind, v.bar, v.level, v.words.as_str(), v.dimmed), ("volume", true, 45, "45%", false));
    }

    #[test]
    fn a_muted_speaker_says_muted_and_keeps_its_level_in_the_bar() {
        let v = view(Osd::Volume(AudioState { volume_pct: 45, muted: true }));
        assert_eq!((v.kind, v.level, v.words.as_str(), v.dimmed), ("volume-off", 45, "Muted", true));
    }

    #[test]
    fn zero_percent_is_drawn_as_silence_not_as_a_sound() {
        let v = view(Osd::Volume(AudioState { volume_pct: 0, muted: false }));
        assert_eq!((v.kind, v.words.as_str(), v.dimmed), ("volume-off", "0%", false));
    }

    #[test]
    fn brightness_is_a_bar_and_a_number() {
        let v = view(Osd::Brightness(70));
        assert_eq!((v.kind, v.bar, v.level, v.words.as_str()), ("brightness", true, 70, "70%"));
    }

    #[test]
    fn mic_and_caps_lock_are_words_with_no_bar() {
        let v = view(Osd::Mic { muted: true });
        assert_eq!((v.kind, v.bar, v.words.as_str()), ("mic-off", false, "Microphone muted"));
        assert_eq!(view(Osd::Mic { muted: false }).words, "Microphone on");
        assert_eq!(view(Osd::CapsLock { on: true }).words, "Caps Lock on");
        let off = view(Osd::CapsLock { on: false });
        assert_eq!((off.kind, off.bar, off.words.as_str()), ("caps-lock", false, "Caps Lock off"));
    }
}
