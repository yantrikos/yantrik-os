//! What Notes says about saving, in the person's words: why a save did not commit and how much
//! of it reached the disk, that a save replaced another program's write, and what an outside
//! change asks of the person. Kept in one place so the notice line, the outside bar and a mind's
//! refusal always agree.

use yantrik_app::documents::{Durability, OutsideState, Replaced, SaveError};

use crate::note::Kept;

/// The footer while saving waits on the person's choice about an outside change.
pub const WAITING_ON_CHOICE: &str = "Not saved · changed outside Notes";
/// The footer while the note's file is gone: its text is on no disk until it is saved as new.
pub const FILE_GONE: &str = "Not saved · its file was moved or deleted";
/// The footer while the open note's unsaved text from before Notes closed waits on the person.
pub const WAITING_ON_RECOVERY: &str = "Not saved · unsaved text from before Notes closed: Restore or Discard";

/// Why a store job ended without an answer (it panicked, or the pool refused it), as the reason
/// in a sentence below.
pub const JOB_LOST: &str = "the storage worker stopped";

/// The notice for a save that did not commit, by how much of it reached the disk. Each one ends
/// with what Save does now, because nothing retries by itself.
pub fn save_failed(error: &SaveError) -> String {
    let why = reason(&error.sentence);
    match error.durability {
        Durability::NotWritten => {
            format!("Not saved: {why}. The file is unchanged and your text is still here; Save tries again.")
        }
        Durability::ContentOnly => format!(
            "Only the text was saved: {why}. The notebook, tags and pin were not; Save tries again, \
             and if Notes closes first they come back in a recovered note."
        ),
        Durability::Unknown => format!(
            "Saving did not finish: {why}. The note may not be safe on disk yet; your text is still \
             here, and Save tries again."
        ),
    }
}

/// Why a save was not even tried, or was refused by the store's own check: the person chooses
/// what happens to the two versions first.
pub fn blocked(outside: &OutsideState) -> &'static str {
    match outside {
        OutsideState::Deleted => {
            "Not saved: this note's file was moved or deleted. Save as new keeps your text as a new note."
        }
        _ => "Not saved: another program changed this note. Keep mine as a copy, Take theirs, or Compare them first.",
    }
}

/// What the outside bar says, or `None` when there is nothing for the person to choose. An
/// outside change to a note with nothing unsaved is not a choice: the note is read again.
pub fn outside(outside: &OutsideState) -> Option<&'static str> {
    match outside {
        OutsideState::Conflict { .. } => {
            Some("Another program changed this note while you were editing. Nothing has been overwritten.")
        }
        OutsideState::Deleted => Some("This note's file was moved or deleted outside Notes. Your text is still here."),
        OutsideState::Clean | OutsideState::Changed { .. } => None,
    }
}

/// A save landed, and replaced what another program did to the file while it was in flight.
pub fn replaced(what: Replaced) -> &'static str {
    match what {
        Replaced::OutsideEdit => {
            "Saved. Another program changed this note while it was being saved, and this save replaced that change."
        }
        Replaced::Deletion => {
            "Saved. This note's file was moved or deleted while it was being saved, and this save wrote it again."
        }
    }
}

/// A note with nothing unsaved, read again after another program changed it.
pub const UPDATED_FROM_DISK: &str = "Updated from the copy on disk.";
/// The same, when what it replaced was our own last write, kept first.
pub const UPDATED_KEEPING_OURS: &str =
    "Updated from the copy on disk; your last version is kept as a separate note with the same title.";
pub const TOOK_THEIRS: &str =
    "Now showing the version on disk. Your text was kept as a separate note with the same title.";
pub const KEPT_MINE: &str = "Your text is now a separate note. The original is left as the other program wrote it.";
pub const SAVED_AS_NEW: &str = "Your text is saved as a new note.";
/// Take theirs found no file: there was nothing to take, so the person's text is what is shown.
pub const NOTHING_TO_TAKE: &str =
    "The file was moved or deleted, so there was nothing to take. Your text is saved as a new note.";
/// Take theirs landed after more typing: the newer text is not replaced unasked.
pub const TYPED_MEANWHILE: &str = "You kept typing, so nothing was replaced. Your earlier text is also kept as \
     a separate note; choose again when you are ready.";

/// The window's text could not be kept as a note of its own, so nothing else was done.
pub fn not_kept(error: &str) -> String {
    format!(
        "Your text could not be kept as a separate note ({}), so this note was left as it is.",
        reason(error)
    )
}

