//! The clipboard, the way the rest of this OS reaches it: `wl-copy` over a pipe, the same call the
//! companion's `clipboard_write` tool and the screenshot path make.
//!
//! Slint does not publish a way to set the selection from Rust, and this is one process on a
//! Wayland session, so the answer is the session's own clipboard tool. If it is missing or
//! refuses, that is reported: a snippet manager that says "copied" without copying is the log
//! line this replaced. The text goes to `wl-copy`'s stdin and nowhere else: it is never logged,
//! put in an answer or kept anywhere by this app.

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::sync::Arc;

/// Where a copy goes. Blocking: from a job only.
pub trait Clipboard: Send + Sync {
    fn copy(&self, text: &str) -> Result<(), String>;
}

/// The session's clipboard, through `wl-copy`. (A test's app copies to `testing::Kept`.)
#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(test, allow(dead_code))]
pub struct WlCopy;

impl Clipboard for WlCopy {
    fn copy(&self, text: &str) -> Result<(), String> {
        let mut child = Command::new("wl-copy")
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| format!("wl-copy could not be started ({e}); nothing was copied"))?;
        let written = match child.stdin.take() {
            Some(mut stdin) => stdin.write_all(text.as_bytes()).map_err(|e| e.to_string()),
            None => Err("wl-copy accepted no input".to_string()),
        };
        let status = child
            .wait()
            .map_err(|e| format!("wl-copy did not finish: {e}"))?;
        written.map_err(|e| format!("could not write to wl-copy: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("wl-copy exited with {status}; nothing was copied"))
        }
    }
}

/// The clipboard an app at these paths copies to: the session's, or under a test one that
/// keeps what it was given, so no test touches the clipboard of the machine it runs on.
pub fn for_app(paths: &yantrik_app::Paths) -> Arc<dyn Clipboard> {
    #[cfg(test)]
    {
        testing::for_app(paths)
    }
    #[cfg(not(test))]
    {
        let _ = paths;
        Arc::new(WlCopy)
    }
}

#[cfg(test)]
pub mod testing {
    //! A clipboard that keeps what it was given, or refuses as `wl-copy` can.

    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex, OnceLock};

    use super::Clipboard;

    #[derive(Default)]
    pub struct Kept {
        texts: Mutex<Vec<String>>,
        refusal: Mutex<Option<String>>,
    }

    impl Kept {
        pub fn texts(&self) -> Vec<String> {
            self.texts.lock().unwrap_or_else(|p| p.into_inner()).clone()
        }

        pub fn refuse(&self, sentence: Option<&str>) {
            *self.refusal.lock().unwrap_or_else(|p| p.into_inner()) = sentence.map(String::from);
        }
    }

    impl Clipboard for Kept {
        fn copy(&self, text: &str) -> Result<(), String> {
            if let Some(why) = self.refusal.lock().unwrap_or_else(|p| p.into_inner()).clone() {
                return Err(why);
            }
            self.texts.lock().unwrap_or_else(|p| p.into_inner()).push(text.to_string());
            Ok(())
        }
    }

    fn kept() -> &'static Mutex<HashMap<PathBuf, Arc<Kept>>> {
        static KEPT: OnceLock<Mutex<HashMap<PathBuf, Arc<Kept>>>> = OnceLock::new();
        KEPT.get_or_init(Default::default)
    }

    /// The clipboard of the app at `paths`: made the first time it is asked for, so a test can
    /// hold it before or after the app starts.
    pub fn of(paths: &yantrik_app::Paths) -> Arc<Kept> {
        kept()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(paths.data_dir().to_path_buf())
            .or_default()
            .clone()
    }

    pub(super) fn for_app(paths: &yantrik_app::Paths) -> Arc<dyn Clipboard> {
        of(paths)
    }
}
