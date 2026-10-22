//! The store's other jobs: reading the library, the Trash, an import and an export, and what the
//! window does when each lands.
//!
//! The editor takes keys while a job runs, so the open note may have text typed meanwhile when a
//! reload, an import or a restore lands. That text is never closed away: the note stays open and
//! unsaved, its autosave owed, and the window says what the job could not do because of it.

use std::path::{Path, PathBuf};

use yantrik_app::app::WindowId;
use yantrik_app::effects::Effects;
use yantrik_ipc_contracts::home_paths;

use super::navigate::show_note;
use super::saving::{self, How};
use super::{
    after_store, begin, drafts, failed, recovery, refreshed, relist, said, set_status, show, windows, WORKER_STOPPED,
};
use crate::command::Command;
use crate::model::{Model, Work};
use crate::note::{Kept, Note};
use crate::store::{Loaded, MAX_NOTES, VAULT_LIMIT};
use crate::view::render::{CHROME, STATUS};
use crate::Notes;

pub fn load(m: &mut Model, fx: &mut Effects<Notes>) {
    begin(m, fx, Work::Load, Command::Loaded(Err(WORKER_STOPPED.into())), |vault| {
        Command::Loaded(vault.load())
    });
}

/// Said when a reload found the open note's file changed while text typed meanwhile was not saved.
const KEPT_OVER_CHANGE: &str =
    "The open note changed on disk while you typed in it. What you typed is kept, unsaved; the note was not read again.";

/// A window's note before a reload: the window, what its note was opened from, and whether it
/// holds text not on disk.
struct Before {
    window: WindowId,
    key: Option<(String, bool)>,
    kept: bool,
}

/// The library as read. Each window's note stays open: read afresh if its file is still there, or,
/// if it has text typed while the library was read, kept as it is.
pub fn loaded(m: &mut Model, fx: &mut Effects<Notes>, result: Result<Loaded, String>) {
    m.busy = None;
    match result {
        Ok(loaded) => {
            let was_read_only = m.vault.read_only().is_some();
            m.vault = m.vault.with_access(loaded.read_only.clone());
            let before: Vec<Before> = m
                .panes()
                .map(|p| {
                    let c = m.current_in(p);
                    let key = c.as_ref().map(|c| (c.id().to_string(), c.trash()));
                    Before { window: p.window, key, kept: c.is_some_and(|c| c.unsaved()) }
                })
                .collect();
            m.library = loaded.notes;
            let again = |m: &Model, key: &Option<(String, bool)>| {
                let (id, trash) = key.as_ref()?;
                m.library.iter().find(|n| n.id == *id && n.trash == *trash).cloned()
            };
            // The file is no longer what the note was read from, so reading it again would have
            // shown something else. What that change means is the outside-change flow's to say;
            // here it is only never read over the text.
            let changed = before.iter().filter(|b| b.kept).any(|b| {
                let c = m.pane_of(b.window).and_then(|p| m.current_in(p));
                c.is_some_and(|c| again(m, &b.key).as_ref().map(|n| &n.content) != c.file.on_disk.as_ref())
            });
            // Closed first in every window that reads its note afresh, so a note read again is a
            // fresh document and not the old one, even when two windows show it.
            for b in before.iter().filter(|b| !b.kept) {
                m.within(b.window, |m| show_note(m, fx, None));
            }
            for b in before.iter().filter(|b| !b.kept) {
                let note = again(m, &b.key);
                m.within(b.window, |m| show_note(m, fx, note));
            }
            m.ready = true;
            // What the last run kept and did not save is weighed once, against the library as
            // first read.
            let offered = recovery::weigh(m, fx, loaded.interrupted.as_deref());
            m.notice = sentences([
                loaded.read_only.as_deref(),
                Some(loaded.notice.as_str()),
                changed.then_some(KEPT_OVER_CHANGE),
                offered.as_deref(),
            ]);
            let writable_again = was_read_only && m.vault.read_only().is_none();
            for b in &before {
                m.within(b.window, |m| {
                    if !b.kept {
                        return show(m, fx);
                    }
                    still_typing(m, fx);
                    // Kept while the vault refused writes: its save may have been refused already,
                    // and the autosave does not try a failed save again until something changes.
                    // Saved now, as the person's own save would be, so no further key is needed.
                    // The store does one thing at a time, so this starts the first window's; the
                    // others' failed saves follow as each lands (saving::retry_failed).
                    if writable_again {
                        saving::save(m, fx, How::Asked);
                    }
                });
            }
            after_store(m, fx);
        }
        Err(e) => failed(m, fx, None, e),
    }
}

