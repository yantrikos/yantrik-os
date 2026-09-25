//! "Standup in 10 minutes."
//!
//! ## Why this is in the service and not in the app
//!
//! The calendar service owns events — that is the whole point of the one-owner rule this
//! calendar was rebuilt around. A reminder is a fact about an event, so it belongs where the
//! events are. In the app it would only fire while a window happened to be open, which is
//! exactly when a person least needs telling.
//!
//! ## What it does not promise
//!
//! **It only runs while this service runs.** The service is started on demand (`autostart =
//! false` in its manifest), so on a machine where nothing has touched the calendar since boot,
//! nothing is ticking and an event can pass unannounced. That is stated in `app.describe` rather
//! than left for somebody to discover, because a reminder that silently does not happen is worse
//! than no reminder.
//!
//! **One reminder per event, at one fixed lead.** There is no per-event alarm field on
//! `CalendarEvent` and nothing writes one, so inventing "reminders" as a feature here would be a
//! control with no data behind it. Ten minutes, once.
//!
//! ## Times are local, and naive
//!
//! `store::parse_iso_datetime` reads `2026-03-18T10:00:00` — no timezone, no offset. So the
//! comparison is against `Local::now().naive_local()`, never `Utc::now()`. Comparing a naive
//! local start against UTC would move every reminder by the machine's offset, which on this
//! machine is five and a half hours.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Local, NaiveDateTime};
use yantrik_ipc_contracts::calendar::EventsParams;

use crate::store::{parse_iso_datetime, EventStore};

/// How far ahead an event is announced.
pub const LEAD: ChronoDuration = ChronoDuration::minutes(10);

/// How often the store is re-read. Well inside the lead, so a reminder is never more than half a
/// minute late, and cheap: one `stat` per file plus a parse of whatever is in range.
const TICK: Duration = Duration::from_secs(30);

/// How late is too late. An event whose start has passed by more than this is not announced at
/// all — a machine woken from suspend should not open with six reminders for meetings that are
/// over.
const STALE: ChronoDuration = ChronoDuration::minutes(5);

/// Start the reminder thread. Never returns; call it from `main` before the server blocks.
pub fn spawn(dir: PathBuf) {
    let spawned = std::thread::Builder::new()
        .name("calendar-reminders".into())
        .spawn(move || run(dir));
    if let Err(e) = spawned {
        tracing::error!(error = %e, "could not start the calendar reminder thread; no event will be announced");
    }
}

fn run(dir: PathBuf) {
    let store = EventStore::new(dir.clone());
    let mut sent = Sent::load(&dir);
    tracing::info!(
        lead_minutes = LEAD.num_minutes(),
        "calendar reminders are running"
    );
    loop {
        std::thread::sleep(TICK);
        let now = Local::now().naive_local();
        for (id, start, title) in due(&store, now) {
            if !sent.record(&id, &start) {
                continue;
            }
            let minutes = (start - now).num_minutes().max(0);
            let when = match minutes {
                0 => "now".to_string(),
                1 => "in a minute".to_string(),
                m => format!("in {m} minutes"),
            };
            tracing::info!(id = %id, title = %title, %when, "announcing an event");
            announce(
                &format!("{title} {when}"),
                &start.format("Starts at %H:%M on %A %e %B").to_string(),
            );
        }
        sent.prune(now);
        sent.save();
    }
}

/// Put one reminder into the machine's one notification store.
///
/// A direct call rather than `yantrik_app_runtime::notify::send`, which would be one line
/// shorter and would link Slint into a headless service. Blocking is fine here: this is the
/// reminder thread and it has nothing else to do for the next thirty seconds.
///
/// If the notifications service cannot be reached the reminder is lost and said so in the log.
/// There is nowhere else to put it — queueing it here would mean announcing a meeting after it
/// had started, which is worse than not announcing it.
fn announce(title: &str, body: &str) {
    if let Err(e) = yantrik_ipc_transport::service::ensure("notifications") {
        tracing::warn!(error = %e, title, "could not reach the notifications service; this reminder is lost");
        return;
    }
    let call = yantrik_ipc_transport::SyncRpcClient::for_service("notifications")
        .with_timeout(Duration::from_secs(2))
        .call(
            "notifications.add",
            serde_json::json!({
                "app": "Calendar",
                "title": title,
                "body": body,
                "urgency": "normal",
            }),
        );
    if let Err(e) = call {
        tracing::warn!(error = %e.message, title, "the notifications service refused a reminder");
    }
}

/// Events starting within the lead and not already past. Returns `(id, start, title)`.
fn due(store: &EventStore, now: NaiveDateTime) -> Vec<(String, NaiveDateTime, String)> {
    // A window either side of now, asked for in the format the store parses. The store lists by
    // overlap, so this also picks up a long event that began earlier; the start-time filter
    // below is what keeps those out.
    let params = EventsParams {
        start_date: (now - STALE).format("%Y-%m-%dT%H:%M:%S").to_string(),
        end_date: (now + LEAD).format("%Y-%m-%dT%H:%M:%S").to_string(),
    };
    let events = match store.list(&params) {
        Ok(events) => events,
        Err(e) => {
            tracing::warn!(error = %e.message, "could not read the calendar for reminders");
            return Vec::new();
        }
    };
    events
        .into_iter()
        .filter(|e| !e.is_all_day)
        .filter_map(|e| {
            let start = parse_iso_datetime(&e.start)?;
            // Inside the lead, and not more than `STALE` in the past.
            (start <= now + LEAD && start >= now - STALE).then_some((e.id, start, e.title))
        })
        .collect()
}

