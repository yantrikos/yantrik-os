//! The tabs a start opens again: every draft the last run left, as it always did ("Recovered
//! unsaved drafts. Review them, then Save or Save As."), and the old build's recovery file once.
//!
//! A recovered tab is unsaved until it is saved, whatever its text, and it is the person's. A
//! draft that cannot be opened (not one this editor wrote, past the budget, a file another
//! draft already holds) is left on disk where it is and said: nothing here removes a draft.

use std::collections::BTreeSet;

use yantrik_app::app::WindowId;
use yantrik_app::documents::{Fingerprint, Source};
use yantrik_app::drafts::Drafts;

use crate::files::drafts::{now_ms, TabDraft};
use crate::files::legacy::{Legacy, LegacyTab};
use crate::model::{self, Model};
use crate::owner::Opener;
use crate::tab::Tab;
use crate::text::budget::{room_for, Weighed};

/// Said when tabs came back.
pub const RECOVERED: &str = "Recovered unsaved drafts. Review them, then Save or Save As.";

/// One tab to open again.
struct Back {
    /// The lane's name for its draft, `None` for one from the old file.
    name: Option<String>,
    draft: TabDraft,
}

/// Opens the drafts `found` and the old build's tabs into `m`, oldest first. Returns the notice.
pub fn restore(m: &mut Model, found: &Drafts, legacy: Legacy) -> String {
    let mut back: Vec<Back> = Vec::new();
    let mut unreadable = 0;
    for name in found.names() {
        match found.get(name).map(TabDraft::parse) {
            Some(Ok(draft)) => back.push(Back { name: Some(name.to_string()), draft }),
            _ => unreadable += 1,
        }
    }
    let mut problems = Vec::new();
    let had_legacy = matches!(legacy, Legacy::Tabs(_));
    match legacy {
        Legacy::Absent => {}
        Legacy::Unreadable(why) => problems.push(why),
        Legacy::Tabs(tabs) => {
            for t in tabs {
                let draft = from_legacy(t);
                // Already the lane's: a start that ended before the old file was set aside.
                let held = back.iter().any(|b| b.draft.path == draft.path && (draft.path.is_some() || b.draft.text == draft.text));
                if !held {
                    back.push(Back { name: None, draft });
                }
            }
        }
    }
    back.sort_by_key(|b| (b.draft.opened_at, b.draft.at));
    let mut waiting = BTreeSet::new();
    let mut left = 0;
    for b in back {
        if !open_one(m, &b, &mut waiting) {
            left += 1;
        }
    }
    // The old file is set aside once every tab it gave is the lane's: at once when none is owed.
    m.legacy = if had_legacy { model::Legacy::Waiting(waiting) } else { model::Legacy::Done };
    if left + unreadable > 0 {
        let dir = "the editor's drafts folder";
        problems.push(format!(
            "{} could not be opened again and {} kept in {dir}.",
            plural(left + unreadable, "draft", "drafts"),
            if left + unreadable == 1 { "is" } else { "are" }
        ));
    }
    let restored = !m.tabs.is_empty();
    match (restored, problems.is_empty()) {
        (true, true) => RECOVERED.into(),
        (true, false) => format!("{RECOVERED} {}", problems.join(" ")),
        (false, _) => problems.join(" "),
    }
}

/// Opens one draft as a tab, if it fits and its file is in no other tab. A draft from the old file
/// is given a name of its own and waits in `waiting` until it is on disk.
fn open_one(m: &mut Model, b: &Back, waiting: &mut BTreeSet<String>) -> bool {
    let weighed: Vec<Weighed> = m.weighed();
    let saved = b.draft.on_disk.as_ref().map_or(0, String::len);
    if !room_for(&weighed, b.draft.text.len().max(saved)) {
        return false;
    }
    let (id, fresh) = m.next_ids();
    let name = b.name.clone().unwrap_or(fresh);
    let source = match &b.draft.path {
        Some(path) => Source::File(path.clone()),
        None => Source::New(name.clone()),
    };
    if m.documents.find(&source).is_some() {
        return false;
    }
    let baseline = b.draft.on_disk.as_ref().map(|t| Fingerprint::of_bytes(t.as_bytes()));
    let doc = m.documents.open(source, b.draft.text.clone(), baseline);
    let view = m.documents.show(doc, WindowId::PRIMARY);
    if let Some(d) = m.documents.get_mut(doc) {
        // Unsaved by the framework's count too, and its draft already on disk when it came from
        // the lane; one from the old file is owed (`waiting`).
        d.edit(|_| ());
        if b.name.is_some() {
            d.draft_asked();
        } else {
            waiting.insert(name.clone());
        }
    }
    let hidden = b.draft.path.as_deref().is_some_and(crate::agent_rule::hidden_from_agents);
    m.tabs.push(Tab {
        id,
        doc,
        view,
        on_disk: b.draft.on_disk.clone(),
        recovered: true,
        opened_by: Opener::Person,
        history: Default::default(),
        hidden,
        draft: name,
        opened_at: b.draft.opened_at,
        inode: None,
        unsure: None,
    });
    true
}

/// A tab from the old file, as a draft of this build's. Its baseline is what its file held; an
/// untitled tab's empty baseline is no file at all.
fn from_legacy(t: LegacyTab) -> TabDraft {
    let on_disk = t.path.as_ref().map(|_| t.baseline);
    TabDraft { version: crate::files::drafts::VERSION, path: t.path, text: t.text, on_disk, opened_at: 0, at: now_ms() }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}
