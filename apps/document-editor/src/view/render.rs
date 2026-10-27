//! The only code that writes the window.
//!
//! The page is given the document's text only when it differs: the page is bound both ways, and
//! setting the text it already shows would move the caret of the person typing on every
//! keystroke. The fields the person types into (the dialog's path, the find bar) are set only
//! when the model put something there, by a cue, so a command arriving while they type never
//! takes their text away. Everything else -- the title, the outline, the counts, the path,
//! whether it is saved -- is derived from the model each time, so no two of them can disagree.
//!
//! The document is untrusted: its text, its title and its headings reach the window as plain
//! strings in `Text` and `TextInput`, which draw them and interpret nothing.

use yantrik_app::app::Changed;
use yantrik_app::rail::RailView as _;

use crate::markdown;
use crate::model::{Cue, Model};
use crate::prompt;
use crate::{DocHeadingEntry, DocumentEditorApp};

/// The document's text changed: the page.
pub const TEXT: &str = "text";
/// Everything else: the title, the outline, the counts, the bars, the dialog, the rail. Drawn
/// after every command.
pub const CHROME: &str = "chrome";

pub fn render(ui: &DocumentEditorApp, m: &Model, changed: &Changed) {
    let reset = m.cues.contains(&Cue::ResetContent);
    if changed.has(TEXT) || reset {
        let text = m.text();
        if reset || ui.get_doc_content().as_str() != text {
            ui.set_doc_content(text.into());
        }
    }
    if changed.has(CHROME) || changed.has(TEXT) {
        chrome(ui, m);
    }
    for cue in &m.cues {
        match *cue {
            Cue::FocusBody => ui.invoke_focus_body(),
            Cue::Select(a, z) => ui.invoke_select_range(a as i32, z as i32),
            Cue::ShowFind => {
                ui.set_doc_show_find(true);
                ui.set_doc_find_query(m.find.query.clone().into());
            }
            Cue::FindFields => {
                ui.set_doc_find_query(m.find.query.clone().into());
                ui.set_doc_replace_text(m.find.replacement.clone().into());
            }
            Cue::Dialog => ui.set_doc_dialog_path(m.dialog_path.clone().into()),
            Cue::ResetContent => {}
        }
    }
}

fn chrome(ui: &DocumentEditorApp, m: &Model) {
    let text = m.text();
    let modified = m.modified();
    ui.set_doc_title(m.title().into());
    ui.set_doc_file_path(m.path().map(|p| p.display().to_string()).unwrap_or_default().into());
    ui.set_doc_is_modified(modified);
    ui.set_doc_word_count(markdown::word_count(text) as i32);
    ui.set_doc_char_count(markdown::char_count(text) as i32);
    let headings: Vec<DocHeadingEntry> = markdown::outline(text)
        .into_iter()
        .map(|h| DocHeadingEntry { title: h.title.into(), level: h.level, block_index: h.offset as i32 })
        .collect();
    yantrik_app::rows::set(ui.get_doc_headings(), headings, |list| ui.set_doc_headings(list));
    ui.set_doc_save_status(save_status(m, modified).into());
    ui.set_doc_find_count(m.find.matches.len() as i32);
    ui.set_notice(m.notice.window().into());
    ui.set_doc_dialog(prompt::dialog(m));
    ui.set_doc_dialog_error(m.dialog_error.clone().into());
    ui.render_rail(&m.rail);
    // The panel's own card, when the rail could not ask and has none of its own up.
    if m.rail.proposal().is_none() {
        if let Some(card) = &m.card {
            ui.set_proposal(crate::AgentProposal {
                title: card.title.as_str().into(),
                body: card.body.as_str().into(),
                source: card.source.as_str().into(),
                impact: card.impact.as_str().into(),
                destructive: card.destructive,
                verb: card.verb.as_str().into(),
            });
            ui.set_proposal_working(false);
        }
    }
}

/// The status line's word for where the document stands.
fn save_status(m: &Model, modified: bool) -> &'static str {
    if m.doc.recovered {
        "Recovered draft"
    } else if modified {
        "Unsaved"
    } else if m.path().is_some() {
        "Saved"
    } else {
        "New document"
    }
}
