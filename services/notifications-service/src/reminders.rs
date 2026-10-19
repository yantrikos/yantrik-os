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
//! ## Two announcements, both notices (design/calendar-heads-up-2026-10-16.md)
//!
//! - **The event's own reminder**, its `reminder_minutes` before it starts (10 when the event does
//!   not say), once, and not at all once it is `STALE` past. Family `reminder`: an obligation, so
//!   it never spends the day's budget, and the person's own "Inbox only" for reminders still holds.
//! - **An early heads-up**, an hour ahead, only for an event with a place. Family `calendar`:
//!   initiative, so it spends one of the day's interrupts, and Settings → Minds → Initiative →
//!   Calendar (On / Inbox only / Off) or "Never this kind" on the card switches it off. Skipped
//!   when under `EARLY_MIN_LEFT` remains, and for an event whose own lead is already
//!   `EARLY_MIN_LEFT` or more.
//!
//! Both go through [`Store::add_os_notice`], filed under the desktop's own harness id, so they meet
//! the same grant decision, quiet hours and controls as a mind's notice. Ids are deterministic
//! (`cal-reminder:<event>:<start>`, `cal-early:<event>:<start>`), so a restart that announces again
//! is answered as a duplicate, and the reminder names the early card in `supersedes`: one card per
//! event.
//!
//! ## Times are local, and naive
//!
//! `parse_stamp` reads `2026-03-18T10:00:00` — no timezone, no offset — which is the wall-clock
//! time the person typed. Each start is placed on the machine's clock through its time zone
//! (`place`), and compared with now as instants. Comparing naive times would put every reminder an
//! hour out across a daylight-saving change; comparing a naive start against UTC would move every
//! one by the machine's offset, which on this machine is five and a half hours.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, Local, LocalResult, NaiveDateTime, SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use yantrik_ipc_contracts::calendar::{parse_stamp, CalendarEvent, DEFAULT_REMINDER_MINUTES};
use yantrik_ipc_contracts::notifications::notice;

use crate::store::Store;

/// How often the calendar is re-read. Well inside the shortest sensible lead, so a reminder is
/// never more than half a minute late, and cheap: one `stat` per file plus a parse.
const TICK: Duration = Duration::from_secs(30);

/// How late is too late. An event whose start has passed by more than this is not announced at
/// all — a machine woken from suspend should not open with six reminders for meetings that are
/// over.
const STALE: ChronoDuration = ChronoDuration::minutes(5);

/// How far ahead the early heads-up comes.
const EARLY_LEAD: ChronoDuration = ChronoDuration::minutes(60);

/// The least time left for an early heads-up to be worth sending (a late boot, an event added
/// late), and the own lead from which an event's reminder is already early enough.
const EARLY_MIN_LEFT: ChronoDuration = ChronoDuration::minutes(30);

/// The least of a long title an early card keeps, however long its place.
const EARLY_TITLE_KEPT: usize = 24;

/// What the cards are filed as.
const NAME: &str = "Calendar";

/// Every notice id this timer makes starts with this, and only those are ever taken back.
const PREFIX: &str = "cal-";

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
        tick(&store, &dir, &mut sent, Local::now());
    }
}

/// One pass: announce what is due, take back the cards of events that moved or went, and keep
/// the record. Takes `now` so a test can drive it.
fn tick<Tz: TimeZone>(store: &Store, dir: &Path, sent: &mut Sent, now: DateTime<Tz>) {
    let read = read_events(dir);
    for due in due(&read.events, now.clone()) {
        if sent.has(due.kind, &due.id, &due.start) {
            continue;
        }
        let answer = store.add_os_notice(NAME, due.notice(&now));
        if answer["stored"] == true {
            tracing::info!(id = %due.id, kind = due.kind.as_str(), duplicate = answer["duplicate"] == true, "announced an event");
        } else if answer["refused"].is_string() {
            // Built here, so a refusal is a fault in this file; asking again every tick would
            // not change the answer.
            tracing::error!(id = %due.id, kind = due.kind.as_str(), why = %answer["refused"], "the store refused a calendar notice");
        } else {
            // Not kept (the store could not be written): not recorded, so the next tick asks again.
            tracing::warn!(id = %due.id, kind = due.kind.as_str(), why = %answer["why"], "a calendar notice was not stored; trying again next tick");
            continue;
        }
        sent.record(due.kind, &due.id, &due.start);
    }
    // An event edited in place keeps its start, so its early card keeps its id: the card takes
    // the new words rather than a second card being posted. A dismissed card is left alone.
    for e in read.events.iter().filter(|e| issued_id(&e.id)) {
        let (Some(start), Some(place)) = (parse_stamp(&e.start), place_of(e)) else { continue };
        if !sent.has(Kind::Early, &e.id, &start) {
            continue;
        }
        if let Err(why) = store.revise_os_title(&notice_id(Kind::Early, &e.id, &start), &early_title(&e.title, &start, place)) {
            tracing::warn!(id = %e.id, %why, "could not put an edited event's words on its early card; trying again next tick");
        }
    }
    // Only from a complete reading: an event that could not be read this tick has not gone.
    if read.complete {
        let live = live_ids(&read.events);
        match store.withdraw_os_notices(PREFIX, |id| live.contains(id)) {
            Ok(0) => {}
            Ok(n) => tracing::info!(withdrawn = n, "took back the cards of events that were moved or deleted"),
            Err(why) => tracing::warn!(%why, "could not take back the cards of moved or deleted events"),
        }
    }
    sent.prune(now.naive_local());
    sent.save();
}

/// The events, and whether every file in the directory was read.
struct Reading {
    events: Vec<CalendarEvent>,
    complete: bool,
}

/// Every stored event. A file that will not read or parse is skipped, the way the calendar
/// service's own listing skips one: a corrupt file is a problem to surface where the events are
/// owned, but it must not stop the reminders for every other event. It does make the reading
/// incomplete, and so does a directory that cannot be listed.
fn read_events(dir: &Path) -> Reading {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        // No calendar yet: nothing to announce, and nothing that was announced is still there.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Reading { events: Vec::new(), complete: true },
        Err(_) => return Reading { events: Vec::new(), complete: false },
    };
    let mut complete = true;
    let mut events = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            complete = false;
            continue;
        };
        let path = entry.path();
        // A dot file is not an event: this timer's own `.reminded.json` lives here too.
        let hidden = entry.file_name().to_string_lossy().starts_with('.');
        if hidden || path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match std::fs::read_to_string(&path).ok().and_then(|data| serde_json::from_str(&data).ok()) {
            Some(event) => events.push(event),
            None => complete = false,
        }
    }
    Reading { events, complete }
}

/// Which announcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    /// The event's own, at its `reminder_minutes`.
    Reminder,
    /// The early heads-up for an event with a place.
    Early,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Reminder => "reminder",
            Kind::Early => "early",
        }
    }
}

