//! Opening the vault: its format checked (and written down, the first time), its folder tried for
//! a write, its folders made, every note and Trash record read within the limits, and an
//! interrupted save replayed as a note of its own. A vault this build may not write is still
//! read, and nothing in it is changed.

use std::os::unix::fs::PermissionsExt as _;
use std::time::UNIX_EPOCH;

use yantrik_app::documents::disk::Step;

use super::records::Record;
use super::{read, valid_id, validate, Vault, LIMIT, MAX_NOTES, META_LIMIT, RECORD_LIMIT, VAULT_LIMIT};
use crate::note::{Kept, Note, NoteText};

/// The library as read, and what to tell the person about it (empty when nothing).
#[derive(Debug, Clone)]
pub struct Loaded {
    pub notes: Vec<Note>,
    pub notice: String,
    /// Why nothing in the vault may be changed, when something stops it (`Vault::mark`, or a
    /// folder that refuses a write): the model's vault takes it on (`Vault::with_access`).
    pub read_only: Option<String>,
    /// The interrupted save this load replayed, so a draft of the same note can be weighed
    /// against it: the newer of the two is the one offered back. Boxed: a note's worth of text,
    /// in a command moved through the app's queue, and almost never there.
    pub interrupted: Option<Box<Interrupted>>,
}

/// An interrupted save, as its journal held it.
#[derive(Debug, Clone)]
pub struct Interrupted {
    /// The note it was a save of.
    pub id: String,
    pub content: NoteText,
    /// When the journal was written, in milliseconds since the epoch: its file's time.
    pub at_ms: u64,
}

/// A note a resolution or a recovery made, as its file name says. At the library's limit these
/// are read first: they hold text the person may have nowhere else.
fn kept_first(id: &str) -> bool {
    Kept::of(id).is_some()
}

/// The most directory entries read: the note limit, with room for sidecars and strays.
const ENTRIES_READ: usize = 10001;

