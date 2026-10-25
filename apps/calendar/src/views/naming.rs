//! Naming an event: a title and a date to the event they name, and an id back to the words a
//! person would use for it.

use chrono::Datelike;

use super::{month_short, parse_datetime, weekday_short, EventRef, Named};

/// The events on `date` whose title is `title`, compared without case or surrounding space.
///
/// `date` is matched as a prefix of the stored start, which is what the rest of this app does with
/// a day; it has to arrive as a full `YYYY-MM-DD` or it will match a range of days instead of one,
/// and the action that calls this checks that before asking.
///
/// Exact titles only. A substring match would make "delete standup" remove "standup with the
/// platform team" on a day holding both, and the caller who meant the other one would have no way
/// to tell that from the answer.
pub fn named_on(events: &[EventRef], title: &str, date: &str) -> Named {
    let wanted = title.trim().to_lowercase();
    let day = date.trim();
    let mut found: Vec<EventRef> = events
        .iter()
        .filter(|e| e.start.starts_with(day) && e.title.trim().to_lowercase() == wanted)
        .cloned()
        .collect();
    match found.len() {
        0 => Named::None,
        1 => Named::One(found.remove(0)),
        _ => Named::Ambiguous(found),
    }
}

/// One event the way a person would name it: "Dentist, Fri 25 Sep 13:00".
///
/// [`named_on`] answers name and date to id; this answers id back, because the other way an
/// action names an event is the store's uuid — the reliable one, which `delete_event` recommends
/// and which an approval grant is bound to byte for byte. A card that asks permission with a
/// uuid alone asks a question nobody can answer (#54), and the app is the only thing on the
/// machine that knows what its own id stands for. It publishes this through `describe` as an
/// index, and the shell puts the hit beside the arguments on the card.
///
/// An all-day event says so rather than reading out the midnight its row is stored at. A start
/// that will not parse contributes no time at all — the title alone is short, and a made-up
/// time in the sentence a person decides on is worse than that.
pub fn name_event(event: &EventRef) -> String {
    let Some(start) = parse_datetime(&event.start) else {
        return event.title.clone();
    };
    let day = start.date();
    let head = format!("{}, {} {} {}", event.title, weekday_short(day), day.day(), month_short(day.month()));
    if event.is_all_day {
        format!("{head}, all day")
    } else {
        format!("{head} {}", start.format("%H:%M"))
    }
}

/// How many entries the naming index publishes at most.
///
/// `describe` is read by agents, and an unbounded table inside one reply is exactly what those
/// replies are told to avoid: a year of a busy calendar would ride along with every "what is on
/// today?" A fixed cap keeps the reply bounded whatever the loaded range holds, and 200 is far
/// more than a visible range of weeks carries — a caller that hit it is a caller whose window has
/// been sitting on a very full month, and even then it keeps the newest names, which are the ones
/// an action taken now was read from.
pub const NAMING_CAP: usize = 200;

/// The id→name index `describe` publishes, from the events the app has in hand (#54).
///
/// Bounded both ways, deliberately. Only the events passed in get a name — the app passes the
/// loaded visible range, the same events the rest of the describe reply is already derived from,
/// so the index never widens what `describe` covers. And at most [`NAMING_CAP`] entries: past
/// that the oldest starts are dropped, because the id an action carries was read from a recent
/// view. A start that will not parse counts as oldest of all — a garbage row should not displace
/// a real event from the index, and [`name_event`] still names it by title if it survives.
pub fn naming_index(events: &[EventRef]) -> Vec<(String, String)> {
    let mut by_start: Vec<&EventRef> = events.iter().collect();
    // `None` sorts before every `Some`, so unparsable starts sit at the old end. Stable, so
    // events the store returned in one order keep it within a shared (or absent) start.
    by_start.sort_by_key(|e| parse_datetime(&e.start));
    if by_start.len() > NAMING_CAP {
        by_start = by_start.split_off(by_start.len() - NAMING_CAP);
    }
    by_start
        .into_iter()
        .map(|e| (e.id.clone(), name_event(e)))
        .collect()
}