// ── Remembering what has been said ──────────────────────────────────────────────────────────

/// Which events have already been announced, keyed by id **and** start time.
///
/// Both, because an event that is moved is news again: the same id at a new time has not been
/// announced. And on disk, because this service is started on demand and stopped freely — with
/// the set in memory only, every restart inside the ten-minute window would announce the same
/// meeting again.
struct Sent {
    path: PathBuf,
    /// id → the start that was announced.
    seen: BTreeMap<String, String>,
    dirty: bool,
}

impl Sent {
    fn load(dir: &PathBuf) -> Self {
        let path = dir.join(".reminded.json");
        let seen = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self {
            path,
            seen,
            dirty: false,
        }
    }

    /// Note that this event at this start is being announced. `false` if it already was.
    fn record(&mut self, id: &str, start: &NaiveDateTime) -> bool {
        let key = start.format("%Y-%m-%dT%H:%M:%S").to_string();
        if self.seen.get(id) == Some(&key) {
            return false;
        }
        self.seen.insert(id.to_string(), key);
        self.dirty = true;
        true
    }

    /// Forget anything whose start is well behind us, so the file does not grow forever.
    fn prune(&mut self, now: NaiveDateTime) {
        let cutoff = now - ChronoDuration::days(1);
        let before = self.seen.len();
        self.seen.retain(|_, start| {
            parse_iso_datetime(start).map(|s| s > cutoff).unwrap_or(false)
        });
        if self.seen.len() != before {
            self.dirty = true;
        }
    }

    fn save(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let Ok(json) = serde_json::to_string(&self.seen) else { return };
        // Temp and rename, so a service killed mid-write does not leave a file that will not
        // parse — which would make every reminder in the window fire a second time.
        let temp = self.path.with_extension(format!("tmp{}", std::process::id()));
        if std::fs::write(&temp, json).is_ok() && std::fs::rename(&temp, &self.path).is_err() {
            let _ = std::fs::remove_file(&temp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use yantrik_ipc_contracts::calendar::CreateEventParams;

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yantrik-reminders-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn at(now: NaiveDateTime, minutes: i64) -> String {
        (now + ChronoDuration::minutes(minutes))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string()
    }

    fn add(store: &EventStore, title: &str, start: String, end: String) -> String {
        store
            .create(&CreateEventParams {
                title: title.into(),
                start,
                end,
                description: String::new(),
                location: None,
                color: String::new(),
                is_all_day: false,
                attendees: Vec::new(),
                creator: None,
            })
            .expect("the event is stored")
            .id
    }

    #[test]
    fn an_event_inside_the_lead_is_due_and_one_outside_it_is_not() {
        let dir = temp_dir();
        let store = EventStore::new(dir.clone());
        let now = Local::now().naive_local();
        let soon = add(&store, "Standup", at(now, 8), at(now, 23));
        add(&store, "Next week", at(now, 60), at(now, 90));
        add(&store, "Yesterday", at(now, -600), at(now, -570));

        let due: Vec<String> = due(&store, now).into_iter().map(|(id, _, _)| id).collect();
        assert_eq!(due, vec![soon]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_event_that_started_long_ago_is_not_announced_after_a_resume() {
        // A machine waking from suspend must not open with reminders for meetings that are over.
        let dir = temp_dir();
        let store = EventStore::new(dir.clone());
        let now = Local::now().naive_local();
        add(&store, "Over", at(now, -30), at(now, -10));
        assert!(due(&store, now).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_event_is_announced_once_and_again_if_it_is_moved() {
        let dir = temp_dir();
        let now = Local::now().naive_local();
        let mut sent = Sent::load(&dir);
        let start = now + ChronoDuration::minutes(9);
        assert!(sent.record("e1", &start));
        assert!(!sent.record("e1", &start), "the same event at the same time is said once");
        let moved = now + ChronoDuration::minutes(40);
        assert!(sent.record("e1", &moved), "a moved event is news again");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn what_was_announced_survives_a_restart_of_the_service() {
        // The service is started on demand and stopped freely. With the set in memory only,
        // every restart inside the ten-minute window would announce the same meeting again.
        let dir = temp_dir();
        let now = Local::now().naive_local();
        let start = now + ChronoDuration::minutes(5);
        {
            let mut sent = Sent::load(&dir);
            assert!(sent.record("e1", &start));
            sent.save();
        }
        let mut reopened = Sent::load(&dir);
        assert!(!reopened.record("e1", &start));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn old_reminders_are_forgotten_so_the_file_does_not_grow_forever() {
        let dir = temp_dir();
        let now = Local::now().naive_local();
        let mut sent = Sent::load(&dir);
        sent.record("old", &(now - ChronoDuration::days(3)));
        sent.record("recent", &(now + ChronoDuration::minutes(5)));
        sent.prune(now);
        assert!(!sent.seen.contains_key("old"));
        assert!(sent.seen.contains_key("recent"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
