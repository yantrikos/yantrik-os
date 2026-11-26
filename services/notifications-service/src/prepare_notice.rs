//! "Prepare for what's next": the card the desktop offers when an event is soon and a note about
//! it still has things to do (`design`: #1416, the agenda's first anticipate rule).
//!
//! The shell's agenda decides *whether* (a linked note with open items, an event within 36 hours,
//! once per event) and says what it found in titles, counts and times. This file makes the card
//! and the store grants its attention through the same path as every other card
//! (`Store::add_os_notice_at`, family `discovery`): so quiet initiative, quiet hours, the person's
//! On / Inbox only / Off for the family, and the day's two interrupts, shared with every other
//! moment and mind, all apply. Over the budget, the card waits in the inbox. A card that is not
//! seen before the event starts is no longer useful (`useful_until`).
//!
//! Switched off in Settings → Minds → Initiative ("prepare"): nothing is stored.

use chrono::{DateTime, Local, NaiveDate, SecondsFormat, TimeZone, Utc};
use yantrik_ipc_contracts::notifications::notice::MOMENT_PREPARE;

use crate::store::Store;

/// What the card is filed as.
const NAME: &str = "Yantrik";
const MAX_EVENT_ID: usize = 64;

/// What the shell says it found: titles, counts, a time, and its own reason in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ready {
    pub event_id: String,
    pub title: String,
    /// The event's start, unix seconds.
    pub start: i64,
    pub note_title: String,
    pub open_items: u32,
    pub why: String,
    /// What the Ask button puts in the Lens.
    pub ask: String,
}

impl Ready {
    /// Read the shell's request. A fault here is the shell's, so it is refused, not retried.
    pub fn parse(params: &serde_json::Value) -> Result<Ready, String> {
        let text = |k: &str| params[k].as_str().unwrap_or_default().trim().to_string();
        let event_id = text("event_id");
        let ok = !event_id.is_empty() && event_id.len() <= MAX_EVENT_ID && event_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if !ok {
            return Err("`event_id` is not an id the calendar issues".into());
        }
        let (title, note_title, why) = (text("title"), text("note_title"), text("why"));
        if title.is_empty() || note_title.is_empty() || why.is_empty() {
            return Err("`title`, `note_title` and `why` are needed".into());
        }
        let start = params["start"].as_i64().ok_or("`start` is the event's start in unix seconds")?;
        let open_items = params["open_items"].as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
        Ok(Ready { event_id, title, start, note_title, open_items, why, ask: text("ask") })
    }
}

/// The card's id: one per event.
pub fn notice_id(event: &str) -> String {
    format!("prep:{event}")
}

/// "Today 15:00", "Tomorrow 15:00", or the weekday and time.
fn when(start: DateTime<Local>, today: NaiveDate) -> String {
    let day = match (start.date_naive() - today).num_days() {
        0 => "Today".to_string(),
        1 => "Tomorrow".to_string(),
        _ => start.format("%A").to_string(),
    };
    format!("{day} {}", start.format("%H:%M"))
}

