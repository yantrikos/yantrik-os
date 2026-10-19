//! App registry — re-exports from yantrik-shell-core, plus the live catalogue.
//!
//! All app registry logic (DesktopEntry, builtin_apps, scan, search) lives in
//! yantrik-shell-core. This module re-exports it, and adds the one thing the shell needs that
//! a pure scan cannot give it: a catalogue that can change while the shell is running.

pub use yantrik_shell_core::apps::*;

use std::sync::{Arc, OnceLock, RwLock};

/// The installed apps, as they are right now.
///
/// This used to be `Arc<Vec<DesktopEntry>>` on AppContext, scanned once during startup and
/// cloned into every closure that needed it. That is a snapshot, and it has one consequence
/// nobody had written down: install something and it does not exist. Not "appears late" —
/// it is absent from the launcher, from the Lens, and from `open_app`, until the shell is
/// restarted. On a desktop whose whole premise is that you can ask it for things, an app you
/// just installed being unaskable is a strange thing to ship.
///
/// So the catalogue is shared and swappable. Readers take a cheap snapshot for the duration of
/// one call and never hold the lock; `refresh` rescans and replaces it in one move, so a reader
/// either sees the whole old list or the whole new one.
#[derive(Clone)]
pub struct Catalogue {
    inner: &'static RwLock<Arc<Vec<DesktopEntry>>>,
}

fn cell() -> &'static RwLock<Arc<Vec<DesktopEntry>>> {
    static CATALOGUE: OnceLock<RwLock<Arc<Vec<DesktopEntry>>>> = OnceLock::new();
    CATALOGUE.get_or_init(|| RwLock::new(Arc::new(Vec::new())))
}

impl Catalogue {
    /// The catalogue, scanning the disk on first use.
    pub fn shared() -> Self {
        let me = Self { inner: cell() };
        if me.get().is_empty() {
            me.refresh();
        }
        me
    }

    /// The current list. Cheap: one Arc clone, no scan, no lock held after returning.
    pub fn get(&self) -> Arc<Vec<DesktopEntry>> {
        match self.inner.read() {
            Ok(guard) => guard.clone(),
            // A poisoned lock means a reader panicked mid-read, which cannot corrupt an Arc
            // swap. An empty launcher would be a worse answer than a stale one.
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Rescan the applications directories and replace the list. Returns how many apps there
    /// are now.
    ///
    /// The scan itself walks a handful of directories and parses small ini files; it is fast
    /// enough to run when the launcher opens, which is exactly when a person who has just
    /// installed something goes looking for it.
    ///
    /// Only what can actually open is kept. Every surface that lists apps reads this — the
    /// launcher, the Lens, `open_app` — so an entry whose program is gone, a built-in tile
    /// nothing routes to, or an app this build has shelved is left out here once rather than at
    /// each of them. The shelf matters most on a machine updated from an older release: the
    /// binary and its .desktop file are both still on the disk, so the scan finds them every
    /// time and it is this filter that keeps the tile off the screen.
    pub fn refresh(&self) -> usize {
        let (kept, dropped): (Vec<DesktopEntry>, Vec<DesktopEntry>) =
            scan().into_iter().partition(crate::wire::dock::entry_is_launchable);
        if !dropped.is_empty() {
            let names: Vec<String> =
                dropped.iter().map(|e| format!("{} ({})", e.name, e.exec)).collect();
            tracing::debug!(apps = ?names, "Left out of the launcher: nothing to run");
        }
        let scanned = Arc::new(kept);
        let count = scanned.len();
        match self.inner.write() {
            Ok(mut guard) => *guard = scanned,
            Err(poisoned) => *poisoned.into_inner() = scanned,
        }
        count
    }
}

impl Default for Catalogue {
    fn default() -> Self {
        Self::shared()
    }
}

impl std::fmt::Debug for Catalogue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Catalogue({} apps)", self.get().len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalogue_is_one_list_shared_by_every_handle() {
        // Two handles are two views of the same thing, not two snapshots. A launcher holding
        // one and a rescan action holding another must not disagree about what is installed.
        let a = Catalogue::shared();
        let b = a.clone();
        let before = a.get().len();
        let after = b.refresh();
        assert_eq!(a.get().len(), after, "a refresh through one handle is visible through both");
        assert!(before > 0 || after == 0, "built-in apps are always present once scanned");
    }

    /// A shelved app never reaches the catalogue, so no surface that reads it can offer one.
    ///
    /// The machine this matters on is one that installed a release with these apps in it: the
    /// scan will keep finding /opt/yantrik/share/applications/yantrik-spreadsheet.desktop until
    /// something removes it, and the launcher, the Lens and `open_app` all read this list.
    #[test]
    fn the_catalogue_leaves_shelved_apps_out() {
        let c = Catalogue::shared();
        c.refresh();
        for entry in c.get().iter() {
            assert!(
                crate::wire::dock::shelved(&entry.app_id).is_none()
                    && crate::wire::dock::shelved_exec(&entry.exec).is_none(),
                "the catalogue lists `{}` ({}), which this build has shelved",
                entry.name,
                entry.exec
            );
        }
    }

    #[test]
    fn a_snapshot_survives_a_refresh_underneath_it() {
        // `get` hands out an Arc, so a caller iterating the list cannot have it changed
        // mid-iteration by a rescan on another thread.
        let c = Catalogue::shared();
        let held = c.get();
        let n = held.len();
        c.refresh();
        assert_eq!(held.len(), n, "the snapshot a caller is reading does not move");
    }
}
