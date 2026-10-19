//! What a cell holds once it is worked out, and what went wrong when it could not be.
//!
//! The review of the first engine (PR #727) found every failure collapsed into `#REF!`: an
//! unknown function, a typo and a cell holding text all read the same. An [`Error`] here carries
//! two things — the short code that stays in the cell, where every spreadsheet puts it, and a
//! sentence in plain words for the status line and for a mind, saying which cell and why.

use std::fmt;

/// The kinds of failure, each with the code a spreadsheet user already knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The formula cannot be read at all: a stray `)`, an operator with nothing after it.
    Syntax,
    /// A function this sheet does not have, or a word that is not a cell.
    Name,
    /// Text where a number was needed, or an argument of the wrong kind.
    Value,
    /// Division by zero.
    DivZero,
    /// A reference to no cell: a range too big, a column past the lookup table.
    Ref,
    /// A lookup that found nothing.
    NotFound,
    /// The cell depends, through any number of others, on itself.
    Cycle,
}

impl Kind {
    pub fn code(self) -> &'static str {
        match self {
            Kind::Syntax => "#ERROR!",
            Kind::Name => "#NAME?",
            Kind::Value => "#VALUE!",
            Kind::DivZero => "#DIV/0!",
            Kind::Ref => "#REF!",
            Kind::NotFound => "#N/A",
            Kind::Cycle => "#CYCLE!",
        }
    }
}

/// A failure: its code, and what happened in words.
#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    pub kind: Kind,
    pub message: String,
}

impl Error {
    pub fn new(kind: Kind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }

    pub fn code(&self) -> &'static str {
        self.kind.code()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.code(), self.message)
    }
}

/// A worked-out value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Empty,
    Number(f64),
    Text(String),
    Bool(bool),
    Error(Error),
}

impl Value {
    /// What a cell's own text means when it is not a formula.
    pub fn from_entry(raw: &str) -> Value {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Value::Empty;
        }
        if let Some(n) = parse_number(trimmed) {
            return Value::Number(n);
        }
        match trimmed.to_ascii_uppercase().as_str() {
            "TRUE" => Value::Bool(true),
            "FALSE" => Value::Bool(false),
            _ => Value::Text(raw.to_string()),
        }
    }

    /// What the cell shows.
    pub fn display(&self) -> String {
        match self {
            Value::Empty => String::new(),
            Value::Number(n) => format_number(*n),
            Value::Text(t) => t.clone(),
            Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
            Value::Error(e) => e.code().to_string(),
        }
    }

    /// The value as JSON, for a mind reading a range.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Value::Empty => serde_json::Value::Null,
            Value::Number(n) => serde_json::json!(n),
            Value::Text(t) => serde_json::json!(t),
            Value::Bool(b) => serde_json::json!(b),
            Value::Error(e) => serde_json::json!({ "error": e.code(), "message": e.message }),
        }
    }

    pub fn error(&self) -> Option<&Error> {
        match self {
            Value::Error(e) => Some(e),
            _ => None,
        }
    }
}

/// A number as a person would type it: `12`, `-3.5`, `1e3`, `.5`. Not `inf`, not `NaN`.
pub fn parse_number(s: &str) -> Option<f64> {
    let s = s.trim();
    let first = s.chars().next()?;
    if !(first.is_ascii_digit() || matches!(first, '-' | '+' | '.')) {
        return None;
    }
    s.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// A number the way a cell shows it: whole numbers without a point, the rest to ten significant
/// places with trailing zeros dropped, so `0.1+0.2` is `0.3` and not `0.30000000000000004`.
pub fn format_number(n: f64) -> String {
    if n == 0.0 {
        return "0".into();
    }
    if n.fract() == 0.0 && n.abs() < 1e15 {
        return format!("{}", n as i64);
    }
    let magnitude = n.abs().log10().floor() as i32;
    if !(-6..15).contains(&magnitude) {
        return format!("{n:e}");
    }
    let decimals = (9 - magnitude).clamp(0, 15) as usize;
    let s = format!("{n:.decimals$}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_show_the_way_a_person_wrote_them() {
        assert_eq!(format_number(7.0), "7");
        assert_eq!(format_number(-3.5), "-3.5");
        assert_eq!(format_number(0.1 + 0.2), "0.3");
        assert_eq!(format_number(1.0 / 3.0), "0.3333333333");
        assert_eq!(format_number(1e20), "1e20");
    }

    #[test]
    fn an_entry_is_a_number_text_a_truth_or_nothing() {
        assert_eq!(Value::from_entry(""), Value::Empty);
        assert_eq!(Value::from_entry("  "), Value::Empty);
        assert_eq!(Value::from_entry("42"), Value::Number(42.0));
        assert_eq!(Value::from_entry(" 1.5 "), Value::Number(1.5));
        assert_eq!(Value::from_entry("true"), Value::Bool(true));
        assert_eq!(Value::from_entry("inf"), Value::Text("inf".into()));
        assert_eq!(Value::from_entry("12 apples"), Value::Text("12 apples".into()));
    }
}
