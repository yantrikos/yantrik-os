//! What a person's own words say about the setting for getting promised things ready (#1184).
//!
//! Kept apart from `commitment_consent` so the tools can ask "is this about the setting?" before
//! writing a memory without reaching the person-only answer (`tests/commitment_prep_consent.rs`
//! reads `tools/` and fails if it names that module).
//!
//! Two questions, answered differently because the cost of a wrong answer differs. A chat turn that
//! is taken for a request to switch the setting is answered with a fixed sentence and is never
//! shown to a model, so it must also never be a promise: [`asks_to_switch`] matches only a switch
//! verb that directly governs the feature. A memory that is taken for being about the setting is
//! dropped, so [`memory_about_the_setting`] must not drop ordinary life that shares words with it.

use std::sync::OnceLock;

use regex::Regex;


/// What a chat turn answers when the person asks it to change the setting: nothing is changed.
pub const SWITCH_REPLY: &str = "Only you can change that: it's the switch in Settings \u{2192} Companion. I haven't changed anything.";

/// Lower case, hyphens as spaces ("the getting-things-ready card"), curly apostrophes straight.
fn plain(text: &str) -> String {
    text.to_lowercase().replace('-', " ").replace('\u{2019}', "'")
}

/// The feature, as an object: "getting things ready", "commitment prep", "preparing my promises".
const SUBJECT: &str = r"(?:the\s+)?(?:(?:getting|get|preparing|prepare|making|make)\s+(?:promised\s+)?(?:things|stuff|drafts|it)\s+ready|promised\s+things\s+ready|commitment\s+prep|prep(?:aring)?\s+for\s+(?:my\s+)?(?:promises|commitments)|(?:preparing|prepare|drafting|draft)(?:\s+\w+){0,3}?\s+(?:for\s+)?(?:my\s+)?(?:promises|commitments))";

/// A word that switches something, matched on word boundaries, or the card's own two buttons.
const SWITCH: &str = r"(?:(?:please|just)\s+)?(?:(?:turn(?:ing)?|switch(?:ing)?)\s+(?:off|on)|stop|disable|enable|pause|resume|do\s+not|don't|no\s+longer|never)(?:\s+you)?(?:\s+from)?|press\s+(?:got it|stop)(?:\s+and\s+(?:got it|stop))?\s+on";

fn switch_request() -> Option<&'static Regex> {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    RE.get_or_init(|| Regex::new(&format!(r"\b(?:{SWITCH})\s+{SUBJECT}\b")).ok()).as_ref()
}

/// Whether the person is asking, in a chat turn, to switch the setting: a switch verb directly
/// followed by the feature. The turn then answers [`SWITCH_REPLY`] and runs no model, because the
/// setting is the person's own click (`person_answer`).
///
/// The verb must govern the feature, so "I promised Maya I would prepare the deck, stop me if I
/// forget" is not a request: it goes to the model, and the promise is noted. (The commitment
/// extractor is not asked: it reads "please stop preparing my promises" as a promise itself.)
pub fn asks_to_switch(text: &str) -> bool {
    switch_request().is_some_and(|re| re.is_match(&plain(text)))
}

/// The feature named on its own, "getting things ready", "commitment prep". Not when it is followed
/// by something else it is being done for: "getting things ready for the party" is ordinary life.
fn names_the_feature(lower: &str) -> bool {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    let Some(re) = RE.get_or_init(|| Regex::new(r"\b(?:getting|get)\s+(?:promised\s+)?things\s+ready\b|\bcommitment\s+prep\b").ok()).as_ref() else {
        return false;
    };
    re.find_iter(lower).any(|m| {
        let rest = lower[m.end()..].trim_start();
        let Some(object) = rest.strip_prefix("for ") else { return true };
        let object = ["my ", "their ", "the user's "].iter().find_map(|p| object.strip_prefix(p)).unwrap_or(object);
        object.starts_with("promise") || object.starts_with("commitment")
    })
}

/// Preparing or drafting, and a promise: the words a memory about the setting shares with ordinary life.
fn prepares_for_promises(lower: &str) -> bool {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    let prepares = RE.get_or_init(|| Regex::new(r"\b(?:prepar\w*|draft\w*)").ok()).as_ref().is_some_and(|re| re.is_match(lower));
    prepares && (lower.contains("promise") || lower.contains("commitment"))
}

/// A reference to the companion or its setting: what turns "prepared a promise" into a statement
/// about the feature.
fn refers_to_the_companion(lower: &str) -> bool {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"\b(?:yantrik|companion|assistant|you|feature|setting|settings|card|switch|switched|stop|stopped|stops|turn(?:ed|s)?\s+(?:off|on)|prefers?\s+not|not\s+to\s+have|(?:do|does)\s+not\s+want|don't\s+want|doesn't\s+want)\b|\bfor\s+me\b",
        )
        .ok()
    })
    .as_ref()
    .is_some_and(|re| re.is_match(lower))
}

/// Whether a memory (a fact, a learned item, an opinion) is about the setting, so that nothing is
/// kept about it: it names the feature on its own, or it is about preparing or drafting for
/// promises and refers to the companion or its setting. A memory that only shares words with it
/// ("User promised to prepare the Q4 deck for Maya", "User is getting things ready for the party")
/// is ordinary life and is kept.
pub fn memory_about_the_setting(text: &str) -> bool {
    let lower = plain(text);
    names_the_feature(&lower) || (prepares_for_promises(&lower) && refers_to_the_companion(&lower))
}
