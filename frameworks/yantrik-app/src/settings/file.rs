//! The settings file on disk: the mark that makes it this framework's, reading it, and the bytes
//! a save writes (design/app-framework-2026-10-21.md, §6).
//!
//! The file is `{"kind": "yantrik-app-settings", "app": "<id>", "schema": n, "settings": {…}}`.
//! The `kind` and the `app` are the mark. A file without them, or with another app's, is not this
//! app's, whatever else it holds: the store does not read it past the mark, does not write over
//! it, and says only its path. Looking for the mark materialises nothing else (every other value
//! is skipped unread by serde), so a secret in such a file never reaches the model, the log, a
//! problem record or the window.
//!
//! Every read of the file, and every fingerprint of it, goes through [`crate::quiet::read_own_marked`]
//! (with [`KIND`] as the mark): no link is followed, no FIFO blocks, and only a regular file of at
//! most [`MAX_BYTES`], with one name (no hard link to another component's file) and the mark in
//! its first bytes, is read past that probe, from the descriptor that was checked. A race that
//! unlinks a second name to make another component's file look sole gives up at most the probe. Anything else is fingerprinted by its metadata alone, so
//! the watcher sees it change without reading a byte of it.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Settings, SettingsError};
use crate::quiet::{self, Looked, Stat};

/// The mark's `kind`.
pub const KIND: &str = "yantrik-app-settings";

/// No settings file is this large; a larger one is not read at all.
pub const MAX_BYTES: u64 = 1 << 20;

/// What a save writes.
#[derive(Serialize)]
struct Envelope<'a> {
    kind: &'a str,
    app: &'a str,
    schema: u32,
    settings: Value,
}

/// The mark alone. Every other field of the document is skipped, not kept.
#[derive(Deserialize)]
struct Mark {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    app: Option<String>,
}

/// The rest, read only once the mark is this app's.
#[derive(Deserialize)]
struct Stored {
    schema: u32,
    settings: Value,
}

/// What the file's content is: its length and a hash of its bytes. Content, not the modification
/// time: an edit that keeps the size and the time (an editor that preserves mtime, a coarse
/// filesystem clock) is still an edit, and a rewrite with the same bytes is not one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    len: u64,
    hash: u64,
}

impl Fingerprint {
    /// A file's fingerprint now, or `None` when it is absent or unreadable. A regular file of one
    /// name (not hard-linked) at most [`MAX_BYTES`] is fingerprinted by its content; anything
    /// else at the path (a link, a FIFO, a device, a larger file) by its metadata alone, and none
    /// of it is read. For a document, whose path is one the person chose; a settings file has its
    /// own ([`Fingerprint::of_settings`]).
    pub fn of(path: &Path) -> Option<Fingerprint> {
        Fingerprint::from(quiet::read_own(path, MAX_BYTES).ok()?)
    }

    /// The fingerprint of a settings file this app owns alone: a file without the mark in its
    /// first bytes is fingerprinted by its metadata, not read, so another component's file at the
    /// settings path (a hard link raced into place) is still seen to change without being read.
    pub fn of_settings(path: &Path) -> Option<Fingerprint> {
        Fingerprint::from(quiet::read_own_marked(path, MAX_BYTES, KIND.as_bytes()).ok()?)
    }

    fn from(looked: Looked) -> Option<Fingerprint> {
        match looked {
            Looked::Missing => None,
            Looked::Read(bytes) => Some(Fingerprint::of_bytes(&bytes)),
            Looked::Refused(stat) => Some(Fingerprint::of_stat(&stat)),
        }
    }

    /// What is known of a thing that is not read: its identity and change marks.
    fn of_stat(stat: &Stat) -> Fingerprint {
        let mut marks = b"not read\0".to_vec();
        marks.extend_from_slice(&stat.dev.to_le_bytes());
        marks.extend_from_slice(&stat.ino.to_le_bytes());
        marks.extend_from_slice(&stat.mtime_ns.to_le_bytes());
        Fingerprint { len: stat.len, hash: hash(&marks) }
    }

    /// The fingerprint the file will have once these bytes are in it.
    pub fn of_bytes(bytes: &[u8]) -> Fingerprint {
        Fingerprint { len: bytes.len() as u64, hash: hash(bytes) }
    }
}