/// The notice id for one announcement of an event at one start. The start is compact so the
/// longest event id the calendar issues (64) still fits `notice::MAX_NOTICE_ID`.
fn notice_id(kind: Kind, event: &str, start: &NaiveDateTime) -> String {
    format!("{PREFIX}{}:{event}:{}", kind.as_str(), start.format("%Y%m%dT%H%M"))
}

/// Whether `id` is one the calendar store would issue (`[A-Za-z0-9-]`, 1–64). A file whose id is
/// anything else was not written by it, and its id would not make a notice id or an `open_event`.
fn issued_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// The place an event has, trimmed, or `None`.
fn place_of(event: &CalendarEvent) -> Option<&str> {
    event.location.as_deref().map(str::trim).filter(|l| !l.is_empty())
}

/// The instant a wall-clock time names on `tz`'s clock. A time that happens twice (the hour the
/// clocks go back) is the first; a time that never happens (the hour they go forward) is the
/// instant an hour later on the wall clock, when the person's 02:30 has arrived.
fn place<Tz: TimeZone>(tz: &Tz, at: &NaiveDateTime) -> Option<DateTime<Tz>> {
    match tz.from_local_datetime(at) {
        LocalResult::Single(t) => Some(t),
        LocalResult::Ambiguous(first, _) => Some(first),
        LocalResult::None => tz.from_local_datetime(&(*at + ChronoDuration::hours(1))).earliest(),
    }
}

/// One announcement that has come due.
#[derive(Debug, Clone)]
struct Due {
    kind: Kind,
    id: String,
    title: String,
    place: Option<String>,
    /// The start as the person entered it.
    start: NaiveDateTime,
    /// The start and the end as instants.
    starts: DateTime<Utc>,
    ends: Option<DateTime<Utc>>,
    reminder_minutes: u32,
}

/// The pure decision — which announcements are due at this instant. Takes `now` (and its time
/// zone) so the whole rule is testable without a clock, a thread or a wait.
///
/// The own reminder is due once its lead has passed and stays due until `STALE` past the start.
/// The early heads-up is due from `EARLY_LEAD` ahead while at least `EARLY_MIN_LEFT` remains, for
/// an event with a place and an own lead under `EARLY_MIN_LEFT`. All-day events are never due:
/// there is no time of day to announce them at. An event whose start will not parse cannot be
/// placed in time, and one whose id the calendar would not issue was not written by it; both are
/// skipped, not guessed at.
fn due<Tz: TimeZone>(events: &[CalendarEvent], now: DateTime<Tz>) -> Vec<Due> {
    let tz = now.timezone();
    let now = now.with_timezone(&Utc);
    let mut out = Vec::new();
    for e in events.iter().filter(|e| !e.is_all_day && issued_id(&e.id)) {
        let Some(start) = parse_stamp(&e.start) else { continue };
        let Some(starts) = place(&tz, &start).map(|t| t.with_timezone(&Utc)) else { continue };
        let ends = parse_stamp(&e.end).and_then(|end| place(&tz, &end)).map(|t| t.with_timezone(&Utc)).filter(|t| *t > starts);
        let left = starts - now;
        let lead = ChronoDuration::minutes(i64::from(e.reminder_minutes));
        let one = |kind| Due {
            kind,
            id: e.id.clone(),
            title: e.title.clone(),
            place: place_of(e).map(str::to_string),
            start,
            starts,
            ends,
            reminder_minutes: e.reminder_minutes,
        };
        if place_of(e).is_some() && lead < EARLY_MIN_LEFT && left <= EARLY_LEAD && left >= EARLY_MIN_LEFT {
            out.push(one(Kind::Early));
        }
        if left <= lead && left >= -STALE {
            out.push(one(Kind::Reminder));
        }
    }
    out
}

/// Every notice id the events as they stand now would be announced under, at any time. A card
/// whose id is not among them is for an event that was deleted or moved, or an early card for an
/// event whose place was taken off.
fn live_ids(events: &[CalendarEvent]) -> BTreeSet<String> {
    events
        .iter()
        .filter(|e| issued_id(&e.id))
        .filter_map(|e| parse_stamp(&e.start).map(|start| (e, start)))
        .flat_map(|(e, start)| {
            // An early card says the event has a place; once it has none, the card is taken back.
            let early = place_of(e).map(|_| notice_id(Kind::Early, &e.id, &start));
            std::iter::once(notice_id(Kind::Reminder, &e.id, &start)).chain(early)
        })
        .collect()
}

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// "in 10 minutes", "in a minute", "now".
fn minutes_phrase(minutes: i64) -> String {
    match minutes {
        m if m <= 0 => "now".to_string(),
        1 => "in a minute".to_string(),
        m => format!("in {m} minutes"),
    }
}

impl Due {
    /// The notice, as `notice::check` reads it. The event's own words — its title and its place
    /// as the person typed them — go in as text and are cleaned and cut there, as any notice's are;
    /// the place only has its line breaks made spaces (the same `notice::plain`), so a second line
    /// of an address is not lost. Nothing here parses, looks up or sends it anywhere.
    fn notice<Tz: TimeZone>(&self, now: &DateTime<Tz>) -> serde_json::Value {
        let minutes = (self.starts - now.with_timezone(&Utc)).num_minutes();
        let action = serde_json::json!({ "kind": "open_event", "ref": self.id });
        match self.kind {
            Kind::Reminder => {
                let why = match self.reminder_minutes {
                    DEFAULT_REMINDER_MINUTES => format!("Calendar events remind you {DEFAULT_REMINDER_MINUTES} minutes before."),
                    0 => "You set this event to remind you when it starts.".to_string(),
                    1 => "You set this event to remind you 1 minute before.".to_string(),
                    m => format!("You set this event to remind you {m} minutes before."),
                };
                serde_json::json!({
                    "notice_id": notice_id(Kind::Reminder, &self.id, &self.start),
                    "family": "reminder",
                    "title": format!("{} {}", self.title.trim(), minutes_phrase(minutes)),
                    "body": self.start.format("Starts at %H:%M on %A %e %B").to_string(),
                    "why": why,
                    "attention": "interrupt",
                    // Useful while the event is on: a reminder held by quiet hours past its end
                    // is not toasted afterwards.
                    "useful_until": rfc3339(self.ends.unwrap_or(self.starts + STALE)),
                    // One card per event: this one takes the early card's place, if it is still up.
                    "supersedes": notice_id(Kind::Early, &self.id, &self.start),
                    "action": action,
                })
            }
            Kind::Early => {
                serde_json::json!({
                    "notice_id": notice_id(Kind::Early, &self.id, &self.start),
                    "family": "calendar",
                    "title": early_title(&self.title, &self.start, self.place.as_deref().unwrap_or_default()),
                    "body": format!("{}. It has a place, so you may need to leave early.", capitalised(&minutes_phrase(minutes))),
                    "why": "This event has a place, so it may need travel.",
                    "attention": "interrupt",
                    // A heads-up shown after its moment is noise.
                    "useful_until": rfc3339(self.starts),
                    "action": action,
                })
            }
        }
    }
}

