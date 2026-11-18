//! The deck a start opens again: the draft the last run left ("Recovered unsaved changes. Save
//! keeps them."), and the old build's recovery file once.
//!
//! A recovered deck is unsaved until it is saved, whatever its content. A draft that cannot be
//! opened (not one this app wrote, past the bounds) is left on disk where it is and said: nothing
//! here removes a draft. With a file named on the command line the file gets the window and every
//! draft is left exactly where it is, to be offered at the next start with no file.

use yantrik_app::documents::{Fingerprint, Source};
use yantrik_app::drafts::Drafts;

use crate::deck::{parse, Deck};
use crate::files::drafts::{now_ms, DeckDraft, VERSION};
use crate::files::legacy::{self, Legacy as OldFile, LegacyDeck};
use crate::model::{self, Legacy, Model};

/// One deck to open again.
struct Back {
    /// The lane's name for its draft, `None` for one from the old file.
    name: Option<String>,
    draft: DeckDraft,
}

/// Said when a draft came back.
fn recovered(draft: &DeckDraft) -> String {
    let to = draft.path.as_ref().map(|p| format!(" to {}", p.display())).unwrap_or_default();
    format!("Recovered unsaved changes{to}. Save keeps them.")
}

/// Opens the newest draft in `found`, or the old build's, into `m`. Returns the notice.
pub fn restore(m: &mut Model, found: &Drafts, old: OldFile, file_named: bool) -> String {
    let mut lane: Vec<Back> = Vec::new();
    let mut unreadable = 0;
    for name in found.names() {
        match found.get(name).map(DeckDraft::parse) {
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
        OldFile::Found(deck) => {
            let draft = from_legacy(deck);
            if in_lane(&back, &draft) {
                // A start that ended before the old file was set aside: its deck is the lane's
                // already, so the file is set aside at the first command.
                m.legacy = Legacy::Owed;
            } else {
                back.push(Back { name: None, draft });
            }
        }
    }
    // The old file's deck first, always: it is the one that is not in the lane, so it is the one
    // that cannot be left for later without being offered again at every start.
    back.sort_by_key(|b| (b.name.is_some(), std::cmp::Reverse(b.draft.at)));
    let mut rest = back.into_iter();
    let restored = rest.next().map(|b| open_one(m, b));
    let left = rest.count() + unreadable;
    if left > 0 {
        problems.push(format!(
            "{left} more draft{} could not be opened at once and {} kept in yPresent's drafts folder.",
            if left == 1 { "" } else { "s" },
            if left == 1 { "is" } else { "are" }
        ));
    }
    match (restored, problems.is_empty()) {
        (Some(said), true) => said,
        (Some(said), false) => format!("{said} {}", problems.join(" ")),
        (None, _) => problems.join(" "),
    }
}

/// Whether the old file's deck is already a draft in the lane: the same file and the same
/// content. A lane draft that differs is another deck, so the old one is never taken for it.
fn in_lane(lane: &[Back], old: &DeckDraft) -> bool {
    lane.iter().any(|b| b.draft.path == old.path && b.draft.content == old.content)
}

/// A file was named on the command line: the drafts are left alone, and the notice says where
/// each one is.
fn held_aside(m: &mut Model, lane: &[Back], old: &OldFile, unreadable: usize) -> String {
    let mut places = Vec::new();
    if !lane.is_empty() || unreadable > 0 {
        places.push(m.state_dir.join(yantrik_app::drafts::DIR));
    }
    let mut unreadable_old = None;
    match old {
        OldFile::Unreadable(why) => unreadable_old = Some(why.clone()),
        OldFile::Found(deck) => {
            if !in_lane(lane, &from_legacy(deck.clone())) {
                places.push(m.state_dir.join(legacy::NAME));
            }
        }
        OldFile::Absent => {}
    }
    if places.is_empty() && unreadable_old.is_none() {
        return String::new();
    }
    let places: Vec<String> = places.iter().map(|p| p.display().to_string()).collect();
    let mut said = match places.is_empty() {
        true => String::new(),
        false => format!(
            "An unsaved draft from a previous session is waiting in {}. It was left alone; open yPresent with no \
             file to pick it up.",
            places.join(" and in ")
        ),
    };
    if let Some(why) = unreadable_old {
        said = format!("{said} {why}").trim().to_string();
    }
    m.held_aside = Some(said.clone());
    said
}

/// Opens one draft as the deck in front, and says so.
fn open_one(m: &mut Model, b: Back) -> String {
    let said = recovered(&b.draft);
    let fresh = m.next_draft();
    let name = b.name.clone().unwrap_or(fresh);
    let source = match &b.draft.path {
        Some(path) => Source::File(path.clone()),
        None => Source::New(name.clone()),
    };
    let baseline = b.draft.on_disk.as_ref().map(|t| Fingerprint::of_bytes(t.as_bytes()));
    let saved = b.draft.on_disk.as_deref().and_then(|t| parse(t).ok());
    let deck = Deck::restored(b.draft.content.clone(), saved);
    let blank = m.doc.id;
    m.documents.close_document(blank);
    let id = m.documents.open(source, deck, baseline);
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
    said
}

/// The old build's deck, as a draft of this build's. Its file's text was never kept, so the deck
/// has no baseline: saving it onto its file is refused as a change on disk, and Save keeps it as
/// a new file instead, rather than guessing that the file is what the deck was typed against.
fn from_legacy(deck: LegacyDeck) -> DeckDraft {
    DeckDraft { version: VERSION, path: deck.path, content: deck.content, on_disk: None, at: now_ms() }
}
