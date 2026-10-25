//! The app's windows (design/notes-gate-2026-10-21.md §2, "Views and windows"). Part of
//! [`Core`]; here so the core's own file stays readable.
//!
//! The runner makes the first window; `fx.open_window()` makes more, each themed, given its
//! chrome, wired by `App::wire_window`, rendered whole and shown. Every render reaches every
//! open window, which draws itself from the one model. No window is privileged: closing any of
//! them, the first included, closes only it, and the app ends when none is left, judged once a
//! command's effects have all been applied (a close and an open in one command replace a window).
//!
//! A second launch's `present` goes through the core ([`Core::present`]): it brings forward a
//! window that is open, and never re-shows a closed one behind the core's back, so what the core
//! counts as open is what is on screen. The one window it opens is the first, during a linger
//! (`fx.linger`, `cx/linger.rs`), when none is open and the process runs on. Either way the app
//! hears it as `Event::Presented`, so it knows the person is back without waiting for the window
//! manager to give the window the keyboard.
//!
//! Activation: every window, the first included, tells the core when the window manager gives it
//! the keyboard or takes it away ([`Core::hear_activation`]). The core keeps which window is
//! active and raises `Event::Activated` when that changes to an open window, and only then.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use slint::ComponentHandle as _;

use super::{Core, Cx, Inbound};
use crate::app::{App, Changed, Event, ExitReason, WindowId};
use crate::problems;
use crate::theme::{self, ThemeSettings};
use crate::window::Window as _;

/// Window ids, unique in the process. The first window is `WindowId::PRIMARY` (0); an id handed
/// out by a command that is then refused is simply never used.
static NEXT_WINDOW: AtomicU32 = AtomicU32::new(1);

pub(crate) fn next_window_id() -> WindowId {
    WindowId(NEXT_WINDOW.fetch_add(1, Ordering::Relaxed))
}

/// The windows beyond the first, and what every new one is made with.
pub(crate) struct Windows<U> {
    /// The first window is open. It is kept, hidden, once closed: the event loop holds it, and a
    /// second launch during a linger opens it again.
    first_open: bool,
    more: BTreeMap<WindowId, U>,
    /// The theme the windows wear now, so a window opened later wears it too.
    theme: Option<ThemeSettings>,
    /// The window the window manager last gave the keyboard to, while it has it.
    active: Option<WindowId>,
    /// Frames drawn by every window since the app started (`cx/drawn.rs`).
    pub(super) frames: u64,
}

impl<U> Windows<U> {
    pub(crate) fn new() -> Windows<U> {
        Windows { first_open: true, more: BTreeMap::new(), theme: None, active: None, frames: 0 }
    }

    /// `window` is not the active one any more: it lost the keyboard, or it closed.
    fn not_active(&mut self, window: WindowId) {
        if self.active == Some(window) {
            self.active = None;
        }
    }
}

impl<A: App> Core<A> {
    /// The windows open now, the first one first.
    pub fn windows_open(&self) -> Vec<WindowId> {
        let windows = self.windows.borrow();
        let first = windows.first_open.then_some(WindowId::PRIMARY);
        first.into_iter().chain(windows.more.keys().copied()).collect()
    }

    /// An open window, by id: a strong handle to it.
    pub fn window(&self, id: WindowId) -> Option<A::Ui> {
        let windows = self.windows.borrow();
        if id == WindowId::PRIMARY {
            return windows.first_open.then(|| self.ui.clone_strong());
        }
        windows.more.get(&id).map(|ui| ui.clone_strong())
    }

    /// The theme the runner started the first window with, for the windows opened after it.
    pub fn set_theme(&self, theme: ThemeSettings) {
        self.windows.borrow_mut().theme = Some(theme);
    }

