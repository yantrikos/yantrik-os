//! One meeting's five sections, gathered from the local sources under a time budget.

use std::time::{Duration, Instant, SystemTime};

use super::emails::{self, EmailSource};
use super::files::{self, FileRoots};
use super::keywords::title_keywords;
use super::model::{clamp, item_id, Brief, Row, DESCRIPTION_LINES, MAX_ROW_CHARS};
use super::notes::{self, NoteSource};
use super::promises::{self, PromiseSource};
use super::window::{fingerprint, Meeting, Own};

/// How long gathering one meeting may take before the sources not yet asked are skipped. The
/// target is 200 ms; this leaves the rest for storing and drawing.
pub const BUDGET: Duration = Duration::from_millis(150);

/// The four local places a brief is made from. Nothing else: there is no way to hand the gatherer
/// a model or a network client.
pub struct Sources<'a> {
    pub emails: &'a dyn EmailSource,
    pub notes: &'a dyn NoteSource,
    pub promises: &'a dyn PromiseSource,
    pub files: &'a FileRoots,
}

/// A brief, and whether every source answered. An empty brief that is not complete is tried
/// again on the next tick rather than recorded as "nothing to say".
#[derive(Debug, Clone)]
pub struct Gathered {
    pub brief: Brief,
    pub complete: bool,
}

/// The first lines of an event's description.
fn description_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| clamp(l, MAX_ROW_CHARS))
        .filter(|l| !l.is_empty())
        .take(DESCRIPTION_LINES)
        .collect()
}

/// Gather the brief for `meeting` at `now` (unix seconds).
pub fn gather(meeting: &Meeting, sources: &Sources<'_>, own: &Own, now: i64) -> Gathered {
    let started = Instant::now();
    let mut complete = true;
    // Each source is asked in turn, while there is time: one that fails or is skipped for time
    // leaves its section empty and the brief incomplete.
    let mut ask = |source: &str, run: &dyn Fn() -> Result<Vec<Row>, String>| -> Vec<Row> {
        if started.elapsed() >= BUDGET {
            complete = false;
            return Vec::new();
        }
        match run() {
            Ok(rows) => rows,
            Err(why) => {
                tracing::debug!(source, error = %why, "meeting brief: a source did not answer");
                complete = false;
                Vec::new()
            }
        }
    };

    let mail = ask("email", &|| emails::section(sources.emails, &meeting.people, own, now));
    let promises = ask("promises", &|| promises::section(sources.promises, &meeting.people));
    let notes = ask("notes", &|| notes::section(sources.notes, &meeting.title, &meeting.people));
    let files = ask("files", &|| Ok(files::section(sources.files, &title_keywords(&meeting.title), SystemTime::now()).0));

    Gathered {
        brief: Brief {
            event_id: meeting.event_id.clone(),
            title: meeting.title.clone(),
            start: meeting.start,
            end: meeting.end,
            when: meeting.when.clone(),
            place: meeting.place.clone(),
            people: meeting.people.iter().map(|p| p.name.clone()).collect(),
            fingerprint: fingerprint(meeting),
            emails: mail,
            notes,
            promises,
            files,
            description: description_lines(&meeting.description),
        },
        complete,
    }
}

/// The id this meeting's brief is stored under.
pub fn id_of(meeting: &Meeting) -> String {
    item_id(&meeting.event_id, meeting.start)
}
