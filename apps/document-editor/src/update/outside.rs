//! The document's file changed outside yDoc: never last-writer-wins.
//!
//! The framework's watch looks at the open document's file and the document judges what it saw
//! (`Document::observe`): our own write is recognised and is nothing; another program's edit to a
//! document with nothing unsaved is followed, the document reading the file again; an edit to a
//! document with unsaved changes is a conflict, said at once, and no save writes over it (the
//! save's own check refuses, as it always did); and a file gone is looked for near where it was,
//! followed when it moved (#86), and said when it was not found, the text unsaved from then on so
//! the close asks about it and its draft is kept.

use std::path::PathBuf;

use yantrik_app::documents::{Fingerprint, Observed, OutsideState, Source};
use yantrik_app::effects::Effects;

use super::find;
use crate::agent_rule::HIDDEN;
use crate::command::Command;
use crate::files::locate::locate;
use crate::files::open::{described, Opened};
use crate::files::read::read_text;
use crate::model::{Cue, Model, Work};
use crate::view::render::TEXT;
use crate::YDoc;

/// The watch saw the file under `key` change.
pub fn seen(m: &mut Model, fx: &mut Effects<YDoc>, key: &str, seen: Observed) {
    if m.documents.by_key(key) != Some(m.doc.id) {
        return;
    }
    let hidden = m.doc.hidden;
    let title = m.title();
    let (on_disk, inode) = (m.doc.on_disk.clone().unwrap_or_default(), m.doc.inode);
    let d = m.d_mut();
    let before = d.outside.clone();
    let after = d.observe(seen).clone();
    if let Some((key, path, seen)) = d.rewatch() {
        fx.watch(&key, path, seen);
    }
    let Some(path) = d.source.path().map(PathBuf::from) else { return };
    match after {
        OutsideState::Changed { .. } => {
            let lost = Command::Reread { from: path.clone(), result: Err("the file could not be read".into()) };
            fx.spawn_or(lost, move |_| Command::Reread { from: path.clone(), result: read_text(&path).map(|text| described(path, text)) });
        }
        OutsideState::Conflict { .. } if !matches!(before, OutsideState::Conflict { .. }) => {
            m.notice.set_about(hidden, conflict(&title), || conflict(HIDDEN));
        }
        OutsideState::Deleted if before != OutsideState::Deleted => {
            let from = path.clone();
            let lost = Command::Located { from: from.clone(), found: None };
            fx.spawn_or(lost, move |_| {
                let found = locate(&path, &on_disk, inode).and_then(|to| read_text(&to).ok().map(|text| described(to, text)));
                Command::Located { from, found }
            });
        }
        _ => {}
    }
}

/// What is said when another program changed the file under unsaved changes.
pub fn conflict(title: &str) -> String {
    format!(
        "{title} was changed outside yDoc while this document has unsaved changes. Your draft is intact; use Save As \
         to keep both versions."
    )
}

/// The document, with nothing unsaved, read again after another program changed its file.
pub fn reread(m: &mut Model, fx: &mut Effects<YDoc>, from: PathBuf, result: Result<Opened, String>) {
    // Read for a file that is no longer the one open (another document opened before it landed):
    // its text is not this document's, and labelling it so would show a clean document that is not.
    if m.path() != Some(from.as_path()) {
        return;
    }
    // A save went out meanwhile: its own check against the file decides, and its landing is the
    // one the document expects. The re-read is set aside, and the save's landing takes the change
    // up again (`after_save`), so it is never left unsaid.
    if m.busy == Some(Work::Save) {
        m.reread_owed = true;
        return;
    }
    let (title, hidden) = (m.title(), m.doc.hidden);
    // Unsaved text is what a re-read must not replace. How many edits the document has had since
    // is not: an earlier re-read that landed first, or text typed and typed back, leaves nothing
    // here to lose, and counting those made a clean document a false conflict.
    let edited = m.text_unsaved();
    // Typed in meanwhile, or the file is no longer a document: the person's text stays, and no
    // save writes over the other version.
    let opened = match result {
        Ok(opened) if !edited => opened,
        Ok(_) => {
            m.d_mut().outside = OutsideState::Conflict { now: None };
            return m.notice.set_about(hidden, conflict(&title), || conflict(HIDDEN));
        }
        Err(why) => {
            m.d_mut().outside = OutsideState::Conflict { now: None };
            let unshown = |name: &str| {
                format!(
                    "{name} was changed outside yDoc and cannot be shown here: {why} The document keeps the text it \
                     had; Save As keeps it."
                )
            };
            return m.notice.set_about(hidden, unshown(&title), || unshown(HIDDEN));
        }
    };
    let d = m.d_mut();
    d.take_theirs(opened.text.clone(), Fingerprint::of_bytes(opened.text.as_bytes()));
    if let Some((key, path, seen)) = d.watch_entry() {
        fx.watch(&key, path, seen);
    }
    m.doc.on_disk = Some(opened.text);
    m.doc.hidden = opened.hidden;
    m.doc.inode = opened.inode;
    m.doc.unsure = None;
    let followed = |name: &str| format!("{name} was changed outside yDoc; the document now shows the file as it is.");
    m.notice.set_about(hidden || opened.hidden, followed(&title), || followed(HIDDEN));
    m.cues.push(Cue::ResetContent);
    find::search(m, false);
    fx.changed(TEXT);
}

