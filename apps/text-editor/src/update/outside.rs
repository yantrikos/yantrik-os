//! A tab's file changed outside the editor: never last-writer-wins.
//!
//! The framework's watch looks at every open tab's file and the document judges what it saw
//! (`Document::observe`): our own write is recognised and is nothing; another program's edit to a
//! tab with nothing unsaved is followed, the tab reading the file again with its old text one
//! *undo* away; an edit to a tab with unsaved changes is a conflict, said at once, and no save
//! writes over it (the save's own check refuses, as it always did); and a file gone is looked for
//! near where it was, followed when it moved (#86), and said when it was not found, its text
//! unsaved from then on so the close asks about it and its draft is kept.

use std::path::PathBuf;

use yantrik_app::documents::{Fingerprint, Observed, OutsideState, Source};
use yantrik_app::effects::Effects;

use super::find;
use crate::command::Command;
use crate::files::locate::locate;
use crate::files::open::{described, Opened};
use crate::files::read::read_text;
use crate::model::{Cue, Model, Work, HIDDEN_TAB};
use crate::tab::TabId;
use crate::view::render::TEXT;
use crate::TextEditor;

/// The watch saw the file under `key` change.
pub fn seen(m: &mut Model, fx: &mut Effects<TextEditor>, key: &str, seen: Observed) {
    let Some(doc) = m.documents.by_key(key) else { return };
    let Some(tab) = m.tab_by_doc(doc).cloned() else { return };
    let Some(d) = m.doc_mut(doc) else { return };
    let before = d.outside.clone();
    let after = d.observe(seen).clone();
    if let Some((key, path, seen)) = d.rewatch() {
        fx.watch(&key, path, seen);
    }
    let (seq, path) = (d.seq, d.source.path().map(PathBuf::from));
    let Some(path) = path else { return };
    match after {
        OutsideState::Changed { .. } => {
            let lost = Command::Reread { tab: tab.id, seq, result: Err("the file could not be read".into()) };
            fx.spawn_or(lost, move |_| Command::Reread {
                tab: tab.id,
                seq,
                result: read_text(&path).map(|text| described(path, text)),
            });
        }
        OutsideState::Conflict { .. } if !matches!(before, OutsideState::Conflict { .. }) => {
            let sentence = conflict(&m.title(&tab));
            m.notice.set_about(tab.hidden, sentence, || conflict(HIDDEN_TAB));
        }
        OutsideState::Deleted if before != OutsideState::Deleted => {
            let (on_disk, inode) = (tab.on_disk.clone().unwrap_or_default(), tab.inode);
            let from = path.clone();
            let lost = Command::Located { tab: tab.id, from: from.clone(), found: None };
            fx.spawn_or(lost, move |_| {
                let found = locate(&path, &on_disk, inode)
                    .and_then(|to| read_text(&to).ok().map(|text| described(to, text)));
                Command::Located { tab: tab.id, from, found }
            });
        }
        _ => {}
    }
}

/// What is said when another program changed a tab's file under unsaved changes.
pub fn conflict(title: &str) -> String {
    format!(
        "{title} was changed outside the editor while this tab has unsaved changes. Your draft is \
         intact; use Save As to keep both versions."
    )
}

/// A tab with nothing unsaved, read again after another program changed its file.
pub fn reread(m: &mut Model, fx: &mut Effects<TextEditor>, id: TabId, seq: u64, result: Result<Opened, String>) {
    let Some(tab) = m.tab(id).cloned() else { return };
    // A save of this tab went out meanwhile: its own check against the file decides, and its
    // landing is the one its document expects. The re-read is set aside, and the save's landing
    // takes the change up again (`after_save`), so it is never left unsaid.
    if m.busy == Some(Work::Save(id)) {
        m.reread_owed.insert(id);
        return;
    }
    let title = m.title(&tab);
    let edited = m.doc(&tab).seq != seq || m.modified(&tab);
    let Some(d) = m.doc_mut(tab.doc) else { return };
    // Typed in meanwhile, or the file is no longer a document: the person's text stays, and no save
    // writes over the other version.
    let opened = match result {
        Ok(opened) if !edited => opened,
        Ok(_) => {
            d.outside = OutsideState::Conflict { now: None };
            return m.notice.set_about(tab.hidden, conflict(&title), || conflict(HIDDEN_TAB));
        }
        Err(why) => {
            d.outside = OutsideState::Conflict { now: None };
            let unshown = |name: &str| {
                format!(
                    "{name} was changed outside the editor and cannot be shown here: {why} The tab keeps \
                     the text it had; Save As keeps it."
                )
            };
            return m.notice.set_about(tab.hidden, unshown(&title), || unshown(HIDDEN_TAB));
        }
    };
    let before = d.content.clone();
    d.take_theirs(opened.text.clone(), Fingerprint::of_bytes(opened.text.as_bytes()));
    if let Some((key, path, seen)) = d.watch_entry() {
        fx.watch(&key, path, seen);
    }
    if let Some(t) = m.tab_mut(id) {
        t.history.record(before);
        t.on_disk = Some(opened.text);
        t.hidden = opened.hidden;
        t.inode = opened.inode;
        t.unsure = None;
    }
    let followed = |name: &str| {
        format!("{name} was changed outside the editor; the tab now shows the file as it is, and Undo goes back.")
    };
    m.notice.set_about(tab.hidden || opened.hidden, followed(&title), || followed(HIDDEN_TAB));
    if m.active_tab().id == id {
        m.cues.push(Cue::ResetContent);
        find::search(m, false);
        fx.changed(TEXT);
    }
}

