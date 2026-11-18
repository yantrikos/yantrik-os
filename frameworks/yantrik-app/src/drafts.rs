//! Drafts: what the person was in the middle of, kept so a crash or a logout loses nothing
//! (design/app-framework-2026-10-21.md, §6).
//!
//! A draft is bytes under a name, at `state_dir/drafts/<name>.json`. The app owns the schema: it
//! serialises what it wants kept and reads it back at the next start from [`Drafts`] in `Boot`.
//! The framework owns the mechanics: every save goes through the ordered lane with the atomic
//! writer, a burst of saves is debounced so typing is not a write per keystroke, a checkpoint at
//! a meaningful transition is written at once, and a discard removes the file. Recovery never
//! depends on a shutdown hook running: the lane has the draft within [`DEBOUNCE`] of the change.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long after the last change a draft is written. Long enough to swallow typing, short
/// enough that a crash loses less than a sentence.
pub const DEBOUNCE: Duration = Duration::from_millis(600);

/// The directory under the state dir.
pub const DIR: &str = "drafts";

/// The drafts found at start, by name.
#[derive(Debug, Clone, Default)]
pub struct Drafts {
    found: BTreeMap<String, Vec<u8>>,
}

impl Drafts {
    /// Reads every draft under `state_dir`. A file that is not a draft's (a wrong name) is left
    /// alone and not listed; a missing directory is no drafts.
    pub fn read(state_dir: &Path) -> Drafts {
        let mut found = BTreeMap::new();
        let Ok(entries) = std::fs::read_dir(state_dir.join(DIR)) else {
            return Drafts::default();
        };
        for entry in entries.flatten() {
            let file = entry.file_name();
            let file = file.to_string_lossy();
            let Some(name) = file.strip_suffix(".json") else {
                continue;
            };
            if !valid_name(name) {
                continue;
            }
            if let Ok(bytes) = std::fs::read(entry.path()) {
                found.insert(name.to_string(), bytes);
            }
        }
        Drafts { found }
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.found.get(name).map(Vec::as_slice)
    }

    /// A draft parsed as JSON, or `None` when there is none or it is not what the app expects.
    pub fn json<T: serde::de::DeserializeOwned>(&self, name: &str) -> Option<T> {
        self.get(name).and_then(|bytes| serde_json::from_slice(bytes).ok())
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.found.keys().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.found.is_empty()
    }
}

/// A draft's name: letters, digits, dashes and underscores, up to 64. Nothing that could leave
/// the drafts directory.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Where the draft `name` lives under `state_dir`. The name must be valid.
pub fn path_in(state_dir: &Path, name: &str) -> PathBuf {
    state_dir.join(DIR).join(format!("{name}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drafts_are_read_by_name_and_odd_files_are_left_alone() {
        let dir = std::env::temp_dir().join(format!("yantrik-drafts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(DIR)).unwrap();
        std::fs::write(path_in(&dir, "note"), br#"{"text":"hi"}"#).unwrap();
        std::fs::write(dir.join(DIR).join("not a draft.json"), b"x").unwrap();
        std::fs::write(dir.join(DIR).join("readme.txt"), b"x").unwrap();
        let drafts = Drafts::read(&dir);
        assert_eq!(drafts.names().collect::<Vec<_>>(), vec!["note"]);
        assert_eq!(drafts.json::<serde_json::Value>("note").unwrap()["text"], "hi");
        assert!(drafts.json::<u32>("note").is_none(), "not what the app expects: none");
        assert!(Drafts::read(&dir.join("absent")).is_empty());
        assert!(!valid_name("../etc") && !valid_name("") && valid_name("letter_2-b"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