/// A save landed, `saved` or not, while a re-read was set aside. A save that landed checked the
/// file held what the document expected, so nothing is owed. One that did not leaves the outside
/// change as it was: taken up again now, as when it was first seen, a conflict for a document with
/// anything unsaved and a re-read for one without. The watch is also re-armed from the baseline,
/// so its next look reads the file afresh whatever happens here.
pub fn after_save(m: &mut Model, fx: &mut Effects<YDoc>, saved: bool) {
    if !std::mem::take(&mut m.reread_owed) || saved {
        return;
    }
    let (modified, title, hidden) = (m.text_unsaved(), m.title(), m.doc.hidden);
    let d = m.d_mut();
    // A save refused for the change the re-read was parked for left a conflict behind it. For a
    // document with nothing unsaved that is the change itself, still owed a re-read.
    let owed = match d.outside {
        OutsideState::Changed { .. } => true,
        OutsideState::Conflict { .. } => !modified,
        _ => false,
    };
    if !owed {
        return;
    }
    if let Some((key, path, seen)) = d.watch_entry() {
        fx.watch(&key, path, seen);
    }
    let path = d.source.path().map(PathBuf::from);
    if modified {
        d.outside = OutsideState::Conflict { now: None };
        let failed = (m.notice.window().to_string(), m.notice.read(true).to_string());
        m.notice.set_about(hidden, format!("{} {}", failed.0, conflict(&title)), || {
            format!("{} {}", failed.1, conflict(HIDDEN))
        });
        return;
    }
    let Some(path) = path else { return };
    let lost = Command::Reread { from: path.clone(), result: Err("the file could not be read".into()) };
    fx.spawn_or(lost, move |_| Command::Reread { from: path.clone(), result: read_text(&path).map(|text| described(path, text)) });
}

/// Where the document's file went, after the watch found it gone.
pub fn located(m: &mut Model, fx: &mut Effects<YDoc>, from: PathBuf, found: Option<Opened>) {
    let hidden = m.doc.hidden;
    let d = m.d_mut();
    // Saved elsewhere, or the file came back, meanwhile: nothing to follow.
    if d.source.path() != Some(from.as_path()) || d.outside != OutsideState::Deleted {
        return;
    }
    let gone = |name: &str| {
        format!(
            "{name} is not there any more, and nothing with the same name and the same contents was found near it. \
             Your draft is untouched \u{2014} Save As to give it a file again."
        )
    };
    let Some(opened) = found else {
        return m.notice.set_about(hidden, gone(&from.display().to_string()), || gone(HIDDEN));
    };
    // The same file, carried somewhere else: the document follows it. Only the path changes; the
    // text and the baseline are untouched, because a move keeps the same bytes.
    fx.unwatch(&d.source.key());
    d.source = Source::File(opened.path.clone());
    d.outside = OutsideState::Clean;
    if let Some((key, path, seen)) = d.watch_entry() {
        fx.watch(&key, path, seen);
    }
    m.doc.hidden = opened.hidden;
    m.doc.inode = opened.inode;
    let moved = format!("This document moved. It is {} now.", opened.path.display());
    m.notice.set_about(hidden || opened.hidden, moved, || "A document an agent is not shown moved; it follows it.".into());
}
