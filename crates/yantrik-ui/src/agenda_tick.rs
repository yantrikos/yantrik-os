//! The agenda's tick, called once from the shell's Think arm (`bridge.rs`, every minute).
//!
//! The brain crate owns the order and the refusals (`yantrik_companion::agenda::tick`); this file
//! reads the two sources it is handed (the calendar service, the Notes library's directory),
//! applies the two switches (Private mode, Settings → Initiative → Brain), and publishes what the
//! agenda looks like for `describe shell` (`agenda`). Titles and counts only.
//!
//! One loop: the shell's Think arm is the only caller. The headless `yantrik think` command
//! (`crates/yantrik/src/main.rs`) is the memory engine's own think cycle and has no agenda.

use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{Duration, Local, NaiveDate, NaiveDateTime, TimeZone};
use rusqlite::Connection;
use serde_json::{json, Value};
use yantrik_companion::agenda::senses::CalEvent;
use yantrik_companion::agenda::{self, Switches};
use yantrik_companion::calendar::backend::{CalendarBackend, ServiceCalendar};
use yantrik_ipc_contracts::calendar::EventsParams;
use yantrik_ipc_contracts::notifications::notice::MOMENT_BRAIN;

static SNAPSHOT: Mutex<Option<Value>> = Mutex::new(None);
/// When the last tick ran (unix seconds) and how many have run since the shell started.
static TICKS: Mutex<(f64, u64)> = Mutex::new((0.0, 0));
/// The calendar service gets this long on the companion's worker; a hung call must not stall
/// every later Think.
const CALENDAR_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

fn publish(v: Value) {
    *SNAPSHOT.lock().unwrap_or_else(|e| e.into_inner()) = Some(v);
}

/// What `describe shell` says under `agenda`.
pub fn for_describe() -> Value {
    let mut v = SNAPSHOT.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_else(|| json!({ "state": "not run yet" }));
    if let Some(o) = v.as_object_mut() {
        let (at, n) = *TICKS.lock().unwrap_or_else(|e| e.into_inner());
        o.insert("last_tick".into(), json!(at));
        o.insert("ticks".into(), json!(n));
        o.insert("prepared".into(), crate::prepare_work::for_describe()); // "Prepare for me" (#1416)
    }
    v
}

/// The Think arm's one call: the agenda's tick, then "Prepare for me". The agenda takes the
/// database for its own pass; the preparation takes it one statement at a time.
pub fn tick_all(companion: &yantrik_companion::CompanionService) {
    {
        let mut t = TICKS.lock().unwrap_or_else(|e| e.into_inner());
        *t = (Local::now().timestamp() as f64, t.1 + 1);
    }
    tick(&companion.db.conn());
    crate::prepare_work::tick(companion);
}

/// Whether the person's Brain switch is on. On until turned off; an unknown answer is on (the
/// default), because the store not having answered yet is not the person saying no.
pub(crate) fn brain_on() -> bool {
    crate::notice_desk::initiative_known().map_or(true, |s| s.moment_on(MOMENT_BRAIN))
}

/// One pass. Never panics and never blocks the Think arm on anything but the calendar call.
pub fn tick(conn: &Connection) {
    let private = crate::private_mode::is_on();
    let enabled = brain_on();
    let now = Local::now().timestamp() as f64;
    let switches = Switches { private, enabled };
    let notes = notes_dir();
    match agenda::tick(conn, now, switches, read_calendar, Some(notes.as_path())) {
        Ok(Some(_)) => {
            // The first anticipate rule: an event soon with a note about it (`prepare_moment`).
            crate::prepare_moment::run(conn, now, switches);
            match agenda::describe(conn, now) {
                Ok(d) => publish(d),
                Err(e) => tracing::warn!(error = %e, "agenda describe failed"),
            }
            let here = Local::now();
            crate::briefing::publish(agenda::told(conn, now, switches, |at| crate::briefing::when_words(at, here)));
        }
        Ok(None) => {
            crate::briefing::publish(None);
            publish(json!({ "state": if private { "paused: private mode" } else { "off: the Brain switch" } }))
        }
        Err(e) => tracing::warn!(error = %e, "agenda tick failed"),
    }
}

/// The next seven days from the calendar service, or `None` when it could not be read.
fn read_calendar() -> Option<Vec<CalEvent>> {
    // The service call runs on a thread so a hung service costs this tick its calendar, not
    // every later Think. A late answer is dropped.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new().name("yos-agenda-calendar".into()).spawn(move || {
        let _ = tx.send(read_calendar_now());
    }).ok()?;
    match rx.recv_timeout(CALENDAR_TIMEOUT) {
        Ok(v) => v,
        Err(_) => {
            tracing::warn!("agenda: the calendar service did not answer in time; this tick goes without it");
            None
        }
    }
}

fn read_calendar_now() -> Option<Vec<CalEvent>> {
    let today = Local::now().date_naive();
    let params = EventsParams {
        start_date: today.format("%Y-%m-%d").to_string(),
        end_date: (today + Duration::days(8)).format("%Y-%m-%d").to_string(),
    };
    let events = match ServiceCalendar.list(&params) {
        Ok(e) => e,
        Err(e) => {
            tracing::debug!(error = %e, "agenda: calendar not readable this tick");
            return None;
        }
    };
    Some(
        events
            .into_iter()
            .filter_map(|e| Some(CalEvent { id: e.id, title: e.title, start: start_secs(&e.start)? }))
            .collect(),
    )
}

/// Unix seconds of an event's start: RFC 3339, a local date-time, or a date (local midnight).
fn start_secs(s: &str) -> Option<f64> {
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(t.timestamp() as f64);
    }
    let local = |n: NaiveDateTime| Local.from_local_datetime(&n).earliest().map(|t| t.timestamp() as f64);
    if let Ok(n) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return local(n);
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0)).and_then(local)
}

/// Where the Notes library is, as the Notes app finds it: `$YANTRIK_NOTES_DIR`, else its data
/// folder (`$YANTRIK_HOME/data/yantrik/notes`, or `$XDG_DATA_HOME` / `~/.local/share`).
pub(crate) fn notes_dir() -> PathBuf {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(d) = env("YANTRIK_NOTES_DIR") {
        return d;
    }
    if let Some(h) = env("YANTRIK_HOME").filter(|h| h.is_absolute()) {
        return h.join("data").join("yantrik").join("notes");
    }
    let data = env("XDG_DATA_HOME").unwrap_or_else(|| env("HOME").unwrap_or_default().join(".local").join("share"));
    data.join("yantrik").join("notes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_parse_in_the_three_shapes_the_calendar_gives() {
        assert_eq!(start_secs("2026-11-27T15:00:00Z"), Some(1_795_791_600.0));
        assert!(start_secs("2026-11-27T15:00:00").is_some());
        assert!(start_secs("2026-11-27").is_some());
        assert!(start_secs("soon").is_none());
    }

    #[test]
    fn the_describe_before_a_first_pass_says_so() {
        assert!(for_describe().get("state").is_some() || for_describe().get("count").is_some());
    }
}
