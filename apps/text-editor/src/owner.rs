//! Who opened each tab, and which tab a caller is told to close when there is no room for more.
//!
//! VM 520, 4 October 2026: a mind hit "Eight tabs are already open", then ran `close` on the tab
//! in front to make room. That tab was the person's: opened and saved by them, not the mind.
//! Nothing was lost because it was saved, but the refusal had told the mind only "close one",
//! and nothing anywhere said whose tabs were whose. So every tab now records who opened it,
//! `describe` says whether it was the caller, and the refusal names one tab the caller may
//! close: the oldest one IT opened with nothing unsaved.
//!
//! Who is calling is decided exactly as the editor's other agent rule decides it
//! (`agent_rule`, #443): `control::agent_is_calling`, the runtime's one answer. A tab opened from
//! the window, or by a call with no agent token from the person's own account, is the person's.
//! The runtime hands a handler an agent's token and nothing else about it, so `opened_by` says
//! only `"agent"`; a digest of the token, never the token, is kept beside it, which is what
//! `opened_by_you` and the refusal compare, so one agent's tabs are told from another's without
//! anyone knowing a name.

use yantrik_app_runtime::control::{agent_is_calling, agent_token};
use yantrik_ipc_transport::reach::token_digest;

use crate::document::{Document, MAX_OPEN_BYTES, MAX_TABS};

/// Who opened a tab. Never written anywhere: see `Document::opened_by`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Opener {
    /// From the window, or a call that is not an agent's.
    #[default]
    Person,
    /// An agent's call. `token` is the SHA-256 of the token it carried, the same digest the
    /// shell publishes reach under.
    Agent { token: Option<String> },
}

impl Opener {
    /// Whoever is calling right now: the dispatch's caller inside an `app.act` or `app.describe`,
    /// the person in a window callback, where no caller is installed.
    pub fn calling() -> Opener {
        if !agent_is_calling() {
            return Opener::Person;
        }
        Opener::Agent { token: agent_token().map(|t| token_digest(&t)) }
    }

    /// What `describe.tabs[i].opened_by` says.
    pub fn label(&self) -> &'static str {
        match self {
            Opener::Person => "person",
            Opener::Agent { .. } => "agent",
        }
    }

    /// Did this caller open `tab`? What `describe.tabs[i].opened_by_you` says.
    ///
    /// The person's own are the person's. An agent's are the ones opened under the same token,
    /// and a tab recovered after a restart is no live agent's. `None` when the caller is an agent
    /// that carried no token, which is every `app.describe` from the mind account: the runtime
    /// installs a token only for `app.act`, so there the answer is not known, and is not guessed.
    pub fn owns(&self, tab: &Opener) -> Option<bool> {
        match (self, tab) {
            (Opener::Agent { token: None }, _) => None,
            (Opener::Person, Opener::Person) => Some(true),
            (Opener::Agent { token: Some(me) }, Opener::Agent { token: Some(it) }) => Some(me == it),
            _ => Some(false),
        }
    }
}

/// The refusal to open or create one more tab, naming the one call that makes room for `me`.
pub fn no_room(docs: &[Document], me: &Opener) -> String {
    let full = if docs.len() >= MAX_TABS {
        format!("{MAX_TABS} tabs are already open, the most the editor keeps; close one before opening another.")
    } else {
        format!(
            "The open tabs would hold more than the {} MiB of text the editor keeps open at once; \
             close one before opening another.",
            MAX_OPEN_BYTES / crate::document::MAX_BYTES
        )
    };
    format!("{full} {}", room_hint(docs, me))
}

/// The refusal of an edit that would take the open tabs past the budget.
pub fn too_big(docs: &[Document], me: &Opener) -> String {
    format!(
        "That would take the open tabs past the {} MiB of text the editor keeps open at once; the \
         tab is unchanged. {}",
        MAX_OPEN_BYTES / crate::document::MAX_BYTES,
        room_hint(docs, me)
    )
}

/// Which tab `me` can close to make room, as the call that closes it.
///
/// A tab is only ever named when closing it loses nothing (`!dirty()`: saved to its file, or
/// empty and never written); a modified tab is never suggested. Tabs are kept in the order they
/// were opened, so the lowest index is the oldest. An agent is pointed only at its own tabs; with
/// none, it is told so and the tab is named as one the person could close, not as one for it.
/// The person may close any tab.
fn room_hint(docs: &[Document], me: &Opener) -> String {
    let clean = |i: &usize| !docs[*i].dirty();
    let all: Vec<usize> = (0..docs.len()).collect();
    let person = matches!(me, Opener::Person);
    let mine: Vec<usize> = all.iter().copied().filter(|i| person || me.owns(&docs[*i].opened_by) == Some(true)).collect();
    let whose = if person { "" } else { " you opened" };
    if let Some(i) = mine.iter().copied().find(|i| clean(i)) {
        return format!(
            "The oldest {} tab{whose} is {i} ({}): `select_tab index={i}` then `close` makes room.",
            state(&docs[i]),
            name(&docs[i]),
        );
    }
    if !mine.is_empty() {
        return if person {
            "Every open tab has unsaved changes; save one before closing it.".to_string()
        } else {
            "Every tab you opened has unsaved changes; `save` or `save_as` one of them before \
             closing it, or ask the person to close one of theirs."
                .to_string()
        };
    }
    match all.into_iter().find(|i| clean(i)) {
        Some(i) => format!(
            "None of the open tabs were opened by you; the person can close one (oldest {}: {i} {}).",
            state(&docs[i]),
            name(&docs[i]),
        ),
        None => "None of the open tabs were opened by you, and every one has unsaved changes; the \
                 person decides which to save and close."
            .to_string(),
    }
}

/// Refuse an agent's `close` of a tab the person opened that holds unsaved changes, or changes
/// `undo` has set aside.
///
/// Closing it would not lose the text by itself, since `close` asks first and `discard` is
/// `sensitive`, but the question lands in the person's window over their own work, and the
/// agent's `save` answers it by writing the person's draft to their file. Nor is a tab clean
/// because it matches its file: `undo` (standard) walks it back to the saved text with the
/// person's typing held only in `redo`, and a `close` then threw that away unasked (security
/// review of #620). A saved person tab with nothing set aside is not held here: the Mind keeps
/// its own hold on that (VM 520, 4 October).
pub fn may_close(tab: &Document) -> Result<(), String> {
    if agent_is_calling() && tab.opened_by == Opener::Person && (tab.dirty() || !tab.redo.is_empty()) {
        return Err("The tab in front was opened by the person and has changes that are not \
                    saved, or that `undo` set aside; an agent does not close it. `select_tab` one \
                    of yours, or leave it to the person."
            .into());
    }
    Ok(())
}

fn state(d: &Document) -> &'static str {
    if d.path.is_some() {
        "saved"
    } else {
        "empty"
    }
}

/// A tab's name as anyone may read it. The refusal lands in `notice`, which `describe` shows to
/// whoever reads next, so a tab an agent is not shown is not named in it either (#443).
fn name(d: &Document) -> String {
    match crate::agent_rule::hidden_from_agents(d.path.as_deref()) {
        Some(_) => crate::HIDDEN_TAB.to_string(),
        None => d.title(),
    }
}
