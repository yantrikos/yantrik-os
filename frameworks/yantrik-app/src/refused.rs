//! A refusal: why a command did not run, said once for the person and the Mind alike
//! (design/app-framework-2026-10-21.md, §7).
//!
//! `update` returns `Err(Refused)` and changes nothing. The runner puts the sentence on the
//! notice line, logs the kind, and writes a problem record for a kind that begins `internal_`,
//! because that is a bug and not a refusal. The surface returns the same sentence as the call's
//! error, word for word.
//!
//! The sentence is English for now. When a second locale lands it becomes a message id with
//! arguments, rendered at this boundary; the `kind` is the stable part and does not change.

use std::fmt;

/// Why a command did not run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    /// A stable snake_case name the Mind can branch on: `no_location`, `busy`, `internal_panic`.
    pub kind: &'static str,
    /// The sentence the person reads, in the writing rules' voice.
    pub sentence: String,
}

impl Refused {
    pub fn new(kind: &'static str, sentence: impl Into<String>) -> Refused {
        Refused {
            kind,
            sentence: sentence.into(),
        }
    }

    /// A bug, not a refusal: the runner files a problem record for it.
    pub fn is_internal(&self) -> bool {
        self.kind.starts_with("internal_")
    }
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.sentence)
    }
}

impl std::error::Error for Refused {}

/// What an app's own error adds to its `Display`: the kind. With it, `?` turns a typed error into
/// a [`Refused`] whose sentence is the error's `Display`.
pub trait Explain: std::error::Error {
    fn kind(&self) -> &'static str;
}

impl<E: Explain> From<E> for Refused {
    fn from(e: E) -> Refused {
        Refused {
            kind: e.kind(),
            sentence: e.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, thiserror::Error)]
    enum WeatherError {
        #[error("Choose a place before refreshing.")]
        NoPlace,
    }
    impl Explain for WeatherError {
        fn kind(&self) -> &'static str {
            match self {
                WeatherError::NoPlace => "no_place",
            }
        }
    }

    #[test]
    fn a_typed_error_becomes_a_refusal_with_its_own_sentence() {
        let r: Refused = WeatherError::NoPlace.into();
        assert_eq!(r.kind, "no_place");
        assert_eq!(r.to_string(), "Choose a place before refreshing.");
        assert!(!r.is_internal());
        assert!(Refused::new("internal_panic", "x").is_internal());
    }
}
