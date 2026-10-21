//! What the Mind reads: `describe`, and the answers the surface's calls give. Projections of the
//! committed model; the same keys, values and sentences Notes has always given.

use serde_json::{json, Value};
use yantrik_app::control::View;
use yantrik_app::documents::{DraftState, Durability, OutsideState, SaveState};

use crate::model::Model;
use crate::note::Note;
use crate::pane::Pane;
use crate::store::{read, LIMIT};

/// The one line a mind reads first, and the state behind it.
///
/// The summary used to be the word "Notes". The `os_apps` listing prints one summary per open
/// window, so that line was the whole of what a mind knew about this app before deciding to look
/// closer: it says whether anything is open, which note, and whether it is saved.
pub fn view(m: &Model) -> View {
    let notes = m.library.iter().filter(|n| !n.trash).count();
    let trashed = m.library.iter().filter(|n| n.trash).count();
    let library = format!(
        "{notes} note{}{}",
        if notes == 1 { "" } else { "s" },
        if trashed == 0 { String::new() } else { format!(", {trashed} in trash") }
    );
    let open = m.current();
    // While its unsaved text from before Notes closed is offered, the window shows that text, not
    // yet in the note and on no disk but its draft: so does every key below about the text.
    let offered = m.offered().is_some();
    let shown = m.shown();
    let mut summary = match &open {
        _ if !m.ready => "Notes — the library has not opened".to_string(),
        None => format!("Notes — nothing open, {library}"),
        Some(n) => {
            let lines = shown.map_or(0, |s| s.text.lines().count());
            // What waits on a choice about an outside change is said in the line a mind reads first.
            let waiting = match n.outside() {
                OutsideState::Conflict { .. } => ", changed outside Notes",
                OutsideState::Deleted => ", its file was moved or deleted",
                OutsideState::Clean | OutsideState::Changed { .. } => "",
            };
            format!(
                "Notes — \"{}\" open{} ({} line{}, {}{waiting}), {library}",
                shown.map(|s| s.title()).unwrap_or_else(|| n.title()),
                if n.trash() { " from Trash" } else { "" },
                lines,
                if lines == 1 { "" } else { "s" },
                if n.trash() {
                    "deleted"
                } else if n.unsaved() || offered {
                    "unsaved"
                } else {
                    "saved"
                }
            )
        }
    };
    if m.busy.is_some() {
        summary.push_str(" · working");
    }
    if !m.notice.is_empty() {
        summary.push_str(&format!(" · {}", m.notice));
    }
    View::new(summary)
        .with("open_note", open.as_ref().map(|n| n.id().to_string()))
        .with("title", shown.map(|s| s.title()))
        .with("path", open.as_ref().map(|n| m.vault.dir().join(n.id()).display().to_string()))
        .with("unsaved", open.as_ref().is_some_and(|n| n.unsaved()) || offered)
        .with("lines", shown.map(|s| s.text.lines().count() as i64))
        .with("characters", shown.map(|s| s.text.chars().count() as i64))
        .with("trashed_note_open", open.as_ref().is_some_and(|n| n.trash()))
        .with("outside", open.as_ref().map(|n| outside(n.outside())))
        .with("save_failure", open.as_ref().and_then(|n| save_failure(&n.doc.save)))
        // An offered text is kept, as the draft it was read from.
        .with("draft", open.as_ref().map(|n| if offered { "kept" } else { draft(&n.doc.draft) }))
        .with(
            "recovery",
            crate::update::recovery::in_order(m)
                .into_iter()
                .map(|o| json!({ "filename": o.id, "title": o.content.title(), "characters": o.content.text.chars().count() }))
                .collect::<Vec<_>>(),
        )
        .with("content", shown.map(|s| s.text.chars().take(4000).collect::<String>()))
        // The `.meta` sidecar travels with the text — `tags`, `notebook` and `pinned` live in it —
        // so it belongs in the view a revision is hashed from: a caller whose "unchanged since I
        // read it" check compares revisions must see a tag change as a change.
        .with("metadata", shown.map(|s| s.meta.clone()))
        .with("notes_directory", m.vault.dir().display().to_string())
        .with("library_ready", m.ready)
        .with("busy", m.busy.is_some())
        .with("notice", m.notice.clone())
        .with("status", m.pane.status.clone())
        .with("folder", m.pane.folder.clone())
        .with("preview", m.pane.preview)
        .with("focus", m.pane.focus_mode)
        .with("note_count", notes as i64)
        .with("trash_count", trashed as i64)
        .with("search_query", m.pane.query.clone())
        .with("matches", m.pane.listing.rows.len() as i64)
        .with(
            "notes",
            m.library
                .iter()
                .take(100)
                .map(|n| {
                    json!({
                        "filename": n.id,
                        "title": n.title(),
                        "trash": n.trash,
                        "pinned": n.content.pinned(),
                        "notebook": n.field("notebook"),
                    })
                })
                .collect::<Vec<_>>(),
        )
        // Every key above is about the focused window, the one the actions act on; these say
        // which window that is, and what each open window shows.
        .with("focused_window", m.pane.window.0 as i64)
        .with("windows", windows(m))
}

/// `describe.windows`: every open window, in the order they were opened, with the note it shows
/// and whether it is the focused one. Two windows naming one note show one note: an edit in
/// either is in both.
fn windows(m: &Model) -> Vec<Value> {
    let mut panes: Vec<&Pane> = m.panes().collect();
    panes.sort_by_key(|p| p.window);
    panes
        .into_iter()
        .map(|p| {
            let note = m.current_in(p);
            json!({
                "window": p.window.0,
                "focused": p.window == m.pane.window,
                "open_note": note.as_ref().map(|n| n.id().to_string()),
                "title": m.shown_in(p).map(|s| s.title()),
                "unsaved": note.as_ref().is_some_and(|n| n.unsaved()) || m.offered_in(p).is_some(),
            })
        })
        .collect()
}

