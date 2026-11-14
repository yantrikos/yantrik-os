//! What a person's own words say about the setting for getting promised things ready (#1184).
//!
//! Kept apart from `commitment_consent` so the tools can ask "is this about the setting?" before
//! writing a memory without reaching the person-only answer (`tests/commitment_prep_consent.rs`
//! reads `tools/` and fails if it names that module).

/// What a chat turn answers when the person asks it to change the setting: nothing is changed.
pub const SWITCH_REPLY: &str = "Only you can change that: it's the switch in Settings \u{2192} Companion. I haven't changed anything.";

/// Whether `text` is about the setting: it names the feature. Hyphens read as spaces, so
/// "the getting-things-ready card" counts.
pub fn about_the_setting(text: &str) -> bool {
    let lower = text.to_lowercase().replace('-', " ");
    let names = ["getting things ready", "get things ready", "getting promised things ready", "commitment prep"];
    let preparing = lower.contains("prepar") && ["promise", "things"].iter().any(|w| lower.contains(w));
    names.iter().any(|n| lower.contains(n)) || preparing
}

/// Whether the person is asking, in a chat turn, to switch the setting: it names the feature and
/// a switch verb. The turn then answers [`SWITCH_REPLY`] and runs no model, because the setting is
/// the person's own click (`person_answer`).
pub fn asks_to_switch(text: &str) -> bool {
    let lower = text.to_lowercase().replace('-', " ");
    let verbs = ["turn off", "turn it off", "switch off", "stop", "disable", "turn on", "enable", "press got it", "press stop"];
    about_the_setting(text) && verbs.iter().any(|v| lower.contains(v))
}
