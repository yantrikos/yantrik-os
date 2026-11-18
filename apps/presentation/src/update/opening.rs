//! Putting a deck in front: a file read in a job, or a new empty one.
//!
//! Nothing is replaced until the file has been read: a path that is not a deck costs nothing, the
//! deck on screen is untouched and the reason is named. A deck with unsaved changes is written to
//! its home first, the one it has or a fresh collision-safe one under `~/Documents/Presentations`,
//! and the caller is told where it went. If that write fails the replacement does not happen at
//! all: losing the work is the one outcome that is not allowed.

use std::path::PathBuf;

use serde_json::json;
use yantrik_app::documents::{Fingerprint, Source};
use yantrik_app::effects::Effects;

use super::{drafts, saving, say, surface};
use crate::agent_rule::HIDDEN;
use crate::caller::Caller;
use crate::command::{Cause, Command, Next};
use crate::deck::{parse, Deck};
use crate::files::open::{most_recent_deck, open_deck, Opened};
use crate::model::{self, Model, Work};
use crate::YPresent;

/// Said when a job ended without an answer (it panicked, or the pool refused it).
const JOB_LOST: &str = "The file could not be read; try opening it again.";

/// Opens the file at `path`, read in a job.
pub fn start(m: &mut Model, fx: &mut Effects<YPresent>, path: PathBuf, then: Cause, caller: Caller) {
    m.busy = Some(Work::Open);
    say(m, "Opening file…");
    let agent = caller.agent;
    let lost = Command::Opened { then: then.clone(), result: Err(JOB_LOST.into()) };
    fx.spawn_or(lost, move |_| Command::Opened { then, result: open_deck(&path, agent).map(Box::new) });
}

/// The Open button: the deck most recently worked on, found and read in a job.
pub fn recent(m: &mut Model, fx: &mut Effects<YPresent>) {
    m.busy = Some(Work::Open);
    say(m, "Opening file…");
    let dir = m.deck_dir.clone();
    let lost = Command::Opened { then: Cause::Window, result: Err(JOB_LOST.into()) };
    fx.spawn_or(lost, move |_| Command::Opened {
        then: Cause::Window,
        result: most_recent_deck(&dir).and_then(|path| open_deck(&path, false)).map(Box::new),
    });
}

/// A file a launch asked to open, once yPresent is free.
pub fn launch(m: &mut Model, fx: &mut Effects<YPresent>, path: PathBuf) {
    m.queue = Some(path);
    next(m, fx);
}

/// The file a launch asked for, when yPresent is free. A second launch handing its file to the
/// running window always replaced the deck in it, after writing that deck to a file of its own
/// if it had unsaved changes; it still does.
pub fn next(m: &mut Model, fx: &mut Effects<YPresent>) {
    if m.busy.is_some() || m.lingering {
        return;
    }
    if let Some(path) = m.queue.take() {
        start(m, fx, path, Cause::Launch, Caller::person());
    }
}

/// An open landed.
pub fn landed(m: &mut Model, fx: &mut Effects<YPresent>, then: Cause, result: Result<Box<Opened>, String>) {
    if m.busy == Some(Work::Open) {
        m.busy = None;
    }
    m.notice.clear();
    match result {
        Ok(opened) => replace(m, fx, Next::File(opened), then),
        Err(e) => {
            refuse(m, fx, &then, e);
            // A close that waited on this read stays undone: the failure is what the person reads.
            super::closing::not_closed(m);
        }
    }
    next(m, fx);
}

/// Said, to the window and to the call that asked.
pub fn refuse(m: &mut Model, fx: &mut Effects<YPresent>, then: &Cause, sentence: String) {
    match then {
        Cause::Surface(_) => surface::refuse(m, fx, sentence),
        Cause::Window | Cause::Launch => say(m, sentence),
    }
}

