//! "Prepared for …": the card the desktop leaves when a "Prepare for me" job has finished (or
//! been stopped or capped), one per item (#1416, slice 5, round 2).
//!
//! The shell's job writes the line ("Prepared for 'Record the 60 s demo video': shot list,
//! script, 5 sources"), the folder, and the title of the note it made. This makes the card and
//! the store grants its attention through the same path as every other card (`add_os_notice_at`,
//! family `discovery`): quiet hours, the person's On / Inbox only / Off for the family and the
//! day's two interrupts all apply. The buttons are the card's: **Open notes** (the note it made),
//! **Open folder** and **Discard**, and "Why you got this". The service stores and shows words;
//! the shell carries out the presses, and checks the folder against its own ledger when it does.
//!
//! Nothing is posted when "Prepare for me" is off.

use chrono::{DateTime, Local, Utc};
use yantrik_ipc_contracts::notifications::opt_in::prepare_for_me_on;

use crate::store::Store;

const NAME: &str = "Yantrik";
const MAX_ITEM_ID: usize = 64;

/// What the shell says it made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    pub item_id: String,
    /// "Prepared for 'X': shot list, script, 5 sources".
    pub line: String,
    /// `~/Documents/Prepared/<day>-<name>`, absolute.
    pub folder: String,
    /// The note it made ("Prepared: X"), empty when the note was not made.
    pub note_title: String,
    pub why: String,
}

impl Done {
    pub fn parse(params: &serde_json::Value) -> Result<Done, String> {
        let text = |k: &str| params[k].as_str().unwrap_or_default().trim().to_string();
        let item_id = text("item_id");
        if item_id.is_empty() || item_id.len() > MAX_ITEM_ID || !item_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err("`item_id` is not an id the calendar issues".into());
        }
        let (line, folder, why) = (text("line"), text("folder"), text("why"));
        if line.is_empty() || why.is_empty() {
            return Err("`line` and `why` are needed".into());
        }
        if !folder.starts_with('/') || folder.contains("..") || !folder.contains("/Prepared/") {
            return Err("`folder` is a folder under Prepared".into());
        }
        Ok(Done { item_id, line, folder, note_title: text("note_title"), why })
    }
}

/// The card's id: one per item, so a repeat is the same card.
pub fn notice_id(item: &str) -> String {
    format!("prepared:{item}")
}

pub fn announce(store: &Store, done: &Done, now: DateTime<Local>) -> serde_json::Value {
    if !prepare_for_me_on(&store.initiative()) {
        return serde_json::json!({ "stored": false, "refused": "Prepare for me is off" });
    }
    let name = done.folder.rsplit('/').next().unwrap_or_default();
    let mut notice = serde_json::json!({
        "notice_id": notice_id(&done.item_id),
        "family": "discovery",
        "title": done.line,
        "body": format!("Drafts are in Documents/Prepared/{name}. Nothing was sent, posted or changed."),
        "why": done.why,
        "attention": "interrupt",
        // The earlier "prepare for what's next" card for this item is answered by this one.
        "supersedes": format!("prep:{}", done.item_id),
        "more": [
            { "kind": "open_folder", "ref": done.folder, "label": "Open folder" },
            { "kind": "discard_prepared", "ref": done.folder, "label": "Discard" },
        ],
    });
    if !done.note_title.is_empty() {
        notice["action"] = serde_json::json!({ "kind": "open_note", "ref": done.note_title, "label": "Open notes" });
    }
    store.add_os_notice_at(NAME, notice, now.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_ipc_contracts::notifications::notice::ActionKind;
    use yantrik_ipc_contracts::notifications::opt_in::set_prepare_for_me;

    fn test_store() -> Store {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!("prepared-notice-{}-{}.json", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_file(&path);
        Store::open(path)
    }

    fn done() -> Done {
        Done {
            item_id: "e1".into(),
            line: "Prepared for \u{2018}Record the 60 s demo video\u{2019}: shot list, script, 5 sources".into(),
            folder: "/home/p/Documents/Prepared/2026-11-26-record-the-60-s-demo-video".into(),
            note_title: "Prepared: Record the 60 s demo video".into(),
            why: "You turned on Prepare for me, and this is due soon.".into(),
        }
    }

    #[test]
    fn the_request_is_read_and_a_bad_folder_is_refused() {
        let p = serde_json::json!({ "item_id": "e1", "line": "L", "folder": "/home/p/Documents/Prepared/x", "why": "w" });
        assert!(Done::parse(&p).is_ok());
        for folder in ["/home/p/Documents/mine", "Prepared/x", "/home/p/Documents/Prepared/../x", ""] {
            let mut bad = p.clone();
            bad["folder"] = folder.into();
            assert!(Done::parse(&bad).is_err(), "{folder}");
        }
        let mut bad = p.clone();
        bad["item_id"] = "../x".into();
        assert!(Done::parse(&bad).is_err());
    }

    #[test]
    fn off_by_default_so_nothing_is_stored() {
        let store = test_store();
        assert_eq!(announce(&store, &done(), Local::now())["stored"], false);
        assert!(store.list().is_empty());
    }

    #[test]
    fn the_card_says_what_was_prepared_and_has_open_notes_open_folder_and_discard_once_per_item() {
        let store = test_store();
        store.set_initiative(|s| { set_prepare_for_me(s, true); Ok(()) }).unwrap();
        let now = Local::now();
        let answer = announce(&store, &done(), now);
        assert_eq!(answer["stored"], true, "{answer}");
        let (title, notice) = store.list().into_iter().find_map(|n| n.notice.map(|c| (n.title, c))).expect("a card");
        assert_eq!(title, done().line);
        assert!(!notice.why.is_empty(), "Why you got this");
        assert_eq!(notice.action.as_ref().map(|a| (a.kind.clone(), a.label.as_str())), Some((ActionKind::OpenNote, "Open notes")));
        let more: Vec<(ActionKind, &str)> = notice.more.iter().map(|a| (a.kind.clone(), a.label.as_str())).collect();
        assert_eq!(more, [(ActionKind::OpenFolder, "Open folder"), (ActionKind::DiscardPrepared, "Discard")]);
        assert!(notice.more.iter().all(|a| a.reference == done().folder));
        assert_eq!(announce(&store, &done(), now)["duplicate"], true, "once per item");
    }
}