/// "Dentist at 15:00, Elm St". The time and the place are the card's reason to exist, so a long
/// title is cut to make room for them, down to `EARLY_TITLE_KEPT` characters and marked with an
/// ellipsis; only a place too long for what is left is cut, by the title's own bound. The title's
/// first line only, as any title, but before the place is added, so a line break in the title does
/// not take the place with it.
fn early_title(title: &str, start: &NaiveDateTime, place: &str) -> String {
    let tail = format!(" at {}, {}", start.format("%H:%M"), notice::plain(place, 0).trim());
    let (title, _) = notice::one_line(title, usize::MAX);
    let room = notice::MAX_TITLE.saturating_sub(tail.chars().count()).max(EARLY_TITLE_KEPT);
    let title = if title.chars().count() > room {
        let kept: String = title.chars().take(room - 1).collect();
        format!("{}…", kept.trim_end())
    } else {
        title
    };
    format!("{title}{tail}")
}

fn capitalised(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

// ── Remembering what has been said ──────────────────────────────────────────────────────────

/// Which announcements have been made, keyed by kind, event id **and** start time.
///
/// The start, because an event that is moved is news again: the same id at a new time has not
/// been announced. And on disk, in the calendar's own directory, because this service restarts
/// with the session — with the set in memory only, a restart inside an event's lead would
/// announce the same meeting again. The store's deterministic notice ids catch that too, but the
/// store prunes its records and this file is what still says "already told" after it has.
///
/// The file and its name are unchanged from when the timer lived in the calendar service. A file
/// from before the early heads-up is a flat `{event id: start}` of reminders, read as such, so an
/// upgrade does not re-announce everything already said.
struct Sent {
    path: PathBuf,
    seen: Seen,
    dirty: bool,
}

/// What is on the disk.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Seen {
    /// event id → the start whose own reminder was announced.
    reminder: BTreeMap<String, String>,
    /// event id → the start whose early heads-up was announced.
    early: BTreeMap<String, String>,
}

impl Seen {
    fn read(text: &str) -> Option<Seen> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum OnDisk {
            Now(Seen),
            Before(BTreeMap<String, String>),
        }
        match serde_json::from_str::<OnDisk>(text).ok()? {
            OnDisk::Now(seen) => Some(seen),
            OnDisk::Before(reminder) => Some(Seen { reminder, early: BTreeMap::new() }),
        }
    }

    fn of(&mut self, kind: Kind) -> &mut BTreeMap<String, String> {
        match kind {
            Kind::Reminder => &mut self.reminder,
            Kind::Early => &mut self.early,
        }
    }
}

fn stamp(start: &NaiveDateTime) -> String {
    start.format("%Y-%m-%dT%H:%M:%S").to_string()
}

impl Sent {
    fn load(dir: &Path) -> Self {
        let path = dir.join(".reminded.json");
        // A file that will not read starts this record empty. Nothing is lost by it: the store
        // answers an announcement it already holds as a duplicate, and shows nothing twice.
        let seen = std::fs::read_to_string(&path).ok().and_then(|text| Seen::read(&text)).unwrap_or_default();
        Self { path, seen, dirty: false }
    }

    /// Whether this announcement of this event at this start was made.
    fn has(&mut self, kind: Kind, id: &str, start: &NaiveDateTime) -> bool {
        self.seen.of(kind).get(id) == Some(&stamp(start))
    }

    /// Note that it was. `false` if it already was.
    fn record(&mut self, kind: Kind, id: &str, start: &NaiveDateTime) -> bool {
        if self.has(kind, id, start) {
            return false;
        }
        self.seen.of(kind).insert(id.to_string(), stamp(start));
        self.dirty = true;
        true
    }

    /// Forget anything whose start is well behind us, so the file does not grow forever.
    fn prune(&mut self, now: NaiveDateTime) {
        let cutoff = now - ChronoDuration::days(1);
        for kind in [Kind::Reminder, Kind::Early] {
            let map = self.seen.of(kind);
            let before = map.len();
            map.retain(|_, start| parse_stamp(start).map(|s| s > cutoff).unwrap_or(false));
            if map.len() != before {
                self.dirty = true;
            }
        }
    }

