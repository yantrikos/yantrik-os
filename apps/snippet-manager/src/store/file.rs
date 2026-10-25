//! The store's one file: how it is read at start, written, read back after a failed write, and
//! how an export and an import are shaped.
//!
//! One versioned JSON file, pretty-printed, written whole or not at all through Foundation's
//! atomic writer. The cost of one file is that one unreadable file is all of them, which is why a
//! file that cannot be parsed is renamed aside and reported rather than overwritten: the bytes are
//! the only record of what the person had, and this build not being able to read them is not a
//! reason to destroy them.
//!
//! Nothing here runs on the UI thread except [`load`], once, before any window exists.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::rules::{validate, MAX_FILE};
use super::text::{language_for, now};
use super::{Collection, Library, Snippet, FIRST_CUSTOM};

/// The schema this build writes and reads. A file from another version is kept aside rather
/// than guessed at.
pub const STATE_VERSION: u32 = 1;

/// The state file inside a store directory.
pub fn state_path(dir: &Path) -> PathBuf {
    dir.join("snippets.json")
}

/// The file, as it is written.
#[derive(Serialize, Deserialize)]
struct StoredState {
    version: u32,
    #[serde(default)]
    next_id: i32,
    #[serde(default)]
    next_collection: i32,
    #[serde(default)]
    collections: Vec<Collection>,
    #[serde(default)]
    snippets: Vec<Snippet>,
}

/// What an export writes and an import reads. Deliberately a subset of [`StoredState`] with the
/// same field name, so the state file itself can be handed to `import` and works.
#[derive(Serialize, Deserialize)]
struct Bundle {
    version: u32,
    #[serde(default)]
    exported_at: u64,
    #[serde(default)]
    snippets: Vec<Snippet>,
}

/// A file read for `import`: an exported set, or a single source file kept as one snippet.
#[derive(Clone)]
pub enum Incoming {
    Bundle(Vec<Snippet>),
    File {
        title: String,
        language: &'static str,
        text: String,
    },
}

/// The store as the start found it, and what it had to say. An empty notice is the normal case.
pub struct Loaded {
    pub library: Library,
    pub notice: String,
}

/// Read the store, or start empty and say why.
///
/// Never fails: a snippet window that refuses to open is worse than one that has forgotten, and
/// every way this can go wrong leaves a working app with a notice on it.
pub fn load(path: &Path) -> Loaded {
    yantrik_foundation::json_store::sweep_stale_temps(path, std::time::Duration::from_secs(3600));
    let raw = match read_capped(path, MAX_FILE) {
        // Nothing saved yet is the ordinary first run, not a fault worth a notice.
        Ok(None) => return Loaded { library: Library::default(), notice: String::new() },
        Ok(Some(raw)) => raw,
        Err(e) => {
            return Loaded {
                library: Library::default(),
                notice: format!("Could not read {}: {e}", path.display()),
            }
        }
    };
    match decode(&raw) {
        Ok(library) => Loaded { library, notice: String::new() },
        Err(reason) => Loaded { library: Library::default(), notice: keep_aside(path, &reason) },
    }
}

/// What the file holds now, after a write that failed: the last write that landed whole, or
/// nothing. Never renames anything aside; that is the start's to do.
pub fn read_back(path: &Path) -> Result<Library, String> {
    match read_capped(path, MAX_FILE)? {
        None => Ok(Library::default()),
        Some(raw) => decode(&raw),
    }
}

/// The library as the file holds it, pretty-printed.
///
/// Pretty on purpose, as Downloads is: this is a file somebody may have to read by hand on the
/// day the app will not start, and the whitespace costs a few hundred bytes.
pub fn encode(library: &Library) -> Result<Vec<u8>, String> {
    let state = StoredState {
        version: STATE_VERSION,
        next_id: library.next_id,
        next_collection: library.next_collection,
        collections: library.collections.clone(),
        snippets: library.snippets.clone(),
    };
    serde_json::to_vec_pretty(&state).map_err(|e| e.to_string())
}

