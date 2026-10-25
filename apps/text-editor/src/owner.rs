//! Who opened each tab, and which tab a caller is told to close when there is no room for more.
//!
//! VM 520, 4 October 2026: a mind hit "Eight tabs are already open", then ran `close` on the tab
//! in front to make room. That tab was the person's: opened and saved by them, not the mind.
//! Nothing was lost because it was saved, but the refusal had told the mind only "close one",
//! and nothing anywhere said whose tabs were whose. So every tab records who opened it,
//! `describe` says whether it was the caller, and the refusal names one tab the caller may
//! close: the oldest one IT opened with nothing unsaved.
//!
//! The runtime hands a handler an agent's token and nothing else about it, so `opened_by` says
//! only `"agent"`; a digest of the token, never the token, is kept beside it, which is what
//! `opened_by_you` and the refusal compare, so one agent's tabs are told from another's without
//! anyone knowing a name.

use yantrik_ipc_transport::reach::token_digest;

use crate::text::budget::{MAX_OPEN_BYTES, MAX_TABS};
use crate::text::limits::MAX_BYTES;

/// Who opened a tab. Never written anywhere: a recovered tab is the person's, and an `opened_by`
/// read back from a draft could be forged to `agent` to take the close protection off a person's
/// draft (security review of #620).
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
    /// The opener a call makes: an agent's, by the digest of the token it carried, or the person.
    pub fn of(agent: bool, token: Option<String>) -> Opener {
        if !agent {
            return Opener::Person;
        }
        Opener::Agent { token: token.map(|t| token_digest(&t)) }
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
    /// that carried no token, which is every `app.describe` from the mind account: there the
    /// answer is not known, and is not guessed.
    pub fn owns(&self, tab: &Opener) -> Option<bool> {
        match (self, tab) {
            (Opener::Agent { token: None }, _) => None,
            (Opener::Person, Opener::Person) => Some(true),
            (Opener::Agent { token: Some(me) }, Opener::Agent { token: Some(it) }) => Some(me == it),
            _ => Some(false),
        }
    }
}

/// What the refusals here need to know of one open tab.
#[derive(Debug, Clone)]
pub struct TabFacts {
    pub modified: bool,
    pub has_file: bool,
    pub opened_by: Opener,
    /// Its name as anyone may read it: `(hidden)` for a file no agent is shown, since the
    /// refusal lands in the notice, which `describe` shows to whoever reads next (#443).
    pub name: String,
}

/// The refusal to open or create one more tab, naming the one call that makes room for `me`.
pub fn no_room(tabs: &[TabFacts], me: &Opener) -> String {
    let full = if tabs.len() >= MAX_TABS {
        format!("{MAX_TABS} tabs are already open, the most the editor keeps; close one before opening another.")
    } else {
        format!(
            "The open tabs would hold more than the {} MiB of text the editor keeps open at once; \
             close one before opening another.",
            MAX_OPEN_BYTES / MAX_BYTES
        )
    };
    format!("{full} {}", room_hint(tabs, me))
}

/// The refusal of an edit that would take the open tabs past the budget.
pub fn too_big(tabs: &[TabFacts], me: &Opener) -> String {
    format!(
        "That would take the open tabs past the {} MiB of text the editor keeps open at once; the \
         tab is unchanged. {}",
        MAX_OPEN_BYTES / MAX_BYTES,
        room_hint(tabs, me)
    )
}

