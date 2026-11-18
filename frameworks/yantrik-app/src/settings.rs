//! An app's settings: typed, versioned, and never lost (design/app-framework-2026-10-21.md, §6).
//!
//! The store owns the *committed* value: what is durably on disk, by revision. The app's model
//! holds the value it is using. A change goes through a command (`fx.save_settings`), the
//! write goes through the ordered persistence lane, and the result comes back as an event. A
//! failed write keeps the desired value in memory and says so; it never rolls the model back.
//!
//! The file is the app's own, `Paths::config_file`, in the marked envelope [`file`] describes.
//! A missing file is the defaults, unwritten. An older schema is migrated and rewritten once. A
//! file that is malformed, from a newer schema than this app knows, or not this app's at all (no
//! mark, or another app's) puts the store in **recovery**: the app runs on the defaults, a
//! problem record is filed, and no save writes over that file until the person chooses to
//! replace it (`Store::take_over`), which first sets it aside beside itself. A file that is not
//! this app's is never read past its mark, and only its path is ever said.
//!
//! Settings an app kept at the old shared place are brought across once by [`former`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

pub mod file;
pub mod former;
mod mark;
#[cfg(all(test, target_os = "linux"))]
mod probe_tests;
#[cfg(test)]
mod tests;

pub use file::{describe, read_file, render, Fingerprint, Opened};

/// What an app's settings must be: plain data with a schema number.
pub trait Settings:
    Serialize + DeserializeOwned + Default + Clone + PartialEq + Send + Sync + 'static
{
    /// Bumped when the shape changes; `migrate` is given every older version.
    const SCHEMA: u32;

    /// The file under `<config>/yantrik/` this app's settings were kept in before they had a
    /// directory of their own (`"weather.json"`), brought across once by [`former`] and left
    /// where it is. `None`, the default, for an app that kept none there: the old place is
    /// then never looked at. A name that is another component's file is refused regardless.
    const FORMER_FILE: Option<&'static str> = None;

    /// The document of an older schema, brought to this one. The default accepts it as is.
    fn migrate(from: u32, raw: Value) -> Result<Value, SettingsError> {
        let _ = from;
        Ok(raw)
    }

    /// The keys of the `FORMER_FILE` as it was before the framework: no envelope, an object of
    /// the app's own fields, read as this schema reads them. Such a file is looked at past its
    /// first bytes only if its first key is one of these, and taken only if every key is one of
    /// these and the object reads as `Self` (`former`). The default, none, recognises no such
    /// shape, so only the framework's own envelope is brought across.
    const LEGACY_KEYS: &'static [&'static str] = &[];
}

/// An app with nothing to remember: `type Settings = ();`.
impl Settings for () {
    const SCHEMA: u32 = 0;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    #[error("the settings file could not be read: {0}")]
    Unreadable(String),
    #[error("the settings file is not in a shape this app reads: {0}")]
    Malformed(String),
    #[error("the file where the settings go is not one this app wrote; it is left as it is")]
    NotOurs,
    #[error("the settings file is from a newer version of this app (schema {found}, this app knows {known})")]
    Newer { found: u32, known: u32 },
    #[error("the settings could not be migrated from schema {from}: {reason}")]
    Migration { from: u32, reason: String },
    #[error("the settings could not be written: {0}")]
    Write(String),
}

/// Why the file on disk is left alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Health {
    /// The file is this app's, or there is none yet.
    Fine,
    /// The file could not be used: the app runs on the defaults and will not overwrite it.
    Recovery(SettingsError),
}

/// A revision of the settings: monotonic within a run, so an older write's outcome never
/// replaces a newer state.
pub type Revision = u64;

/// The store: the committed value, the desired one, and what the file is worth.
pub struct Store<T: Settings> {
    path: PathBuf,
    /// The app whose mark the file must carry, and every write carries.
    app: String,
    committed: T,
    desired: T,
    revision: Revision,
    committed_revision: Revision,
    health: Health,
    /// The file as last read or written, so an outside change is told from this app's own echo.
    fingerprint: Option<Fingerprint>,
    /// The bytes handed to the lane for each revision not yet known to have landed. A file that
    /// reads as one of these is this app's own write seen early (the watcher polled between the
    /// rename and the outcome's arrival), not an outside edit.
    in_flight: BTreeMap<Revision, Fingerprint>,
}