/// The window's text is kept as a note, but the other version could not be read to replace it.
pub fn theirs_unread(error: &str) -> String {
    format!(
        "Your text was kept as a separate note, but the version on disk could not be read ({}), so \
         nothing was replaced.",
        reason(error)
    )
}

/// A note with nothing unsaved changed on disk and could not be read again.
pub fn reread_failed(error: &str) -> String {
    format!("This note changed on disk but could not be read again ({}); Refresh tries again.", reason(error))
}

/// Our own last write could not be kept as a note, so the note was not updated from the disk.
pub fn ours_not_kept(error: &str) -> String {
    format!(
        "This note changed on disk, but your last version could not be kept as a separate note ({}), \
         so it was not replaced. It stays here, unsaved: Keep mine as a copy, Take theirs, or Compare.",
        reason(error)
    )
}

/// The version on disk could not be read for a comparison.
pub fn compare_unread(error: &str) -> String {
    format!("The version on disk could not be read ({}), so there is nothing to compare.", reason(error))
}

/// Another program wrote the note's file in the instant before a save replaced it, and its version
/// is kept as this note.
pub fn theirs_kept(title: &str) -> String {
    format!("Saved. Another program wrote this note's file just before; its version is kept as a separate note, \"{title}\".")
}

/// The same, when a note in the library already holds exactly that version.
pub fn theirs_held(title: &str) -> String {
    format!("Saved. Another program wrote this note's file just before; its version is the same as \"{title}\", which is kept.")
}

/// The same, when its version could not be a note (too large, or not text a note may hold).
pub fn theirs_aside(path: &std::path::Path) -> String {
    format!("Saved. Another program wrote this note's file just before; its version is kept as a file at {}.", path.display())
}

/// The same, when its version could not be written anywhere: it is only in the library list.
pub fn theirs_unwritten(title: &str, why: &str) -> String {
    format!(
        "Saved. Another program wrote this note's file just before, and its version could not be written ({}). It is in the list as \"{title}\", not on disk yet: open it and save it.",
        reason(why)
    )
}

/// The same, when its version could not be kept at all.
pub fn theirs_lost(why: &str) -> String {
    format!(
        "Saved. Another program wrote this note's file just before, and its version could not be kept ({}).",
        reason(why)
    )
}

/// The start of the sentence a draft that could not be written is said with.
pub const DRAFT_NOT_KEPT_START: &str = "Notes could not keep a recovery copy of this note";

/// A draft could not be written: if Notes stops now, the unsaved text is lost.
pub fn draft_not_kept(why: &str) -> String {
    format!(
        "{DRAFT_NOT_KEPT_START} ({}). If Notes closes before it is saved, the unsaved text is lost; Save keeps it.",
        reason(why)
    )
}

/// How many notes have text offered back.
pub fn offered(count: usize) -> String {
    match count {
        1 => "1 note has unsaved text from before Notes closed: Restore or Discard it.".into(),
        n => format!("{n} notes have unsaved text from before Notes closed: Restore or Discard each."),
    }
}

/// The recovery bar, with the note it offers open and its unsaved text drawn read-only.
pub fn offer_open(title: &str) -> String {
    format!("Notes closed before \"{title}\" was saved. This is the unsaved text; Restore keeps it, Discard drops it.")
}

/// The recovery bar, for a note in the library that is not open.
pub fn offer_listed(title: &str) -> String {
    format!("Notes closed before \"{title}\" was saved. Show opens it with the unsaved text, to Restore or Discard it.")
}

/// The recovery bar, for a note whose file is gone, so there is nothing to open.
///
/// `characters` is the length of the text, or `None` where the window may not give a measure of
/// it (`mind_view`).
pub fn offer_gone(title: &str, characters: Option<usize>) -> String {
    let length = characters.map(|n| format!(" ({n} characters)")).unwrap_or_default();
    format!(
        "Notes closed before \"{title}\" was saved, and its file is gone. Restore keeps the unsaved text{length} \
         as a note; Discard drops it."
    )
}

/// Drafts found at start that could not be read, kept where they are.
pub fn drafts_unreadable(paths: &[std::path::PathBuf]) -> Option<String> {
    match paths {
        [] => None,
        [one] => Some(format!("A recovery copy could not be read; it is kept at {}.", one.display())),
        many => Some(format!(
            "{} recovery copies could not be read; they are kept in {}.",
            many.len(),
            many[0].parent().unwrap_or(&many[0]).display()
        )),
    }
}

