//! The document a start opens again: the draft the last run left, as it always did ("A draft from
//! a previous session was recovered. Review it, then Save or Save As."), and the old build's
//! recovery file once.
//!
//! A recovered document is unsaved until it is saved, whatever its text. A draft that cannot be
//! opened (not one this app wrote, past the bounds) is left on disk where it is and said: nothing
//! here removes a draft. With a file named on the command line the file gets the window and every
//! draft is left exactly where it is, to be offered at the next start with no file.

use yantrik_app::documents::{Fingerprint, Source};
use yantrik_app::drafts::Drafts;

use crate::files::drafts::{now_ms, DocDraft};
use crate::files::legacy::{Legacy as OldFile, LegacyDoc};
use crate::model::{self, Legacy, Model};

/// Said when a draft came back.
pub const RECOVERED: &str = "A draft from a previous session was recovered. Review it, then Save or Save As.";

/// One document to open again.
struct Back {
    /// The lane's name for its draft, `None` for one from the old file.
    name: Option<String>,
    draft: DocDraft,
}

/// Opens the newest draft in `found`, or the old build's, into `m`. Returns the notice.
pub fn restore(m: &mut Model, found: &Drafts, old: OldFile, file_named: bool) -> String {
    let mut lane: Vec<Back> = Vec::new();
    let mut unreadable = 0;
    for name in found.names() {
        match found.get(name).map(DocDraft::parse) {
            Some(Ok(draft)) => lane.push(Back { name: Some(name.to_string()), draft }),
            _ => unreadable += 1,
        }
    }
    if file_named {
        return held_aside(m, &lane, &old, unreadable);
    }
    let mut problems = Vec::new();
    let mut back = lane;
    match old {
        OldFile::Absent => {}
        OldFile::Unreadable(why) => {
            // Preserved rather than overwritten: it may still be the only copy of something.
            problems.push(why);
        }
        OldFile::Found(doc) => {
            let draft = from_legacy(doc);
            if in_lane(&back, &draft) {
                // A start that ended before the old file was set aside: its text is the lane's
                // already, so the file is set aside at the first command.
                m.legacy = Legacy::Owed;
            } else {
                back.push(Back { name: None, draft });
            }
        }
    }
    // The old file's document first, always: it is the one that is not in the lane, so it is the
    // one that cannot be left for later without being offered again at every start.
    back.sort_by_key(|b| (b.name.is_some(), std::cmp::Reverse(b.draft.at)));
    let mut rest = back.into_iter();
    let restored = rest.next().map(|b| open_one(m, b)).is_some();
    let left = rest.count() + unreadable;
    if left > 0 {
        problems.push(format!(
            "{left} more draft{} could not be opened at once and {} kept in yDoc's drafts folder.",
            if left == 1 { "" } else { "s" },
            if left == 1 { "is" } else { "are" }
        ));
    }
    match (restored, problems.is_empty()) {
        (true, true) => RECOVERED.into(),
        (true, false) => format!("{RECOVERED} {}", problems.join(" ")),
        (false, _) => problems.join(" "),
    }
}

/// Whether the old file's document is already a draft in the lane: the same file and the same
/// text. A lane draft that differs is another document, so the old text is never taken for it.
fn in_lane(lane: &[Back], old: &DocDraft) -> bool {
    lane.iter().any(|b| b.draft.path == old.path && b.draft.text == old.text)
}

/// A file was named on the command line: the drafts are left alone, and the notice says where
/// each one is.
fn held_aside(m: &mut Model, lane: &[Back], old: &OldFile, unreadable: usize) -> String {
    let mut places = Vec::new();
    if !lane.is_empty() || unreadable > 0 {
        places.push(m.state_dir.join(yantrik_app::drafts::DIR));
    }
    match old {
        OldFile::Unreadable(why) => return why.clone(),
        OldFile::Found(doc) => {
            if !in_lane(lane, &from_legacy(doc.clone())) {
                places.push(crate::files::legacy::old_dir(&m.state_dir).join(crate::files::legacy::NAME));
            }
        }
        OldFile::Absent => {}
    }
    if places.is_empty() {
        return String::new();
    }
    let places: Vec<String> = places.iter().map(|p| p.display().to_string()).collect();
    let said = format!(
        "An unsaved draft from a previous session is waiting in {}. It was left alone; open yDoc with no file to pick it up.",
        places.join(" and in ")
    );
    m.held_aside = Some(said.clone());
    said
}

/// Opens one draft as the document in front.
fn open_one(m: &mut Model, b: Back) {
    let fresh = m.next_draft();
    let name = b.name.clone().unwrap_or(fresh);
    let source = match &b.draft.path {
        Some(path) => Source::File(path.clone()),
        None => Source::New(name.clone()),
    };
    let baseline = b.draft.on_disk.as_ref().map(|t| Fingerprint::of_bytes(t.as_bytes()));
    let blank = m.doc.id;
    m.documents.close_document(blank);
    let id = m.documents.open(source, b.draft.text.clone(), baseline);
    let hidden = b.draft.path.as_deref().is_some_and(crate::agent_rule::hidden_from_agents);
    m.doc = model::place(&mut m.documents, id, b.draft.on_disk.clone(), name.clone(), hidden, None);
    m.doc.recovered = true;
    let d = m.d_mut();
    // Unsaved by the framework's count too, and its draft already on disk when it came from the
    // lane; one from the old file is owed (`Legacy::Waiting`).
    d.edit(|_| ());
    if b.name.is_some() {
        d.draft_asked();
    } else {
        m.legacy = Legacy::Waiting(name);
    }
}

/// The old build's document, as a draft of this build's. Its baseline is what its file held; an
/// untitled document's empty baseline is no file at all.
fn from_legacy(doc: LegacyDoc) -> DocDraft {
    let on_disk = doc.path.as_ref().map(|_| doc.baseline);
    DocDraft { version: crate::files::drafts::VERSION, path: doc.path, text: doc.text, on_disk, at: now_ms() }
}
