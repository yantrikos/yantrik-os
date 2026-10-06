//! What the person picked, per mind, kept across restarts: the model and the effort each mind
//! answers with, and the models picked most recently (the model menu lists them first).
//! `~/.config/yantrik/picker.json`. No key in it: a model is `<account>/<model>`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How many recent models the menu lists first.
pub const RECENT: usize = 5;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MindChoice {
    /// `<account>/<model>`, or "" for the mind's own.
    pub model: String,
    /// `easy` … `xhigh`, or "".
    pub effort: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Choices {
    /// By harness id.
    pub minds: BTreeMap<String, MindChoice>,
    /// Newest first.
    pub recent: Vec<String>,
}

impl Choices {
    pub fn of(&self, mind: &str) -> MindChoice {
        self.minds.get(mind).cloned().unwrap_or_default()
    }

    /// `mind` answers with `model` from now on; it goes to the front of the recent list.
    pub fn pick_model(&mut self, mind: &str, model: &str) {
        self.minds.entry(mind.to_string()).or_default().model = model.to_string();
        self.recent.retain(|m| m != model);
        self.recent.insert(0, model.to_string());
        self.recent.truncate(RECENT * 2);
    }

    pub fn pick_effort(&mut self, mind: &str, effort: &str) {
        self.minds.entry(mind.to_string()).or_default().effort = effort.to_string();
    }
}

pub fn path() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/root"));
    home.join(".config/yantrik/picker.json")
}

pub fn load(path: &Path) -> Choices {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(path: &Path, choices: &Choices) -> Result<(), String> {
    let text = serde_json::to_string_pretty(choices).map_err(|e| e.to_string())?;
    crate::private_file::write(path, format!("{text}\n").as_bytes(), crate::private_file::Publish::Replace, || Ok(()))
}