/// Drafts of notes in another library, kept where they are.
pub fn drafts_elsewhere(paths: &[std::path::PathBuf]) -> Option<String> {
    match paths {
        [] => None,
        [one] => Some(format!("A recovery copy of a note in another library is kept at {}.", one.display())),
        many => Some(format!(
            "{} recovery copies of notes in another library are kept in {}.",
            many.len(),
            many[0].parent().unwrap_or(&many[0]).display()
        )),
    }
}

pub const NOTHING_OFFERED: &str = "There is no unsaved text from before Notes closed.";
pub const RESTORED: &str = "Your text is back in this note.";
/// Restored over a note that is not the one the text was written over (its text, or its
/// notebook, tags or pin): the outside bar decides.
pub const RESTORED_BESIDE: &str = "Your text is back. The note on disk changed after it was written (its text, \
     or its notebook, tags or pin), so choose what happens to the two versions.";
/// Restored for a note whose file is gone: there is one version, and the bar offers Save as new.
pub const RESTORED_GONE: &str = "Your text is back. The note's file is gone; Save as new keeps your text.";
pub const DISCARDED: &str = "The unsaved text was discarded; the note is as it is on disk.";
/// The same, for a note whose file is gone: there is no note on disk to be as it was.
pub const DISCARDED_GONE: &str = "The unsaved text was discarded. Its note's file was already gone, so nothing of it is kept.";
/// An edit refused on a note whose offer waits.
pub const CHOOSE_FIRST: &str =
    "This note has unsaved text from before Notes closed. Restore or Discard it first, then edit.";

/// The word on a library card for a note Notes made to keep text aside, so it is told apart from
/// the note it was kept from, whose title it has.
pub fn kept_marker(kept: Kept) -> &'static str {
    match kept {
        Kept::Copy => "Copy",
        Kept::Recovered => "Recovered",
    }
}

/// Typed while the note was moved to the Trash: kept, as a note of its own.
pub const TYPED_DURING_TRASH: &str = "Moved to Trash. What you typed meanwhile is kept as a new note.";

pub const WAIT_BEFORE_CLOSING: &str = "Please wait for the current operation before closing.";
/// A window's close that would ask, while another window's close prompt waits for its answer.
pub const ANSWER_THE_OTHER_FIRST: &str = "Answer the other window's question first.";
/// A close asked to save, and no save could start.
pub const NOT_SAVED_NOW: &str = "The note could not be saved now, so the window stays open.";

/// A close that asked about several notes, one of which did not save: which one, and why.
pub fn close_failed(title: &str, sentence: &str) -> String {
    format!("\"{title}\" was not saved, so Notes stays open. {sentence}")
}

/// A save failed of a note the person's window does not show: which note, and why.
pub fn note_failed(title: &str, sentence: &str) -> String {
    format!("\"{title}\": {sentence}")
}

/// *Open in new window* with no note open, and none named.
pub const NOTHING_FOR_A_WINDOW: &str = "No note is open to show in a new window.";
/// What a mind can do about it.
pub const NAME_A_NOTE: &str = "Name one with `title`, or open one first with `open_note`.";

/// A new window could not be made.
pub fn window_not_opened(why: &str) -> String {
    format!("A new window could not be opened ({}). The note is still open here.", reason(why))
}

/// A call's window closed before it could be answered.
pub const WINDOW_GONE: &str = "The window this call was about has closed.";

/// Undo or redo in a window whose note was changed in another window since its own last edit:
/// its history is dropped, never put back over the other window's text.
pub const UNDO_DROPPED: &str =
    "Nothing to undo in this window: the note was changed in another window since, so its history here was cleared.";