impl Vault {
    /// The vault's folders, made private, and refused if either is not a real directory.
    pub fn initialize(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        for name in [".recovery-v2", ".trash-v2"] {
            let p = self.dir.join(name);
            std::fs::create_dir_all(&p).map_err(|e| e.to_string())?;
            if !std::fs::symlink_metadata(&p).map_err(|e| e.to_string())?.is_dir() {
                return Err("Recovery and Trash must be real directories.".into());
            }
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn load(&self) -> Result<Loaded, String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let read_only = self.mark().and_then(|()| self.try_a_write()).err();
        // A vault open read-only is only read: its folders are neither made nor re-permissioned.
        if read_only.is_none() {
            self.initialize()?;
        }
        let mut notes = vec![];
        let mut bytes = 0;
        let mut skipped = 0;
        let mut over = Over::default();
        let mut entries: Vec<_> = std::fs::read_dir(&self.dir)
            .map_err(|e| e.to_string())?
            .take(ENTRIES_READ)
            .filter_map(Result::ok)
            .collect();
        // Kept notes first, so a limit leaves out an ordinary note rather than the one a
        // resolution just wrote. The library is put back in file-name order below.
        entries.sort_by_key(|e| {
            let name = e.file_name();
            (!kept_first(&name.to_string_lossy()), name)
        });
        for e in entries {
            let Some(id) = e.file_name().to_str().map(String::from) else {
                continue;
            };
            if !id.ends_with(".md") {
                continue;
            }
            let result = (|| {
                valid_id(&id)?;
                let text = read(&e.path(), LIMIT)?.ok_or("Missing note")?;
                validate(&text)?;
                let meta = read(&e.path().with_extension("meta"), META_LIMIT)?.unwrap_or_default();
                Ok::<_, String>((text, meta))
            })();
            let Ok((text, meta)) = result else {
                skipped += 1;
                continue;
            };
            if notes.len() >= MAX_NOTES {
                over.notes += 1;
                continue;
            }
            if bytes + text.len() > VAULT_LIMIT {
                over.text += 1;
                continue;
            }
            bytes += text.len();
            let modified = e
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            notes.push(Note {
                id,
                content: crate::note::NoteText { text, meta },
                modified,
                trash: false,
                stored: true,
            });
        }
        notes.sort_by(|a, b| a.id.cmp(&b.id));
        let trash = self.dir.join(".trash-v2");
        // Read-only, the Trash was neither made nor checked above: one that is missing, or is not
        // a real directory, has nothing to read.
        let trashed = if read_only.is_none() || std::fs::symlink_metadata(&trash).is_ok_and(|m| m.is_dir()) {
            Some(std::fs::read_dir(&trash).map_err(|e| e.to_string())?)
        } else {
            None
        };
        for e in trashed.into_iter().flatten().filter_map(Result::ok).take(MAX_NOTES) {
            if e.path().extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            if let Ok(Some(s)) = read(&e.path(), RECORD_LIMIT) {
                match serde_json::from_str::<Record>(&s) {
                    Ok(r) if valid_id(&r.id).is_ok() && validate(&r.text).is_ok() && bytes + r.text.len() <= VAULT_LIMIT => {
                        bytes += r.text.len();
                        notes.push(r.into_trashed());
                    }
                    _ => skipped += 1,
                }
            }
        }
        // Replaying writes a note and removes the journal: read-only, the journal waits for a
        // load that may write.
        if read_only.is_none() {
            // Through a vault this load found writable, whatever the last one found.
            if let Some((recovered, journal)) = self.with_access(None).replay_journal(&mut notes)? {
                return Ok(Loaded { notes, notice: recovered, read_only, interrupted: Some(Box::new(journal)) });
            }
        }
        let notice = [unloaded(skipped), over.sentence()].into_iter().flatten().collect::<Vec<_>>().join(" ");
        Ok(Loaded { notes, read_only, notice, interrupted: None })
    }

    /// A vault whose marker lets this build write it can still refuse writes (a folder made
    /// read-only, a disk remounted since it was marked). One file made and removed through the
    /// store's create path finds that out at the load, so the library opens read-only with the
    /// reason instead of failing later as unavailable. Its name starts with a dot and is not
    /// `.md`, so one a crash leaves behind is never a note.
    fn try_a_write(&self) -> Result<(), String> {
        let probe = self.dir.join(format!(".{}.write-check", uuid7::uuid7()));
        let made = self.disk.create_new(&probe, b"");
        let _ = self.disk.remove(&probe);
        match made {
            // A directory sync that failed after the file was made is a question of durability,
            // which a save says when it meets it, not a refusal.
            Err(e) if e.step != Step::SyncDir => Err(format!(
                "This library is open read-only: Notes cannot write in its folder ({}). Make the folder \
                 writable, then Refresh.",
                crate::update::said::reason(&e.error)
            )),
            _ => Ok(()),
        }
    }

    /// An interrupted save, kept as a note of its own unless the vault already holds exactly it:
    /// an outside edit made since must survive as well. `None` when there was no journal.
    fn replay_journal(&self, notes: &mut Vec<Note>) -> Result<Option<(String, Interrupted)>, String> {
        let journal = self.journal();
        let Some(s) = read(&journal, RECORD_LIMIT)? else {
            return Ok(None);
        };
        let r: Record = serde_json::from_str(&s)
            .map_err(|_| "Recovery file is unreadable; it has been preserved.")?;
        valid_id(&r.id)?;
        validate(&r.text)?;
        let at_ms = std::fs::metadata(&journal)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as u64);
        let identical = notes
            .iter()
            .any(|a| !a.trash && a.id == r.id && a.content.text == r.text && a.content.meta == r.meta);
        if !identical {
            let id = Kept::Recovered.new_id();
            let content = r.content();
            let saved = self
                .write_note(&id, &content, None, |text| text.is_none(), str::is_empty)
                .map_err(|f| f.error.sentence)?;
            notes.push(Note { id, content, modified: saved.modified, trash: false, stored: true });
        }
        self.remove(&journal)?;
        let interrupted = Interrupted { content: r.content(), id: r.id, at_ms };
        Ok(Some(("Recovered the interrupted save. Your original notes were preserved.".into(), interrupted)))
    }
}

/// Notes left out because the library is at one of its limits.
#[derive(Debug, Default)]
struct Over {
    notes: usize,
    text: usize,
}

impl Over {
    /// Which limit bit, how many notes it left out, and the way out.
    fn sentence(&self) -> Option<String> {
        let (limit, left_out) = if self.notes > 0 {
            ("2,000 notes", self.notes + self.text)
        } else if self.text > 0 {
            ("32 MiB of text", self.text)
        } else {
            return None;
        };
        Some(format!("Library holds more than {limit}; {left_out} not shown. Move some to the Trash, then Refresh."))
    }
}

/// Files that are not notes this build can read.
fn unloaded(skipped: usize) -> Option<String> {
    (skipped > 0).then(|| format!("{skipped} files could not be loaded (their format, or a note over its size limit). Originals are untouched."))
}
