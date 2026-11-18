//! What the base app window's bar asks for, done to the real window (#256).
//!
//! `AppWindow` in the UI kit draws an app's one title bar and asks for four things: move the
//! window, minimise it, maximise or restore it, close it. None of them can be done in Slint's
//! language, so the bar raises callbacks and [`window_chrome!`](crate::window_chrome) connects
//! them here. Each is a request to the compositor, which does the work, the way every desktop's
//! own title bar works: labwc moves and resizes the window, and keeps snapping it to the edges.
//!
//! Close is the one that is not a request to the compositor. It is the window's own close
//! request, the same event labwc's close button used to send, so an app that asks about unsaved
//! work (`on_close_requested`) is asked here too, and one that does not ask simply closes.

use slint::winit_030::WinitWindowAccessor;

/// Hand the move to the compositor: it follows the pointer until the button is let go.
///
/// Called while the button that pressed on the bar is still down; the compositor needs that
/// press to start a move from (Wayland's `xdg_toplevel.move`). A window that is not on a winit
/// backend has no compositor to ask, and stays where it is.
pub fn drag(window: &slint::Window) {
    let moved = window.with_winit_window(|w| w.drag_window());
    match moved {
        Some(Ok(())) => {}
        Some(Err(e)) => tracing::debug!(error = %e, "the compositor did not take the move"),
        None => tracing::debug!("not a winit window, so nothing to move"),
    }
}

/// Put the window away, into the taskbar.
///
/// Straight to winit, not through `Window::set_minimized`. Slint records "minimised" when asked,
/// and Wayland never tells a window it has been brought back, so that record stayed true after
/// the first time: every later Minimise looked like no change and was never sent, and while Slint
/// believed the window minimised it stopped following the compositor's maximise and restore.
/// Found on VM 520 with a protocol trace: the first Minimise sent `xdg_toplevel.set_minimized`,
/// the second sent nothing, and Restore after it did nothing either.
pub fn minimize(window: &slint::Window) {
    if window
        .with_winit_window(|w| w.set_minimized(true))
        .is_none()
    {
        window.set_minimized(true);
    }
}

/// Ask for a window to come back from the taskbar and take focus, for a second launch of its app.
///
/// A request, which Wayland may decline: a client cannot raise itself without a token from the
/// compositor. The shell raises the window as well when the second launch exits
/// (`wire::dock::exit_verdict`); this is what a window whose app was started from a terminal gets.
pub fn bring_back(window: &slint::Window) {
    let asked = window.with_winit_window(|w| {
        w.set_minimized(false);
        w.focus_window();
    });
    if asked.is_none() {
        window.set_minimized(false);
    }
}

/// Maximise a window that is not, restore one that is. Returns what it asked for.
pub fn toggle_maximize(window: &slint::Window) -> bool {
    let maximize = !window.is_maximized();
    window.set_maximized(maximize);
    maximize
}

/// Ask the window to close, through the app's own close request.
pub fn close(window: &slint::Window) {
    let _ = window.dispatch_event(slint::platform::WindowEvent::CloseRequested);
}

/// Call `f` each time the window becomes the active one: brought back to the front, clicked into,
/// or switched to. For a window that has to check something it may have missed while the person
/// was elsewhere (Calendar asks whether the date has turned). A window that is not on a winit
/// backend is never told, and `f` is never called.
///
/// Not for a window the framework runs: it hears this as `Event::Activated`, and winit keeps one
/// event hook per window, so calling this on such a window would replace the framework's.
pub fn on_activated(window: &slint::Window, mut f: impl FnMut() + 'static) {
    on_focus_changed(window, move |focused| {
        if focused {
            f();
        }
    });
}

/// Call `f` with `true` each time the window gains the keyboard from the window manager and
/// `false` each time it loses it (winit's `Focused`), as winit reports it: a repeat is passed on
/// as it came. Slint still handles every event itself. A window that is not on a winit backend
/// is never told. The window's one winit hook: a later call replaces an earlier one.
pub fn on_focus_changed(window: &slint::Window, f: impl FnMut(bool) + 'static) {
    hear(window, f, || {});
}

/// The framework's one winit hook on a window it runs: `focused` with each `Focused`, as
/// [`on_focus_changed`] passes it on, and `drawn` at each `RedrawRequested`, a frame the window
/// is about to draw. On Wayland that is a frame the compositor let it draw: winit holds a redraw
/// until the compositor's frame callback, which a minimised or covered window is not sent, so a
/// window off screen hears none. Slint still handles every event itself.
pub(crate) fn hear(
    window: &slint::Window,
    mut focused: impl FnMut(bool) + 'static,
    mut drawn: impl FnMut() + 'static,
) {
    use slint::winit_030::{winit::event::WindowEvent, EventResult};
    window.on_winit_window_event(move |_, event| {
        match event {
            WindowEvent::Focused(now) => focused(*now),
            WindowEvent::RedrawRequested => drawn(),
            _ => {}
        }
        EventResult::Propagate
    });
}

/// Connect an `AppWindow`'s bar to its window. Once, after the component is made, in a crate
/// whose root `.slint` exports the kit's `WindowChrome` global:
///
/// ```rust,ignore
/// let ui = NotesApp::new()?;
/// yantrik_app::window_chrome!(ui);
/// ```
///
/// A macro because every app's `WindowChrome` is a type of its own, generated into that app's
/// crate from the kit's `.slint`; a macro names it where the app can see it.
#[macro_export]
macro_rules! window_chrome {
    ($ui:expr) => {{
        use $crate::slint::ComponentHandle as _;
        let ui = &$ui;
        let chrome = ui.global::<WindowChrome>();
        let weak = ui.as_weak();
        chrome.on_drag(move || {
            if let Some(ui) = weak.upgrade() {
                $crate::chrome::drag(ui.window());
            }
        });
        let weak = ui.as_weak();
        chrome.on_minimize(move || {
            if let Some(ui) = weak.upgrade() {
                $crate::chrome::minimize(ui.window());
            }
        });
        let weak = ui.as_weak();
        chrome.on_toggle_maximize(move || {
            if let Some(ui) = weak.upgrade() {
                let maximized = $crate::chrome::toggle_maximize(ui.window());
                ui.global::<WindowChrome>().set_maximized(maximized);
            }
        });
        let weak = ui.as_weak();
        chrome.on_close(move || {
            if let Some(ui) = weak.upgrade() {
                $crate::chrome::close(ui.window());
            }
        });
        // The compositor maximises and restores too, and every one of those changes the size.
        let weak = ui.as_weak();
        chrome.on_resized(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<WindowChrome>()
                    .set_maximized(ui.window().is_maximized());
            }
        });
    }};
}
