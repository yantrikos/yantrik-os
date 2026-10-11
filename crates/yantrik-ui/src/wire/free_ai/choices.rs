//! What the person chose on the free AI card, kept across restarts: which providers they
//! skipped, which they switched off for the pool, and how far each sign-up got.
//!
//! `~/.config/yantrik/free-ai.json`, a file of its own beside settings.yaml (as the mind panel's
//! is): a press on this card is not a settings save. No key is ever in it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Choices {
    /// Skipped, and so left out of "Next"; undone with Undo skip.
    pub skipped: BTreeSet<String>,
    /// Switched off for the pool; the key, if any, is kept.
    pub off: BTreeSet<String>,
    /// How far a sign-up got: "sign-up-opened" or "waiting".
    pub stage: BTreeMap<String, String>,
}

pub fn path() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/root"));
    home.join(".config").join("yantrik").join("free-ai.json")
}

/// The saved choices; a missing or unreadable file is none, which is logged and left alone.
pub fn load(path: &Path) -> Choices {
    let mut c: Choices = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            tracing::warn!(path = %path.display(), error = %e, "the free AI card's choices could not be read; starting from none");
            Choices::default()
        }),
        Err(_) => Choices::default(),
    };
    // Waiting for a key is a moment, not a setting: a restart goes back to the key page, so the
    // clipboard is not held, and its history not paused, for a key nobody is copying any more.
    for stage in c.stage.values_mut() {
        if stage == "waiting" {
            *stage = "sign-up-opened".to_string();
        }
    }
    c
}

/// Write them: a temporary file, then a rename.
pub fn save(path: &Path, choices: &Choices) -> Result<(), String> {
    let dir = path.parent().ok_or("the free AI card's file has no directory")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;
    let text = serde_json::to_string_pretty(choices).map_err(|e| e.to_string())?;
    let temp = path.with_extension("json.new");
    std::fs::write(&temp, format!("{text}\n")).map_err(|e| format!("could not write {}: {e}", temp.display()))?;
    std::fs::rename(&temp, path).map_err(|e| format!("could not replace {}: {e}", path.display()))
}
