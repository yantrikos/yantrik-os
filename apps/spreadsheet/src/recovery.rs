//! The sheet a start opens again: the draft the last run left ("Recovered unsaved changes. Save
//! keeps them.").
//!
//! A recovered sheet is unsaved until it is saved, whatever its content. A draft that cannot be
//! opened (not one this app wrote, past the bounds) is left on disk where it is and said: nothing
//! here removes a draft. With a file named on the command line the file gets the window and every
//! draft is left exactly where it is, to be offered at the next start with no file.

use yantrik_app::documents::{Fingerprint, Source};
use yantrik_app::drafts::Drafts;

use crate::address::{parse_cell, Cell};
use crate::book::Book;
use crate::files::drafts::SheetDraft;
use crate::model::{self, Model};
use crate::sheet::Sheet;

/// One sheet to open again.
struct Back {
    /// The lane's name for its draft.
    name: String,
    draft: SheetDraft,
    sheet: Sheet,
}

/// Said when a draft came back.
fn recovered(draft: &SheetDraft) -> String {
    let to = draft.path.as_ref().map(|p| format!(" to {}", p.display())).unwrap_or_default();
    match &draft.typing {
        Some(cell) => format!("Recovered unsaved changes{to}. The text you were typing in {cell} is in that cell. Save keeps it."),
        None => format!("Recovered unsaved changes{to}. Save keeps them."),
    }
}

/// Opens the newest draft in `found` into `m`. Returns the notice.
pub fn restore(m: &mut Model, found: &Drafts, file_named: bool) -> String {
    let mut lane: Vec<Back> = Vec::new();
    let mut unreadable = 0;
    for name in found.names() {
        let back = found
            .get(name)
            .map(SheetDraft::parse)
            .and_then(Result::ok)
            .and_then(|draft| Sheet::from_rows(&draft.rows).ok().map(|sheet| (draft, sheet)));
        match back {
            Some((draft, sheet)) => lane.push(Back { name: name.to_string(), draft, sheet }),
            None => unreadable += 1,
        }
    }
    if file_named {
        return held_aside(m, &lane, unreadable);
    }
    lane.sort_by_key(|b| std::cmp::Reverse(b.draft.at));
    let mut rest = lane.into_iter();
    let restored = rest.next().map(|b| open_one(m, b));
    let left = rest.count() + unreadable;
    let problem = (left > 0).then(|| {
        format!(
            "{left} more draft{} could not be opened at once and {} kept in ySheets' drafts folder.",
            if left == 1 { "" } else { "s" },
            if left == 1 { "is" } else { "are" }
        )
    });
    match (restored, problem) {
        (Some(said), None) => said,
        (Some(said), Some(problem)) => format!("{said} {problem}"),
        (None, problem) => problem.unwrap_or_default(),
    }
}

/// A file was named on the command line: the drafts are left alone, and the notice says where
/// they are.
fn held_aside(m: &mut Model, lane: &[Back], unreadable: usize) -> String {
    if lane.is_empty() && unreadable == 0 {
        return String::new();
    }
    let place = m.state_dir.join(yantrik_app::drafts::DIR);
    let said = format!(
        "An unsaved draft from a previous session is waiting in {}. It was left alone; open ySheets with no file to \
         pick it up.",
        place.display()
    );
    m.held_aside = Some(said.clone());
    said
}

/// Opens one draft as the sheet in front, and says so.
fn open_one(m: &mut Model, b: Back) -> String {
    let said = recovered(&b.draft);
    let Back { name, draft, sheet } = b;
    let source = match &draft.path {
        Some(path) => Source::File(path.clone()),
        None => Source::New(name.clone()),
    };
    let baseline = draft.on_disk.as_ref().map(|t| Fingerprint::of_bytes(t.as_bytes()));
    let blank = m.doc.id;
    m.documents.close_document(blank);
    let id = m.documents.open(source, Book::opened(sheet), baseline);
    let hidden = draft.path.as_deref().is_some_and(crate::agent_rule::hidden_from_agents);
    m.doc = model::place(&mut m.documents, id, draft.on_disk.clone(), name, hidden, None);
    m.doc.recovered = true;
    // Unsaved by the framework's count too, and its draft already on disk.
    m.d_mut().edit(|_| ());
    m.draft_asked();
    let at = parse_cell(&draft.active).unwrap_or(Cell::new(0, 0));
    m.active = Cell::new(at.row.min(m.sheet().rows() - 1), at.col.min(m.sheet().cols() - 1));
    said
}
