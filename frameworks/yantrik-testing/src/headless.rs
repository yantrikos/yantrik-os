//! Slint on a software window with no window system.

use slint::platform::{
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    EventLoopProxy, Platform, WindowAdapter,
};
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Duration;

thread_local! {
    /// Every window this platform made, so a test can draw the one behind a component: the
    /// frames a window asks for are what it costs at rest (`TestApp::at_rest`).
    static MADE: RefCell<Vec<Weak<MinimalSoftwareWindow>>> = const { RefCell::new(Vec::new()) };
}

struct Proxy;

impl EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), slint::EventLoopError> {
        Ok(())
    }

    fn invoke_from_event_loop(
        &self,
        event: Box<dyn FnOnce() + Send>,
    ) -> Result<(), slint::EventLoopError> {
        // Onto the one UI thread, as a step of its own, whichever thread asked: a closure
        // that upgrades a `slint::Weak` must run where the component was made. A panic in
        // it is said and does not take the thread (and every later test) with it.
        let step: super::Step = Box::new(move || {
            if let Err(panic) =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(event))
            {
                let what = panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "a non-string payload".into());
                eprintln!(
                    "yantrik-testing: a closure posted to the UI thread panicked: {what}"
                );
            }
        });
        super::ui_thread()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .send(step)
            .map_err(|_| slint::EventLoopError::EventLoopTerminated)
    }
}

struct Headless;

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        MADE.with(|made| {
            let mut made = made.borrow_mut();
            made.retain(|w| w.strong_count() > 0);
            made.push(Rc::downgrade(&window));
        });
        Ok(window)
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Proxy))
    }

    fn duration_since_start(&self) -> Duration {
        crate::display_clock::now()
    }
}

/// Installs the platform on the calling thread. Once per process; a second call is nothing.
pub fn install() {
    // Err means a platform is already set: fine.
    let _ = slint::platform::set_platform(Box::new(Headless));
}

/// Draws `window` if it asked to be drawn since it was last drawn, with Slint's own software
/// renderer into a buffer of the window's size, as a display would: whether it drew. Drawing is
/// what registers the properties a frame reads, so a window that is never drawn never asks again.
/// `false` for a window this platform did not make.
pub(crate) fn draw_if_needed(window: &slint::Window) -> bool {
    let adapter = MADE.with(|made| {
        made.borrow()
            .iter()
            .filter_map(Weak::upgrade)
            .find(|w| std::ptr::eq(w.window(), window))
    });
    let Some(adapter) = adapter else {
        return false;
    };
    let size = adapter.size();
    let (width, height) = (size.width.max(1), size.height.max(1));
    let mut buffer = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
    adapter.draw_if_needed(|renderer| {
        renderer.render(buffer.make_mut_slice(), width as usize);
    })
}