/// Which tab `me` can close to make room, as the call that closes it.
///
/// A tab is only ever named when closing it loses nothing (saved to its file, or empty and never
/// written); a modified tab is never suggested. Tabs are kept in the order they were opened, so
/// the lowest index is the oldest. An agent is pointed only at its own tabs; with none, it is told
/// so and the tab is named as one the person could close. The person may close any tab.
fn room_hint(tabs: &[TabFacts], me: &Opener) -> String {
    let clean = |i: &usize| !tabs[*i].modified;
    let person = matches!(me, Opener::Person);
    let mine: Vec<usize> =
        (0..tabs.len()).filter(|i| person || me.owns(&tabs[*i].opened_by) == Some(true)).collect();
    let whose = if person { "" } else { " you opened" };
    if let Some(i) = mine.iter().copied().find(|i| clean(i)) {
        return format!(
            "The oldest {} tab{whose} is {i} ({}): `select_tab index={i}` then `close` makes room.",
            state(&tabs[i]),
            tabs[i].name,
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
    match (0..tabs.len()).find(|i| clean(i)) {
        Some(i) => format!(
            "None of the open tabs were opened by you; the person can close one (oldest {}: {i} {}).",
            state(&tabs[i]),
            tabs[i].name,
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
/// because it matches its file: `undo` walks it back to the saved text with the person's typing
/// held only in `redo`, and a `close` then threw that away unasked (security review of #620).
pub fn may_close(agent: bool, tab: &TabFacts, redo_held: bool) -> Result<(), String> {
    if agent && tab.opened_by == Opener::Person && (tab.modified || redo_held) {
        return Err("The tab in front was opened by the person and has changes that are not \
                    saved, or that `undo` set aside; an agent does not close it. `select_tab` one \
                    of yours, or leave it to the person."
            .into());
    }
    Ok(())
}

fn state(t: &TabFacts) -> &'static str {
    if t.has_file {
        "saved"
    } else {
        "empty"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(has_file: bool, modified: bool, by: &Opener, name: &str) -> TabFacts {
        TabFacts { modified, has_file, opened_by: by.clone(), name: name.into() }
    }

    fn agent(token: &str) -> Opener {
        Opener::of(true, Some(token.into()))
    }

    #[test]
    fn opened_by_you_is_true_only_for_the_caller_that_opened_the_tab() {
        let (a, b) = (agent("tok-a"), agent("tok-b"));
        assert_eq!(a.owns(&a), Some(true));
        assert_eq!(a.owns(&b), Some(false));
        assert_eq!(a.owns(&Opener::Person), Some(false));
        assert_eq!(Opener::Person.owns(&Opener::Person), Some(true));
        assert_eq!(Opener::Person.owns(&a), Some(false));
        assert_eq!(Opener::of(true, None).owns(&a), None, "a describe with no token is not guessed");
        assert!(!format!("{a:?}").contains("tok-a"), "only a digest of the token is kept");
    }

    #[test]
    fn the_refusal_names_the_callers_oldest_saved_tab_and_never_a_modified_one() {
        let me = agent("tok-me");
        let tabs = vec![
            tab(true, false, &Opener::Person, "person.txt"),
            tab(true, true, &me, "mine-modified.txt"),
            tab(true, false, &me, "mine.txt"),
        ];
        let said = no_room(&tabs, &me);
        assert!(said.contains("`select_tab index=2` then `close`"), "{said}");
        assert!(!said.contains("person.txt") && !said.contains("mine-modified"), "{said}");
        let said = no_room(&tabs, &Opener::Person);
        assert!(said.contains("index=0"), "the person may close any: {said}");
    }

    #[test]
    fn an_agent_that_opened_no_tab_is_told_so_and_shown_what_the_person_could_close() {
        let tabs = vec![tab(true, true, &Opener::Person, "a"), tab(true, false, &Opener::Person, "b")];
        let said = too_big(&tabs, &agent("tok-x"));
        assert!(said.contains("None of the open tabs were opened by you"), "{said}");
        assert!(said.contains("1 b"), "{said}");
        let all_modified = vec![tab(true, true, &Opener::Person, "a")];
        assert!(no_room(&all_modified, &agent("t")).contains("the person decides"));
    }

    #[test]
    fn an_agent_does_not_close_the_persons_unsaved_tab() {
        let t = tab(true, true, &Opener::Person, "a");
        assert!(may_close(true, &t, false).is_err());
        assert!(may_close(false, &t, false).is_ok(), "the person may");
        let saved = tab(true, false, &Opener::Person, "a");
        assert!(may_close(true, &saved, true).is_err(), "redo holds the person's typing");
        assert!(may_close(true, &saved, false).is_ok());
    }
}