/// Every snippet, as an export writes them.
pub fn encode_bundle(library: &Library) -> Result<Vec<u8>, String> {
    let bundle = Bundle {
        version: STATE_VERSION,
        exported_at: now(),
        snippets: library.snippets.clone(),
    };
    serde_json::to_vec_pretty(&bundle).map_err(|e| e.to_string())
}

/// Write bytes over `path`, whole or not at all, and say why not in words.
pub fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    yantrik_foundation::json_store::write_atomically(path, bytes)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Take a file in: an exported set, or a single source file kept as one snippet.
///
/// The two are told apart by reading it, not by its name: anything that parses as a bundle of
/// this version is one, and everything else is code.
pub fn read_import(path: &Path) -> Result<Incoming, String> {
    let raw = read_capped(path, MAX_FILE)?.ok_or_else(|| format!("no file at {}", path.display()))?;
    if let Ok(bundle) = serde_json::from_str::<Bundle>(&raw) {
        if bundle.version == STATE_VERSION && !bundle.snippets.is_empty() {
            return Ok(Incoming::Bundle(bundle.snippets));
        }
    }
    let title = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "Untitled Snippet".into());
    Ok(Incoming::File { title, language: language_for(path), text: raw })
}

/// Parse a state file, or say why it cannot be used at all.
fn decode(raw: &str) -> Result<Library, String> {
    let mut state: StoredState =
        serde_json::from_str(raw).map_err(|e| format!("it is not valid snippet JSON: {e}"))?;
    if state.version != STATE_VERSION {
        return Err(format!(
            "it is version {}, and this build reads version {STATE_VERSION}",
            state.version
        ));
    }
    // One unusable row is not a reason to lose the other nineteen, and an id that repeats would
    // make `save` ambiguous, so a duplicate is dropped rather than guessed at.
    let mut seen: Vec<i32> = Vec::new();
    state.snippets.retain(|s| {
        let keep = s.id > 0 && !seen.contains(&s.id) && validate(s).is_ok();
        if keep {
            seen.push(s.id);
        }
        keep
    });
    state
        .collections
        .retain(|c| c.id >= FIRST_CUSTOM && !c.name.trim().is_empty());

    let highest = state.snippets.iter().map(|s| s.id).max().unwrap_or(0);
    let highest_collection = state.collections.iter().map(|c| c.id).max().unwrap_or(0);
    Ok(Library {
        next_id: state.next_id.max(highest + 1).max(1),
        next_collection: state.next_collection.max(highest_collection + 1).max(FIRST_CUSTOM),
        snippets: state.snippets,
        collections: state.collections,
    })
}

/// Move an unreadable file out of the way and say so.
///
/// Kept, never deleted: it is the only record of what the person had, and a mind or a human can
/// read it even when this build cannot. The app then starts empty, which is the honest state —
/// it really does not know what was there.
fn keep_aside(path: &Path, reason: &str) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "snippets.json".into());
    let kept = path.with_file_name(format!("{name}.corrupt-{}", now()));
    match std::fs::rename(path, &kept) {
        Ok(()) => format!(
            "The saved snippets could not be read ({reason}). They have been kept as {} \
             and this window is starting empty.",
            kept.display()
        ),
        // The rename failing is worse than the parse failing: the next save would write over
        // the only copy there is. Say exactly that, rather than reporting a rescue that did not
        // happen.
        Err(e) => format!(
            "The saved snippets could not be read ({reason}) and could not be moved aside \
             ({e}). {} will be overwritten by the next change.",
            path.display()
        ),
    }
}

/// Read a file, refusing one too large to be a snippet store.
fn read_capped(path: &Path, limit: usize) -> Result<Option<String>, String> {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    if metadata.len() > limit as u64 {
        return Err(format!("{} is larger than {} MiB", path.display(), limit / 1024 / 1024));
    }
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|e| format!("{} is not UTF-8 text: {e}", path.display()))?;
    Ok(Some(text))
}
