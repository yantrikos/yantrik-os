//! Two Slint windows in one process: how a `Platform` hands out one window per component.
//!
//! `yantrik-lock` gets away with one `MinimalSoftwareWindow` resized per output because every
//! surface shows the same component. A bar and a popover are different components, so each needs
//! its own window, and Slint asks the platform for a window exactly when a component is created.
//! The trick: queue the window first, create the component second; the platform pops it.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};

thread_local! {
    static NEXT: RefCell<VecDeque<Rc<MinimalSoftwareWindow>>> = const { RefCell::new(VecDeque::new()) };
}

struct Queued;

impl Platform for Queued {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        NEXT.with(|q| q.borrow_mut().pop_front())
            .map(|w| w as Rc<dyn WindowAdapter>)
            .ok_or_else(|| slint::PlatformError::Other("a component was created with no window queued".into()))
    }
}

pub fn install() {
    slint::platform::set_platform(Box::new(Queued)).expect("one platform per process");
}

/// A window for the component created by `make` (which must create exactly one component).
/// `ReusedBuffer`: the renderer paints only what changed into a buffer we keep, and tells us the
/// dirty rectangle, so an idle bar paints nothing and a clock tick repaints a few hundred pixels.
pub fn with_window<T>(make: impl FnOnce() -> T) -> (Rc<MinimalSoftwareWindow>, T) {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    NEXT.with(|q| q.borrow_mut().push_back(window.clone()));
    let component = make();
    (window, component)
}
