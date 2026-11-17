//! The companion's changing tools ask the person before they change anything (#1311).
//!
//! One shared ask lives in `yantrik_companion_tools::asks`; this module is where each tool of
//! this crate builds its card: what the person is shown, whole, about the thing about to change.
//! A tool calls the helper for its kind of change at the point of effect, after its arguments
//! are validated, and hands it the effect as a closure that runs once, only on the person's
//! Allow. No tool builds a card of its own and none asks by hand.

use yantrik_companion_tools::asks::{act, Card};

pub use yantrik_companion_tools::asks::{fields, Question, PLAIN_ROWS, PLAIN_VALUE_CHARS, TOTAL_CHARS, WHOLE_VALUE_CHARS};

pub mod calendar;
pub mod forget_topic;
pub mod mail;
pub mod messages;
pub mod plans;
pub mod words;

/// Plan work (a recipe, a schedule, an automation, a queued task, a started recipe): ask, then run `effect`.
pub fn plan<T>(card: &Card, effect: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    act(Question::PlanWork, card, effect)
}

#[cfg(test)]
mod tests_calendar;
#[cfg(test)]
mod tests_forget;
#[cfg(test)]
mod tests_mail;
#[cfg(test)]
mod tests_messages;
#[cfg(test)]
mod tests_plans;
#[cfg(test)]
mod tests_sources;
#[cfg(test)]
mod tests_support;
