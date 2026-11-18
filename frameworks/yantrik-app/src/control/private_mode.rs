//! Private mode, for an app's surface on this runtime (#924).
//!
//! The view itself is `yantrik_surface::private`'s, the same for a window and a service: in
//! Private mode `app.describe` answers with the app's id, `"private": true`, a neutral summary,
//! what is known of its windows, and only the keys of its own state it listed as not personal.
//! This module is the window side of it:
//!
//! - the socket door ([`super::ControlRpc`]) reads Private mode on its RPC thread for every call,
//!   where file IO belongs, and hands the answer to the UI thread with the call, so there is no
//!   cache to go stale and no restart: turning it on holds from the next call;
//! - a framework app's registry ([`wire`]) is told which keys the app shows (`Intents::
//!   shown_in_private`, default none), what its windows are (`open`, `windows_open`, from the
//!   core), and where an in-process read — a test's `describe` — learns Private mode.
//!
//! Under test (`cfg(test)` or the `testing` feature, which only a dev-dependency may turn on) a
//! test can also pin its own app private ([`testing::pin`]) without touching the file, or any
//! other test's app. A pin only adds: the app is private when pinned or when the file says so,
//! never less private than the person made it. The socket door never reads a pin: it reads the
//! file, under test as in a shipped build.

use std::rc::Weak;

use yantrik_surface::{LocalRegistry, PrivacySource};
/// The line a surface's summary is in Private mode, and the keys the private view writes itself.
pub use yantrik_surface::{PRIVATE_SUMMARY, RESERVED_KEYS};

use crate::app::App;
use crate::control::View;
use crate::cx::Core;
use crate::paths::Paths;

/// Wire `registry`, a framework app's, for Private mode: `shown` are the keys of its own state it
/// keeps (each listed by name, `Intents::shown_in_private`), its windows are read from `core`,
/// and an in-process read asks [`source`].
pub(crate) fn wire<A: App>(registry: &mut LocalRegistry, core: Weak<Core<A>>, shown: &[&'static str], paths: &Paths) {
    for key in shown {
        registry.show_in_private(key);
    }
    registry.set_presence(Box::new(move || {
        let windows = core.upgrade().map(|core| core.windows_open().len()).unwrap_or(0);
        View::new(A::ID).with("open", windows > 0).with("windows_open", windows as u64)
    }));
    if let Some(source) = source(paths) {
        registry.set_privacy_source(source);
    }
}

/// What an in-process read of the app at `paths` asks beside the file the shell publishes (which
/// the registry always reads): under test, this app's pin; in a shipped build, nothing.
fn source(paths: &Paths) -> Option<Box<PrivacySource>> {
    #[cfg(any(test, feature = "testing"))]
    {
        let key = paths.state_dir().to_path_buf();
        Some(Box::new(move || testing::pinned(&key)))
    }
    #[cfg(not(any(test, feature = "testing")))]
    {
        let _ = paths;
        None
    }
}

/// Private mode for an app under test, pinned per app.
#[cfg(any(test, feature = "testing"))]
pub mod testing {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use crate::paths::Paths;

    /// The apps under test pinned private, by their state directory (unique to each test's app).
    static PINNED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

    /// Pin the app whose files are at `paths` private, or take the pin away, as its own
    /// `describe` and `act` see it from the next call on. Every other app under test is untouched,
    /// and taking a pin away leaves the app as private as the person's file says.
    pub fn pin(paths: &Paths, private: bool) {
        let key = paths.state_dir().to_path_buf();
        let mut pinned = PINNED.lock().unwrap_or_else(|p| p.into_inner());
        pinned.retain(|k| *k != key);
        if private {
            pinned.push(key);
        }
    }

    pub(super) fn pinned(key: &Path) -> bool {
        PINNED.lock().unwrap_or_else(|p| p.into_inner()).iter().any(|k| k == key)
    }
}
