//! Section 2: the person's notes that mention the meeting's words or its people.

use super::keywords::{person_terms, title_keywords};
use super::model::{clamp, Open, Row, MAX_NOTES, MAX_ROW_CHARS};
use super::window::Person;

/// The most searches one meeting makes of the notes service. It reads every note per search.
pub const MAX_QUERIES: usize = 8;

/// One note found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteHit {
    pub title: String,
    /// As the service reports it; sorts the same way it reads.
    pub modified_at: String,
}

/// Where notes are searched: the notes service, or a stand-in in a test.
pub trait NoteSource {
    /// Notes whose title, text or tags contain `query`, any order.
    fn search(&self, query: &str) -> Result<Vec<NoteHit>, String>;
}

/// The searches a meeting makes: the title's words first, then each person's name.
pub fn queries(title: &str, people: &[Person]) -> Vec<String> {
    let mut out = title_keywords(title);
    for p in people {
        for term in person_terms(&p.name) {
            if !out.iter().any(|q| q.eq_ignore_ascii_case(&term)) {
                out.push(term);
            }
        }
    }
    out.truncate(MAX_QUERIES);
    out
}

/// Up to three notes, the ones that match most searches first, then the newest.
///
/// `Err` carries the first failure, so the caller knows the section may be incomplete; the notes
/// that were found are lost with it, because half a section is worse than a retry on the next tick.
pub fn section(source: &dyn NoteSource, title: &str, people: &[Person]) -> Result<Vec<Row>, String> {
    let mut found: Vec<(NoteHit, usize)> = Vec::new();
    let mut failed: Option<String> = None;
    for q in queries(title, people) {
        // Every search is tried even after one fails, so a source that fetches ahead learns all
        // of what was wanted at once.
        let hits = match source.search(&q) {
            Ok(hits) => hits,
            Err(why) => {
                failed.get_or_insert(why);
                continue;
            }
        };
        for hit in hits {
            if hit.title.trim().is_empty() {
                continue;
            }
            match found.iter_mut().find(|(h, _)| h.title == hit.title) {
                Some((_, count)) => *count += 1,
                None => found.push((hit, 1)),
            }
        }
    }
    if let Some(why) = failed {
        return Err(why);
    }
    found.sort_by(|(a, na), (b, nb)| nb.cmp(na).then_with(|| b.modified_at.cmp(&a.modified_at)));
    Ok(found
        .into_iter()
        .take(MAX_NOTES)
        .map(|(hit, _)| Row {
            text: clamp(&hit.title, MAX_ROW_CHARS),
            open: Some(Open::Note { title: hit.title }),
        })
        .collect())
}