    fn save(&mut self) {
        if !self.dirty {
            return;
        }
        let Ok(json) = serde_json::to_string(&self.seen) else { return };
        // Temp and rename, so a service killed mid-write does not leave a file that will not
        // parse. A write that fails stays dirty and is tried again next tick; meanwhile the
        // store's duplicate ids keep a restart from showing anything twice.
        let temp = self.path.with_extension(format!("tmp{}", std::process::id()));
        let written = std::fs::write(&temp, json).and_then(|()| std::fs::rename(&temp, &self.path));
        match written {
            Ok(()) => self.dirty = false,
            Err(e) => {
                let _ = std::fs::remove_file(&temp);
                tracing::warn!(path = %self.path.display(), error = %e, "could not write what the calendar timer has announced");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, NaiveDate};
    use std::sync::atomic::{AtomicU32, Ordering};
    use yantrik_ipc_contracts::notifications::notice::{Attention, Family, FamilySetting, GrantReason, Notice, QuietHours};
    use yantrik_ipc_contracts::notifications::Notification;

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

    /// A fixed instant, on a clock with no daylight saving, far enough ahead that no card the
    /// tests store is past its `useful_until` on the real clock the store expires by. The tests hand the clock to `due`
    /// rather than reading one, so none of them sleeps and none of them cares when it runs.
    fn nine_am() -> DateTime<Utc> {
        Utc.from_utc_datetime(&parse_stamp("2099-03-18T09:00:00").unwrap())
    }

    fn at(now: DateTime<Utc>, minutes: i64) -> String {
        stamp(&(now.naive_utc() + ChronoDuration::minutes(minutes)))
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

    fn placed(id: &str, title: &str, start: &str, place: &str) -> CalendarEvent {
        let mut e = event(id, title, start, DEFAULT_REMINDER_MINUTES);
        e.location = Some(place.into());
        e
    }

    fn due_ids<Tz: TimeZone>(events: &[CalendarEvent], now: DateTime<Tz>) -> Vec<String> {
        due(events, now).into_iter().filter(|d| d.kind == Kind::Reminder).map(|d| d.id).collect()
    }

    fn kinds<Tz: TimeZone>(events: &[CalendarEvent], now: DateTime<Tz>) -> Vec<(String, Kind)> {
        let mut out: Vec<(String, Kind)> = due(events, now).into_iter().map(|d| (d.id, d.kind)).collect();
        out.sort();
        out
    }

    fn write(dir: &Path, e: &CalendarEvent) {
        std::fs::write(dir.join(format!("{}.json", e.id)), serde_json::to_string(e).unwrap()).unwrap();
    }

    fn cards(store: &Store) -> Vec<(Notification, Notice)> {
        store.list().into_iter().filter_map(|n| n.notice.clone().map(|x| (n, x))).collect()
    }

    /// One card by its notice id, dismissed or not.
    fn card(store: &Store, notice_id: &str) -> Option<(Notification, Notice)> {
        store.since(0).changed.into_iter().find_map(|n| n.notice.clone().filter(|x| x.notice_id == notice_id).map(|x| (n, x)))
    }

    // ── The own reminder (unchanged rules) ──────────────────────────────────────────────────

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
        let events = read_events(&dir).events;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].reminder_minutes, DEFAULT_REMINDER_MINUTES);
        assert_eq!(due_ids(&events, now), vec!["old"], "inside ten minutes: due");
        assert!(due_ids(&events, now - ChronoDuration::minutes(5)).is_empty(), "outside: not yet");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_event_in_the_past_is_not_announced_after_a_resume() {
        // A machine waking from suspend must not open with reminders for meetings that are over,
        // nor heads-ups for them.
        let now = nine_am();
        let over = placed("over", "Over", &at(now, -30), "Elm St");
        assert!(kinds(&[over], now).is_empty());
        // Just inside STALE, though — an event that started four minutes ago is late news,
        // still worth one reminder, and no heads-up.
        let late = placed("late", "Late", &at(now, -4), "Elm St");
        assert_eq!(kinds(&[late], now), vec![("late".into(), Kind::Reminder)]);
    }

    #[test]
    fn an_all_day_event_gets_neither_announcement() {
        let now = nine_am();
        for minutes in [5, 45, 60] {
            let mut holiday = placed("holiday", "Holiday", &at(now, minutes), "Goa");
            holiday.is_all_day = true;
            assert!(kinds(&[holiday], now).is_empty(), "there is no time of day to announce it at");
        }
    }

    #[test]
    fn an_event_whose_start_will_not_parse_or_whose_id_the_calendar_would_not_issue_is_skipped() {
        let now = nine_am();
        let broken = event("broken", "Broken", "not a time", DEFAULT_REMINDER_MINUTES);
        let planted = event("../x:y", "Planted", &at(now, 5), DEFAULT_REMINDER_MINUTES);
        let long = event(&"a".repeat(65), "Long", &at(now, 5), DEFAULT_REMINDER_MINUTES);
        assert!(kinds(&[broken, planted, long], now).is_empty());
        // The longest id it does issue still makes a notice id the store takes.
        let id = notice_id(Kind::Reminder, &"a".repeat(64), &parse_stamp("2026-12-31T23:59:00").unwrap());
        assert!(notice::valid_notice_id(&id), "{id} ({} chars)", id.len());
    }

    #[test]
    fn a_corrupt_event_file_does_not_stop_the_other_reminders_but_makes_the_reading_incomplete() {
        let dir = temp_dir();
        let now = nine_am();
        std::fs::write(dir.join("junk.json"), "{not json").unwrap();
        write(&dir, &event("good", "Good", &at(now, 5), DEFAULT_REMINDER_MINUTES));
        let read = read_events(&dir);
        assert_eq!(due_ids(&read.events, now), vec!["good"]);
        assert!(!read.complete);
        std::fs::remove_file(dir.join("junk.json")).unwrap();
        // This timer's own record sits beside the events and is not one of them.
        let mut sent = Sent::load(&dir);
        sent.record(Kind::Reminder, "good", &parse_stamp(&at(now, 5)).unwrap());
        sent.save();
        assert!(dir.join(".reminded.json").is_file());
        assert!(read_events(&dir).complete);
        assert!(read_events(&dir.join("never-made")).complete, "no calendar yet is a complete reading of nothing");
        let _ = std::fs::remove_dir_all(dir);
    }

    // ── The early heads-up ────────────────────────────────────────────────────────────────

    #[test]
    fn the_early_heads_up_comes_an_hour_ahead_only_for_an_event_with_a_place() {
        let now = nine_am();
        let dentist = |minutes| placed("dentist", "Dentist", &at(now, minutes), "Elm St");
        assert!(kinds(&[dentist(61)], now).is_empty(), "61 minutes out: nothing");
        assert_eq!(kinds(&[dentist(60)], now), vec![("dentist".into(), Kind::Early)], "60: the early card");
        assert_eq!(kinds(&[dentist(30)], now), vec![("dentist".into(), Kind::Early)], "30 still remain: early");
        assert!(kinds(&[dentist(29)], now).is_empty(), "29: no early card, and the own reminder is not due yet");
        assert_eq!(kinds(&[dentist(10)], now), vec![("dentist".into(), Kind::Reminder)], "10: the own reminder only");

        // No place, or a place of spaces only: never early.
        let standup = event("standup", "Standup", &at(now, 60), DEFAULT_REMINDER_MINUTES);
        let blank = placed("blank", "Blank", &at(now, 60), " \t\n ");
        assert!(kinds(&[standup, blank], now).is_empty());

        // An own lead of 30 or more is already early: no second card.
        let mut flight = placed("flight", "Flight", &at(now, 40), "Airport");
        flight.reminder_minutes = 45;
        assert_eq!(kinds(std::slice::from_ref(&flight), now), vec![("flight".into(), Kind::Reminder)]);
        flight.reminder_minutes = 30;
        assert!(kinds(std::slice::from_ref(&flight), now).iter().all(|(_, k)| *k == Kind::Reminder));
        flight.reminder_minutes = 29;
        assert_eq!(kinds(&[flight], now), vec![("flight".into(), Kind::Early)], "40 out, own lead 29: early");
    }

    #[test]
    fn a_moved_event_is_due_again_under_new_ids() {
        let now = nine_am();
        let before = placed("dentist", "Dentist", &at(now, 60), "Elm St");
        let after = placed("dentist", "Dentist", &at(now, 55), "Elm St");
        let a = due(&[before], now);
        let b = due(&[after], now);
        assert_eq!((a.len(), b.len()), (1, 1));
        assert_ne!(notice_id(Kind::Early, "dentist", &a[0].start), notice_id(Kind::Early, "dentist", &b[0].start));
    }

    #[test]
    fn the_cards_say_what_the_design_says() {
        let now = nine_am();
        let e = placed("dentist", "Dentist", "2099-03-18T10:00:00", "Elm St");
        let early = &due(std::slice::from_ref(&e), now)[0];
        let n = notice::check(&early.notice(&now)).unwrap();
        assert_eq!(n.notice_id, "cal-early:dentist:20990318T1000");
        assert_eq!(n.family, Family::Calendar);
        assert_eq!(n.title, "Dentist at 10:00, Elm St");
        assert_eq!(n.body, "In 60 minutes. It has a place, so you may need to leave early.");
        assert_eq!(n.why, "This event has a place, so it may need travel.");
        assert_eq!(n.action.as_ref().map(|a| (a.kind.as_str().to_string(), a.reference.clone())), Some(("open_event".into(), "dentist".into())));
        assert_eq!(n.useful_until.as_deref(), Some("2099-03-18T10:00:00Z"));
        assert!(n.clamped.is_empty(), "{:?}", n.clamped);

        let ten_to = now + ChronoDuration::minutes(50);
        let own = &due(&[e], ten_to)[0];
        let n = notice::check(&own.notice(&ten_to)).unwrap();
        assert_eq!(n.notice_id, "cal-reminder:dentist:20990318T1000");
        assert_eq!(n.family, Family::Reminder);
        assert_eq!(n.title, "Dentist in 10 minutes");
        assert_eq!(n.body, "Starts at 10:00 on Wednesday 18 March");
        assert_eq!(n.why, "Calendar events remind you 10 minutes before.");
        assert_eq!(n.supersedes.as_deref(), Some("cal-early:dentist:20990318T1000"));

        let mut own = own.clone();
        own.reminder_minutes = 25;
        assert_eq!(notice::check(&own.notice(&ten_to)).unwrap().why, "You set this event to remind you 25 minutes before.");
    }

    #[test]
    fn the_event_text_is_cleaned_as_any_notice_and_the_place_is_shown_as_typed() {
        let now = nine_am();
        let hostile = placed(
            "dentist",
            "Dent\u{202e}ist\u{1b}[2J",
            "2099-03-18T10:00:00",
            "  Elm St\n12 \u{202e}Fake line · verified  ",
        );
        let n = notice::check(&due(&[hostile], now)[0].notice(&now)).unwrap();
        assert!(!n.title.chars().any(|c| c.is_control() || ('\u{202a}'..='\u{202e}').contains(&c)), "{:?}", n.title);
        // Line breaks become spaces; the words stay the person's own, in their order.
        assert!(n.title.ends_with("Elm St 12  Fake line · verified"), "{:?}", n.title);
        // A long title gives way to the time and the place (the review of #861).
        let wordy = placed("wordy", &"Quarterly planning review with the whole team ".repeat(2), "2099-03-18T10:00:00", "Elm St");
        let n = notice::check(&due(&[wordy], now)[0].notice(&now)).unwrap();
        assert!(n.title.ends_with("… at 10:00, Elm St"), "{:?}", n.title);
        assert!(n.title.chars().count() <= notice::MAX_TITLE && n.clamped.is_empty(), "{:?} {:?}", n.title, n.clamped);
        // Long places are cut by the title's bound, and named.
        let long = placed("far", "Far", "2099-03-18T10:00:00", &"Road ".repeat(40));
        let n = notice::check(&due(&[long], now)[0].notice(&now)).unwrap();
        assert!(n.title.chars().count() <= notice::MAX_TITLE && n.clamped.contains(&"title".to_string()));
    }

    #[test]
    fn a_long_title_is_cut_so_the_card_keeps_its_time_and_place() {
        let now = nine_am();
        let long = "Quarterly planning review with the regional facilities and procurement teams";
        let n = notice::check(&due(&[placed("q", long, "2099-03-18T10:00:00", "Elm St")], now)[0].notice(&now)).unwrap();
        assert!(n.title.ends_with("… at 10:00, Elm St") && n.title.starts_with("Quarterly planning review"), "{:?}", n.title);
        assert!(n.title.chars().count() <= notice::MAX_TITLE && n.clamped.is_empty(), "{:?} {:?}", n.title, n.clamped);
        // A line break in the title no longer takes the place with it.
        let n = notice::check(&due(&[placed("q", "Dentist\nbring the form", "2099-03-18T10:00:00", "Elm St")], now)[0].notice(&now)).unwrap();
        assert_eq!(n.title, "Dentist at 10:00, Elm St");
        // A short title is untouched, and however long the place, the title keeps its start.
        assert_eq!(early_title("Dentist", &parse_stamp("2099-03-18T10:00:00").unwrap(), "Elm St"), "Dentist at 10:00, Elm St");
        let t = early_title(long, &parse_stamp("2099-03-18T10:00:00").unwrap(), &"Road ".repeat(40));
        assert!(t.starts_with("Quarterly planning revi…"), "{t:?}");
    }

    // ── Recurring events ─────────────────────────────────────────────────────────────────

    #[test]
    fn each_occurrence_of_a_recurring_event_is_announced_at_its_own_start() {
        // The store holds occurrences, not rules: a synced calendar arrives one event per
        // occurrence (`singleEvents=true`), and a stored `recurrence` string is never expanded
        // here. Each occurrence is its own announcement; an occurrence that moves to next week
        // under the same id is news again.
        let now = nine_am();
        let mut this_week = placed("yoga-0318", "Yoga", &at(now, 60), "Studio");
        this_week.recurrence = Some("RRULE:FREQ=WEEKLY".into());
        let next_week = placed("yoga-0325", "Yoga", &at(now, 60 + 7 * 24 * 60), "Studio");
        assert_eq!(kinds(&[this_week.clone(), next_week.clone()], now), vec![("yoga-0318".into(), Kind::Early)]);
        let a_week_on = now + ChronoDuration::days(7);
        assert_eq!(kinds(&[this_week.clone(), next_week], a_week_on), vec![("yoga-0325".into(), Kind::Early)]);

        let mut sent = Sent::load(&temp_dir());
        let start = parse_stamp(&this_week.start).unwrap();
        assert!(sent.record(Kind::Early, "yoga-0318", &start));
        assert!(!sent.record(Kind::Early, "yoga-0318", &start));
        assert!(sent.record(Kind::Early, "yoga-0318", &(start + ChronoDuration::days(7))), "the same id a week on is news");
    }

    // ── Time zones and daylight saving ─────────────────────────────────────────────────────

    /// A clock with daylight saving, as the UK keeps it in 2026: GMT, then BST (+1) from 01:00 UTC
    /// on 29 March to 01:00 UTC on 25 October. Built here because the tests must not depend on
    /// the machine's own zone.
    #[derive(Clone, Copy, Debug)]
    struct London;

    impl London {
        fn gmt() -> FixedOffset {
            FixedOffset::east_opt(0).unwrap()
        }
        fn bst() -> FixedOffset {
            FixedOffset::east_opt(3600).unwrap()
        }
    }

    impl TimeZone for London {
        type Offset = FixedOffset;
        fn from_offset(_: &FixedOffset) -> London {
            London
        }
        fn offset_from_local_date(&self, local: &NaiveDate) -> LocalResult<FixedOffset> {
            self.offset_from_local_datetime(&local.and_hms_opt(12, 0, 0).unwrap())
        }
        fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> LocalResult<FixedOffset> {
            let fits: Vec<FixedOffset> = [London::bst(), London::gmt()]
                .into_iter()
                .filter(|off| self.offset_from_utc_datetime(&(*local - ChronoDuration::seconds(i64::from(off.local_minus_utc())))) == *off)
                .collect();
            match fits[..] {
                [] => LocalResult::None,
                [one] => LocalResult::Single(one),
                [first, second] => LocalResult::Ambiguous(first, second),
                _ => unreachable!(),
            }
        }
        fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
            self.offset_from_utc_datetime(&utc.and_hms_opt(12, 0, 0).unwrap())
        }
        fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> FixedOffset {
            let on = parse_stamp("2026-03-29T01:00:00").unwrap();
            let off = parse_stamp("2026-10-25T01:00:00").unwrap();
            if *utc >= on && *utc < off { London::bst() } else { London::gmt() }
        }
    }

