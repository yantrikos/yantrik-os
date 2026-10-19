//! Capture: screenshots and screen recording, for the person and, within limits, for minds
//! (design/capture-2026-10-15.md).
//!
//! - [`grab`] takes the pixels (grim), for this and for `control_screen`'s text reading alike.
//! - [`shot`] is a screenshot: the clean frame, the file, the clipboard.
//! - [`recorder`] is a recording: wf-recorder in a transient user unit, stopped cleanly.
//! - [`names`], [`encoder`] and [`disk`] are the small rules: what a file is called, which
//!   encoder, how much room.
//! - [`notice`] is what the person is told, and the buttons on it.
//!
//! The bar is `wire::capture_bar`; the control surface is `control_capture`.
//!
//! Nothing is captured while the screen is locked, by anyone. The shell's state rule refuses
//! every act on the socket while locked, which covers the keys (they go through `yos act shell`)
//! and every mind; this module holds it for what is already in flight: a timer that runs out
//! after the lock, and a recording, which the lock stops.

pub mod disk;
pub mod encoder;
pub mod grab;
pub mod names;
pub mod notice;
pub mod recorder;
pub mod shot;

use std::sync::Mutex;
use std::time::Duration;

use slint::ComponentHandle;

use crate::{App, CaptureState};

/// Who asked for a capture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Who {
    Person,
    /// A mind. `name` is what the person is told and `describe shell` says. `id` is which mind it
    /// is (`mind_view::mind_id_now`): the agent's id for a believed token, else the attached
    /// mind. A name is not an identity: every agent with a token was "an agent", so any of them
    /// could stop another's recording (security review of #828, finding 4). `None` when the shell
    /// could not tell, which owns nothing.
    Mind { name: String, id: Option<String> },
}

impl Who {
    /// Whether `asker` is the one who started this: the person for the person's, the same mind
    /// by its id for a mind's. A mind the shell could not tell owns nothing, not even its own.
    pub fn started_by(&self, asker: &Who) -> bool {
        match (self, asker) {
            (Who::Person, Who::Person) => true,
            (Who::Mind { id: Some(mine), .. }, Who::Mind { id: Some(theirs), .. }) => mine == theirs,
            _ => false,
        }
    }
}

/// The shell's window, for the workers that need to ask it something or change what it shows.
static UI: Mutex<Option<slint::Weak<App>>> = Mutex::new(None);

fn ui() -> Option<slint::Weak<App>> {
    UI.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Called once, when the shell wires itself: keeps the window for the workers, and stops any
/// recording a shell before this one left running.
pub fn wire(ui: &App) {
    *UI.lock().unwrap_or_else(|p| p.into_inner()) = Some(ui.as_weak());
    let _ = std::thread::Builder::new().name("yantrik-capture-leftovers".into()).spawn(recorder::stop_leftovers);
}

/// Whether the desktop is locked now, asked of the UI thread. From a worker only — the UI
/// thread would wait for itself. No answer counts as locked: a capture is the thing to refuse
/// when the shell cannot say.
pub fn locked_now() -> bool {
    let Some(weak) = ui() else { return true };
    let (tx, rx) = std::sync::mpsc::channel();
    let asked = weak.upgrade_in_event_loop(move |ui| {
        let _ = tx.send(crate::control::locked_screen(ui.get_current_screen()));
    });
    if asked.is_err() {
        return true;
    }
    rx.recv_timeout(Duration::from_secs(2)).unwrap_or(true)
}

/// The red dot: on with its elapsed time, or off. From any thread.
pub(crate) fn show_recording(on: bool, elapsed: &str) {
    let Some(weak) = ui() else { return };
    let elapsed = elapsed.to_string();
    let _ = weak.upgrade_in_event_loop(move |ui| {
        let state = ui.global::<CaptureState>();
        state.set_recording(on);
        state.set_elapsed(elapsed.into());
    });
}

/// Take the bar and the toasts off the screen and wait for the frame without them. From a
/// worker; blocks for the UI thread's answer and [`shot::SETTLE`].
pub(crate) fn hide_furniture() {
    let Some(weak) = ui() else { return };
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = weak.upgrade_in_event_loop(move |ui| {
        ui.global::<CaptureState>().set_hiding(true);
        crate::wire::capture_bar::hide();
        let _ = tx.send(());
    });
    let _ = rx.recv_timeout(Duration::from_secs(2));
    std::thread::sleep(shot::SETTLE);
}

/// Put the toasts back after the picture.
pub(crate) fn restore_furniture() {
    let Some(weak) = ui() else { return };
    let _ = weak.upgrade_in_event_loop(|ui| ui.global::<CaptureState>().set_hiding(false));
}

/// The lock callback, before the screen goes dark: a recording stops, cleanly.
pub fn on_screen_lock() {
    crate::wire::capture_bar::hide();
    recorder::stop_soon(recorder::Why::Locked);
}

/// Logging out, restarting or shutting down: the recording is finished before the session that
/// would kill it ends. Blocking for as long as the file takes; `control_power::run` is a worker.
pub fn before_session_ends() {
    let _ = recorder::stop(recorder::Why::LoggingOut, None);
}

/// `describe shell`'s `capture`.
pub fn for_describe() -> serde_json::Value {
    recorder::for_describe()
}
