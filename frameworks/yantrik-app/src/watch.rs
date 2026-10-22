//! The housekeeping thread: one per app, watching the files that change under it
//! (design/app-framework-2026-10-21.md, §6). It polls every two seconds, by content fingerprint
//! and not by mtime alone, so an edit that keeps the size and the time is still seen, and a
//! replaced or deleted file too. It posts what it saw; the core decides what it means.
//!
//! Watched: the app's own settings file (an outside edit), the shell's `settings.yaml` (the
//! theme, applied live), and the files of the app's open documents (`fx.watch`). Polling, not inotify: no new dependency, and it works on the
//! network and 9p mounts a development machine runs from.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::documents::{Observed, Watched};
use crate::settings::Fingerprint;
use crate::theme::{self, ThemeSettings};

/// How often the files are looked at.
pub const EVERY: Duration = Duration::from_secs(2);

/// What the watcher saw.
#[derive(Debug, Clone, PartialEq)]
pub enum Seen {
    /// The app's settings file is not what it was.
    SettingsFile,
    /// The shell's theme is not what it was.
    Theme(ThemeSettings),
    /// Whether the Mind can answer is not what it was (the first tick says what it is).
    Reach(yantrik_mind::Reach),
    /// A watched document's file is not what was last seen (`fx.watch`), under `arming`.
    File { key: String, seen: Observed, arming: u64 },
}

/// Where sightings go: the core's queue.
pub trait Notice: Send + Sync {
    fn seen(&self, seen: Seen);
}

/// A running watcher; dropping it stops the thread at its next tick.
pub struct Watch {
    stop: Arc<AtomicBool>,
}

impl Watch {
    /// Watches `settings_file`, the shell's theme and the documents' files in `files`, reporting
    /// to `notice`. `theme_now` is what the window was started with, so only a change is
    /// reported.
    pub fn start(
        name: &str,
        settings_file: PathBuf,
        theme_now: ThemeSettings,
        files: Watched,
        notice: Arc<dyn Notice>,
    ) -> Watch {
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        std::thread::Builder::new()
            .name(format!("{name}-watch"))
            .spawn(move || {
                let mut settings_seen = Fingerprint::of_settings(&settings_file);
                let shell_settings = theme::settings_path();
                let mut shell_seen = Fingerprint::of(&shell_settings);
                let mut theme_seen = theme_now;
                let mut reach_seen: Option<yantrik_mind::Reach> = None;
                while !stopping.load(Ordering::Acquire) {
                    std::thread::sleep(EVERY);
                    if stopping.load(Ordering::Acquire) {
                        break;
                    }
                    // The Mind's reach is read from what its own watcher last heard: a bounded
                    // first wait on this thread, nothing after, and never on the UI thread (§5).
                    let reach_now = yantrik_mind::reach();
                    if reach_seen != Some(reach_now) {
                        reach_seen = Some(reach_now);
                        notice.seen(Seen::Reach(reach_now));
                    }
                    let now = Fingerprint::of_settings(&settings_file);
                    if now != settings_seen {
                        settings_seen = now;
                        notice.seen(Seen::SettingsFile);
                    }
                    let shell_now = Fingerprint::of(&shell_settings);
                    if shell_now != shell_seen {
                        shell_seen = shell_now;
                        let theme_now = theme::load();
                        if theme_now != theme_seen {
                            theme_seen = theme_now;
                            notice.seen(Seen::Theme(theme_now));
                        }
                    }
                    for (key, seen, arming) in files.poll() {
                        notice.seen(Seen::File { key, seen, arming });
                    }
                }
            })
            .expect("the watch thread");
        Watch { stop }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop();
    }
}
