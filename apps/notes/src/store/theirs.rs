//! What a save's replace found another program had written (design/notes-gate-2026-10-21.md §2,
//! "a save that won"; #915 review finding 1). The replace is an atomic exchange
//! (`Disk::exchange`), so the bytes it took the place of come back at the same instant ours went
//! in. Judged against what the note knew was there (`SaveTicket::judge`), anything else was
//! written between the store's check and the replace, and is kept, never dropped: as a
//! `recovered-` note when it can be one, or as a file beside the notes when it cannot.
//!
//! What a replace can hand back unread (`Displaced::Unread`) is a file over the framework's
//! 64 MiB read-back limit, or one it could not read. A note of ours never is one: the store
//! checks the file against the note's own text (at most 256 KiB) before the replace, so only
//! another program's file put there in that instant can be, and it is left where the disk kept
//! it and said.

use std::path::PathBuf;

use yantrik_app::documents::disk::{Displaced, Step};
use yantrik_app::documents::{Replaced, SaveTicket, Witnessed};

use super::{read, validate, Vault, LIMIT};
use crate::note::{Kept, Note, NoteText};

/// The other program's version a save replaced, and where it is now.
#[derive(Debug, Clone)]
pub enum Theirs {
    /// Kept as this new note.
    Kept(Note),
    /// The library already holds exactly these bytes, in this note: nothing new was written.
    Same(String),
    /// Kept as a file beside the notes, here: the disk could not read it back, or it is not text
    /// a note may hold.
    Aside(PathBuf),
    /// It could not be written anywhere (the reason): its text is listed as this note, which is
    /// not on disk until it is opened and saved.
    Unwritten { note: Note, why: String },
    /// It could not be written anywhere, nor listed (not text a note may hold), and why.
    Lost(String),
}

/// A live note of the library, as a save job is told of it: its file name and its text's length.
/// A note whose length matches is read from disk and compared byte for byte.
pub type Alike = (String, usize);

impl Vault {
    /// What the replace displaced, judged; and, when it was another program's write, that write
    /// kept. `alike` are the library's live notes, so bytes one of them already holds are not
    /// kept twice (#915 review: a sync client bouncing one stale copy back and forth).
    pub(super) fn keep_theirs(
        &self,
        ticket: &SaveTicket<NoteText>,
        displaced: Option<Displaced>,
        alike: &[Alike],
    ) -> (Witnessed, Option<Theirs>) {
        let Some(displaced) = displaced else { return (Witnessed::Same, None) };
        // The very bytes this save wrote: another program wrote what we did, and nothing of
        // theirs is lost.
        if matches!(&displaced, Displaced::Bytes(b) if b.as_slice() == ticket.content.text.as_bytes()) {
            return (Witnessed::Same, None);
        }
        let witnessed = ticket.judge(&displaced);
        let kept = match (&displaced, witnessed) {
            (Displaced::Unread(path), _) => Some(Theirs::Aside(path.clone())),
            (Displaced::Bytes(bytes), Witnessed::Replaced(Replaced::OutsideEdit)) => Some(self.keep_bytes(bytes, alike)),
            _ => None,
        };
        (witnessed, kept)
    }

    fn keep_bytes(&self, bytes: &[u8], alike: &[Alike]) -> Theirs {
        if let Some(id) = self.holding(bytes, alike) {
            return Theirs::Same(id);
        }
        let text = std::str::from_utf8(bytes).ok().filter(|t| validate(t).is_ok());
        let Some(text) = text else { return self.aside(bytes) };
        let content = NoteText { text: text.into(), meta: String::new() };
        let id = Kept::Recovered.new_id();
        match self.save_new(&id, &content) {
            Ok(saved) => Theirs::Kept(Note { id, content: saved.content, modified: saved.modified, trash: false, stored: true }),
            Err(why) => Theirs::Unwritten { note: Note { id, content, ..Note::default() }, why },
        }
    }

    /// The live note whose file holds exactly `bytes` now, if any: read from disk and compared
    /// byte for byte, never taken from the library as last loaded, since another program may
    /// have changed or removed that file since.
    pub(super) fn holding(&self, bytes: &[u8], alike: &[Alike]) -> Option<String> {
        alike.iter().filter(|(_, len)| *len == bytes.len()).find_map(|(id, _)| {
            let (path, _) = self.files(id).ok()?;
            let text = read(&path, LIMIT).ok().flatten()?;
            (text.as_bytes() == bytes).then(|| id.clone())
        })
    }

    /// Bytes no note may hold, kept as a file of their own beside the notes. Its name does not
    /// end `.md`, so it is never listed as a note.
    fn aside(&self, bytes: &[u8]) -> Theirs {
        let path = self.dir().join(format!("recovered-{}.txt", uuid7::uuid7()));
        match self.disk.create_new(&path, bytes) {
            // Made, even if its directory sync then failed: it is there to be found.
            Err(e) if e.step != Step::SyncDir => Theirs::Lost(e.error),
            _ => Theirs::Aside(path),
        }
    }
}
