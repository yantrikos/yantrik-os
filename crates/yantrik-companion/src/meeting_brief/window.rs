//! Which events get a brief: timed, not cancelled, starting within ten minutes and not yet
//! started, with at least one other person on the invitation.

use std::collections::HashSet;

use chrono::{Local, NaiveDateTime, TimeZone};
use yantrik_ipc_contracts::calendar::{parse_stamp, Attendee, CalendarEvent};

use super::keywords::display_name;
use super::model::LEAD_SECS;

/// The person's own identity, so their address is never listed as someone to prepare for.
#[derive(Debug, Clone, Default)]
pub struct Own {
    /// Lower case.
    pub addresses: HashSet<String>,
    pub name: String,
}

impl Own {
    pub fn new(addresses: impl IntoIterator<Item = String>, name: &str) -> Self {
        Self {
            addresses: addresses.into_iter().map(|a| a.trim().to_lowercase()).filter(|a| !a.is_empty()).collect(),
            name: name.trim().to_lowercase(),
        }
    }

    fn is_me(&self, a: &Attendee) -> bool {
        let email = a.email.trim().to_lowercase();
        if !email.is_empty() {
            return self.addresses.contains(&email);
        }
        !self.name.is_empty() && a.name.trim().to_lowercase() == self.name
    }

    /// Whether `address` is one of the person's own.
    pub fn has_address(&self, address: &str) -> bool {
        self.addresses.contains(&address.trim().to_lowercase())
    }
}

/// Someone else on the invitation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    /// Lower case; may be empty for a name typed without an address.
    pub email: String,
}

/// An event that is due a brief, with what the brief is made from.
#[derive(Debug, Clone)]
pub struct Meeting {
    pub event_id: String,
    pub title: String,
    pub start: i64,
    pub end: i64,
    pub when: String,
    pub place: Option<String>,
    pub description: String,
    pub people: Vec<Person>,
}

/// Local, naive stamp to unix seconds. An hour that does not exist (the clocks went forward) is
/// read as the hour after it, the way the calendar shows it.
pub fn to_unix(stamp: NaiveDateTime) -> Option<i64> {
    match Local.from_local_datetime(&stamp) {
        chrono::LocalResult::Single(t) => Some(t.timestamp()),
        chrono::LocalResult::Ambiguous(first, _) => Some(first.timestamp()),
        chrono::LocalResult::None => Local.from_local_datetime(&(stamp + chrono::Duration::hours(1))).earliest().map(|t| t.timestamp()),
    }
}

/// Whether the title says the event was called off. The calendar keeps no status of its own, and
/// mail and calendar systems mark a cancellation in the title.
pub fn is_cancelled(title: &str) -> bool {
    let t = title.trim_start().to_lowercase();
    ["cancelled", "canceled"].iter().any(|word| {
        t.strip_prefix(word).is_some_and(|rest| rest.starts_with([':', ' ', '-', '\u{2013}', '\u{2014}']) || rest.is_empty())
    })
}

/// The invitation's other people: everyone not the person, each once.
pub fn others(attendees: &[Attendee], own: &Own) -> Vec<Person> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for a in attendees.iter().filter(|a| !own.is_me(a)) {
        let email = a.email.trim().to_lowercase();
        let name = display_name(&a.name, &email);
        if name.is_empty() && email.is_empty() {
            continue;
        }
        if seen.insert(if email.is_empty() { name.to_lowercase() } else { email.clone() }) {
            out.push(Person { name, email });
        }
    }
    out
}

fn clock(stamp: &str) -> Option<String> {
    parse_stamp(stamp).map(|t| t.format("%H:%M").to_string())
}

/// The events of `events` that are due a brief at `now` (unix seconds): a timed event that is
/// not cancelled, whose start is ahead by at most ten minutes, with someone else on it.
pub fn select(events: &[CalendarEvent], now: i64, own: &Own) -> Vec<Meeting> {
    let mut due = Vec::new();
    for e in events {
        if e.is_all_day || is_cancelled(&e.title) {
            continue;
        }
        let (Some(start_at), Some(end_at)) = (parse_stamp(&e.start), parse_stamp(&e.end)) else { continue };
        let (Some(start), Some(end)) = (to_unix(start_at), to_unix(end_at)) else { continue };
        let ahead = start - now;
        if ahead <= 0 || ahead > LEAD_SECS {
            continue;
        }
        let people = others(&e.attendees, own);
        if people.is_empty() {
            continue;
        }
        let when = match (clock(&e.start), clock(&e.end)) {
            (Some(a), Some(b)) => format!("{a}\u{2013}{b}"),
            (Some(a), None) => a,
            _ => String::new(),
        };
        due.push(Meeting {
            event_id: e.id.clone(),
            title: e.title.trim().to_string(),
            start,
            end: end.max(start + 60),
            when,
            place: e.location.as_deref().map(str::trim).filter(|l| !l.is_empty()).map(String::from),
            description: e.description.clone(),
            people,
        });
    }
    due.sort_by_key(|m| m.start);
    due
}

/// A stable fingerprint of what a brief depends on that the person can change: the title, the
/// start and the people. FNV-1a, because it is stored and must not change between builds.
pub fn fingerprint(m: &Meeting) -> String {
    let mut people: Vec<String> = m.people.iter().map(|p| format!("{}<{}>", p.name.to_lowercase(), p.email)).collect();
    people.sort();
    let text = format!("{}\u{1f}{}\u{1f}{}", m.title, m.start, people.join("\u{1e}"));
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}
