//! The draft a Mind made, kept in its own file beside the person's (#1184).
//!
//! A Mind's `compose` never touches the person's kept or open draft: it has a slot of its own,
//! `mind_draft.json`, which holds the text and the files the draft is to carry. The file is what
//! lets the attachment, and so the refusal to send without it, outlive an Email restart: the
//! person's draft file carries text only.

use std::path::{Path, PathBuf};

use super::draft::{write_whole, Draft};

/// A Mind's draft: its text, and the files it carries.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MindDraft {
    #[serde(default)]
    pub text: Draft,
    #[serde(default)]
    pub attachments: Vec<String>,
    /// The person changed the text after the Mind wrote it: it is theirs now, and no Mind's
    /// `compose` replaces it.
    #[serde(default)]
    pub touched: bool,
    /// The promise this draft was prepared for (`commitment_prep`), so forgetting the promise, or
    /// its item expiring, clears the draft (`forget_prepared`). Absent for any other Mind's draft.
    #[serde(default)]
    pub commitment: Option<String>,
    /// How many times `park` has written this draft: a copy with a higher number is the newer one.
    #[serde(default)]
    pub generation: u64,
}

impl MindDraft {
    /// What the entry for it says: "Draft Yantrik prepared for maya@example.com".
    pub fn label(&self) -> String {
        match self.text.to.trim() {
            "" => "Draft Yantrik prepared".to_string(),
            to => format!("Draft Yantrik prepared for {to}"),
        }
    }

    /// The names of the files it carries, for the bar over the composer.
    pub fn file_names(&self) -> String {
        let name = |p: &String| Path::new(p).file_name().map_or_else(|| p.clone(), |n| n.to_string_lossy().into_owned());
        self.attachments.iter().map(name).collect::<Vec<_>>().join(", ")
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.attachments.is_empty()
    }
}

/// Write the slot whole, readable only by its owner; an empty one removes the file.
pub fn save_mind_draft(path: &Path, draft: &MindDraft) -> Result<(), String> {
    if draft.is_empty() {
        return clear_mind_draft(path);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(draft).map_err(|e| format!("could not write the draft: {e}"))?;
    write_whole(path, text.as_bytes())
}

/// The slot, or `None` when there is not one. A file that is there and cannot be read is an
/// error: a prepared draft treated as absent would be sent, one day, without its file.
pub fn load_mind_draft(path: &Path) -> Result<Option<MindDraft>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("could not read {}: {e}", path.display())),
    };
    if text.trim().is_empty() {
        return Ok(None);
    }
    let draft: MindDraft = serde_json::from_str(&text).map_err(|e| format!("{} is not a draft: {e}", path.display()))?;
    Ok((!draft.is_empty()).then_some(draft))
}

/// Where the draft for one promise waits while another promise's draft holds the slot: beside it,
/// as `mind_draft.<id>.json`. `None` for an id that is not plain letters, digits, `-` or `_`, so an
/// id can never name a path.
pub fn shelf_path(slot: &Path, commitment: &str) -> Option<PathBuf> {
    let plain = !commitment.is_empty() && commitment.len() <= 40 && commitment.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    plain.then(|| slot.with_file_name(format!("mind_draft.{commitment}.json")))
}

pub fn clear_mind_draft(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("could not remove {}: {e}", path.display())),
    }
}

/// The file a Mind asked to attach, as a path Email will carry: it exists, it is a file, it is
/// under the person's home, and it is not a protected place (keys, configuration, Yantrik's own
/// data). Links are followed first, so a link in a document folder cannot lead out of them.
pub fn checked_attachment(asked: &str, home: Option<&Path>) -> Result<String, String> {
    let asked = asked.trim();
    let Some(home) = home else { return Err("There is no home folder to attach a file from.".into()) };
    let path = Path::new(asked);
    if !path.is_absolute() {
        return Err("The file to attach must be given as a full path.".into());
    }
    let real: PathBuf = std::fs::canonicalize(path).map_err(|_| "The file to attach is not there.".to_string())?;
    if !real.is_file() {
        return Err("The file to attach is not a file.".into());
    }
    let home = std::fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    if !real.starts_with(&home) {
        return Err("Only files in your home folder can be attached.".into());
    }
    if yantrik_ipc_contracts::home_paths::is_protected(&real) || yantrik_ipc_contracts::home_paths::is_protected(path) {
        return Err("That file is in a protected place and cannot be attached.".into());
    }
    Ok(real.to_string_lossy().into_owned())
}