    fn london(wall: &str) -> DateTime<London> {
        London.from_local_datetime(&parse_stamp(wall).unwrap()).earliest().unwrap()
    }

    #[test]
    fn the_hour_the_clocks_go_forward_is_counted_as_it_passes() {
        // 00:30 GMT on 29 March; at 01:00 the clocks jump to 02:00. An event at 02:15 (wall) is
        // 45 real minutes away, not 105: a naive comparison would send the heads-up an hour late.
        let now = london("2026-03-29T00:30:00");
        let dentist = placed("dentist", "Dentist", "2026-03-29T02:15:00", "Elm St");
        assert_eq!(kinds(std::slice::from_ref(&dentist), now), vec![("dentist".into(), Kind::Early)]);
        let d = &due(&[dentist], now)[0];
        let n = notice::check(&d.notice(&now)).unwrap();
        assert_eq!(n.body, "In 45 minutes. It has a place, so you may need to leave early.");
        assert_eq!(n.title, "Dentist at 02:15, Elm St", "the time as the person entered it");

        // 01:30 never happens that night: it is placed when the wall clock reaches it, at 02:30.
        assert_eq!(place(&London, &parse_stamp("2026-03-29T01:30:00").unwrap()).unwrap(), london("2026-03-29T02:30:00"));
        let gap = event("gap", "Gap", "2026-03-29T01:30:00", DEFAULT_REMINDER_MINUTES);
        assert_eq!(due_ids(std::slice::from_ref(&gap), london("2026-03-29T02:21:00")), vec!["gap"], "9 real minutes before 02:30");
        assert!(due_ids(&[gap], london("2026-03-29T00:50:00")).is_empty(), "40 real minutes out");
    }

