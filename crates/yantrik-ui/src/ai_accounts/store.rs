//! The catalogue as last built, kept in memory for every surface and on disk so the picker is not
//! empty while the first listing runs: `~/.cache/yantrik/model-catalogue.json`, at mode 600. It
//! holds account ids, names and models, never a key (a test writes keys and looks for them).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::models::{AccountModels, CatalogueModel};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Catalogue {
    pub accounts: Vec<AccountModels>,
    /// When it was last built, seconds since the epoch.
    pub at: i64,
}

impl Catalogue {
    pub fn models(&self) -> impl Iterator<Item = &CatalogueModel> {
        self.accounts.iter().flat_map(|a| a.models.iter())
    }

    /// A model by its gateway id, `<account>/<model>`.
    pub fn find(&self, id: &str) -> Option<&CatalogueModel> {
        self.models().find(|m| m.id == id)
    }
}

pub fn path() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/root"));
    home.join(".cache/yantrik/model-catalogue.json")
}

pub fn load(path: &std::path::Path) -> Catalogue {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(path: &std::path::Path, catalogue: &Catalogue) -> Result<(), String> {
    let text = serde_json::to_string_pretty(catalogue).map_err(|e| e.to_string())?;
    crate::private_file::write(path, text.as_bytes(), crate::private_file::Publish::Replace, || Ok(()))
}
