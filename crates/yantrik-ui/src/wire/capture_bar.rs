//! The capture bar (Print), and the red dot's click.
//!
//! The bar is `CaptureBarWindow` (capture_bar_window.slint), a window of its own above the apps.
//! This opens it, keeps its choices, and turns its Capture or Start into a screenshot or a
//! recording. Every path that takes a picture hides the bar and the toasts first and waits for
//! the frame without them (`capture::hide_furniture`), so neither is ever in it.

use std::cell::RefCell;
use std::time::Duration;

use slint::ComponentHandle;

use crate::capture::recorder::{self, Audio, Target, Why};
use crate::capture::shot::{self, Area, Keep, Shot};
use crate::capture::{self, grab::Display, notice, Who};
use crate::{App, CaptureBarWindow, CaptureState};

/// Below this width, in logical pixels, the bar stacks into two rows. One row is about 900px
/// with Record's audio choice; the narrowest screen this OS supports is 1024px wide, less the
/// margins labwc keeps.
const COMPACT_BELOW: f32 = 980.0;

thread_local! {
    static WINDOW: RefCell<Option<CaptureBarWindow>> = const { RefCell::new(None) };
}

/// The timer's three choices, in the bar's order.
pub fn delay_for(choice: i32) -> Duration {
    match choice {
        1 => Duration::from_secs(3),
        2 => Duration::from_secs(10),
        _ => Duration::ZERO,
    }
}

fn audio_for(choice: i32) -> Audio {
    match choice {
        1 => Audio::System,
        2 => Audio::Mic,
        _ => Audio::None,
    }
}

/// The red dot's click, and the leftover check. Once, at startup.
pub fn wire(ui: &App) {
    capture::wire(ui);
    ui.global::<CaptureState>().on_stop(|| {
        std::thread::spawn(|| {
            let _ = recorder::stop(Why::Person, None);
        });
    });
}

/// Open the bar, on Screenshot or on Record. UI thread. Not while the desktop is locked: the
/// keys cannot reach here then (the state rule refuses their `yos` call), and nothing else may.
pub fn open(ui: &App, record: bool) {
    if crate::control::locked_screen(ui.get_current_screen()) {
        return;
    }
    let size = ui.window().size();
    let width = size.width as f32 / ui.window().scale_factor().max(0.1);
    WINDOW.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            match make() {
                Ok(w) => *slot = Some(w),
                Err(e) => {
                    tracing::warn!(error = %e, "no capture bar window");
                    return;
                }
            }
        }
        let w = slot.as_ref().expect("just made");
        crate::wire::theme::sync_window_theme!(ui, w);
        w.set_kind(i32::from(record));
        // The button stops the person's own recording; over a mind's it is Start, which stops
        // the mind's first (recorder::makes_way).
        w.set_recording(recorder::running_for_person());
        w.set_compact(width > 0.0 && width < COMPACT_BELOW);
        if let Err(e) = w.show() {
            tracing::warn!(error = %e, "could not show the capture bar");
            return;
        }
        w.invoke_focus_bar();
    });
}

/// Put the bar away. Any thread's work that runs on the UI thread; a no-op when it is not up.
pub fn hide() {
    WINDOW.with(|slot| {
        if let Some(w) = slot.borrow().as_ref() {
            let _ = w.hide();
        }
    });
}

fn make() -> Result<CaptureBarWindow, slint::PlatformError> {
    let w = CaptureBarWindow::new()?;
    // The shell runs under SLINT_FULLSCREEN=1 and every window of this process reads it, as the
    // agents' pop-out found (#231).
    w.window().set_fullscreen(false);
    let weak = w.as_weak();
    macro_rules! keep {
        ($on:ident, $set:ident) => {{
            let weak = weak.clone();
            w.$on(move |i| {
                if let Some(w) = weak.upgrade() {
                    w.$set(i);
                }
            });
        }};
    }
    keep!(on_chose_kind, set_kind);
    keep!(on_chose_area, set_area);
    keep!(on_chose_timer, set_timer);
    keep!(on_chose_pointer, set_pointer);
    keep!(on_chose_audio, set_audio);
    w.on_dismiss(hide);
    let go = weak.clone();
    w.on_go(move || {
        let Some(w) = go.upgrade() else { return };
        let region = w.get_area() == 0;
        let delay = delay_for(w.get_timer());
        if w.get_kind() == 0 {
            let area = if region { Area::Drag } else { Area::Screen };
            person_shot(area, delay, w.get_pointer() == 1, Keep::FileAndClipboard);
        } else if recorder::running_for_person() {
            hide();
            std::thread::spawn(|| {
                let _ = recorder::stop(Why::Person, None);
            });
        } else {
            start_from_bar(region, delay, audio_for(w.get_audio()));
        }
    });
    Ok(w)
}

/// The person's screenshot: the bar's Capture, Shift+Print, Ctrl+Print, Super+Shift+S. Returns
/// at once; the work (the person's drag, the timer, grim) is on a worker.
pub fn person_shot(area: Area, delay: Duration, pointer: bool, keep: Keep) {
    let _ = std::thread::Builder::new().name("yantrik-capture-shot".into()).spawn(move || {
        capture::hide_furniture();
        let taken = shot::take(Shot { display: Display::Desktop, area, delay, pointer, keep, who: Who::Person });
        capture::restore_furniture();
        match taken {
            Ok(shot::Taken::Saved(path)) => notice::screenshot_saved(&path),
            Ok(shot::Taken::Copied) => notice::screenshot_copied(),
            Err(e) => notice::screenshot_failed(&e),
        }
    });
}

/// The bar's Start: a recording of the person's screen, or of a rectangle they drag. The one
/// place a desktop recording is built; the control surface offers minds Mind View only.
fn start_from_bar(region: bool, delay: Duration, audio: Audio) {
    let _ = std::thread::Builder::new().name("yantrik-capture-start".into()).spawn(move || {
        capture::hide_furniture();
        let started = (|| {
            let region = if region { Some(shot::drag()?) } else { None };
            std::thread::sleep(delay);
            if capture::locked_now() {
                return Err("the screen locked before the recording started, so none did".to_string());
            }
            recorder::start(Target::Desktop { region }, audio, Who::Person)
        })();
        capture::restore_furniture();
        if let Err(e) = started {
            notice::recording_failed(&e);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timer_choices_are_off_three_and_ten_seconds() {
        assert_eq!(delay_for(0), Duration::ZERO);
        assert_eq!(delay_for(1), Duration::from_secs(3));
        assert_eq!(delay_for(2), Duration::from_secs(10));
        assert_eq!(delay_for(7), Duration::ZERO);
    }

    #[test]
    fn the_window_title_is_the_one_the_rule_and_the_window_lists_know() {
        let window = include_str!("../../../yantrik-ui-slint/ui/capture_bar_window.slint");
        assert!(window.contains(&format!("title: \"{}\"", crate::windows::CAPTURE_WINDOW_TITLE)));
        let rc = include_str!("../../../../config/labwc/rc.xml");
        assert!(
            rc.contains(&format!("<windowRule title=\"{}\"", crate::windows::CAPTURE_WINDOW_TITLE)),
            "rc.xml keeps the bar above the apps by its title"
        );
    }
}
