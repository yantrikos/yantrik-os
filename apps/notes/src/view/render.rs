//! The only code that writes a window, by the part of the model that changed. Every window draws
//! itself from its own pane and the note that pane shows.

use slint::{ModelRc, VecModel};
use yantrik_app::app::{Changed, WindowId};
use yantrik_app::documents::OutsideState;

use crate::model::Model;
use crate::note::NoteText;
use crate::pane::{Cue, Pane};
use crate::update::recovery::Showing;
use crate::update::{closing, leaving, recovery, said};
use crate::{BookRow, NoteRow, NotesApp, PreviewBlock};

/// The busy flag, the footer's save state, the notice line, and whether the library takes edits.
pub const STATUS: &str = "status";
/// A note was shown: everything about it, its text and its fields included.
pub const NOTE: &str = "note";
/// The open note changed: its title, word count, unsaved flag, and its text in every window but
/// the one it was typed in, whose editor already shows it.
pub const EDIT: &str = "edit";
/// The editor's text, when the model put a text there the person did not type.
pub const CONTENT: &str = "content";
/// The notebook and tags fields, when the model set them.
pub const FIELDS: &str = "fields";
/// What the model set of the window's own state: section, search, preview, focus, dialog.
pub const CHROME: &str = "chrome";
pub const LIBRARY: &str = "library";
pub const PREVIEW: &str = "preview";
/// What an outside change asks of the person (the bar, and the file's side of the comparison),
/// and what the recovery bar offers back.
pub const OUTSIDE: &str = "outside";
/// Focus, caret and scroll, once.
pub const CUES: &str = "cues";
/// Only the model changed (what `describe` reads); nothing on screen does.
pub const MODEL_ONLY: &str = "model";

pub fn render(ui: &NotesApp, window: WindowId, m: &Model, changed: &Changed) {
    // A window whose pane is gone is closing; one not yet given a pane draws nothing yet.
    let Some(pane) = m.pane_of(window) else { return };
    let current = m.current_in(pane);
    if changed.has(STATUS) {
        ui.set_busy(m.busy.is_some());
        ui.set_status(pane.status.clone().into());
        // The footer's amber goes with its words: a file found gone is unsaved with nothing typed.
        ui.set_modified(unsaved_in(m, pane));
        // Said with the notice, which every load that finds the library read-only sets.
        ui.set_read_only(m.vault.read_only().is_some());
    }
    // The notice gives way to the outside bar, so it is drawn again whenever the bar may be.
    if changed.has(STATUS) || changed.has(OUTSIDE) || changed.has(NOTE) {
        ui.set_notice(notice(m, pane).into());
    }
    if changed.has(NOTE) || changed.has(EDIT) {
        // A note whose unsaved text from before Notes closed is offered is drawn from that text,
        // read-only, so the person sees what Restore keeps and what Discard drops; and it is
        // unsaved until then, so the footer says so in amber.
        let shown = m.shown_in(pane);
        ui.set_opened(current.is_some());
        ui.set_note_title(shown.map(|c| c.title()).unwrap_or_default().into());
        ui.set_trashed(current.as_ref().is_some_and(|c| c.trash()));
        ui.set_pinned(shown.is_some_and(|c| c.pinned()));
        ui.set_modified(unsaved_in(m, pane));
        // An edit turns the deleted bar's Let it go into Discard.
        ui.set_outside_edited(current.as_ref().is_some_and(|c| c.edited_since_gone()));
        ui.set_words(shown.map_or(0, |c| c.words()) as i32);
    }
    // Set only where it differs: the window it was typed in already shows it, and its caret stays.
    if changed.has(NOTE) || changed.has(CONTENT) || changed.has(EDIT) {
        let text = m.shown_in(pane).map(|c| c.text.as_str()).unwrap_or_default();
        if ui.get_content().as_str() != text {
            ui.set_content(text.into());
        }
    }
    if changed.has(NOTE) || changed.has(FIELDS) {
        if ui.get_notebook().as_str() != pane.notebook {
            ui.set_notebook(pane.notebook.clone().into());
        }
        if ui.get_tags().as_str() != pane.tags {
            ui.set_tags(pane.tags.clone().into());
        }
    }
    if changed.has(CHROME) {
        // Each window draws only the prompt that asks in it.
        let prompt = if leaving::asks_in(m, window) {
            leaving::prompt(m)
        } else if closing::asks_in(m, window) {
            closing::prompt(m)
        } else {
            None
        };
        let prompt = prompt.unwrap_or_default();
        ui.set_close_heading(prompt.heading.into());
        ui.set_close_said(prompt.sentence.into());
        ui.set_close_saves(prompt.saves);
        ui.set_close_save_label(prompt.save.into());
        ui.set_folder(pane.folder.clone().into());
        ui.set_query(pane.query.clone().into());
        ui.set_preview(pane.preview);
        ui.set_focus_mode(pane.focus_mode);
        // The prompt dialog only with a live prompt of this window's behind it (update/prompts.rs).
        let orphaned = pane.dialog == closing::CLOSE_DIALOG && !(leaving::asks_in(m, window) || closing::asks_in(m, window));
        ui.set_dialog(if orphaned { 0 } else { pane.dialog });
        ui.set_path(pane.path.clone().into());
    }
    if changed.has(NOTE) || changed.has(OUTSIDE) {
        outside(ui, m, pane);
    }
    if changed.has(LIBRARY) {
        library(ui, pane);
    }
    if changed.has(PREVIEW) {
        let blocks: Vec<PreviewBlock> =
            pane.blocks.iter().map(|b| PreviewBlock { text: b.text.clone().into(), kind: b.kind }).collect();
        ui.set_blocks(ModelRc::new(VecModel::from(blocks)));
    }
    // Last: a focus or a caret lands on what was just drawn. The keys are moved only in the
    // focused window: a window behind it that was shown a note does not take them, since taking
    // them is what tells Notes the person moved there.
    if changed.has(CUES) {
        let focused = window == m.pane.window;
        for cue in &pane.cues {
            match *cue {
                Cue::Shown => {
                    ui.invoke_reset_position();
                    if focused {
                        ui.invoke_focus_content();
                    }
                }
                Cue::CaretAt(offset) => ui.invoke_caret(offset),
                _ if !focused => {}
                Cue::FocusEditor => ui.invoke_focus_editor(),
                Cue::FocusShortcuts => ui.invoke_focus_shortcuts(),
                Cue::FocusContent => ui.invoke_focus_content(),
            }
        }
    }
}

