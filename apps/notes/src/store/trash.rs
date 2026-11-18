//! The Trash, and the two ways a note crosses the vault's edge: an import and an export.

use std::path::Path;

use super::records::Record;
use super::{read, validate, Vault, CHANGED_ON_DISK, LIMIT, META_LIMIT};
use crate::note::{Note, NoteText};

impl Vault {
    /// Moves a note to the Trash: its record written first (never over another), then its files
    /// removed. Refused when its files are not what `on_disk` says they hold.
    pub fn trash(&self, id: &str, on_disk: &NoteText, modified: u64) -> Result<(), String> {
        let (text_path, meta_path) = self.files(id)?;
        if read(&text_path, LIMIT)?.as_deref() != Some(on_disk.text.as_str())
            || read(&meta_path, META_LIMIT)?.unwrap_or_default() != on_disk.meta
        {
            return Err(CHANGED_ON_DISK.into());
        }
        let record = Record::of(id, on_disk, Some(on_disk), modified);
        self.put(&self.trash_record(id), &record.json()?, false).map_err(|e| e.error)?;
        self.remove(&text_path)?;
        if meta_path.exists() {
            self.remove(&meta_path)?;
        }
        self.sync_dir()
    }

    /// Puts a trashed note back under its own file name, refusing rather than overwrite a note
    /// that has taken it since.
    pub fn restore(&self, note: &Note) -> Result<Note, String> {
        let (text_path, meta_path) = self.files(&note.id)?;
        if text_path.exists() || meta_path.exists() {
            return Err("A note already uses this filename. Nothing was overwritten.".into());
        }
        let saved = self.save_new(&note.id, &note.content)?;
        self.remove(&self.trash_record(&note.id))?;
        Ok(Note { id: note.id.clone(), content: saved.content, modified: saved.modified, trash: false, stored: true })
    }

    /// A text file from anywhere, added as a new note. `room` is what the library may still hold.
    pub fn import(&self, path: &Path, room: usize) -> Result<Note, String> {
        let text = read(path, LIMIT)?.ok_or("File not found")?;
        validate(&text)?;
        if text.len() > room {
            return Err("Library reached its 32 MiB text limit.".into());
        }
        let mut note = Note::blank("Imported note");
        note.content.text = text;
        let saved = self.save_new(&note.id, &note.content)?;
        Ok(Note { content: saved.content, modified: saved.modified, stored: true, ..note })
    }

    /// A note's text written to a new file outside the vault: never over anything already there.
    /// Not a write into the vault, so a vault open read-only still exports: it is how its notes
    /// get out. Into the vault, or any folder in it, it is a write like any other, and refused
    /// while read-only: a vault a newer Notes wrote may be a writable folder all the same.
    pub fn export(&self, path: &Path, text: &str) -> Result<(), String> {
        if let Some(why) = &self.read_only {
            if self.holds(path) {
                return Err(why.clone());
            }
        }
        self.disk.create_new(path, text.as_bytes()).map_err(|e| e.error)
    }

    /// Whether `path` would land in the vault or a folder in it, through any link on the way. A
    /// parent that cannot be resolved holds nothing: the create there fails on its own.
    fn holds(&self, path: &Path) -> bool {
        let resolved = |p: &Path| std::fs::canonicalize(p).ok();
        match (path.parent().and_then(resolved), resolved(&self.dir)) {
            (Some(parent), Some(vault)) => parent.starts_with(vault),
            _ => false,
        }
    }
}
