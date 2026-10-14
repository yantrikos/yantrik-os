//! What the person picked, kept across restarts: the one model every mind on Yantrik models
//! answers with, the effort each mind thinks at, and the models picked most recently (the model
//! menu lists them first).
//!
//! The pick is one for every mind (design/onboarding-one-model-one-memory-2026-10-10.md): a
//! harness's config says `"model": "picked"`, the gateway resolves it here at each call, and so
//! changing it in the picker changes it for every mind at once, with no config rewritten.
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
    /// The one model picked, `<account>/<model>`, or "" for none yet.
    pub model: String,
    /// By harness id. A mind's own `model` here is from before the pick was one for every mind,
    /// and holds only until the next pick.
    pub minds: BTreeMap<String, MindChoice>,
    /// Newest first.
    pub recent: Vec<String>,
}

impl Choices {
    pub fn of(&self, mind: &str) -> MindChoice {
        let mut c = self.minds.get(mind).cloned().unwrap_or_default();
        if c.model.is_empty() {
            c.model = self.model.clone();
        }
        c
    }

    /// Every mind answers with `model` from now on (picked in the picker while `_mind` was
    /// showing); it goes to the front of the recent list.
    pub fn pick_model(&mut self, _mind: &str, model: &str) {
        self.model = model.to_string();
        for c in self.minds.values_mut() {
            c.model.clear();
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_pick_changes_the_model_for_every_mind_at_once() {
        let mut c = Choices::default();
        // From before: a mind with a model of its own.
        c.minds.insert("hermes".into(), MindChoice { model: "old/m".into(), effort: "high".into() });
        assert_eq!(c.of("hermes").model, "old/m");
        assert_eq!(c.of("pi").model, "");
        c.pick_model("mind", "groq/gpt-oss-120b");
        for mind in ["mind", "hermes", "pi", "openclaw", "companion"] {
            assert_eq!(c.of(mind).model, "groq/gpt-oss-120b", "{mind}");
        }
        assert_eq!(c.of("hermes").effort, "high", "effort stays each mind's own");
        assert_eq!(c.recent[0], "groq/gpt-oss-120b");
    }
}
