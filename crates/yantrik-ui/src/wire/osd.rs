//! The on-screen display: what the volume, brightness, mic and Caps Lock keys answer with.
//!
//! The pill itself is the kit's `YOsd` (y_osd.slint), in a window of its own (osd_window.slint)
//! that labwc keeps above app windows (rc.xml). This decides what it says. It is always
//! given a reading taken from the machine after the change, never the value that was asked for,
//! so what a person sees is what the machine has. It is shown only by the control-surface
//! actions that make a change, which is what both the media keys and a mind go through: the
//! audio watcher's echo of a change does not show it, so another app moving the volume in the
//! background does not put a pill over what the person is reading.

use std::cell::RefCell;
use std::time::Duration;

use slint::{ComponentHandle, Timer, TimerMode};
use yantrik_os::audio::AudioState;

use crate::OsdWindow;

/// What there is to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Osd {
    Volume(AudioState),
    Brightness(u8),
    Mic { muted: bool },
    CapsLock { on: bool },
}

/// What the pill draws, in the shape `OsdWindow.show-osd` takes.
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

/// How long the window stays up: the pill's own hold (1200 ms, y_osd.slint) plus its fade out.
const WINDOW_HOLD: Duration = Duration::from_millis(1400);

/// How long the new window gets to map before the keyboard is handed back.
const REFOCUS_AFTER: Duration = Duration::from_millis(150);

thread_local! {
    static WINDOW: RefCell<Option<OsdWindow>> = const { RefCell::new(None) };
    /// The one single-shot timer that takes the window away; every key press restarts it.
    static HIDE: Timer = Timer::default();
}

/// Show it, from any thread but the UI thread's own work: the part that spawns a process
/// (who has the keyboard) is done here, then the window is put up on the UI thread.
///
/// The pill lives in a window of its own so that it is above app windows (it was behind them
/// when it was drawn in the shell's window, review of #579). `may_cover` is the answer of
/// `card_watch::hold_windows` taken by the action: while an approval card waits, nothing is
/// put over the shell, so the change is made and the pill is not shown. A new window takes
/// keyboard focus in labwc, so whoever had it gets it back: the pill must never take a key.
pub fn present(osd: Osd, may_cover: bool) {
    if !may_cover {
        return;
    }
    let restore = crate::windows::focus_to_restore();
    let _ = slint::invoke_from_event_loop(move || show_on_ui_thread(osd, restore));
}

fn show_on_ui_thread(osd: Osd, restore: Option<String>) {
    let v = view(osd);
    let newly_up = WINDOW.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            match OsdWindow::new() {
                Ok(w) => {
                    // The shell runs under SLINT_FULLSCREEN=1 and every window of this process
                    // reads it, as the agents' pop-out found (#231).
                    w.window().set_fullscreen(false);
                    *slot = Some(w);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "no on-screen display window");
                    return false;
                }
            }
        }
        let w = slot.as_ref().expect("just made");
        w.invoke_show_osd(v.kind.into(), v.bar, v.level, v.words.into(), v.dimmed);
        let was_up = w.window().is_visible();
        match w.show() {
            Ok(()) => !was_up,
            Err(e) => {
                tracing::warn!(error = %e, "could not show the on-screen display");
                false
            }
        }
    });
    HIDE.with(|t| {
        t.start(TimerMode::SingleShot, WINDOW_HOLD, || {
            WINDOW.with(|slot| {
                if let Some(w) = slot.borrow().as_ref() {
                    let _ = w.hide();
                }
            });
        });
    });
    if let (true, Some(title)) = (newly_up, restore) {
        std::thread::spawn(move || {
            std::thread::sleep(REFOCUS_AFTER);
            crate::windows::present(&title);
        });
    }
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

    /// Drawn in the shell's own window the pill was behind every app (review of #579). It must
    /// be a window of its own that the compositor keeps on top, and never one that takes keys.
    #[test]
    fn the_pill_is_its_own_window_kept_above_app_windows_and_never_in_the_shell() {
        let rc = include_str!("../../../../config/labwc/rc.xml");
        let rule = rc.split("<windowRule title=\"Yantrik OSD\"").nth(1).expect("rc.xml has a windowRule for the OSD window");
        let rule = rule.split("</windowRule>").next().unwrap();
        assert!(rule.contains("ToggleAlwaysOnTop"), "labwc keeps it above app windows: {rule}");
        assert!(rule.contains("skipTaskbar=\"yes\"") && rule.contains("skipWindowSwitcher=\"yes\""), "it is no window to switch to: {rule}");
        let window = include_str!("../../../yantrik-ui-slint/ui/osd_window.slint");
        assert!(window.contains(&format!("title: \"{}\"", crate::windows::OSD_WINDOW_TITLE)), "title matches the rule and the list filter");
        assert!(window.contains("no-frame: true"));
        assert!(!window.contains("TouchArea") && !window.contains("FocusScope"), "nothing in it catches a key or a pointer");
        let app = include_str!("../../../yantrik-ui-slint/ui/app.slint");
        assert!(!app.contains("YOsd"), "the shell's window no longer draws a pill nobody can see");
    }

    #[test]
    fn showing_goes_through_one_present_that_gives_the_keyboard_back() {
        let src = include_str!("osd.rs");
        assert!(src.contains("pub fn present(osd: Osd, may_cover: bool)"));
        assert!(src.contains("if !may_cover"), "held while an approval card waits");
        assert!(src.contains("focus_to_restore"), "whoever had the keyboard gets it back");
    }
}