impl<T: Settings> Store<T> {
    /// Opens `app`'s store at `path`. Never fails: a file that cannot be used leaves the store
    /// in recovery, which `health` says.
    pub fn open(path: PathBuf, app: &str) -> Store<T> {
        // A temporary a killed process left beside the file is swept, if it is the app's own;
        // nothing else would. Never one that may hold a displaced file of someone else's.
        file::sweep_own_temps(&path, app, std::time::Duration::from_secs(600));
        let (committed, health, rewrite) = match read_file::<T>(&path, app) {
            Opened::Read { value, rewrite } => (value, Health::Fine, rewrite),
            Opened::Missing => (T::default(), Health::Fine, false),
            Opened::Recovery(e) => {
                if e == SettingsError::NotOurs {
                    tracing::warn!(app, file = %path.display(), "a file this app did not write is where its settings go; it is left as it is, unread");
                }
                (T::default(), Health::Recovery(e), false)
            }
        };
        let fingerprint = Fingerprint::of_settings(&path, app);
        let mut store = Store {
            path,
            app: app.to_string(),
            desired: committed.clone(),
            committed,
            revision: 0,
            committed_revision: 0,
            health,
            fingerprint,
            in_flight: BTreeMap::new(),
        };
        if rewrite {
            // An older schema, migrated: owed one write at the new schema.
            store.revision = 1;
        }
        store
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What the app should use: the latest value asked for, committed or not.
    pub fn get(&self) -> &T {
        &self.desired
    }

    pub fn committed(&self) -> &T {
        &self.committed
    }

    pub fn health(&self) -> &Health {
        &self.health
    }

    pub fn in_recovery(&self) -> bool {
        matches!(self.health, Health::Recovery(_))
    }

    /// Whether a desired value has not reached the disk.
    pub fn is_pending(&self) -> bool {
        self.revision != self.committed_revision
    }

    /// A new desired value. Returns the revision a write of it carries, or `None` when nothing
    /// changed. In recovery, the value is held in memory and not written (see `take_over`).
    pub fn desire(&mut self, value: T) -> Option<Revision> {
        if value == self.desired && !self.is_pending() {
            return None;
        }
        self.desired = value;
        self.revision += 1;
        Some(self.revision)
    }

    /// The bytes to write for the desired value at `revision`, unless recovery forbids it. The
    /// bytes are remembered as in flight, so the file reading as them is this app's own write.
    pub fn bytes_to_write(&mut self) -> Option<(Revision, Vec<u8>)> {
        if self.in_recovery() || !self.is_pending() {
            return None;
        }
        let bytes = render(&self.app, &self.desired).ok()?;
        self.in_flight.insert(self.revision, Fingerprint::of_bytes(&bytes));
        Some((self.revision, bytes))
    }

    /// The write of `revision` landed. Only the newest revision moves the committed value; an
    /// older write's success is nothing. The file is this app's now, whatever it was before: the
    /// lane writes only over a file with this app's mark (`file::replace`), so recovery ends.
    pub fn committed_at(&mut self, revision: Revision) {
        let written = self.in_flight.remove(&revision);
        if revision > self.committed_revision {
            self.committed_revision = revision;
            if revision == self.revision {
                self.committed = self.desired.clone();
            }
            // What was written is known without reading the file back, so an outside edit that
            // lands between the write and this is not mistaken for the app's own.
            self.fingerprint = written.or_else(|| Fingerprint::of_settings(&self.path, &self.app));
            if self.in_recovery() {
                tracing::info!(app = %self.app, "a write of this app's own landed over the unusable settings file; recovery ends");
                self.health = Health::Fine;
            }
        }
        self.in_flight.retain(|r, _| *r > self.committed_revision);
    }

    /// The person chose to replace an unusable file with this app's settings. The file is set
    /// aside beside itself first (never deleted, never written over); then recovery ends and the
    /// desired value is owed a write. A file that cannot be set aside keeps the store in
    /// recovery, so it is not written over either.
    pub fn take_over(&mut self) {
        if !self.in_recovery() {
            return;
        }
        match file::set_aside(&self.path) {
            Ok(kept) => {
                if let Some(kept) = kept {
                    tracing::info!(app = %self.app, file = %self.path.display(), kept = %kept.display(), "the unusable settings file is set aside; this app's settings take its place");
                }
                self.fingerprint = None;
                self.health = Health::Fine;
                self.revision += 1;
            }
            Err(e) => {
                tracing::warn!(app = %self.app, file = %self.path.display(), error = %e, "the unusable settings file could not be set aside; it is left as it is and not written over");
            }
        }
    }

    /// Whether the file changed under this app since it last read or wrote it.
    pub fn changed_outside(&self) -> bool {
        Fingerprint::of_settings(&self.path, &self.app) != self.fingerprint
    }

    /// Re-reads a file that changed outside this app.
    pub fn reload(&mut self) -> Reloaded<T> {
        let now = Fingerprint::of_settings(&self.path, &self.app);
        if now == self.fingerprint {
            return Reloaded::Unchanged;
        }
        // The app's own write, seen by the watcher before its outcome reached the core.
        if now.as_ref().is_some_and(|n| self.in_flight.values().any(|f| f == n)) {
            return Reloaded::Unchanged;
        }
        let was_unusable = self.in_recovery();
        let outside = match read_file::<T>(&self.path, &self.app) {
            Opened::Read { value, .. } => value,
            Opened::Missing => T::default(),
            Opened::Recovery(e) => {
                self.health = Health::Recovery(e.clone());
                self.fingerprint = now;
                return Reloaded::Unusable(e);
            }
        };
        self.fingerprint = now;
        self.health = Health::Fine;
        // A file repaired by hand while the app held a value it could not save: the repair is
        // the person's choice (§6), and the held value gives way to it.
        if self.is_pending() && outside != self.desired && !was_unusable {
            return Reloaded::Conflict(Conflict { outside, mine: self.desired.clone() });
        }
        self.committed = outside.clone();
        self.committed_revision = self.revision;
        self.desired = outside.clone();
        self.in_flight.clear();
        Reloaded::Adopted(outside)
    }

    /// Resolves a conflict the app's way: `value` becomes desired and is owed a write.
    pub fn resolve(&mut self, value: T) -> Revision {
        self.desired = value;
        self.revision += 1;
        self.revision
    }
}

/// An outside edit landed while this app had a pending edit of its own.
#[derive(Debug, Clone, PartialEq)]
pub struct Conflict<T> {
    pub outside: T,
    pub mine: T,
}

/// What a re-read of the file came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Reloaded<T> {
    /// The file is as this app last read or wrote it.
    Unchanged,
    /// The outside value, now committed and desired.
    Adopted(T),
    /// This app had a pending edit of its own; nothing was changed, the app decides.
    Conflict(Conflict<T>),
    /// The file is no longer usable; the store is in recovery and keeps its values.
    Unusable(SettingsError),
}
