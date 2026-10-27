//! The Slint platform an app runs on: winit for the person's windows, and this crate's streamed
//! window ([`super::streamed`]) for a Mind's.
//!
//! Slint makes every window through one platform, so this one wraps winit's backend and decides
//! per window: inside [`super::streamed::make`] it makes a streamed window, everywhere else it asks
//! winit. The event loop, the clock, the clipboard and the rest are winit's, unchanged.
//!
//! It is installed only where winit is the backend Slint would pick: `SLINT_BACKEND` unset,
//! `winit`, or `winit-<renderer>`, the renderer kept. Another backend (a KMS console, Qt) runs as
//! it did, and a Mind's window is refused there with a sentence ([`installed`] is `false`).
//!
//! Depends on `i-slint-backend-winit` directly, at the exact version `slint` itself uses: Slint
//! does not re-export the backend, and the two move together.

use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use slint::platform::{Clipboard, EventLoopProxy, Platform, WindowAdapter};

static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Whether this process draws on this crate's platform, so a Mind's window can be made.
pub fn installed() -> bool {
    INSTALLED.load(Ordering::Relaxed)
}

/// What `SLINT_BACKEND` asks for, as this platform reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asked<'a> {
    /// winit, with this renderer, or winit's own choice (`None`).
    Winit(Option<&'a str>),
    /// Another backend, which this platform does not wrap.
    Other,
}

/// What `SLINT_BACKEND` asks for. Slint reads the variable as a backend, a backend and a renderer
/// (`winit-software`), or a renderer alone (`software`), which runs on its default backend, winit.
pub fn renderer_asked(backend: &str) -> Asked<'_> {
    match backend.trim() {
        "" | "winit" => Asked::Winit(None),
        "qt" | "testing" | "winit-" => Asked::Other,
        other if other.starts_with("linuxkms") || other.starts_with("qt-") => Asked::Other,
        other => Asked::Winit(Some(other.strip_prefix("winit-").unwrap_or(other))),
    }
}

/// Installs the platform, before any window is made. A backend that is not winit, or a winit that
/// cannot start, leaves Slint to choose as it always did, and says why.
pub fn install() {
    let backend = std::env::var("SLINT_BACKEND").unwrap_or_default();
    let Asked::Winit(renderer) = renderer_asked(&backend) else {
        tracing::info!(backend, "not winit, so a Mind's window cannot be drawn by this app");
        return;
    };
    let winit = match i_slint_backend_winit::Backend::new_with_renderer_by_name(renderer) {
        Ok(winit) => winit,
        Err(e) => {
            tracing::warn!(error = %e, "winit did not start here; Slint chooses the backend itself");
            return;
        }
    };
    match slint::platform::set_platform(Box::new(Displays::over(Box::new(winit)))) {
        Ok(()) => INSTALLED.store(true, Ordering::Relaxed),
        Err(e) => tracing::warn!(error = %e, "a platform was already set; a Mind's window cannot be drawn"),
    }
}

/// winit's backend, and the streamed window for a window made for a Mind.
pub(crate) struct Displays {
    winit: Box<dyn Platform>,
}

impl Displays {
    /// Over `winit`, any platform: winit's under the runner, a recording one under a test.
    pub(crate) fn over(winit: Box<dyn Platform>) -> Displays {
        Displays { winit }
    }
}

impl Platform for Displays {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        if super::streamed::making() {
            return Ok(super::streamed::new_adapter());
        }
        self.winit.create_window_adapter()
    }

    fn run_event_loop(&self) -> Result<(), slint::PlatformError> {
        self.winit.run_event_loop()
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        self.winit.new_event_loop_proxy()
    }

    fn duration_since_start(&self) -> std::time::Duration {
        self.winit.duration_since_start()
    }

    fn click_interval(&self) -> std::time::Duration {
        self.winit.click_interval()
    }

    fn cursor_flash_cycle(&self) -> std::time::Duration {
        self.winit.cursor_flash_cycle()
    }

    /// The person's clipboard and primary selection: never written from a Mind's window (#972 H1).
    fn set_clipboard_text(&self, text: &str, clipboard: Clipboard) {
        if super::streamed::in_a_minds_window() {
            tracing::debug!("a Mind's window asked to copy; the person's clipboard is not touched");
            return;
        }
        self.winit.set_clipboard_text(text, clipboard)
    }

    /// ... and never read into one.
    fn clipboard_text(&self, clipboard: Clipboard) -> Option<String> {
        if super::streamed::in_a_minds_window() {
            tracing::debug!("a Mind's window asked to paste; the person's clipboard is not read");
            return None;
        }
        self.winit.clipboard_text(clipboard)
    }

    fn debug_log(&self, arguments: core::fmt::Arguments) {
        self.winit.debug_log(arguments)
    }

    fn open_url(&self, url: &str) -> Result<(), slint::PlatformError> {
        self.winit.open_url(url)
    }
}

#[cfg(test)]
#[path = "clipboard_tests.rs"]
mod clipboard_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_winit_is_wrapped_and_its_renderer_is_kept() {
        assert_eq!(renderer_asked(""), Asked::Winit(None));
        assert_eq!(renderer_asked("winit"), Asked::Winit(None));
        assert_eq!(renderer_asked("winit-software"), Asked::Winit(Some("software")));
        assert_eq!(renderer_asked("winit-femtovg"), Asked::Winit(Some("femtovg")));
        assert_eq!(renderer_asked("software"), Asked::Winit(Some("software")), "a renderer alone runs on winit");
        assert_eq!(renderer_asked("linuxkms"), Asked::Other);
        assert_eq!(renderer_asked("linuxkms-software"), Asked::Other);
        assert_eq!(renderer_asked("qt"), Asked::Other);
        assert_eq!(renderer_asked("winit-"), Asked::Other);
        assert!(!installed(), "tests draw on their own platform");
    }
}