fn hash(bytes: &[u8]) -> u64 {
    // FNV-1a: small, dependency-free, and this is a change detector, not a security hash.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// What `read_file` made of the file.
pub enum Opened<T: Settings> {
    /// The settings as read (migrated if they were older), and whether a rewrite is owed.
    Read { value: T, rewrite: bool },
    /// No file: the defaults, nothing written.
    Missing,
    /// The file is not usable, or not this app's; the defaults, in recovery.
    Recovery(SettingsError),
}

/// A serde error without the values it quotes: a settings file may hold a token, and this
/// sentence goes to the log, a problem record and the window. The kind and the place stay.
pub fn describe(e: &serde_json::Error) -> String {
    use serde_json::error::Category;
    let kind = match e.classify() {
        Category::Io => "could not be read",
        Category::Syntax => "is not valid JSON",
        Category::Eof => "ends early",
        Category::Data => "has a value of the wrong kind",
    };
    if e.line() > 0 {
        format!("{kind} (line {}, column {})", e.line(), e.column())
    } else {
        kind.to_string()
    }
}

/// Whether `bytes` carry `app`'s mark. Nothing but the mark is kept from them.
pub fn is_marked(bytes: &[u8], app: &str) -> bool {
    serde_json::from_slice::<Mark>(bytes)
        .is_ok_and(|m| m.kind.as_deref() == Some(KIND) && m.app.as_deref() == Some(app))
}

/// The bytes of the marked settings file at `path`: `Ok(None)` when there is nothing there,
/// `Err(NotOurs)` for a link, a hard link to another file, a FIFO, a device, a directory, a file
/// too large, or a file without the mark in its first [`crate::quiet::PROBE`] bytes, none of
/// which is read past that probe ([`crate::quiet::read_own_marked`]).
pub(crate) fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, SettingsError> {
    bounded(quiet::read_own_marked(path, MAX_BYTES, KIND.as_bytes()))
}

/// The bytes at `path` for the migration's look at an old file, which may be a legacy file with
/// no mark: the same checks but no mark needle ([`crate::quiet::read_own`]).
pub(crate) fn read_bounded_unmarked(path: &Path) -> Result<Option<Vec<u8>>, SettingsError> {
    bounded(quiet::read_own(path, MAX_BYTES))
}

fn bounded(looked: std::io::Result<Looked>) -> Result<Option<Vec<u8>>, SettingsError> {
    match looked {
        Ok(Looked::Missing) => Ok(None),
        Ok(Looked::Read(bytes)) => Ok(Some(bytes)),
        Ok(Looked::Refused(_)) => Err(SettingsError::NotOurs),
        Err(e) => Err(SettingsError::Unreadable(e.to_string())),
    }
}

/// Reads and migrates `app`'s file at `path`, without touching it. A file without `app`'s mark
/// is `NotOurs`, and nothing of it past the mark is read.
pub fn read_file<T: Settings>(path: &Path, app: &str) -> Opened<T> {
    let bytes = match read_bounded(path) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Opened::Missing,
        Err(e) => return Opened::Recovery(e),
    };
    if !is_marked(&bytes, app) {
        return Opened::Recovery(SettingsError::NotOurs);
    }
    let stored: Stored = match serde_json::from_slice(&bytes) {
        Ok(stored) => stored,
        Err(e) => return Opened::Recovery(SettingsError::Malformed(describe(&e))),
    };
    if stored.schema > T::SCHEMA {
        return Opened::Recovery(SettingsError::Newer { found: stored.schema, known: T::SCHEMA });
    }
    let rewrite = stored.schema < T::SCHEMA;
    let raw = if rewrite {
        match T::migrate(stored.schema, stored.settings) {
            Ok(raw) => raw,
            Err(e) => return Opened::Recovery(e),
        }
    } else {
        stored.settings
    };
    match parse::<T>(raw) {
        Ok(value) => Opened::Read { value, rewrite },
        Err(e) => Opened::Recovery(e),
    }
}

fn parse<T: DeserializeOwned>(raw: Value) -> Result<T, SettingsError> {
    serde_json::from_value::<T>(raw).map_err(|e| SettingsError::Malformed(describe(&e)))
}

/// The bytes the file holds for `app`'s `value`.
pub fn render<T: Settings>(app: &str, value: &T) -> Result<Vec<u8>, SettingsError> {
    let settings = serde_json::to_value(value).map_err(|e| SettingsError::Write(e.to_string()))?;
    render_raw(app, T::SCHEMA, settings)
}

/// The bytes for settings already in their document form, at `schema` (an import from before).
pub(crate) fn render_raw(app: &str, schema: u32, settings: Value) -> Result<Vec<u8>, SettingsError> {
    let envelope = Envelope { kind: KIND, app, schema, settings };
    serde_json::to_vec_pretty(&envelope).map_err(|e| SettingsError::Write(e.to_string()))
}

fn refused(path: &Path) -> String {
    format!("{} is not a settings file this app wrote; it is left as it is", path.display())
}