/// Put `next` in front, writing the deck it replaces to a file of its own first when it has
/// unsaved changes.
pub fn replace(m: &mut Model, fx: &mut Effects<YPresent>, next: Next, then: Cause) {
    if m.lingering {
        // No window to put it in: the deck stays as it is and its draft is kept.
        let sentence = "yPresent's window is closed while it finishes a save; launch it again to bring the window back.";
        return refuse(m, fx, &then, sentence.into());
    }
    if m.modified() {
        // The deck asked for is the deck in front, and it has unsaved changes: writing them to
        // its file would make what was just read stale, and reading it back would drop them.
        if matches!(&next, Next::File(opened) if m.path() == Some(opened.path.as_path())) {
            let sentence = "That deck is already open and has unsaved changes. Save it first, or open another.";
            return refuse(m, fx, &then, sentence.into());
        }
        let agent = matches!(&then, Cause::Surface(c) if c.agent);
        return saving::rescue(m, fx, next, then, agent);
    }
    put_in_front(m, fx, next, then, None);
}

/// The deck in front closed: unwatched, its draft gone, its view and the table's entry too.
fn close_current(m: &mut Model, fx: &mut Effects<YPresent>) {
    let name = m.doc.draft.clone();
    // Speaker notes asked of this deck are not for the next one.
    m.asking = None;
    if let Some(d) = m.documents.close_document(m.doc.id) {
        fx.unwatch(&d.source.key());
    }
    drafts::forget(m, fx, &name);
    m.reread_owed = false;
}

/// The cues every new deck in front starts with: the first slide on the canvas, nothing searched.
fn arrived(m: &mut Model) {
    m.reload = true;
    m.hits.clear();
    m.hit_at = 0;
}

/// `next` in front, and the call that asked for it answered from what it did.
pub fn put_in_front(m: &mut Model, fx: &mut Effects<YPresent>, next: Next, then: Cause, rescued: Option<PathBuf>) {
    let saved_to = rescued.map(|p| p.display().to_string());
    // Whether the deck being replaced was one an agent is not shown: the notice about where it
    // went is read by an agent through `describe`.
    let was_hidden = m.doc.hidden;
    match next {
        Next::Blank(title) => {
            close_current(m, fx);
            let draft = m.next_draft();
            m.doc = model::blank(&mut m.documents, draft, Deck::blank(title));
            arrived(m);
            if let Cause::Surface(_) = &then {
                let answer = json!({
                    "deck_title": m.title(), "slides": m.deck().len(), "index": m.deck().current(),
                    "previous_deck_saved_to": saved_to,
                });
                surface::answer(m, fx, answer);
            }
        }
        Next::File(opened) => {
            // The file was read as a deck before anything was displaced, so this parse cannot
            // fail; if it ever did, the deck on screen is the one thing left alone.
            let Ok(content) = parse(&opened.text) else {
                return refuse(m, fx, &then, format!("{}: it is not a yPresent deck", opened.path.display()));
            };
            close_current(m, fx);
            let draft = m.next_draft();
            let baseline = Fingerprint::of_bytes(opened.text.as_bytes());
            let id = m.documents.open(Source::File(opened.path.clone()), Deck::opened(content), Some(baseline));
            if let Some((key, path, seen)) = m.documents.get(id).and_then(|d| d.watch_entry()) {
                fx.watch(&key, path, seen);
            }
            let opened = *opened;
            m.doc = model::place(&mut m.documents, id, Some(opened.text), draft, opened.hidden, opened.inode);
            arrived(m);
            if let Cause::Surface(caller) = &then {
                // A file an agent is not shown (a link out of the places it may name) is opened
                // for the window, and the agent learns nothing of it.
                let answer = if caller.agent && m.doc.hidden {
                    json!({ "opened": HIDDEN })
                } else {
                    json!({
                        "opened": opened.path.display().to_string(), "slides": m.deck().len(),
                        "deck_title": m.title(), "previous_deck_saved_to": saved_to,
                    })
                };
                surface::answer(m, fx, answer);
            } else if let Some(said) = (matches!(then, Cause::Launch)).then(|| m.held_aside.clone()).flatten() {
                // Last, because a successful open clears the notice and a draft held aside is the
                // one thing a person opening a file still needs to be told.
                m.notice.set(said);
            }
        }
    }
    // The window's own opens say where the deck they replaced went; a call is told in its answer.
    if let (Some(saved), Cause::Window | Cause::Launch) = (&saved_to, &then) {
        let kept = format!("The deck you were editing was saved to {saved}.");
        let held = m.notice.window().to_string();
        let for_agents = format!("The deck you were editing was saved to {HIDDEN}.");
        m.notice.set_about(was_hidden, if held.is_empty() { kept } else { format!("{kept} {held}") }, || for_agents);
    }
}
