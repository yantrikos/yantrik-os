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
    if window.with_winit_window(|w| w.set_minimized(true)).is_none() {
        window.set_minimized(true);
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
pub fn on_activated(window: &slint::Window, mut f: impl FnMut() + 'static) {
    use slint::winit_030::{winit::event::WindowEvent, EventResult};
    window.on_winit_window_event(move |_, event| {
        if matches!(event, WindowEvent::Focused(true)) {
            f();
        }
        EventResult::Propagate
    });
}

/// Connect an `AppWindow`'s bar to its window. Once, after the component is made, in a crate
/// whose root `.slint` exports the kit's `WindowChrome` global:
///
/// ```rust,ignore
/// let ui = NotesApp::new()?;
/// yantrik_app_runtime::window_chrome!(ui);
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
                ui.global::<WindowChrome>().set_maximized(ui.window().is_maximized());
            }
        });
    }};
}
