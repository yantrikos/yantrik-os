//! The housekeeping thread: one per app, watching the files that change under it
//! (design/app-framework-2026-10-21.md, §6). It polls every two seconds while anything has
//! been moving and backs off to [`BACKOFF`] once a stretch of looks has seen nothing, by
//! content fingerprint and not by mtime alone, so an edit that keeps the size and the time is
//! still seen, and a replaced or deleted file too. It posts what it saw; the core decides
//! what it means.
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

/// How often the files are looked at while anything is moving.
pub const EVERY: Duration = Duration::from_secs(2);

/// How often they are looked at once [`QUIET_BEFORE_BACKOFF`] looks have seen nothing. An
/// idle desktop changes on the scale of minutes, not seconds; an app that wakes every two
/// seconds all day for nothing is what a person on battery feels as drain (#983).
pub const BACKOFF: Duration = Duration::from_secs(10);

/// How many consecutive quiet looks are still slept through at `EVERY` before `BACKOFF`
/// takes over, so the next edit in a burst never waits out a backoff.
const QUIET_BEFORE_BACKOFF: u32 = 3;

/// How long to sleep before the next look, given how many consecutive looks saw nothing.
/// Zero quiet ticks is the first look after start, so startup behaves as it always did; a
/// sighting resets the count, so a change is at worst one backoff late.
fn next_interval(quiet_ticks: u32) -> Duration {
    if quiet_ticks >= QUIET_BEFORE_BACKOFF {
        BACKOFF
    } else {
        EVERY
    }
}

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
    /// Watches `settings_file` (app `name`'s, judged by its mark), the shell's theme and the
    /// documents' files in `files`, reporting to `notice`. `theme_now` is what the window was
    /// started with, so only a change is reported.
    pub fn start(
        name: &str,
        settings_file: PathBuf,
        theme_now: ThemeSettings,
        files: Watched,
        notice: Arc<dyn Notice>,
    ) -> Watch {
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let app = name.to_string();
        std::thread::Builder::new()
            .name(format!("{name}-watch"))
            .spawn(move || {
                let mut settings_seen = Fingerprint::of_settings(&settings_file, &app);
                let shell_settings = theme::settings_path();
                let mut shell_seen = Fingerprint::of(&shell_settings);
                let mut theme_seen = theme_now;
                let mut reach_seen: Option<yantrik_mind::Reach> = None;
                let mut quiet = 0u32;
                while !stopping.load(Ordering::Acquire) {
                    std::thread::sleep(next_interval(quiet));
                    if stopping.load(Ordering::Acquire) {
                        break;
                    }
                    let mut saw = false;
                    // The Mind's reach is read from what its own watcher last heard: a bounded
                    // first wait on this thread, nothing after, and never on the UI thread (§5).
                    let reach_now = yantrik_mind::reach();
                    if reach_seen != Some(reach_now) {
                        reach_seen = Some(reach_now);
                        saw = true;
                        notice.seen(Seen::Reach(reach_now));
                    }
                    let now = Fingerprint::of_settings(&settings_file, &app);
                    if now != settings_seen {
                        settings_seen = now;
                        saw = true;
                        notice.seen(Seen::SettingsFile);
                    }
                    let shell_now = Fingerprint::of(&shell_settings);
                    if shell_now != shell_seen {
                        shell_seen = shell_now;
                        let theme_now = theme::load();
                        if theme_now != theme_seen {
                            theme_seen = theme_now;
                            saw = true;
                            notice.seen(Seen::Theme(theme_now));
                        }
                    }
                    for (key, seen, arming) in files.poll() {
                        saw = true;
                        notice.seen(Seen::File { key, seen, arming });
                    }
                    quiet = if saw { 0 } else { quiet.saturating_add(1) };
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The defect: the thread slept `EVERY` forever, so an app at rest woke every two
    /// seconds all day reading files that could not have changed (#983). The first look
    /// after start and the first few after any sighting still go at `EVERY`, so a burst of
    /// edits is followed at the old speed; only a stretch of quiet backs off.
    #[test]
    fn quiet_ticks_back_the_interval_off_and_a_sighting_resets_them() {
        assert_eq!(next_interval(0), EVERY, "the first look after start");
        assert_eq!(next_interval(1), EVERY);
        assert_eq!(next_interval(QUIET_BEFORE_BACKOFF - 1), EVERY);
        assert_eq!(next_interval(QUIET_BEFORE_BACKOFF), BACKOFF);
        assert_eq!(next_interval(u32::MAX), BACKOFF, "the count saturates, never wraps");
        assert_eq!(next_interval(0), EVERY, "a sighting resets the count to zero");
    }
}
