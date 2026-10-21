//! Slint on a software window with no window system.

use slint::platform::{
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    EventLoopProxy, Platform, WindowAdapter,
};
use std::rc::Rc;

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
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Proxy))
    }
}

/// Installs the platform on the calling thread. Once per process; a second call is nothing.
pub fn install() {
    // Err means a platform is already set: fine.
    let _ = slint::platform::set_platform(Box::new(Headless));
}