/// `describe.outside`: what is known of changes made to the open note's file by another program.
/// `conflict` and `deleted` wait on a choice (`keep_mine_as_copy`, `take_theirs`, `compare`), and
/// until then the note is not saved; `changed` is a change to a note with nothing unsaved, while
/// Notes reads it again (then `clean`).
fn outside(state: &OutsideState) -> &'static str {
    match state {
        OutsideState::Clean => "clean",
        OutsideState::Changed { .. } => "changed",
        OutsideState::Conflict { .. } => "conflict",
        OutsideState::Deleted => "deleted",
    }
}

/// `describe.draft`: whether the open note's unsaved edits are kept for recovery after a crash
/// (`kept`), could not be (`failed`: they are lost if Notes stops before a save), or there is
/// nothing unsaved to keep (`none`).
fn draft(state: &DraftState) -> &'static str {
    match state {
        DraftState::None => "none",
        DraftState::Kept { .. } => "kept",
        DraftState::Failed { .. } => "failed",
    }
}

/// `describe.save_failure`: how much of the open note's last save reached the disk when it
/// failed (`not_written`, `content_only`, `unknown`), or nothing when it did not fail.
fn save_failure(state: &SaveState) -> Option<&'static str> {
    let SaveState::Failed { error, .. } = state else { return None };
    Some(match error.durability {
        Durability::NotWritten => "not_written",
        Durability::ContentOnly => "content_only",
        Durability::Unknown => "unknown",
    })
}

/// What the open note now is, read back out of the model and off the disk rather than predicted:
/// `matches_disk` is the file compared with the text in the window. `None` when nothing is open.
pub fn written(m: &Model) -> Option<Value> {
    let n = m.current()?;
    let path = m.vault.dir().join(n.id());
    let disk = read(&path, LIMIT).ok().flatten();
    Some(json!({
        "filename": n.id(),
        "title": n.title(),
        "path": path.display().to_string(),
        "lines": n.text().lines().count(),
        "characters": n.text().chars().count(),
        "words": n.content().words(),
        "saved": !n.unsaved(),
        "on_disk": disk.is_some(),
        "matches_disk": disk.as_deref() == Some(n.text()),
        "trashed": n.trash(),
        "notebook": n.content().field("notebook"),
        "tags": n.content().field("tags"),
        "notice": m.notice,
    }))
}

/// What the library list is showing now: how many rows, and the first few titles.
pub fn listed(m: &Model) -> (usize, Vec<String>) {
    let rows = &m.pane.listing.rows;
    (rows.len(), rows.iter().take(5).map(|r| r.title.clone()).collect())
}

/// How much of one note's text `read_notes` returns, and the most its whole answer may be, as
/// serialized JSON. A mind is handed a safe read's answer up to 4,000 characters, so a page has to
/// fit under that with room for its envelope; what does not fit is the next page, not a silent cut.
pub const READ_NOTE_CHARS: usize = 700;
const READ_NOTES_ANSWER: usize = 3_400;

/// One page of the notes outside the Trash whose text or metadata contain `query` (all of them
/// when it is empty), newest first, each with its title, filename, modified time and text.
///
/// A mind asked to list every note that mentions a date had no way in but `open_note` then
/// `describe`, once per note; and given one long answer, its work log kept the first few
/// thousand characters. So a page is sized to be read whole: each text is cut at
/// [`READ_NOTE_CHARS`], the page ends before it would pass [`READ_NOTES_ANSWER`], and the answer
/// names the exact call for the next page (`skip`), so a long library is never mistaken for a
/// short one.
pub fn read_notes(notes: &[Note], query: &str, skip: usize) -> Value {
    let q = query.to_lowercase();
    let mut matching: Vec<&Note> = notes
        .iter()
        .filter(|n| !n.trash)
        .filter(|n| q.is_empty() || n.content.text.to_lowercase().contains(&q) || n.content.meta.to_lowercase().contains(&q))
        .collect();
    matching.sort_by(|a, b| b.modified.cmp(&a.modified));
    let total = matching.len();
    // The envelope's own size, with room for the longest `how_to_see_the_rest` and a cut list.
    let mut used = 600 + query.len();
    let mut out = Vec::new();
    let mut cut = Vec::new();
    for n in matching.iter().skip(skip) {
        let text: String = n.content.text.chars().take(READ_NOTE_CHARS).collect();
        let entry = json!({ "title": n.title(), "file": n.id, "modified": n.modified, "text": text });
        let size = entry.to_string().len() + n.title().len() + 4;
        // A page always carries at least one note, or a long first note would page forever.
        if !out.is_empty() && used + size > READ_NOTES_ANSWER {
            break;
        }
        used += size;
        if n.content.text.chars().count() > READ_NOTE_CHARS {
            cut.push(n.title());
        }
        out.push(entry);
    }
    let returned = out.len();
    let next = skip.saturating_add(returned);
    let left = total.saturating_sub(next);
    let query_arg = if query.is_empty() { String::new() } else { format!(" with query {query:?}") };
    json!({
        "query": query,
        "matching": total,
        "skip": skip,
        "returned": returned,
        "notes": out,
        "left_out": left,
        "texts_cut_short": cut,
        "how_to_see_the_rest": if left > 0 {
            format!("{left} more: call read_notes again{query_arg} and skip {next}")
        } else if !cut.is_empty() {
            "all matching notes are here; for a whole cut text, open_note its title and read describe.content".to_string()
        } else { String::new() },
    })
}
