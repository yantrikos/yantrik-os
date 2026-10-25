//! What Calendar is: the model every command changes and every render and `describe` reads.

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use chrono::{NaiveDate, NaiveDateTime};
use serde_json::Value;
use yantrik_app::cx::Inbound;
use yantrik_app::intents::InvocationId;
use yantrik_app::mind::Mind;
use yantrik_app::queue::Post;
use yantrik_app::rail::{Proposal, Rail};
use yantrik_ipc_contracts::calendar::CalendarRevision;

use crate::answers::Owed;
use crate::backend::Machine;
use crate::events::{self, CalEvent};
use crate::notice::Notice;
use crate::today::{Shown, Today};
use crate::views::{self, ViewMode, WeekStart};
use crate::work::{Held, Range};
use crate::Calendar;

/// The parts of the model `render` redraws by name.
///
/// The month, the selected day, the view and everything drawn from the events in hand: the grid,
/// the agenda and both time grids.
pub const PART_CALENDAR: &str = "calendar";
/// The "now" line and the form's timezone strip.
pub const PART_CLOCK: &str = "clock";
pub const PART_NOTICE: &str = "notice";
/// Whether the new-event form is open.
pub const PART_FORM: &str = "form";
/// What the form's fields hold. The person types into them (they are the window's in-out
/// properties), so they are written only when a command names this part — opening the form —
/// and never on a redraw of everything, which would put back what the person had typed over.
pub const PART_FORM_FIELDS: &str = "form.fields";
/// The header's AI button, and the card it puts up when the rail cannot ask.
pub const PART_AI: &str = "ai";
/// What a command names when nothing on screen changed.
pub const NOTHING: &str = "nothing";

/// How long an event runs when nothing says otherwise.
pub const DEFAULT_EVENT_MINUTES: i32 = 60;

/// The new-event form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Form {
    pub open: bool,
    /// What the fields are filled with when the form opens.
    pub title: String,
    pub date: String,
    pub time: String,
    pub notes: String,
    /// How long the event the form is about should run.
    ///
    /// The form has no duration field -- it asks for a title, a date, a time and notes -- so a
    /// template's answer to "how long" waits here until Save. `DEFAULT_EVENT_MINUTES` otherwise.
    pub duration_min: i32,
    /// A save of the form is in flight: a second press waits for it rather than storing the
    /// event twice.
    pub saving: bool,
    /// Which opening of the form this is. A Save that lands closes the form only if it is still
    /// the one that was saved, not one the person has opened since.
    pub generation: u64,
}

impl Default for Form {
    fn default() -> Form {
        Form {
            open: false,
            title: String::new(),
            date: String::new(),
            time: String::new(),
            notes: String::new(),
            duration_min: DEFAULT_EVENT_MINUTES,
            saving: false,
            generation: 0,
        }
    }
}

/// The header's AI button.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ai {
    /// The button's spinner: a question about the day is out.
    pub working: bool,
    /// The day the question was about, as its card says it ("from September 2026 23").
    pub day: String,
    /// The card the button puts up itself when the rail cannot ask (the Mind's reach is not
    /// ready): the question still goes to the companion, as it always did, and what came back is
    /// drawn on the same card the rail draws on.
    pub card: Option<Proposal>,
    pub asking: bool,
}

/// The reads of the store: numbered in the order they are taken, so that of two reads for the
/// range on screen the later one is what is shown.
#[derive(Clone, Debug, Default)]
pub struct Reads {
    pub sequence: Arc<AtomicU64>,
    /// The number of the read whose events are in hand.
    pub shown: u64,
    /// Plain reads in flight. The twenty-second watch waits while one is, rather than piling a
    /// second read onto a slow service.
    pub in_flight: u32,
}

/// The core's endpoint, so `describe` can hand back a read it had to make (`follow.rs`).
#[derive(Clone)]
pub struct Inbox(pub Post<Inbound<Calendar>>);

impl std::fmt::Debug for Inbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Inbox")
    }
}