    /// Every open window draws what changed.
    pub(super) fn render_windows(&self, changed: &Changed) {
        // A snapshot: a window drawing may fire a callback, and nothing may hold the table then.
        let (first_open, more): (bool, Vec<(WindowId, A::Ui)>) = {
            let windows = self.windows.borrow();
            let more = windows.more.iter().map(|(id, ui)| (*id, ui.clone_strong())).collect();
            (windows.first_open, more)
        };
        let model = self.model.borrow();
        if first_open {
            A::render(&self.ui, WindowId::PRIMARY, &model, changed);
        }
        for (id, ui) in more {
            A::render(&ui, id, &model, changed);
        }
    }

    /// The new theme, on every window.
    pub(super) fn apply_theme(&self, theme: &ThemeSettings) {
        self.windows.borrow_mut().theme = Some(*theme);
        self.ui.apply_theme(theme);
        let more: Vec<A::Ui> = self.windows.borrow().more.values().map(|ui| ui.clone_strong()).collect();
        for ui in more {
            ui.apply_theme(theme);
        }
    }

    /// `fx.open_window()`: the window made, themed, wired, drawn whole and shown. One that cannot
    /// be made is said to the app (`Event::WindowNotOpened`), so it can drop what it attached to
    /// the id; none is made once the process is ending.
    pub(super) fn open_window(&self, id: WindowId) {
        if self.exit.get().is_some() {
            tracing::debug!(app = self.id, "a window asked for while ending; not made");
            return;
        }
        let ui = match A::Ui::make() {
            Ok(ui) => ui,
            Err(e) => {
                let why = format!("a window could not be made: {e}");
                tracing::warn!(app = self.id, "{why}");
                problems::record_failure(self.id, "window", &why);
                self.event(Event::WindowNotOpened { window: id, why });
                return;
            }
        };
        let theme = self.windows.borrow().theme.unwrap_or_else(theme::load);
        ui.apply_theme(&theme);
        ui.connect_chrome();
        self.hear_window(id, &ui);
        let post = self.post();
        ui.window().on_close_requested(move || {
            // As the first window's: the app decides, a close now or a flow ending in `fx.close`.
            let _ = post.post(Inbound::Event(Event::CloseRequested(id)));
            crate::runner::drain_on_ui_thread();
            slint::CloseRequestResponse::KeepWindowShown
        });
        self.windows.borrow_mut().more.insert(id, ui.clone_strong());
        if let Some(core) = self.me.upgrade() {
            A::wire_window(&Cx::new(core), &ui, id);
        }
        A::render(&ui, id, &self.model.borrow(), &Changed::all());
        if let Err(e) = ui.show() {
            // Not on screen, so not open: the app hears of it as of a window never made.
            let why = format!("a window could not be shown: {e}");
            tracing::warn!(app = self.id, "{why}");
            problems::record_failure(self.id, "window", &why);
            self.windows.borrow_mut().more.remove(&id);
            slint::Timer::single_shot(Duration::ZERO, move || drop(ui));
            self.event(Event::WindowNotOpened { window: id, why });
            return;
        }
        // A window on screen again: a linger the app was in is over.
        self.linger_ended();
    }

    /// A window closed: only it. Whether that was the last is judged by
    /// [`Core::exit_if_no_window`], once the command's effects are all applied.
    pub(super) fn close_window(&self, window: WindowId) {
        self.windows.borrow_mut().not_active(window);
        if window == WindowId::PRIMARY {
            let _ = self.ui.hide();
            self.windows.borrow_mut().first_open = false;
            return;
        }
        let removed = self.windows.borrow_mut().more.remove(&window);
        if let Some(ui) = removed {
            let _ = ui.hide();
            // The close may come from inside this window's own callback: the component is
            // dropped from the event loop, after that callback has returned.
            slint::Timer::single_shot(Duration::ZERO, move || drop(ui));
        }
    }