    #[test]
    fn the_hour_the_clocks_go_back_is_counted_once() {
        // 01:00 BST on 25 October; at 02:00 BST the clocks go back to 01:00 GMT. An event at
        // 02:30 (wall) is 2½ real hours away, not 1½: a naive comparison would send the heads-up
        // an hour early.
        let now = london("2026-10-25T01:00:00");
        let dentist = placed("dentist", "Dentist", "2026-10-25T02:30:00", "Elm St");
        assert!(kinds(std::slice::from_ref(&dentist), now).is_empty());
        // 01:30 GMT (the second 01:30) is the one an hour before.
        let second = London.from_local_datetime(&parse_stamp("2026-10-25T01:30:00").unwrap()).latest().unwrap();
        assert_eq!(kinds(&[dentist], second), vec![("dentist".into(), Kind::Early)]);
        // An event at the 01:30 that happens twice is placed at the first.
        assert_eq!(place(&London, &parse_stamp("2026-10-25T01:30:00").unwrap()).unwrap(), london("2026-10-25T01:30:00"));
    }

    #[test]
    fn a_zone_ahead_of_utc_compares_instants_not_wall_times() {
        // The machine in India: an event at 15:00 local is 09:30 UTC. At 14:00 local it is an
        // hour out, whatever the UTC wall clock says.
        let ist = FixedOffset::east_opt(5 * 3600 + 1800).unwrap();
        let now = ist.from_local_datetime(&parse_stamp("2026-10-16T14:00:00").unwrap()).unwrap();
        let dentist = placed("dentist", "Dentist", "2026-10-16T15:00:00", "MG Road");
        assert_eq!(kinds(&[dentist], now), vec![("dentist".into(), Kind::Early)]);
    }

    // ── What has been said ─────────────────────────────────────────────────────────────────