/// Make the card for `ready` at `now`, and answer as the store does.
pub fn announce(store: &Store, ready: &Ready, now: DateTime<Local>) -> serde_json::Value {
    if !store.initiative().moment_on(MOMENT_PREPARE) {
        return serde_json::json!({ "stored": false, "refused": "the person turned this moment off" });
    }
    let Some(starts) = Utc.timestamp_opt(ready.start, 0).single().map(|t| t.with_timezone(&Local)) else {
        return serde_json::json!({ "stored": false, "refused": "`start` is not a time" });
    };
    if starts <= now {
        return serde_json::json!({ "stored": false, "refused": "the event has started" });
    }
    let items = format!("{} open item{}", ready.open_items, if ready.open_items == 1 { "" } else { "s" });
    let mut notice = serde_json::json!({
        "notice_id": notice_id(&ready.event_id),
        "family": "discovery",
        "title": format!("{}: {}", when(starts, now.date_naive()), ready.title),
        "body": format!("\u{201c}{}\u{201d} has {items}.", ready.note_title),
        "why": ready.why,
        "attention": "interrupt",
        "useful_until": starts.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Secs, true),
        "action": { "kind": "open_note", "ref": ready.note_title, "label": format!("Open {}", ready.note_title) },
    });
    if !ready.ask.is_empty() {
        notice["ask"] = serde_json::json!({ "kind": "ask_mind", "ref": ready.ask, "label": "Ask Pi for a shot list" });
    }
    store.add_os_notice_at(NAME, notice, now.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> Store {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!("prepare-notice-{}-{}.json", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_file(&path);
        Store::open(path)
    }

    fn ready() -> Ready {
        Ready {
            event_id: "e1".into(),
            title: "Record the 60 s demo video".into(),
            start: 0,
            note_title: "Launch prep".into(),
            open_items: 4,
            why: "Your note has 4 open items and matches this event.".into(),
            ask: "Make a shot list for \u{2018}Record the 60 s demo video\u{2019} from my note \u{2018}Launch prep\u{2019}".into(),
        }
    }

    #[test]
    fn the_request_is_read_and_a_bad_one_is_refused() {
        let p = serde_json::json!({ "event_id": "e1", "title": "T", "start": 5, "note_title": "N", "open_items": 4, "why": "w", "ask": "a" });
        assert_eq!(Ready::parse(&p).unwrap().open_items, 4);
        for bad in [serde_json::json!({ "event_id": "../x", "title": "T", "start": 5, "note_title": "N", "why": "w" }), serde_json::json!({ "event_id": "e1", "title": "T", "note_title": "N", "why": "w" }), serde_json::json!({ "event_id": "e1", "title": "", "start": 5, "note_title": "N", "why": "w" })] {
            assert!(Ready::parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_day_is_said_in_words() {
        let now = Local.with_ymd_and_hms(2026, 11, 26, 9, 0, 0).unwrap();
        let tomorrow = Local.with_ymd_and_hms(2026, 11, 27, 15, 0, 0).unwrap();
        assert_eq!(when(tomorrow, now.date_naive()), "Tomorrow 15:00");
        assert_eq!(when(now, now.date_naive()), "Today 09:00");
    }

    #[test]
    fn the_card_names_the_event_the_note_and_two_buttons_and_never_sends_anything() {
        let store = test_store();
        let now = Local::now();
        let mut r = ready();
        r.start = (now + chrono::Duration::hours(20)).timestamp();
        let answer = announce(&store, &r, now);
        assert_eq!(answer["stored"], true, "{answer}");
        let card = store.list().into_iter().find_map(|n| n.notice.map(|c| (n.title, n.body, c))).expect("a card");
        assert!(card.0.contains("Record the 60 s demo video"), "{}", card.0);
        assert!(card.1.contains("Launch prep") && card.1.contains("4 open items"), "{}", card.1);
        let notice = card.2;
        assert_eq!(notice.action.as_ref().map(|a| a.reference.as_str()), Some("Launch prep"));
        let ask = notice.ask.expect("the Lens button");
        assert_eq!(ask.kind, yantrik_ipc_contracts::notifications::notice::ActionKind::AskMind);
        assert!(ask.reference.contains("Launch prep"));
        assert!(!notice.why.is_empty(), "Why you got this");
        // The same event again is the same card.
        assert_eq!(announce(&store, &r, now)["duplicate"], true);
    }

    #[test]
    fn only_two_interrupts_a_day_and_the_third_waits_in_the_inbox() {
        let store = test_store();
        let now = Local::now();
        let mut granted = Vec::new();
        for i in 0..3 {
            let mut r = ready();
            r.event_id = format!("e{i}");
            r.start = (now + chrono::Duration::hours(20)).timestamp();
            announce(&store, &r, now);
        }
        for n in store.list() {
            if let Some(c) = n.notice {
                granted.push(c.granted);
            }
        }
        use yantrik_ipc_contracts::notifications::notice::Attention;
        assert_eq!(granted.iter().filter(|g| **g == Attention::Interrupt).count(), 2, "{granted:?}");
        assert_eq!(granted.iter().filter(|g| **g == Attention::Inbox).count(), 1, "{granted:?}");
    }

    #[test]
    fn switched_off_nothing_is_stored_and_a_started_event_is_refused() {
        let store = test_store();
        let now = Local::now();
        let mut r = ready();
        r.start = (now + chrono::Duration::hours(20)).timestamp();
        store.set_initiative(|s| { s.set_moment(MOMENT_PREPARE, false); Ok(()) }).unwrap();
        assert!(announce(&store, &r, now)["refused"].is_string());
        assert!(store.list().is_empty());
        store.set_initiative(|s| { s.set_moment(MOMENT_PREPARE, true); Ok(()) }).unwrap();
        r.start = (now - chrono::Duration::hours(1)).timestamp();
        assert!(announce(&store, &r, now)["refused"].is_string());
    }
}
