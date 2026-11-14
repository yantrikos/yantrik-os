//! Meeting briefs: ten minutes before a calendar event that has other people in it, the OS has
//! what the person already has about those people ready, and says so once.
//!
//! This is the OS's own work, deterministic and local. Nothing in this folder calls a model,
//! reaches a network or runs a tool for anyone: it reads the calendar service, the email cache,
//! the notes service, the world model's commitments and file names, and writes one row in the
//! anticipation table. The shell draws the card and opens whatever the person presses Open on,
//! as the person (`design/calendar-heads-up-2026-10-16.md`: the OS raises heads-ups, a mind
//! does not; `design/initiative-inbox-2026-10-16.md`: evidence is never followed on its own).
//!
//! One file per job:
//! - [`window`]: which events get a brief, and who the other people are.
//! - [`keywords`]: the words of a title and the names of people that a search is made of.
//! - [`emails`], [`notes`], [`promises`], [`files`]: one source each, behind a trait so the
//!   gatherer can be run without any of the real stores.
//! - [`assemble`]: the five sections of one meeting, under a time budget.
//! - [`engine`]: the tick: store once per event and start, replace on change, honour dismissal.
//! - [`local`]: the real sources, and the worker's tick.
//! - [`remote`]: the calendar, the notes service and the file walk, asked on a thread of their
//!   own so the companion's worker never waits on them; [`prefetch`] says what to ask ahead.
//! - [`model`]: the brief itself, as it is stored and as the shell reads it.

pub mod assemble;
pub mod emails;
pub mod engine;
pub mod files;
pub mod keywords;
pub mod local;
pub mod model;
pub mod notes;
pub mod prefetch;
pub mod promises;
pub mod remote;
pub mod window;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_engine;
#[cfg(test)]
mod tests_remote;
#[cfg(test)]
mod tests_window;
#[cfg(test)]
mod tests_sources;

pub use assemble::{gather, Gathered, Sources};
pub use engine::{cards, press, tick, unavailable, Outcome};
pub use model::{item_id, Brief, Card, Counts, Open, Outcomes, Press, Row};
pub use window::{Meeting, Own, Person};
