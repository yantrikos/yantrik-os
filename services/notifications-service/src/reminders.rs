//! "Standup in 10 minutes."
//!
//! ## Why the calendar's reminder timer runs here
//!
//! A reminder set for tomorrow morning has to fire tomorrow morning, whether or not anybody
//! opens the calendar between now and then. The timer used to live in the calendar service,
//! which is started on demand (`autostart = false` in its manifest) and stopped freely — so on a
//! machine where nothing had touched the calendar since boot, nothing was ticking and the event
//! passed unannounced (#78). This service is autostarted by the shell and stays up for the whole
//! session, which is exactly the property a timer needs, and it is where the announcement was
//! heading anyway: reminders were already RPC calls into this store.
//!
//! The calendar service still owns the events — that rule does not move. This timer reads the
//! event files where that service writes them and never writes one; the only file it keeps is
//! its own record of what has been said.
//!
//! ## What it does not promise
//!
//! It runs while this service runs, which is while the session is up. A machine with no desktop
//! session has nothing to announce to and nothing ticking — stated here rather than left to be
//! discovered, because a reminder that silently does not happen is worse than no reminder.
//!
//! One reminder per event per start time: the event's own `reminder_minutes` before it starts,
//! once, and not at all once it is `STALE` past.
//!
//! ## Times are local, and naive
//!
//! `parse_stamp` reads `2026-03-18T10:00:00` — no timezone, no offset. So the comparison is
//! against `Local::now().naive_local()`, never `Utc::now()`. Comparing a naive local start
//! against UTC would move every reminder by the machine's offset, which on this machine is five
//! and a half hours.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Local, NaiveDateTime};
use yantrik_ipc_contracts::calendar::{parse_stamp, CalendarEvent};
use yantrik_ipc_contracts::notifications::{AddRequest, Source, Urgency};

use crate::store::Store;

/// How often the calendar is re-read. Well inside the shortest sensible lead, so a reminder is
/// never more than half a minute late, and cheap: one `stat` per file plus a parse.
const TICK: Duration = Duration::from_secs(30);

/// How late is too late. An event whose start has passed by more than this is not announced at
/// all — a machine woken from suspend should not open with six reminders for meetings that are
/// over.
const STALE: ChronoDuration = ChronoDuration::minutes(5);

/// Start the reminder thread. Never returns; call it from `main` before the server blocks.
pub fn spawn(store: Arc<Store>, dir: PathBuf) {
    let spawned = std::thread::Builder::new()
        .name("calendar-reminders".into())
        .spawn(move || run(store, dir));
    if let Err(e) = spawned {
        tracing::error!(error = %e, "could not start the calendar reminder thread; no event will be announced");
    }
}

/// Where the calendar service stores its events. This must agree with that service's own
/// `calendar_dir()` — it is the other end of a file handoff, not a choice this module gets to
/// make. Note it is not the XDG rule this service's own store follows: the calendar's fallback
/// when there is no `$HOME` is `/tmp/yantrik-calendar`, and reading anywhere else would mean
/// silently reminding about a different calendar than the one being written.
pub fn calendar_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".local/share/yantrik/calendar")
    } else {
        PathBuf::from("/tmp/yantrik-calendar")
    }
}

fn run(store: Arc<Store>, dir: PathBuf) {
    let mut sent = Sent::load(&dir);
    tracing::info!(calendar = %dir.display(), "calendar reminders are running");
    loop {
        std::thread::sleep(TICK);
        let now = Local::now().naive_local();
        for due in due(&read_events(&dir), now) {
            if !sent.record(&due.id, &due.start) {
                continue;
            }
            let minutes = (due.start - now).num_minutes().max(0);
            let when = match minutes {
                0 => "now".to_string(),
                1 => "in a minute".to_string(),
                m => format!("in {m} minutes"),
            };
            tracing::info!(id = %due.id, title = %due.title, %when, "announcing an event");
            announce(
                &store,
                &format!("{} {when}", due.title),
                &due.start.format("Starts at %H:%M on %A %e %B").to_string(),
            );
        }
        sent.prune(now);
        sent.save();
    }
}

/// Every stored event. A file that will not read or parse is skipped, the way the calendar
/// service's own listing skips one: a corrupt file is a problem to surface where the events are
/// owned, but it must not stop the reminders for every other event.
fn read_events(dir: &Path) -> Vec<CalendarEvent> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .filter_map(|p| {
            std::fs::read_to_string(p)
                .ok()
                .and_then(|data| serde_json::from_str(&data).ok())
        })
        .collect()
}

