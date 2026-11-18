//! Section 3: open promises between the person and someone on the invitation, in both directions.

use super::keywords::{person_terms, short_name};
use super::model::{clamp, Open, Row, MAX_PROMISES, MAX_ROW_CHARS};
use super::window::Person;

/// The most people whose promises are looked up.
const MAX_PEOPLE: usize = 6;

/// One open promise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Promise {
    pub text: String,
    /// What it came from, when that is something that can be opened.
    pub open: Option<Open>,
}

/// Where promises are read: the world model, or a stand-in in a test. Names match without regard
/// to case. Only promises still open are returned.
pub trait PromiseSource {
    /// What `name` promised.
    fn made_by(&self, name: &str) -> Result<Vec<Promise>, String>;
    /// What was promised to `name`.
    fn made_to(&self, name: &str) -> Result<Vec<Promise>, String>;
}

/// Up to three rows: "You told Ravi: send the Q4 numbers", "Maya said: share the deck".
pub fn section(source: &dyn PromiseSource, people: &[Person]) -> Result<Vec<Row>, String> {
    let mut rows: Vec<Row> = Vec::new();
    for person in people.iter().take(MAX_PEOPLE) {
        let name = short_name(&person.name);
        for term in person_terms(&person.name) {
            for p in source.made_to(&term)? {
                push(&mut rows, format!("You told {name}: {}", p.text), p.open);
            }
            for p in source.made_by(&term)? {
                push(&mut rows, format!("{name} said: {}", p.text), p.open);
            }
        }
    }
    rows.truncate(MAX_PROMISES);
    Ok(rows)
}

fn push(rows: &mut Vec<Row>, text: String, open: Option<Open>) {
    let text = clamp(&text, MAX_ROW_CHARS);
    if !rows.iter().any(|r| r.text == text) {
        rows.push(Row { text, open });
    }
}
