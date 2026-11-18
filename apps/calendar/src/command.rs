//! Every change to the model is one of these, wherever it came from: a button, a timer, a job's
//! result, the Mind.

use yantrik_app::mind::AskError;
use yantrik_app::rail::RailCommand;
use yantrik_ipc_contracts::calendar::CalendarEvent;

use crate::asks::{AddAsk, NameAsk, UpdateAsk};
use crate::model::Inbox;
use crate::notice;
use crate::views::ViewMode;
use crate::work::{Listing, Range};

/// Which way `show_month` goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Next,
    Previous,
}

/// Which door a new event came in by: the form's Save, or `add_event`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Door {
    Form,
    Surface,
}

/// Which job ended without a result (a panic, or the pool at its bound).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Work {
    Save(Door),
    /// The event a delete was taking off, by id when it was known.
    Remove(Option<String>),
    Change,
    Open,
    Between,
}

/// A create landed. The store is read back afterwards in a job of its own, so a read that fails
/// cannot make a stored event look unsaved.
#[derive(Debug, Clone)]
pub struct Saved {
    pub door: Door,
    pub title: String,
    pub date: String,
    pub time: String,
    pub all_day: bool,
    /// The id the store gave the event and the reminder lead it recorded.
    pub result: Result<(String, u32), String>,
    /// The opening of the form that was saved (`Form::generation`); unused for `add_event`.
    pub form: u64,
}

/// A delete landed.
#[derive(Debug, Clone)]
pub struct Removed {
    pub id: String,
    /// The event as a notice names it: “its title”, or the id it was asked for by.
    pub named: String,
    /// The title the store held, or why the event is still there.
    pub result: Result<String, String>,
}

/// An update landed.
#[derive(Debug, Clone)]
pub struct Moved {
    pub id: String,
    /// What the store holds after the write, read back.
    pub result: Result<CalendarEvent, String>,
}

/// The own-creation rule said no: the event is untouched.
#[derive(Debug, Clone)]
pub struct Kept {
    pub refused: notice::Refused,
    pub title: String,
    pub who: Option<String>,
    pub id: String,
    /// The caller's sentence: the rule, the ids and the other door.
    pub why: String,
}

#[derive(Debug, Clone)]
pub enum Command {
    // ── The window ──
    PrevMonth,
    NextMonth,
    DayClicked(i32),
    /// The header's view switch: `view-mode` as the screen numbers it.
    SwitchView(i32),
    Today,
    /// The New button: a blank form on the selected day.
    OpenForm,
    /// A template pressed: the form, with its title and its length.
    UseTemplate { title: String, minutes: i32 },
    SaveForm { title: String, date: String, time: String, notes: String },
    CancelForm,
    /// The trash icon on a row of the selected day's agenda, by its position in that list.
    DeleteRow(i32),
    DismissNotice,
    /// The notice's "Open event": go to the day of the event a refusal was about.
    OpenNoticeEvent,
    /// The header's AI button, and the rail's "What does this day look like?".
    Explain,
    /// The header's AI button pressed again: the card goes away.
    DismissAi,
    Rail(RailCommand),
    /// The core's endpoint, handed over once at wire time (`model::Inbox`).
    Attach(Inbox),

    // ── Time ──
    /// The minute timer: the "now" line moves.
    Minute,
    /// The day may have turned.
    Midnight,
    /// The twenty-second check that the store has not moved under the window.
    Watch,
    /// Read the store for the range on screen if anything moved: the first read, at start.
    Refresh,
    /// The window became the active one again.
    Activated,
    /// The process is asked to end (`Event::ExitRequested`): a logout, a signal, the display
    /// going, or a person's quit.
    Ending,
    /// The person closed the window while writes were still in flight: it closes, and the app
    /// runs on without it until they have landed (`fx.linger`).
    Closing(yantrik_app::app::WindowId),
    /// A second launch opened the window again while that close waited (`Event::Presented`): the
    /// close is cancelled.
    Reopened,

    // ── The surface ──
    SelectDay(i32),
    ShowDate(String),
    Between(Range),
    ShowMonth(Step),
    SetView(ViewMode),
    Add(Box<AddAsk>),
    Delete { ask: NameAsk, own: bool, caller: crate::who::Caller },
    Update { ask: Box<UpdateAsk>, own: bool, caller: crate::who::Caller },

    // ── What the jobs found ──
    Listed(Box<Listing>),
    /// The read `describe` had to make because the store had moved (`follow.rs`), handed back so
    /// the window shows what the Mind was just told.
    Followed(Box<Listing>),
    /// A plain read ended without a result.
    ReadFailed,
    Saved(Box<Saved>),
    Removed(Box<Removed>),
    Moved(Box<Moved>),
    Kept(Box<Kept>),
    /// A surface call's job refused before it wrote anything: the event could not be named, or
    /// read. Nothing is said in the window, as before.
    Declined(String),
    WorkFailed(Work),
    NoticeEvent(Result<CalendarEvent, String>),
    BetweenRead(Result<serde_json::Value, String>),
    /// The header's own question, answered (the rail could not ask).
    Explained(Result<String, AskError>),
}