/// One event whose reminder has come due: what to announce and when it starts.
struct Due {
    id: String,
    title: String,
    start: NaiveDateTime,
}

/// The pure decision — which events get announced at this instant. Takes `now` so the whole
/// rule is testable without a clock, a thread or a wait.
///
/// An event is due once its own lead has passed (`reminder_minutes` before the start — the
/// field defaults on read, so an event stored before per-event reminders existed keeps the ten
/// minutes it always had) and stays due until `STALE` past its start. All-day events are never
/// due: there is no time of day to announce them at. An event whose start will not parse cannot
/// be placed in time and is skipped, not guessed at.
fn due(events: &[CalendarEvent], now: NaiveDateTime) -> Vec<Due> {
    events
        .iter()
        .filter(|e| !e.is_all_day)
        .filter_map(|e| {
            let start = parse_stamp(&e.start)?;
            let lead = ChronoDuration::minutes(i64::from(e.reminder_minutes));
            (start <= now + lead && start >= now - STALE).then_some(Due {
                id: e.id.clone(),
                title: e.title.clone(),
                start,
            })
        })
        .collect()
}

/// Put one reminder into the store this process owns.
///
/// A direct call, not a socket RPC: the timer moved into this service precisely so there is no
/// "if the notifications service cannot be reached" any more — the store is right here, and a
/// reminder cannot be lost to a service that happens to be down. `sender: None`, like the
/// freedesktop door: nothing crossed a socket, so there are no peer credentials to attribute it
/// to, and the row says what the machine can actually stand behind.
fn announce(store: &Store, title: &str, body: &str) {
    store.add_from(
        AddRequest {
            app: "Calendar".to_string(),
            title: title.to_string(),
            body: body.to_string(),
            urgency: Urgency::Normal,
            source: Source::Yantrik,
            ..Default::default()
        },
        None,
    );
}

// ── Remembering what has been said ──────────────────────────────────────────────────────────

/// Which events have already been announced, keyed by id **and** start time.
///
/// Both, because an event that is moved is news again: the same id at a new time has not been
/// announced. And on disk, in the calendar's own directory, because this service restarts with
/// the session — with the set in memory only, a restart inside an event's lead would announce
/// the same meeting again. The file and its name are unchanged from when the timer lived in the
/// calendar service, so an upgrade does not re-announce everything already said.
struct Sent {
    path: PathBuf,
    /// id → the start that was announced.
    seen: BTreeMap<String, String>,
    dirty: bool,
}

