//! The brain's agenda: one ordered store of everything worth doing, and the reason for the order.
//!
//! Plain code over a `rusqlite::Connection`: no model, no network, no clock (the caller passes
//! `now`), no thread, no timer. The caller decides when to tick.
//!
//! - [`store`]    — the `agenda` table, upsert, state changes, snooze and dismiss
//! - [`gather`]   — mirrors five other stores into the table (reads them, never writes them)
//! - [`senses`]   — mirrors the calendar and the notes library (what the person is doing this week)
//! - [`notes_dir`] — reads the notes library's `.md` files (read-only), for `senses`
//! - [`prepare`]  — the first anticipate rule: an event soon, a linked note with open items
//! - [`told`]     — "What's going on": the note a mind starts every turn with
//! - [`tick`]     — one pass for the shell's Think arm: the switches, then `gather`, then `senses`
//! - [`salience`] — the readable score and the ranked list
//! - [`describe`] — counts and the top entries, for people to look at

pub mod describe;
pub mod gather;
pub mod notes_dir;
pub mod prepare;
pub mod salience;
pub mod senses;
pub mod store;
pub mod tick;
pub mod triggers;
pub mod told;

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests_gather;
#[cfg(test)]
mod tests_prepare;
#[cfg(test)]
mod tests_private;
#[cfg(test)]
mod tests_review;
#[cfg(test)]
mod tests_salience;
#[cfg(test)]
mod tests_senses;
#[cfg(test)]
mod tests_store;
#[cfg(test)]
mod tests_told;

pub use describe::describe;
pub use gather::{gather, GatherReport, SourceReport};
pub use salience::{ranked, salience, Salience, UrgencyFn};
pub use store::{migrate, Entry, NewEntry};
pub use tick::{tick, Switches, TickReport};
pub use told::{told, Told};

/// What kind of thing an entry is. Stored as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Something the person said they would do (`wm_commitments`).
    Promise,
    /// An open thread the monitor is tracking (`life_threads`).
    Thread,
    /// Two memories that disagree (`conflicts`).
    Conflict,
    /// A scheduled task (`scheduled_tasks`).
    Schedule,
    /// A queued task (`task_queue`).
    Task,
    /// A calendar event in the next seven days.
    Event,
    /// A note edited lately with unticked items.
    NoteTodo,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Promise => "promise",
            Kind::Thread => "thread",
            Kind::Conflict => "conflict",
            Kind::Schedule => "schedule",
            Kind::Task => "task",
            Kind::Event => "event",
            Kind::NoteTodo => "note_todo",
        }
    }
    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "promise" => Kind::Promise,
            "thread" => Kind::Thread,
            "conflict" => Kind::Conflict,
            "schedule" => Kind::Schedule,
            "task" => Kind::Task,
            "event" => Kind::Event,
            "note_todo" => Kind::NoteTodo,
            _ => return None,
        })
    }
}

/// Where an entry stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Open,
    Prepared,
    WaitingPerson,
    WaitingModel,
    Snoozed,
    Done,
    Dropped,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Open => "open",
            State::Prepared => "prepared",
            State::WaitingPerson => "waiting_person",
            State::WaitingModel => "waiting_model",
            State::Snoozed => "snoozed",
            State::Done => "done",
            State::Dropped => "dropped",
        }
    }
    pub fn parse(s: &str) -> Option<State> {
        Some(match s {
            "open" => State::Open,
            "prepared" => State::Prepared,
            "waiting_person" => State::WaitingPerson,
            "waiting_model" => State::WaitingModel,
            "snoozed" => State::Snoozed,
            "done" => State::Done,
            "dropped" => State::Dropped,
            _ => return None,
        })
    }
    /// Done and dropped entries are closed: they are not ranked or counted as active.
    pub fn is_closed(self) -> bool {
        matches!(self, State::Done | State::Dropped)
    }
}

/// What kind of step is next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextStep {
    /// Plain code can take it.
    Code,
    /// It needs the person.
    Person,
    /// It needs a model turn.
    Model,
}

impl NextStep {
    pub fn as_str(self) -> &'static str {
        match self {
            NextStep::Code => "code",
            NextStep::Person => "person",
            NextStep::Model => "model",
        }
    }
    pub fn parse(s: &str) -> Option<NextStep> {
        Some(match s {
            "code" => NextStep::Code,
            "person" => NextStep::Person,
            "model" => NextStep::Model,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests_triggers;