    /// `ui`, as `window`, tells the core when the window manager gives it the keyboard or takes
    /// it away (winit's `Focused`), and each frame it draws (`RedrawRequested`, counted by
    /// [`Core::frame_drawn`]). The runner installs the first window's; `open_window` every later
    /// one's. The hook holds the core weakly: the window it lives in is the core's.
    ///
    /// The hook only enqueues, as the close handler does: the app's `update` runs in the drain
    /// after it, not inside winit's dispatch of the event. A frame is only counted.
    pub(crate) fn hear_window(&self, window: WindowId, ui: &A::Ui) {
        let core = self.me.clone();
        let drew = self.me.clone();
        let focused = move |focused| {
            let posted = core.upgrade().is_some_and(|core| core.focus_changed(window, focused));
            if posted {
                crate::runner::drain_on_ui_thread();
            }
        };
        let drawn = move || {
            if let Some(core) = drew.upgrade() {
                core.frame_drawn();
            }
        };
        crate::chrome::hear(ui.window(), focused, drawn);
    }

    /// The window manager's word on `window`: it has the keyboard now, or it lost it. Gaining it
    /// posts `Event::Activated` when the window is open and was not already the active one, and
    /// says so; the caller drains. A loss only forgets it, so the next gain raises again.
    fn focus_changed(&self, window: WindowId, focused: bool) -> bool {
        if !focused {
            self.windows.borrow_mut().not_active(window);
            return false;
        }
        if !self.windows_open().contains(&window) {
            return false;
        }
        let was = self.windows.borrow_mut().active.replace(window);
        if was == Some(window) {
            return false;
        }
        if let Err(dropped) = self.post.post(Inbound::Event(Event::Activated(window))) {
            // Not heard, so not counted: the window manager's next word about it tries again.
            tracing::debug!(app = self.id, ?dropped, "an activation was not queued");
            self.windows.borrow_mut().active = was;
            return false;
        }
        true
    }

    /// What the window manager would say, for a test (the headless platform has none): `window`
    /// gained the keyboard (`true`) or lost it (`false`). The same path as winit's `Focused`
    /// through [`Core::hear_window`]'s hook, with the drain called here, as `TestApp::seen`
    /// drains what it posts. Behind the `testing` feature, as `TestApp` reaches it.
    #[cfg(any(test, feature = "testing"))]
    pub fn focus_for_test(&self, window: WindowId, focused: bool) {
        if self.focus_changed(window, focused) {
            self.drain();
        }
    }

    /// The process ends when no window is open, unless it lingers (`fx.linger`): the linger
    /// then decides the end.
    pub(super) fn exit_if_no_window(&self) {
        if self.windows_open().is_empty() && !self.lingering() {
            self.request_exit(ExitReason::LastWindowClosed);
        }
    }

    /// A second launch asked for this app (`present`): an open window is brought forward, the
    /// first open one first; during a linger, the first window is opened again, drawn whole, and
    /// the linger is over. The app hears which window as `Event::Presented`, once it is shown.
    pub fn present(&self) -> Result<(), String> {
        let open = self.windows_open().into_iter().next();
        let (window, reopened) = match open {
            Some(window) => (window, false),
            None if self.lingering() => (self.reopen_first(), true),
            None => return Err("the app's windows are closing".into()),
        };
        let ui = self.window(window).ok_or("the window is closing")?;
        if let Err(e) = ui.show() {
            if reopened {
                // Not on screen, so not open: the linger goes on, under its backstop.
                self.windows.borrow_mut().first_open = false;
            }
            return Err(format!("the window could not be shown: {e}"));
        }
        if reopened {
            self.linger_ended();
        }
        crate::chrome::bring_back(ui.window());
        self.event(Event::Presented(window));
        Ok(())
    }

    /// The first window, counted open again and drawn whole from the model, for `present` to
    /// show. The event loop kept it, hidden, and it is still wired by `App::wire`. It is the only
    /// window open, so the whole render is its alone, with a callback it fires queued as ever.
    fn reopen_first(&self) -> WindowId {
        self.windows.borrow_mut().first_open = true;
        self.render_all();
        WindowId::PRIMARY
    }
}