pub fn trash(m: &mut Model, fx: &mut Effects<Notes>) {
    let Some((id, on_disk, modified)) = m
        .current()
        .filter(|c| !c.trash())
        .map(|c| (c.id().to_string(), c.file.on_disk.clone().unwrap_or_else(|| c.content().clone()), c.file.modified))
    else {
        return;
    };
    let lost = Command::Trashed { id: id.clone(), result: Err(WORKER_STOPPED.into()) };
    begin(m, fx, Work::Trash, lost, move |vault| {
        let result = vault.trash(&id, &on_disk, modified);
        Command::Trashed { id, result }
    });
}

pub fn trashed(m: &mut Model, fx: &mut Effects<Notes>, id: &str, result: Result<(), String>) {
    m.busy = None;
    match result {
        Ok(()) => {
            if let Some(n) = m.library.iter_mut().find(|n| n.id == id && !n.trash) {
                n.trash = true;
            }
            // The Trash holds the note as its file held it. Text typed while the job ran is in
            // the window only: it becomes a note of its own, saved at once, never closed away.
            let typed = m.current().filter(|c| c.id() == id && c.unsaved()).map(|c| c.content().clone());
            let doc = m.current_id();
            // Its text moves to the new note, whose own draft takes over.
            let from = doc.and_then(|doc| m.documents.get(doc)).and_then(drafts::kept_name);
            show_note(m, fx, None);
            // The note is in the Trash for every window that showed it.
            if let Some(doc) = doc {
                windows::follow_to(m, fx, doc, None);
            }
            match typed {
                Some(content) => {
                    let kept = Note { id: Kept::Recovered.new_id(), content, ..Note::default() };
                    m.library.push(kept.clone());
                    show_note(m, fx, Some(kept));
                    drafts::moved(m, fx, from);
                    show(m, fx);
                    // Said until the next action: the save that lands next must not take it back.
                    m.notice = said::TYPED_DURING_TRASH.into();
                    m.lasting_notice = true;
                    saving::save(m, fx, How::Asked);
                }
                None => {
                    show(m, fx);
                    m.notice = "Moved to Trash. You can restore it from the library.".into();
                }
            }
            after_store(m, fx);
        }
        Err(e) => failed(m, fx, None, e),
    }
}

pub fn restore(m: &mut Model, fx: &mut Effects<Notes>) {
    let Some(note) = m.current().filter(|c| c.trash()).map(|c| c.as_note()) else { return };
    begin(m, fx, Work::Restore, Command::Restored(Err(WORKER_STOPPED.into())), move |vault| {
        Command::Restored(vault.restore(&note))
    });
}

/// An agent names only a file in the person's home, outside its protected places (#443, as the
/// Text Editor's `open`): read as a note, any file the person can read was `describe`'s 4000
/// characters, the snippets store and every Notes draft included (security review of #942,
/// finding 1). Asked in the job, before the file is looked at; the person names any file.
fn agents_file(agent: bool, path: &Path, rule: fn(&str, &Path) -> Result<(), String>) -> Result<(), String> {
    if !agent {
        return Ok(());
    }
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    rule(&path.to_string_lossy(), &home).map_err(|why| format!("an agent names only a file in the person's home, outside its protected places: {why}"))
}

