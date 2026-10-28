//! The sheet's file changed outside ySheets: never last-writer-wins.
//!
//! The framework's watch looks at the open sheet's file and the document judges what it saw
//! (`Document::observe`): our own write is recognised and is nothing; another program's edit to a
//! sheet with nothing unsaved is followed, the sheet reading the file again; an edit to a sheet
//! with unsaved changes is a conflict, said at once, and no save writes over it (the save's own
//! check refuses); and a file gone is looked for near where it was, followed when it moved (#86),
//! and said when it was not found, the content unsaved from then on so its draft is kept.
//!
//! The file is read again, and the sheet it makes worked out, in a job.

use std::path::PathBuf;

use yantrik_app::documents::{Observed, OutsideState, Source};
use yantrik_app::effects::Effects;

use super::cells;
use crate::agent_rule::HIDDEN;
use crate::command::Command;
use crate::files::open::{read_sheet, OpenedSheet};
use crate::files::read::locate;
use crate::model::{Cue, Model, Redraw, Work};
use crate::YSheets;

/// The watch saw the file under `key` change.
pub fn seen(m: &mut Model, fx: &mut Effects<YSheets>, key: &str, seen: Observed) {
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
        OutsideState::Changed { .. } => spawn_reread(fx, path),
        OutsideState::Conflict { .. } if !matches!(before, OutsideState::Conflict { .. }) => {
            m.notice.set_about(hidden, conflict(&title), || conflict(HIDDEN));
        }
        OutsideState::Deleted if before != OutsideState::Deleted => {
            let from = path.clone();
            let lost = Command::Located { from: from.clone(), found: None };
            fx.spawn_or(lost, move |_| {
                let found = locate(&path, &on_disk, inode).and_then(|to| read_sheet(&to).ok().map(Box::new));
                Command::Located { from, found }
            });
        }
        _ => {}
    }
}

/// The sheet's file read again, in a job.
fn spawn_reread(fx: &mut Effects<YSheets>, path: PathBuf) {
    let lost = Command::Reread { from: path.clone(), result: Err("the file could not be read".into()) };
    fx.spawn_or(lost, move |_| Command::Reread { from: path.clone(), result: read_sheet(&path).map(Box::new) });
}

/// What is said when another program changed the file under unsaved changes.
pub fn conflict(title: &str) -> String {
    format!(
        "{title} was changed outside ySheets while this sheet has unsaved changes. Your changes are intact and \
         nothing was written over the other version; `save` with a path (or Save As in the window) keeps yours in a file of its own."
    )
}

/// The sheet, with nothing unsaved, read again after another program changed its file.
pub fn reread(m: &mut Model, fx: &mut Effects<YSheets>, from: PathBuf, result: Result<Box<OpenedSheet>, String>) {
    // Read for a file that is no longer the one open (another sheet opened before it landed): its
    // content is not this sheet's, and labelling it so would show a clean sheet that is not.
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
    // Unsaved content is what a re-read must not replace. How many edits the sheet has had since
    // is not: content typed and typed back leaves nothing here to lose, and counting those made a
    // clean sheet a false conflict.
    let edited = m.content_unsaved();
    // Typed in meanwhile, or the file is no longer a sheet: the person's content stays, and no
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
                    "{name} was changed outside ySheets and cannot be shown here: {why} The sheet keeps the content \
                     it had; Save keeps it."
                )
            };
            return m.notice.set_about(hidden, unshown(&title), || unshown(HIDDEN));
        }
    };
    let OpenedSheet { file, sheet, baseline, .. } = *opened;
    let mut book = m.book().clone();
    book.followed(sheet);
    let d = m.d_mut();
    d.take_theirs(book, baseline);
    if let Some((key, path, seen)) = d.watch_entry() {
        fx.watch(&key, path, seen);
    }
    let revision = m.book().revision();
    m.doc.seen_revision = revision;
    m.doc.on_disk = Some(file.text);
    m.doc.hidden = file.hidden;
    m.doc.inode = file.inode;
    m.doc.unsure = None;
    let followed = |name: &str| format!("{name} was changed outside ySheets; the sheet now shows the file as it is.");
    m.notice.set_about(hidden || file.hidden, followed(&title), || followed(HIDDEN));
    // The grid is theirs: drawn whole, the selection kept inside it, the formula bar shown again.
    m.redraw = Redraw::All;
    m.watch = None;
    cells::select(m, m.active);
    m.cues.push(Cue::LoadField);
}

/// A save landed, `saved` or not, while a re-read was set aside. A save that landed checked the
/// file held what the sheet expected, so nothing is owed. One that did not leaves the outside
/// change as it was: taken up again now, as when it was first seen, a conflict for a sheet with
/// anything unsaved and a re-read for one without. The watch is also re-armed from the baseline,
/// so its next look reads the file afresh whatever happens here.
pub fn after_save(m: &mut Model, fx: &mut Effects<YSheets>, saved: bool) {
    if !std::mem::take(&mut m.reread_owed) || saved {
        return;
    }
    let (modified, title, hidden) = (m.content_unsaved(), m.title(), m.doc.hidden);
    let d = m.d_mut();
    // A save refused for the change the re-read was parked for left a conflict behind it. For a
    // sheet with nothing unsaved that is the change itself, still owed a re-read.
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
    if let Some(path) = path {
        spawn_reread(fx, path);
    }
}

/// Where the sheet's file went, after the watch found it gone.
pub fn located(m: &mut Model, fx: &mut Effects<YSheets>, from: PathBuf, found: Option<Box<OpenedSheet>>) {
    let hidden = m.doc.hidden;
    let d = m.d_mut();
    // Saved elsewhere, or the file came back, meanwhile: nothing to follow.
    if d.source.path() != Some(from.as_path()) || d.outside != OutsideState::Deleted {
        return;
    }
    let gone = |name: &str| {
        format!(
            "{name} is not there any more, and nothing with the same name and the same contents was found near it. \
             Your changes are untouched \u{2014} `save` with a path (or Save As in the window) keeps your sheet in a new file."
        )
    };
    let Some(found) = found else {
        return m.notice.set_about(hidden, gone(&from.display().to_string()), || gone(HIDDEN));
    };
    let opened = found.file;
    // The same file, carried somewhere else: the sheet follows it. Only the path changes; the
    // content and the baseline are untouched, because a move keeps the same bytes.
    fx.unwatch(&d.source.key());
    d.source = Source::File(opened.path.clone());
    d.outside = OutsideState::Clean;
    if let Some((key, path, seen)) = d.watch_entry() {
        fx.watch(&key, path, seen);
    }
    m.doc.hidden = opened.hidden;
    m.doc.inode = opened.inode;
    let moved = format!("This sheet moved. It is {} now.", opened.path.display());
    m.notice.set_about(hidden || opened.hidden, moved, || "A sheet an agent is not shown moved; it follows it.".into());
}
