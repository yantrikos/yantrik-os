//! The answer each intent is owed, settled in the command where its own work lands.
//!
//! An intent's reply is read from the model when it settles. If it settled only once nothing of
//! it was left running, a forced re-read under the same invocation could leave the reply waiting
//! while something else — the person's trash icon, another call — wrote the model, and the Mind
//! would be answered about an event it never named (review of #921, finding 1). So each intent
//! settles in the command that carries its own result: a write when the store's answer lands, a
//! read when its read lands. Nothing can land between the answer being written and being read.
//!
//! A reading intent's answer is built from what it asked for and what its own read listed, never
//! from wherever the window is when that read lands: another call or the person's own navigation
//! may have moved the window in between (review of #921, delta Low 1). When it has, the answer
//! says so in `window_moved_since`, and still describes what the Mind asked for.

use serde_json::{json, Value};
use yantrik_app::effects::Effects;

use crate::events::{self, CalEvent};
use crate::model::Model;
use crate::views::{self, ViewMode};
use crate::work::{Listing, Read};
use crate::Calendar;

/// Which reading intent is waiting on its read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    SelectDay,
    /// The date as it was asked for.
    ShowDate(String),
    ShowMonth,
    Today,
    SetView,
}

/// Where the intent put the window: the month, the day picked in it (0 for none) and the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub year: i32,
    pub month: u32,
    pub day: i32,
    pub view: ViewMode,
}

impl Target {
    fn of(m: &Model) -> Target {
        Target { year: m.year, month: m.month, day: m.selected_day, view: m.view }
    }
}

/// What a reading intent is owed: its kind, and, once its command has moved the window, where it
/// put it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owed {
    pub kind: Kind,
    pub target: Option<Target>,
}

/// Remember what the invocation this command runs under is owed, if it came through the surface.
pub fn ask(m: &mut Model, fx: &Effects<Calendar>, kind: Kind) {
    if let Some(id) = fx.invocation() {
        m.asked.insert(id, Owed { kind, target: None });
    }
}

/// The window has moved for the command being applied. If a reading intent is owed under its
/// invocation, where the window now is becomes that intent's target, and its read must list what
/// the store holds for it whatever the revision says: the answer is built from that listing. Says
/// whether a read is owed.
pub fn target_now(m: &mut Model, fx: &Effects<Calendar>) -> bool {
    let target = Target::of(m);
    match fx.invocation().and_then(|id| m.asked.get_mut(&id)) {
        Some(owed) => {
            owed.target = Some(target);
            true
        }
        None => false,
    }
}

/// Settle the invocation this command runs under with `answer`, if one is owed. The reply reads
/// `m.answer` when this command commits, before anything else can change it.
pub fn settle(m: &mut Model, fx: &mut Effects<Calendar>, answer: Value) {
    if fx.invocation().is_some() {
        m.answer = answer;
        fx.settle(Ok(()));
    }
}

/// A read landed under an invocation: answer the reading intent that asked for it, from its own
/// target and this read's own result.
pub fn read_landed(m: &mut Model, fx: &mut Effects<Calendar>, listing: &Listing) {
    let Some(owed) = fx.invocation().and_then(|id| m.asked.remove(&id)) else { return };
    let target = owed.target.unwrap_or_else(|| Target::of(m));
    let held: &[CalEvent] = match &listing.read {
        Read::Listed { events, .. } => events,
        // A failed read shows no events, as the window does after one.
        Read::Failed(_) => &[],
        // The store could not be asked and the window held this range (`work::reread`): what is
        // in hand is that range's events, as before the read.
        Read::Unmoved if m.range == Some(listing.wanted) => &m.events,
        Read::Unmoved => &[],
    };
    let mut answer = reading(m, &owed.kind, target, held);
    if Target::of(m) != target {
        answer["window_moved_since"] = json!(moved_since(m));
    }
    settle(m, fx, answer);
}

/// A read under an invocation ended without a result: the framework refuses the call, and what it
/// was owed is forgotten.
pub fn read_failed(m: &mut Model, fx: &Effects<Calendar>) {
    if let Some(id) = fx.invocation() {
        m.asked.remove(&id);
    }
}

/// The sentence for an answer whose window has moved since: where it is now.
fn moved_since(m: &Model) -> String {
    let day = if m.selected_day > 0 { format!("day {}", m.selected_day) } else { "no day picked".to_string() };
    format!(
        "The window has since moved: it shows {}, {day}, in the {} view.",
        m.month_title(),
        m.view.as_str()
    )
}

/// What the intent's target holds, in the shape the intent always answered with.
fn reading(m: &Model, kind: &Kind, t: Target, held: &[CalEvent]) -> Value {
    let showing = events::month_title(t.year, t.month);
    let titles: Vec<String> =
        events::events_on_day(held, t.year, t.month, t.day).into_iter().map(|e| e.title.clone()).collect();
    let selected = views::selected_date(t.year, t.month, t.day);
    match kind {
        Kind::SelectDay => json!({ "day": t.day, "events": titles }),
        Kind::ShowDate(date) => {
            // The week the date is in, on this window's week start: in the week view that is what
            // is then on screen, and the date is in it.
            let (week_start, week_end) = views::week_bounds(selected, m.week_start);
            json!({
                "showing": showing,
                "date": date,
                "week_start": week_start.to_string(),
                "week_end": week_end.to_string(),
                "events": titles,
            })
        }
        Kind::ShowMonth => json!({ "showing": showing }),
        Kind::Today => json!({ "showing": showing, "day": t.day }),
        Kind::SetView => shown(m, t, held),
    }
}

/// What `set_view` answers: what is on screen, not the word that was asked for. This answered
/// `{"view": "week"}` while the week grid was empty of everything -- no events, no column headers,
/// a "now" line on midnight -- which reads as a view that was shown. It is the same fabrication
/// `add_event` was making in September, one layer up.
fn shown(m: &Model, t: Target, held: &[CalEvent]) -> Value {
    let source = events::source_events(held);
    let selected = views::selected_date(t.year, t.month, t.day);
    match t.view {
        ViewMode::Week => {
            let (start, end) = views::week_bounds(selected, m.week_start);
            json!({
                "view": "week",
                "week_start": start.to_string(),
                "week_end": end.to_string(),
                "events": views::week_view(&source, selected, m.week_start).events.len(),
            })
        }
        ViewMode::Day => {
            let day = views::day_view(&source, selected);
            json!({ "view": "day", "day": day.title, "events": day.events.len() })
        }
        ViewMode::Month => json!({
            "view": "month",
            "showing": events::month_title(t.year, t.month),
            "events": events::events_in_month(held, t.year, t.month),
        }),
    }
}