/// Whether the footer is amber: the window's note is not on disk as shown (its edits, its file gone,
/// or its unsaved text from before Notes closed, offered and waiting).
fn unsaved_in(m: &Model, pane: &Pane) -> bool {
    m.current_in(pane).is_some_and(|c| c.unsaved()) || m.offered_in(pane).is_some()
}

/// The notice line as a window draws it. Nothing when the notice is about another window alone (a
/// close refused there), or when it only repeats what this window's outside bar already says (a
/// save refused over a change outside): the bar says that once, in full, and the footer says it
/// in two words.
fn notice<'a>(m: &'a Model, pane: &Pane) -> &'a str {
    let line = m.notice_for(pane.window);
    let repeats_bar = bar(m, pane).is_some_and(|(state, _)| line == said::blocked(state));
    if repeats_bar {
        ""
    } else {
        line
    }
}

/// What a window's outside bar asks about, and says: its note's conflict or deleted file, unless
/// the person put the bar away with *Not now*.
fn bar<'a>(m: &'a Model, pane: &Pane) -> Option<(&'a OutsideState, &'static str)> {
    let doc = m.documents.get(m.document_in(pane)?)?;
    let sentence = said::outside(&doc.outside).filter(|_| !pane.outside_later)?;
    Some((&doc.outside, sentence))
}

/// The outside bar and the comparison, and the recovery bar: what this window's note asks of the
/// person. Two windows on one note show the same bar, and either's answer is the note's.
fn outside(ui: &NotesApp, m: &Model, pane: &Pane) {
    let current = m.current_in(pane);
    // The bar: 1 a conflict, 2 a deleted file.
    let offered = bar(m, pane);
    ui.set_outside(match offered {
        Some((OutsideState::Deleted, _)) => 2,
        Some(_) => 1,
        None => 0,
    });
    ui.set_outside_said(offered.map(|(_, sentence)| sentence).unwrap_or_default().into());
    ui.set_outside_edited(current.as_ref().is_some_and(|c| c.edited_since_gone()));
    ui.set_theirs(pane.theirs.as_ref().map(|t| t.text.clone()).unwrap_or_default().into());
    ui.set_theirs_gone(pane.theirs.is_none());
    ui.set_mine_fields(current.as_ref().map(|c| fields_line(c.content())).unwrap_or_default().into());
    ui.set_theirs_fields(pane.theirs.as_ref().map(fields_line).unwrap_or_default().into());
    let offered = recovery::showing_in(m, pane);
    ui.set_recovery(offered.map_or(0, |(state, _)| state as i32));
    ui.set_recovery_said(
        offered
            .map(|(state, o)| match state {
                Showing::Open => said::offer_open(&o.content.title()),
                Showing::Listed => said::offer_listed(&o.content.title()),
                Showing::Gone => said::offer_gone(&o.content.title(), o.content.text.chars().count()),
            })
            .unwrap_or_default()
            .into(),
    );
    // The note an offer is for takes no keys until the person chooses.
    let waiting = current.as_ref().is_some_and(|c| m.recovery.offer_for(c.id()).is_some());
    ui.set_recovering(waiting);
}

/// A note's sidecar in one line, for the comparison: two versions can differ there and nowhere
/// else.
fn fields_line(note: &NoteText) -> String {
    let mut parts = Vec::new();
    for (key, label) in [("notebook", "Notebook"), ("tags", "Tags")] {
        let value = note.field(key);
        if !value.is_empty() {
            parts.push(format!("{label}: {value}"));
        }
    }
    if note.pinned() {
        parts.push("Pinned".into());
    }
    if parts.is_empty() {
        "No notebook, tags or pin".into()
    } else {
        parts.join(" · ")
    }
}

fn library(ui: &NotesApp, pane: &Pane) {
    let listing = &pane.listing;
    ui.set_all_count(listing.all_count);
    ui.set_pin_count(listing.pin_count);
    ui.set_trash_count(listing.trash_count);
    let books: Vec<BookRow> = listing
        .books
        .iter()
        .map(|b| BookRow { name: b.name.clone().into(), count: b.count, selected: b.selected })
        .collect();
    ui.set_books(ModelRc::new(VecModel::from(books)));
    let rows: Vec<NoteRow> = listing
        .rows
        .iter()
        .map(|r| NoteRow {
            id: r.id.clone().into(),
            title: r.title.clone().into(),
            preview: r.preview.clone().into(),
            date: r.date.clone().into(),
            marker: r.marker.into(),
            pinned: r.pinned,
            selected: r.selected,
        })
        .collect();
    ui.set_notes(ModelRc::new(VecModel::from(rows)));
}