pub fn import(m: &mut Model, fx: &mut Effects<Notes>, path: PathBuf, agent: bool) {
    if m.library.len() >= MAX_NOTES {
        m.notice = "The library supports up to 2,000 notes.".into();
        fx.changed(STATUS);
        return;
    }
    let room = VAULT_LIMIT.saturating_sub(m.library_text());
    begin(m, fx, Work::Import, Command::Imported(Err(WORKER_STOPPED.into())), move |vault| {
        Command::Imported(agents_file(agent, &path, home_paths::may_read_file).and_then(|()| vault.import(&path, room)))
    });
}

pub fn export(m: &mut Model, fx: &mut Effects<Notes>, path: PathBuf, agent: bool) {
    // What the window shows: an offered text while it waits.
    let text = m.shown().map(|c| c.text.clone()).unwrap_or_default();
    begin(m, fx, Work::Export, Command::Exported(Err(WORKER_STOPPED.into())), move |vault| {
        Command::Exported(agents_file(agent, &path, home_paths::may_write_file).and_then(|()| vault.export(&path, &text)))
    });
}

/// Which job a note came back from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opened {
    /// The Trash: the note every window showed there is the restored one now.
    Restored,
    Imported,
}

/// A note restored from the Trash, or imported: in the library, open, and the whole library
/// shown again. If the open note has text typed while the job ran, it stays open instead, and the
/// new note waits in the list.
pub fn opened(m: &mut Model, fx: &mut Effects<Notes>, result: Result<Note, String>, how: Opened) {
    m.busy = None;
    match result {
        Ok(note) => {
            m.library.retain(|a| a.id != note.id);
            m.library.push(note.clone());
            let kept = m.unsaved();
            let title = note.title();
            if !kept {
                let from = m.current_id();
                show_note(m, fx, Some(note.clone()));
                if let (Opened::Restored, Some(doc)) = (how, from) {
                    windows::follow_to(m, fx, doc, Some(&note));
                }
            }
            m.pane.folder = "all".into();
            m.pane.query.clear();
            m.pane.dialog = 0;
            m.notice.clear();
            fx.changed(CHROME);
            landed(m);
            if kept {
                // A call waiting on it is answered with this too, not with the note left open.
                let sentence = not_opened(&title);
                if let Some(asked) = m.asked.as_mut() {
                    asked.refused = Some(sentence.clone());
                }
                m.notice = sentence;
                still_typing(m, fx);
            } else {
                show(m, fx);
            }
            after_store(m, fx);
        }
        Err(e) => failed(m, fx, None, e),
    }
}

/// Said when an imported or restored note was not opened over text typed meanwhile.
fn not_opened(title: &str) -> String {
    format!("\"{title}\" is in the library but was not opened: the open note has unsaved text typed meanwhile, so it stays open.")
}

/// The open note kept, with text typed while a job ran: its history and place left alone (it is
/// not shown afresh), the list redrawn round it, and its autosave armed again, since a debounce
/// that ran out while the store was busy saved nothing.
fn still_typing(m: &mut Model, fx: &mut Effects<Notes>) {
    relist(m, fx);
    refreshed(m, fx);
}

/// The sentences that say something, as one line.
fn sentences<'a>(parts: impl IntoIterator<Item = Option<&'a str>>) -> String {
    parts.into_iter().flatten().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
}

pub fn exported(m: &mut Model, fx: &mut Effects<Notes>, result: Result<(), String>) {
    m.busy = None;
    match result {
        Ok(()) => {
            m.pane.dialog = 0;
            m.notice = "Exported to the requested file.".into();
            set_status(m, "All changes saved");
            fx.changed(CHROME);
            fx.changed(STATUS);
            landed(m);
            after_store(m, fx);
        }
        Err(e) => failed(m, fx, None, e),
    }
}

/// The import or export a call from the surface waits on has landed.
fn landed(m: &mut Model) {
    if let Some(asked) = m.asked.as_mut() {
        asked.landed = true;
    }
}
