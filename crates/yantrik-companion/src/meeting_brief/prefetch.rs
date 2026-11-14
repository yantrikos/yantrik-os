//! What the fetcher is asked for before a brief needs it.
//!
//! A brief is made at ten minutes. If the notes and files were only asked for once the event
//! was inside that window, the answers would arrive a tick later and the card at nine. So the
//! searches of every meeting starting within the lead plus two minutes are asked for ahead, and
//! the brief is complete the first time it is gathered.

use super::keywords::title_keywords;
use super::model::LEAD_SECS;
use super::notes::queries;
use super::remote::{Missing, Snapshot};
use super::window::{select_within, Own};

/// How much earlier than the lead the searches are asked for.
pub const AHEAD_SECS: i64 = 120;

/// Add to `missing` the searches the meetings about to be due will make, that the snapshot does
/// not already hold. Nothing is asked of a calendar that has not answered.
pub fn want(snapshot: &Snapshot, now: i64, own: &Own, missing: &Missing) {
    let Some(events) = snapshot.events() else { return };
    for meeting in select_within(events, now, own, LEAD_SECS + AHEAD_SECS) {
        for query in queries(&meeting.title, &meeting.people) {
            if !snapshot.has_notes(&query) {
                missing.want_notes(&query);
            }
        }
        let words = title_keywords(&meeting.title);
        if !words.is_empty() && !snapshot.has_files(&words) {
            missing.want_files(&words);
        }
    }
}