/// The app a save's bytes are marked for: only bytes the store rendered are a settings save.
fn saved_for(bytes: &[u8]) -> Option<String> {
    match serde_json::from_slice::<Mark>(bytes) {
        Ok(Mark { kind: Some(kind), app: Some(app) }) if kind == KIND && !app.is_empty() => Some(app),
        _ => None,
    }
}

/// Whether a save of `bytes` may replace what is at `path`: nothing, or a file with the same mark
/// the bytes carry. The answer is a sentence with the path and nothing of the file.
pub fn may_replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let app = saved_for(bytes).ok_or_else(|| refused(path))?;
    match read_bounded(path) {
        Ok(None) => Ok(()),
        Ok(Some(existing)) if is_marked(&existing, &app) => Ok(()),
        Ok(Some(_)) | Err(SettingsError::NotOurs) => Err(refused(path)),
        Err(e) => Err(e.to_string()),
    }
}

/// The lane's settings write: `bytes` at `path`, only over nothing or this app's own file.
///
/// Looked at first ([`may_replace`]), then written by exchange
/// ([`crate::documents::exchange::replace_if`]), which checks what it actually displaced: a file
/// without the mark that arrived between the look and the write goes back where it was, or, if it
/// cannot, is kept whole beside the path. Never written over unseen, except on a filesystem with
/// no exchange (NFS, 9p, FUSE), where the look before is the only check.
pub fn replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
    may_replace(path, bytes)?;
    let app = saved_for(bytes).ok_or_else(|| refused(path))?;
    if let Some(dir) = path.parent() {
        private_dir(dir).map_err(|e| e.to_string())?;
    }
    use crate::documents::exchange::{replace_if, Guarded};
    match replace_if(path, bytes, MAX_BYTES, |theirs| is_marked(theirs, &app)) {
        Ok(Guarded::Placed) => Ok(()),
        Ok(Guarded::Refused) => Err(refused(path)),
        Ok(Guarded::KeptAside(kept)) => {
            tracing::warn!(app = %app, file = %path.display(), kept = %kept.display(), "a file this app did not write arrived as its settings were saved; it is kept beside them, unread");
            Ok(())
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Removes the temporaries a killed process left beside `app`'s settings file, once they are
/// older than `older_than`: `.<file>.<…>.tmp` (the atomic writer's) and `.<file>.<…>.exchange`
/// (the guarded write's). Only one whose content is known to be the app's own is removed: empty,
/// marked as `app`'s, or the start of a file this framework rendered for `app`, cut short. An
/// exchange temp may hold what the exchange displaced, someone else's file, and that is never
/// removed; neither is anything not read as a regular file of one name.
pub fn sweep_own_temps(path: &Path, app: &str, older_than: std::time::Duration) {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().map(|n| n.to_string_lossy().to_string())) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let prefix = format!(".{name}.");
    let suffixes = [".tmp".to_string(), format!(".{}", crate::documents::exchange::EXCHANGE_SUFFIX)];
    let head = rendered_head(app);
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().to_string();
        if !file.starts_with(&prefix) || !suffixes.iter().any(|s| file.ends_with(s.as_str())) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age >= older_than);
        if !old {
            continue;
        }
        let ours = match quiet::read_own(&entry.path(), MAX_BYTES) {
            Ok(Looked::Read(bytes)) => bytes.is_empty() || is_marked(&bytes, app) || head.starts_with(&bytes) || bytes.starts_with(&head),
            _ => false,
        };
        if ours {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// How every file this framework renders for `app` begins (`render_raw`: the mark first).
fn rendered_head(app: &str) -> Vec<u8> {
    let full = render_raw(app, 0, Value::Null).unwrap_or_default();
    let cut = full.windows(b"\"schema\"".len()).position(|w| w == b"\"schema\"").unwrap_or(full.len());
    full[..cut].to_vec()
}

/// Makes `dir` and any missing parent owner-only (0700): `apps/` and `apps/<id>/` list app
/// names and set-aside files, which are no one else's to see.
pub(crate) fn private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Moves the file at `path` aside, to `<file>.set-aside-<seconds>.json` beside it, so a save can
/// take its place: kept, never deleted, never overwritten. `Ok(None)` when there is nothing there.
pub fn set_aside(path: &Path) -> std::io::Result<Option<PathBuf>> {
    if std::fs::symlink_metadata(path).is_err() {
        return Ok(None);
    }
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    for n in 0..1000u32 {
        let suffix = if n == 0 { String::new() } else { format!("-{n}") };
        let kept = path.with_file_name(format!("{name}.set-aside-{secs}{suffix}.json"));
        if std::fs::symlink_metadata(&kept).is_ok() {
            continue;
        }
        std::fs::rename(path, &kept)?;
        return Ok(Some(kept));
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "no free name to set the file aside under"))
}