impl Sent {
    fn load(dir: &Path) -> Self {
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
            parse_stamp(start).map(|s| s > cutoff).unwrap_or(false)
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
    use yantrik_ipc_contracts::calendar::DEFAULT_REMINDER_MINUTES;

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

    /// A fixed instant. The tests hand the clock to `due` rather than reading one, so none of
    /// them sleeps and none of them cares when it runs.
    fn nine_am() -> NaiveDateTime {
        parse_stamp("2026-03-18T09:00:00").unwrap()
    }

    fn at(now: NaiveDateTime, minutes: i64) -> String {
        (now + ChronoDuration::minutes(minutes))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string()
    }

    fn event(id: &str, title: &str, start: &str, reminder_minutes: u32) -> CalendarEvent {
        CalendarEvent {
            id: id.into(),
            title: title.into(),
            description: String::new(),
            start: start.into(),
            end: start.into(),
            is_all_day: false,
            location: None,
            attendees: Vec::new(),
            recurrence: None,
            calendar_id: "default".into(),
            remote_id: None,
            creator: None,
            reminder_minutes,
        }
    }

    fn due_ids(events: &[CalendarEvent], now: NaiveDateTime) -> Vec<String> {
        due(events, now).into_iter().map(|d| d.id).collect()
    }

    #[test]
    fn an_event_is_announced_its_own_lead_before_it_starts() {
        let now = nine_am();
        let events = vec![
            // An hour's warning, asked for, fifty minutes out: inside its own lead.
            event("early-bird", "Flight", &at(now, 50), 60),
            // Fifty minutes out with the default lead: not yet.
            event("default", "Standup", &at(now, 50), DEFAULT_REMINDER_MINUTES),
            // Eight minutes out with the default lead: due.
            event("soon", "Tea", &at(now, 8), DEFAULT_REMINDER_MINUTES),
            // Zero is a legal lead: announced from the moment it starts (until STALE).
            event("at-start", "Bell", &at(now, 0), 0),
            // One minute away, no lead wanted: not due yet.
            event("no-lead", "Silent", &at(now, 1), 0),
        ];
        let mut due = due_ids(&events, now);
        due.sort();
        assert_eq!(due, vec!["at-start", "early-bird", "soon"]);
    }

    #[test]
    fn an_event_file_from_before_per_event_reminders_keeps_its_ten_minutes() {
        // The field defaults on read: an event stored before #78 was fixed gets the lead the
        // one fixed timer always gave it, not zero and not a parse failure.
        let dir = temp_dir();
        let now = nine_am();
        std::fs::write(
            dir.join("old.json"),
            format!(
                r#"{{"id":"old","title":"Old event","description":"","start":"{}","end":"{}","is_all_day":false,"location":null,"attendees":[],"recurrence":null,"calendar_id":"default","remote_id":null}}"#,
                at(now, 9),
                at(now, 39)
            ),
        )
        .unwrap();
        let events = read_events(&dir);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].reminder_minutes, DEFAULT_REMINDER_MINUTES);
        assert_eq!(due_ids(&events, now), vec!["old"], "inside ten minutes: due");
        assert!(due_ids(&events, now - ChronoDuration::minutes(5)).is_empty(), "outside: not yet");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_event_that_started_long_ago_is_not_announced_after_a_resume() {
        // A machine waking from suspend must not open with reminders for meetings that are over.
        let now = nine_am();
        let over = event("over", "Over", &at(now, -30), DEFAULT_REMINDER_MINUTES);
        assert!(due_ids(&[over], now).is_empty());
        // Just inside STALE, though — an event that started four minutes ago is late news,
        // still worth one announcement.
        let late = event("late", "Late", &at(now, -4), DEFAULT_REMINDER_MINUTES);
        assert_eq!(due_ids(&[late], now), vec!["late"]);
    }

    #[test]
    fn an_all_day_event_is_never_announced() {
        let now = nine_am();
        let mut holiday = event("holiday", "Holiday", &at(now, 5), DEFAULT_REMINDER_MINUTES);
        holiday.is_all_day = true;
        assert!(due_ids(&[holiday], now).is_empty(), "there is no time of day to announce it at");
    }

    #[test]
    fn an_event_whose_start_will_not_parse_is_skipped_rather_than_guessed_at() {
        let now = nine_am();
        let broken = event("broken", "Broken", "not a time", DEFAULT_REMINDER_MINUTES);
        assert!(due_ids(&[broken], now).is_empty());
    }

    #[test]
    fn a_corrupt_event_file_does_not_stop_the_other_reminders() {
        let dir = temp_dir();
        let now = nine_am();
        std::fs::write(dir.join("junk.json"), "{not json").unwrap();
        std::fs::write(
            dir.join("good.json"),
            serde_json::to_string(&event("good", "Good", &at(now, 5), DEFAULT_REMINDER_MINUTES))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(due_ids(&read_events(&dir), now), vec!["good"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_event_is_announced_once_and_again_if_it_is_moved() {
        let dir = temp_dir();
        let now = nine_am();
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
        // The service restarts with the session. With the set in memory only, every restart
        // inside an event's lead would announce the same meeting again.
        let dir = temp_dir();
        let now = nine_am();
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
        let now = nine_am();
        let mut sent = Sent::load(&dir);
        sent.record("old", &(now - ChronoDuration::days(3)));
        sent.record("recent", &(now + ChronoDuration::minutes(5)));
        sent.prune(now);
        assert!(!sent.seen.contains_key("old"));
        assert!(sent.seen.contains_key("recent"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_reminder_lands_in_the_store_this_process_owns() {
        // The whole point of the move: no socket between the timer and the store, so a
        // reminder cannot be lost to a service that is not running.
        let dir = temp_dir();
        let store = Store::open(dir.join("notifications.json"));
        announce(&store, "Standup in 9 minutes", "Starts at 09:09 on Wednesday");
        let list = store.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].app, "Calendar");
        assert_eq!(list[0].title, "Standup in 9 minutes");
        assert_eq!(list[0].urgency, Urgency::Normal);
        let _ = std::fs::remove_dir_all(dir);
    }
}
