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

/// Say which app this window is, to the compositor: the Wayland `xdg_toplevel` app_id.
///
/// Without it the compositor, the taskbar and any window picker knew a window of ours only by
/// its title, which an app can rename (#877). An empty id — a window that never said — sets
/// nothing, so the shell's own window and any window not built on `AppWindow` is left as it
/// was.
///
/// Slint keeps the id once for the whole process and gives it to the compositor as it makes
/// the window — Wayland takes an app_id no other time than before the window is first seen —
/// and since an app is its own process, that one id is this window's id. `window_chrome!` is
/// the user, running after the component is made and before it is shown. The window is asked
/// of so that one already shown is said of rather than silently passed; the id takes effect
/// on Wayland and X11 only, and nowhere else.
pub fn set_app_id(window: &slint::Window, app_id: &str) {
    if app_id.is_empty() {
        return;
    }
    if window.is_visible() {
        tracing::debug!(app_id, "the window is shown already, and no app_id goes out after that");
    }
    if let Err(e) = slint::set_xdg_app_id(app_id) {
        tracing::debug!(error = %e, "no windowing system to take the app_id");
    }
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
        // Before the window is shown, which is the one time Wayland takes an app_id. An
        // `AppWindow` copies its `app-id` into the global; anything else says nothing and is
        // left as it was.
        $crate::chrome::set_app_id(ui.window(), &chrome.get_app_id());
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

#[cfg(test)]
mod app_id_tests {
    use super::*;

    /// `set_app_id` reaches a real compositor only on a live winit window; on the software
    /// window below there is none, and the whole point is that the helper then does nothing
    /// rather than panicking, so every app's `window_chrome!` can call it unconditionally. That
    /// the macro actually calls it, and that the id reaches the global it reads, is pinned
    /// against the sources below — the same reason `yantrik-ui`'s `window_action_tests` pins its
    /// handlers to source: the real thing needs a window and a compositor.
    #[test]
    fn set_app_id_on_a_window_with_no_compositor_is_silently_nothing() {
        use slint::platform::{
            software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
            Platform, WindowAdapter,
        };
        // The same headless platform `tests/ui-preview` runs on: a software window with no
        // compositor behind it, which is exactly the case the helper must survive.
        struct NoCompositor(std::rc::Rc<MinimalSoftwareWindow>);
        impl Platform for NoCompositor {
            fn create_window_adapter(
                &self,
            ) -> Result<std::rc::Rc<dyn WindowAdapter>, slint::PlatformError> {
                Ok(self.0.clone())
            }
        }
        let adapter = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        let _ = slint::platform::set_platform(Box::new(NoCompositor(adapter.clone())));
        let window: &slint::Window = adapter.window();
        set_app_id(window, "notes");
        set_app_id(window, "");
    }

    /// The expansion of `window_chrome!` hands the id to `set_app_id` itself, not from a callback.
    fn macro_source() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/chrome.rs");
        let whole = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let from = whole
            .find("macro_rules! window_chrome")
            .expect("chrome.rs still defines window_chrome!");
        let rest = &whole[from..];
        let end = rest
            .find("\n#[cfg(test)]")
            .unwrap_or_else(|| panic!("window_chrome macro runs to the tests? {} bytes", rest.len()));
        rest[..end].to_string()
    }

    #[test]
    fn window_chrome_gives_the_window_its_apps_id_before_anything_else() {
        let mac = macro_source();
        assert!(
            mac.contains("$crate::chrome::set_app_id(ui.window(), &chrome.get_app_id())"),
            "the macro must call set_app_id itself — Wayland takes an app_id only before the \
             window first appears, so no callback the person triggers later can carry it. \
             Macro as written:\n{mac}"
        );
        assert!(
            mac.contains("set_app_id") && mac.find("set_app_id") < mac.find("on_drag"),
            "set_app_id must run before the callbacks are wired, while the window is still \
             unseen. Macro as written:\n{mac}"
        );
    }

    /// An id that never reaches the global is an id the macro cannot read. Both halves of the
    /// Slint side, pinned because Slint compiles no Rust we could call here.
    #[test]
    fn an_app_window_carries_its_app_id_to_the_global_the_macro_reads() {
        let kit = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("yantrik-ui-kit/slint");
        let global = std::fs::read_to_string(kit.join("window_chrome.slint"))
            .expect("the kit still ships window_chrome.slint");
        assert!(
            global.contains("in property <string> app-id"),
            "WindowChrome must carry an `app-id` a component may feed, or the macro has nothing \
             to read:\n{global}"
        );
        assert!(
            global.contains("WindowChrome.app-id = from-prop"),
            "the carriage must assign the global from within a component — Slint accepts that, \
             while a `WindowChrome.app-id: …` binding from a component is a parse error:\n{global}"
        );
        assert!(
            global.contains("if from-prop != \"\""),
            "a window that never said an id must not carry an empty one over one that did:\n{global}"
        );
        let window = std::fs::read_to_string(kit.join("app_window.slint"))
            .expect("the kit still ships app_window.slint");
        assert!(
            window.contains("CarryAppId { from-prop: root.app-id; }"),
            "AppWindow must feed the carriage its `app-id` — an app's generated Rust type sees \
             only what its root declares, never this inherited property:\n{window}"
        );
    }
}
