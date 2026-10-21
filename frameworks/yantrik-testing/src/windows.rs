//! The app's windows, as a test sees them and as the window manager would treat them: which are
//! open, a second launch, a close button, which window has the keyboard, and a linger with none.

use serde_json::Value;
use yantrik_app::app::{App, Event, WindowId};
use yantrik_app::launch::Launch;

use crate::TestApp;

impl<A: App> TestApp<A> {
    /// The windows open, the first one first.
    pub fn windows(&self) -> Vec<WindowId> {
        self.with(|u| u.core.windows_open())
    }

    /// Reads an open window (`None` once it is closed), as `ui` reads the first.
    pub fn window<R: Send + 'static>(
        &self,
        id: WindowId,
        f: impl FnOnce(&A::Ui) -> R + Send + 'static,
    ) -> Option<R> {
        self.with(move |u| u.core.window(id).map(|ui| f(&ui)))
    }

    /// A second launch asked for the app (`present`): the window brought forward, or the first
    /// opened again during a linger, and `Event::Presented` raised once it is shown.
    pub fn present(&self) -> Result<(), String> {
        self.with(|u| u.core.present())
    }

    /// A second launch handed the person's launch over, as its copy does through the `launch`
    /// action, from the person's dock with nothing to open: `Event::Launch`, then the window
    /// shown as [`TestApp::present`] shows it. The answer is the one the second copy reads.
    pub fn launch_again(&self) -> Result<Value, String> {
        let profile = self.paths().profile().to_string();
        let launch = Launch::empty(&profile);
        self.act("launch", serde_json::json!({ "launch": launch }))?.settle()
    }

    /// Whether the app runs on with no window (`fx.linger`), and nothing has ended it yet.
    pub fn lingering(&self) -> bool {
        self.with(|u| u.core.lingering())
    }

    /// The person pressed a window's close button: `Event::CloseRequested` for it.
    pub fn close_requested(&self, id: WindowId) {
        self.event(Event::CloseRequested(id));
    }

    /// The window manager gave `id` the keyboard, as winit's `Focused(true)` says it under the
    /// runner: `Event::Activated` for it, unless it is already the active window or is closed.
    /// With this crate's `testing` feature.
    #[cfg(feature = "testing")]
    pub fn activate(&self, id: WindowId) {
        self.with(move |u| u.core.focus_for_test(id, true));
    }

    /// The window manager took the keyboard from `id` (another window, or another app's): no
    /// event, but its next [`TestApp::activate`] raises one again. With this crate's `testing`
    /// feature.
    #[cfg(feature = "testing")]
    pub fn deactivate(&self, id: WindowId) {
        self.with(move |u| u.core.focus_for_test(id, false));
    }
}