/// A save of tab `id` landed, `saved` or not, while a re-read of it was set aside. A save that
/// landed checked the file held what the tab expected, so nothing is owed. One that did not leaves
/// the outside change as it was: taken up again now, as when it was first seen, a conflict for a
/// tab with anything unsaved and a re-read for one without. The watch is also re-armed from the
/// baseline, so its next look reads the file afresh whatever happens here.
pub fn after_save(m: &mut Model, fx: &mut Effects<TextEditor>, id: TabId, saved: bool) {
    if !m.reread_owed.remove(&id) || saved {
        return;
    }
    let Some(tab) = m.tab(id).cloned() else { return };
    let modified = m.modified(&tab);
    let title = m.title(&tab);
    let Some(d) = m.doc_mut(tab.doc) else { return };
    if !matches!(d.outside, OutsideState::Changed { .. }) {
        return;
    }
    if let Some((key, path, seen)) = d.watch_entry() {
        fx.watch(&key, path, seen);
    }
    let (seq, path) = (d.seq, d.source.path().map(PathBuf::from));
    if modified {
        d.outside = OutsideState::Conflict { now: None };
        let failed = (m.notice.window().to_string(), m.notice.read(true).to_string());
        m.notice.set_about(tab.hidden, format!("{} {}", failed.0, conflict(&title)), || {
            format!("{} {}", failed.1, conflict(HIDDEN_TAB))
        });
        return;
    }
    let Some(path) = path else { return };
    let lost = Command::Reread { tab: id, seq, result: Err("the file could not be read".into()) };
    fx.spawn_or(lost, move |_| Command::Reread { tab: id, seq, result: read_text(&path).map(|text| described(path, text)) });
}

/// Where a tab's file went, after the watch found it gone.
pub fn located(m: &mut Model, fx: &mut Effects<TextEditor>, id: TabId, from: PathBuf, found: Option<Opened>) {
    let Some(tab) = m.tab(id).cloned() else { return };
    let Some(d) = m.doc_mut(tab.doc) else { return };
    // Saved elsewhere, or the file came back, meanwhile: nothing to follow.
    if d.source.path() != Some(from.as_path()) || d.outside != OutsideState::Deleted {
        return;
    }
    let gone = |name: &str| {
        format!(
            "{name} is not there any more, and nothing with the same name and the same contents was found \
             near it. Your draft is untouched — Save As to give it a file again."
        )
    };
    let Some(opened) = found else {
        return m.notice.set_about(tab.hidden, gone(&from.display().to_string()), || gone(HIDDEN_TAB));
    };
    // The same file, carried somewhere else: the tab follows it. Only the path changes; the text
    // and the baseline are untouched, because a move keeps the same bytes.
    fx.unwatch(&d.source.key());
    d.source = Source::File(opened.path.clone());
    d.outside = OutsideState::Clean;
    if let Some((key, path, seen)) = d.watch_entry() {
        fx.watch(&key, path, seen);
    }
    if let Some(t) = m.tab_mut(id) {
        t.hidden = opened.hidden;
        t.inode = opened.inode;
    }
    let moved = format!("This file moved. It is {} now.", opened.path.display());
    m.notice.set_about(tab.hidden || opened.hidden, moved, || "A file an agent is not shown moved; its tab follows it.".into());
}