pub struct Model {
    pub year: i32,
    pub month: u32,
    /// The day picked in the month shown; 0 when none is (after moving to another month).
    pub selected_day: i32,
    pub view: ViewMode,
    pub week_start: WeekStart,
    /// The day this window last saw as today: the marker, the day it opens on, and the day a
    /// window left on today follows when the date turns.
    pub today: Today,
    /// The clock as last read, for the "now" line, and the local clock's distance from UTC.
    pub now: NaiveDateTime,
    pub utc_offset: i32,
    pub events: Vec<CalEvent>,
    /// The date range `events` was read for, or `None` when the last read failed and the next
    /// redraw should ask again rather than show an empty calendar for the life of the process.
    pub range: Option<Range>,
    /// What the store was at when `events` was read, or `None` when that could not be asked.
    ///
    /// The window is not the only writer any more — the mind's own tools and Google sync go
    /// through the same service — so "the range has not changed" stopped being a reason to
    /// believe the events in hand are the events on disk. See `work::reread`.
    pub revision: Option<CalendarRevision>,
    pub reads: Reads,
    pub form: Form,
    pub notice: Notice,
    /// The events a trash icon is taking off, so a second press of it waits for the first.
    pub removing: Vec<String>,
    /// Creates, deletes and updates sent to the store and not yet landed.
    pub writes: u32,
    /// The session or a person asked the app to end (`Event::ExitRequested`): it ends once
    /// `writes` is zero, and starts no new write meanwhile. Nothing cancels it.
    pub ending: bool,
    /// The person closed the window while writes were in flight: the window is closed and the app
    /// lingers without one (`fx.linger`), ending once they land, unless a second launch opens the
    /// window again first (`Event::Presented`), which cancels the close.
    pub closing: bool,
    pub ai: Ai,
    pub rail: Rail,
    pub mind: Arc<dyn Mind>,
    pub machine: Machine,
    /// What the surface call being settled is answered with, written in the command that settles
    /// it (`answers.rs`).
    pub answer: Value,
    /// What each reading intent still waiting on its read will answer with, by invocation.
    pub asked: HashMap<InvocationId, Owed>,
    pub inbox: Option<Inbox>,
}

impl Model {
    /// A window opening on today, with nothing read yet.
    pub fn new(machine: Machine, mind: Arc<dyn Mind>) -> Model {
        let today = Today::new(machine.clock.as_ref());
        let opens_on = Shown::on(today.date());
        let now = machine.clock.now();
        Model {
            year: opens_on.year,
            month: opens_on.month,
            selected_day: opens_on.day as i32,
            view: ViewMode::Month,
            week_start: machine.week_start,
            today,
            now,
            utc_offset: machine.clock.utc_offset_seconds(),
            events: Vec::new(),
            range: None,
            revision: None,
            reads: Reads::default(),
            form: Form::default(),
            notice: Notice::default(),
            removing: Vec::new(),
            writes: 0,
            ending: false,
            closing: false,
            ai: Ai::default(),
            rail: Rail::new(mind.clone()),
            mind,
            machine,
            answer: Value::Null,
            asked: HashMap::new(),
            inbox: None,
        }
    }

    /// The date the week and day views are about (see `views::selected_date`).
    pub fn selected_date(&self) -> NaiveDate {
        views::selected_date(self.year, self.month, self.selected_day)
    }

    /// The dates the views on screen need.
    pub fn visible_range(&self) -> Range {
        views::visible_range(self.year, self.month, self.view, self.selected_day, self.week_start)
    }

    /// The week the selected day is in, on this window's week start.
    pub fn week_bounds(&self) -> Range {
        views::week_bounds(self.selected_date(), self.week_start)
    }

    pub fn month_title(&self) -> String {
        events::month_title(self.year, self.month)
    }

    /// What the window holds, for a read deciding whether to list again.
    pub fn held(&self) -> Held {
        Held { range: self.range, revision: self.revision.clone() }
    }

    /// What is on the selected day, in the order the store returned it.
    pub fn on_selected_day(&self) -> Vec<&CalEvent> {
        events::events_on_day(&self.events, self.year, self.month, self.selected_day)
    }

    /// The titles on the selected day, as the reading intents answer them.
    pub fn titles_on_selected_day(&self) -> Vec<String> {
        self.on_selected_day().into_iter().map(|e| e.title.clone()).collect()
    }
}