    #[test]
    fn an_announcement_is_recorded_by_kind_and_again_if_the_event_is_moved() {
        let dir = temp_dir();
        let mut sent = Sent::load(&dir);
        let start = parse_stamp("2026-03-18T09:09:00").unwrap();
        assert!(sent.record(Kind::Reminder, "e1", &start));
        assert!(!sent.record(Kind::Reminder, "e1", &start), "the same event at the same time is said once");
        assert!(sent.record(Kind::Early, "e1", &start), "the early card is a separate announcement");
        let moved = start + ChronoDuration::minutes(31);
        assert!(sent.record(Kind::Reminder, "e1", &moved), "a moved event is news again");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn what_was_announced_survives_a_restart_and_an_old_file_still_reads() {
        let dir = temp_dir();
        let start = parse_stamp("2026-03-18T09:05:00").unwrap();
        {
            let mut sent = Sent::load(&dir);
            assert!(sent.record(Kind::Reminder, "e1", &start));
            assert!(sent.record(Kind::Early, "e2", &start));
            sent.save();
        }
        let mut reopened = Sent::load(&dir);
        assert!(!reopened.record(Kind::Reminder, "e1", &start));
        assert!(!reopened.record(Kind::Early, "e2", &start));

        // A file written before the early heads-up: a flat map of reminders.
        std::fs::write(dir.join(".reminded.json"), r#"{"old":"2026-03-18T09:05:00","reminder":"2026-03-18T09:05:00"}"#).unwrap();
        let mut old = Sent::load(&dir);
        assert!(old.has(Kind::Reminder, "old", &start));
        assert!(old.has(Kind::Reminder, "reminder", &start), "an event whose id is a field name is still an event");
        assert!(!old.has(Kind::Early, "old", &start));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn old_announcements_are_forgotten_so_the_file_does_not_grow_forever() {
        let dir = temp_dir();
        let now = parse_stamp("2026-03-18T09:00:00").unwrap();
        let mut sent = Sent::load(&dir);
        sent.record(Kind::Reminder, "old", &(now - ChronoDuration::days(3)));
        sent.record(Kind::Early, "old", &(now - ChronoDuration::days(3)));
        sent.record(Kind::Reminder, "recent", &(now + ChronoDuration::minutes(5)));
        sent.prune(now);
        assert!(!sent.seen.reminder.contains_key("old") && !sent.seen.early.contains_key("old"));
        assert!(sent.seen.reminder.contains_key("recent"));
        let _ = std::fs::remove_dir_all(dir);
    }

    // ── Into the store ─────────────────────────────────────────────────────────────────────

    fn calendar() -> (Store, PathBuf, PathBuf) {
        let dir = temp_dir();
        let cal = dir.join("calendar");
        std::fs::create_dir_all(&cal).unwrap();
        (Store::open(dir.join("notifications.json")), dir, cal)
    }

    #[test]
    fn both_announcements_land_as_the_desktops_own_notices_and_the_reminder_replaces_the_early_card() {
        let (store, dir, cal) = calendar();
        let now = nine_am();
        write(&cal, &placed("dentist", "Dentist", &at(now, 60), "Elm St"));
        write(&cal, &event("standup", "Standup", &at(now, 60), DEFAULT_REMINDER_MINUTES));
        let mut sent = Sent::load(&cal);

        tick(&store, &cal, &mut sent, now);
        let shown = cards(&store);
        assert_eq!(shown.len(), 1, "an early card for the dentist and none for the standup");
        let (n, early) = &shown[0];
        assert_eq!(early.notice_id, "cal-early:dentist:20990318T1000");
        assert_eq!(early.harness.id, notice::OS_HARNESS);
        assert_eq!(n.app, "Calendar");
        let sender = n.sender.as_ref().unwrap();
        assert!(sender.desktop && sender.pid as u32 == std::process::id(), "{sender:?}");
        assert_eq!((early.granted.clone(), early.grant_reason.clone()), (Attention::Interrupt, GrantReason::Granted));
        assert_eq!(store.since(0).changed.len(), 1);

        // Fifty minutes on: both own reminders. The dentist's takes the early card's place.
        let later = now + ChronoDuration::minutes(50);
        tick(&store, &cal, &mut sent, later);
        let mut shown: Vec<String> = cards(&store).into_iter().map(|(_, x)| x.notice_id).collect();
        shown.sort();
        assert_eq!(shown, ["cal-reminder:dentist:20990318T1000", "cal-reminder:standup:20990318T1000"], "one card per event");
        let (_, own) = card(&store, "cal-reminder:dentist:20990318T1000").unwrap();
        assert_eq!(own.family, Family::Reminder);
        assert_eq!(own.grant_reason, GrantReason::Granted);
        let (gone, _) = card(&store, "cal-early:dentist:20990318T1000").unwrap();
        assert!(gone.dismissed);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn minds_flooding_reminders_and_spending_the_budget_first_never_stop_the_persons_reminder_interrupting() {
        let (store, dir, cal) = calendar();
        let now = nine_am();
        // Two minds on two accounts, each within every cap of its own, fill the minds' shared
        // hourly limit for reminder interrupts and spend the day's budget.
        let send = |from: &str, uid: u32, id: &str, family: &str| {
            let post = notice::NoticePost {
                notice: serde_json::json!({ "notice_id": id, "family": family, "title": "Stretch", "why": "You asked.", "attention": "interrupt" }),
                harness: notice::NoticeHarness { id: from.into(), uid: Some(uid), name: from.into(), ..Default::default() },
                os_id: false,
            };
            store.add_notice(post, &dir)
        };
        for (from, uid) in [("m0", 1000), ("m1", 1001)] {
            for i in 0..notice::OBLIGATION_INTERRUPTS_PER_HOUR {
                assert_eq!(send(from, uid, &format!("r{i}"), "reminder")["stored"], true);
            }
        }
        send("m0", 1000, "d0", "discovery");
        send("m1", 1001, "d1", "discovery");
        let theirs = send("m1", 1001, "r-late", "reminder");
        let late = store.get(theirs["id"].as_str().unwrap()).unwrap().notice.unwrap();
        assert_eq!(late.grant_reason, GrantReason::Rate, "the flood is real: a mind's next reminder waits");

        // Then the person's own event comes due.
        write(&cal, &event("dentist", "Dentist", &at(now, 10), DEFAULT_REMINDER_MINUTES));
        let mut sent = Sent::load(&cal);
        tick(&store, &cal, &mut sent, now);
        let (_, own) = card(&store, "cal-reminder:dentist:20990318T0910").unwrap();
        assert_eq!((own.granted, own.grant_reason, own.held_until), (Attention::Interrupt, GrantReason::Granted, None));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_dismissed_early_card_is_left_alone() {
        let (store, dir, cal) = calendar();
        let now = nine_am();
        write(&cal, &placed("dentist", "Dentist", &at(now, 60), "Elm St"));
        let mut sent = Sent::load(&cal);
        tick(&store, &cal, &mut sent, now);
        let (early, _) = card(&store, "cal-early:dentist:20990318T1000").unwrap();
        store.record_feedback(&early.id, notice::FeedbackKind::NotNow).unwrap();
        let before = card(&store, "cal-early:dentist:20990318T1000").unwrap();
        assert!(before.0.dismissed);
        // Edited after it was dismissed: not brought back, and its words are not touched.
        write(&cal, &placed("dentist", "Dentist", &at(now, 60), "Oak St"));
        tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(1));
        tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(50));
        let after = card(&store, "cal-early:dentist:20990318T1000").unwrap();
        assert_eq!(after.0.revision, before.0.revision, "not touched by the supersede or the edit");
        assert_eq!(after.0.title, "Dentist at 10:00, Elm St");
        assert_eq!(after.1.feedback, before.1.feedback);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_own_reminder_never_spends_the_budget_and_the_third_early_card_waits_in_the_inbox() {
        let (store, dir, cal) = calendar();
        let now = nine_am();
        for (id, title) in [("a", "Dentist"), ("b", "Bank"), ("c", "Barber")] {
            write(&cal, &placed(id, title, &at(now, 60), "High St"));
        }
        let mut sent = Sent::load(&cal);
        tick(&store, &cal, &mut sent, now);
        let mut reasons: Vec<GrantReason> = cards(&store).into_iter().map(|(_, x)| x.grant_reason).collect();
        reasons.sort_by_key(|r| r.as_str().to_string());
        assert_eq!(reasons, [GrantReason::BudgetSpent, GrantReason::Granted, GrantReason::Granted], "two a day; the third waits");
        // Their own reminders all interrupt, budget spent or not.
        tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(50));
        let own: Vec<GrantReason> = cards(&store).into_iter().filter(|(_, x)| x.family == Family::Reminder).map(|(_, x)| x.grant_reason).collect();
        assert_eq!(own, [GrantReason::Granted, GrantReason::Granted, GrantReason::Granted]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_persons_calendar_setting_and_quiet_initiative_reach_the_early_card_but_not_the_reminder() {
        for (setting, quiet, expect) in [
            (FamilySetting::Off, false, GrantReason::FamilyOff),
            (FamilySetting::InboxOnly, false, GrantReason::FamilyInboxOnly),
            (FamilySetting::On, true, GrantReason::QuietInitiative),
        ] {
            let (store, dir, cal) = calendar();
            store
                .set_initiative(|s| {
                    s.quiet = quiet;
                    s.set(&Family::Calendar, setting)
                })
                .unwrap();
            let now = nine_am();
            write(&cal, &placed("dentist", "Dentist", &at(now, 60), "Elm St"));
            let mut sent = Sent::load(&cal);
            tick(&store, &cal, &mut sent, now);
            let (_, early) = card(&store, "cal-early:dentist:20990318T1000").unwrap();
            assert_eq!(early.grant_reason, expect);
            assert_eq!(early.hidden(), setting == FamilySetting::Off, "Off is stored hidden");
            tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(50));
            let (_, own) = card(&store, "cal-reminder:dentist:20990318T1000").unwrap();
            assert_eq!((own.granted, own.grant_reason), (Attention::Interrupt, GrantReason::Granted), "{setting:?} quiet={quiet}");
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn quiet_hours_hold_both() {
        let (store, dir, cal) = calendar();
        // The grant reads the machine's clock: quiet from an hour before now to an hour after.
        let hour = chrono::Timelike::hour(&Local::now()) as u8;
        let quiet = QuietHours { start: (hour + 23) % 24, end: (hour + 1) % 24 };
        store.set_initiative(|s| { s.quiet_hours = Some(quiet); Ok(()) }).unwrap();
        let now = nine_am();
        write(&cal, &placed("dentist", "Dentist", &at(now, 60), "Elm St"));
        let mut sent = Sent::load(&cal);
        tick(&store, &cal, &mut sent, now);
        tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(50));
        for id in ["cal-early:dentist:20990318T1000", "cal-reminder:dentist:20990318T1000"] {
            let (_, x) = card(&store, id).unwrap();
            assert_eq!((x.granted, x.grant_reason), (Attention::Inbox, GrantReason::QuietHours), "{id}");
            assert!(x.held_until.is_some(), "{id}: toasted when they end, if still useful");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_restart_inside_the_window_announces_nothing_again() {
        let (store, dir, cal) = calendar();
        let now = nine_am();
        write(&cal, &placed("dentist", "Dentist", &at(now, 60), "Elm St"));
        tick(&store, &cal, &mut Sent::load(&cal), now);
        let revision = store.revision();

        // From `Sent`: a new process reads the record and does not ask the store at all.
        tick(&store, &cal, &mut Sent::load(&cal), now + ChronoDuration::minutes(1));
        assert_eq!(store.revision(), revision);

        // From the duplicate id: the record is gone (pruned, unreadable), and the store answers
        // what it already holds as a duplicate.
        std::fs::remove_file(cal.join(".reminded.json")).unwrap();
        let mut forgot = Sent::load(&cal);
        tick(&store, &cal, &mut forgot, now + ChronoDuration::minutes(2));
        assert_eq!(cards(&store).len(), 1);
        assert_eq!(store.revision(), revision, "nothing changed in the store");
        assert!(forgot.has(Kind::Early, "dentist", &parse_stamp(&at(now, 60)).unwrap()), "and the record is made again");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_store_that_cannot_keep_it_is_asked_again_next_tick() {
        let dir = temp_dir();
        let cal = dir.join("calendar");
        std::fs::create_dir_all(&cal).unwrap();
        // A sidecar that is a directory cannot be read, so the store refuses every write.
        std::fs::create_dir_all(crate::store::sidecar_path(&dir.join("notifications.json"))).unwrap();
        let store = Store::open(dir.join("notifications.json"));
        assert!(store.blocked().is_some());
        let now = nine_am();
        write(&cal, &placed("dentist", "Dentist", &at(now, 60), "Elm St"));
        let mut sent = Sent::load(&cal);
        tick(&store, &cal, &mut sent, now);
        assert!(!sent.has(Kind::Early, "dentist", &parse_stamp(&at(now, 60)).unwrap()), "nothing remembered before success");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_card_of_a_deleted_or_moved_event_is_taken_back_but_never_on_an_incomplete_reading() {
        let (store, dir, cal) = calendar();
        let now = nine_am();
        write(&cal, &placed("dentist", "Dentist", &at(now, 60), "Elm St"));
        write(&cal, &placed("bank", "Bank", &at(now, 60), "High St"));
        let mut sent = Sent::load(&cal);
        tick(&store, &cal, &mut sent, now);
        assert_eq!(store.showing().len(), 2);

        // A file that will not parse makes the reading incomplete: nothing is taken back.
        std::fs::write(cal.join("bank.json"), "{half an ev").unwrap();
        tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(1));
        assert_eq!(store.showing().len(), 2, "an event that could not be read has not gone");

        // Deleted: its card is taken back, without feedback (the person did nothing).
        std::fs::remove_file(cal.join("bank.json")).unwrap();
        tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(2));
        let (bank, x) = card(&store, "cal-early:bank:20990318T1000").unwrap();
        assert!(bank.dismissed && x.feedback.is_empty());

        // Moved an hour later: the old card goes, and the new start is announced when its time
        // comes.
        write(&cal, &placed("dentist", "Dentist", &at(now, 120), "Elm St"));
        tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(3));
        assert!(store.showing().is_empty());
        tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(60));
        assert_eq!(store.showing().iter().filter_map(|n| n.notice.as_ref().map(|x| x.notice_id.clone())).collect::<Vec<_>>(), ["cal-early:dentist:20990318T1100"]);

        // Edited in place (same start, new title and place): the same card, with the new words.
        let (before, _) = card(&store, "cal-early:dentist:20990318T1100").unwrap();
        write(&cal, &placed("dentist", "Dentist (Dr Rao)", &at(now, 120), "Oak St"));
        tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(61));
        assert_eq!(store.showing().len(), 1);
        let (after, x) = card(&store, "cal-early:dentist:20990318T1100").unwrap();
        assert_eq!((after.id.as_str(), after.title.as_str()), (before.id.as_str(), "Dentist (Dr Rao) at 11:00, Oak St"));
        assert_eq!((x.grant_reason, x.feedback.len()), (before.notice.unwrap().grant_reason, 0), "not granted or counted again");
        // Unchanged on the next tick: nothing written.
        tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(62));
        assert_eq!(card(&store, "cal-early:dentist:20990318T1100").unwrap().0.revision, after.revision);
        // The place taken off: the early card, which says it has one, is taken back.
        write(&cal, &event("dentist", "Dentist (Dr Rao)", &at(now, 120), DEFAULT_REMINDER_MINUTES));
        tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(63));
        assert!(store.showing().is_empty());
        write(&cal, &placed("dentist", "Dentist (Dr Rao)", &at(now, 120), "Oak St"));

        // A mind's card is never the timer's to take back.
        let mut theirs = serde_json::json!({ "notice_id": "cal-early:x", "family": "calendar", "title": "t", "why": "w" });
        theirs["attention"] = "inbox".into();
        let post = notice::NoticePost {
            notice: theirs,
            harness: notice::NoticeHarness { id: "pi".into(), uid: Some(1000), ..Default::default() },
            os_id: false,
        };
        assert_eq!(store.add_notice(post, &dir)["stored"], true);
        tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(64));
        assert!(store.showing().iter().any(|n| n.notice.as_ref().is_some_and(|x| x.harness.id == "pi")));
        let _ = std::fs::remove_dir_all(dir);
    }
}