/// The close prompt: its heading and its sentence. `titles` are the notes with edits not on
/// disk; `unstored` counts the notes listed in the library that are not on disk at all (another
/// program's version that could not be written), which are kept as recovery copies.
pub fn close_prompt(titles: &[String], quitting: bool, read_only: bool, unstored: usize) -> (String, String) {
    let heading = match titles {
        [] if !quitting => "Close this window?".to_string(),
        [] => "Close Notes?".to_string(),
        [title] if !quitting => format!("Save \"{title}\" before closing?"),
        [title] => format!("Save \"{title}\" before Notes closes?"),
        many => format!("Save {} notes before Notes closes?", many.len()),
    };
    let mut body = if titles.is_empty() {
        String::new()
    } else if read_only {
        "This library is open read-only, so your changes cannot be saved here. Cancel, then Export, keeps a \
         copy of the text; Don't save closes without it."
            .to_string()
    } else if titles.len() > 1 {
        format!("Your changes to {} are not saved yet.", titles.join(", "))
    } else {
        "Your changes to this note are not saved yet.".to_string()
    };
    if unstored > 0 {
        let notes = if unstored == 1 { "1 note in the list is".to_string() } else { format!("{unstored} notes in the list are") };
        let more = format!(
            "{notes} not on disk yet. Notes keeps a recovery copy and offers it at the next start; Cancel, then open \
             and save it, to keep it now."
        );
        body = if body.is_empty() { more } else { format!("{body} {more}") };
    }
    (heading, body)
}

/// What the person is about to do when a note whose file is gone asks first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leaving {
    /// Open another note, make one, Refresh: the window moves off the note.
    Note,
    /// Close the window.
    Window,
    /// Quit Notes.
    Notes,
}

/// The prompt for one note whose file is gone: its text is on no disk, so leaving it asks. *Save
/// as new* keeps it as a new note; *Don't save* lets it go. `edited`: the person changed it, so
/// what is at stake is their changes, not only a text they may have meant to delete.
pub fn gone_prompt(title: &str, leaving: Leaving, edited: bool) -> (String, String) {
    let heading = match leaving {
        Leaving::Note => format!("Save \"{title}\" as a new note first?"),
        Leaving::Window => format!("Save \"{title}\" as a new note before closing?"),
        Leaving::Notes => format!("Save \"{title}\" as a new note before Notes closes?"),
    };
    let body = if edited {
        "Its file was moved or deleted outside Notes, and your changes to it are not saved. Save as new keeps \
         them as a new note; Don't save lets them go."
    } else {
        "Its file was moved or deleted outside Notes, so this text is on no disk. Save as new keeps it as a new \
         note; Don't save lets it go."
    };
    (heading, body.to_string())
}

/// Said beside a close prompt over several notes, for those of them whose file is gone.
pub fn gone_among(titles: &[String]) -> String {
    match titles {
        [one] => format!("\"{one}\" has no file any more: Save keeps it as a new note."),
        many => format!("{} have no file any more: Save keeps each as a new note.", many.join(", ")),
    }
}

/// *Let it go* (or *Discard*, when it was edited) on a note whose file is gone: said once.
pub fn let_go(title: &str, edited: bool) -> String {
    if edited {
        format!("\"{title}\" and your unsaved changes to it are let go; nothing was kept.")
    } else {
        format!("\"{title}\" is let go; nothing was kept.")
    }
}

/// The prompt's *Save*, and its *Save as new* for a note whose file is gone.
pub const SAVE: &str = "Save";
pub const SAVE_AS_NEW: &str = "Save as new";

/// An action that needs the store while it is busy.
pub const WAIT_FOR_THE_STORE: &str = "Please wait for the current operation.";

/// *Let it go* with no note whose file is gone open.
pub const NOTHING_TO_LET_GO: &str = "Nothing to let go: the open note's file is still there.";

/// A reason from the store or the disk, as the middle of a sentence: no closing stop, and a
/// leading capital lowered unless it starts an acronym ("Permission denied" reads "permission
/// denied"; "UTF-8" stays).
pub fn reason(sentence: &str) -> String {
    let trimmed = sentence.trim().trim_end_matches('.');
    let mut chars = trimmed.chars();
    match (chars.next(), chars.next()) {
        (Some(first), second) if first.is_uppercase() && !second.is_some_and(char::is_uppercase) => {
            first.to_lowercase().chain(trimmed[first.len_utf8()..].chars()).collect()
        }
        _ => trimmed.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::reason;

    #[test]
    fn a_reason_reads_as_the_middle_of_a_sentence() {
        assert_eq!(reason("Permission denied (os error 13)"), "permission denied (os error 13)");
        assert_eq!(
            reason("A note can contain up to 256 KiB and 10,000 lines."),
            "a note can contain up to 256 KiB and 10,000 lines"
        );
        assert_eq!(reason("UTF-8 only"), "UTF-8 only");
        assert_eq!(reason("injected failure at Write"), "injected failure at Write");
        assert_eq!(reason(""), "");
    }
}
