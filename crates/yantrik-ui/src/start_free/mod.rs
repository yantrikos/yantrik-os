//! "Start free, no account" (#1167): the keyless free models the OS already has, offered as one
//! card the person presses, on first-run's "Who should do the thinking?" page and wherever the
//! desktop says the companion has nowhere to send words.
//!
//! - `offers`: which accounts the card is built from, and what it says on its face (pure).
//! - `press`: what the press does, in order, and undoes when a step fails (over [`press::Steps`]).
//! - `real`: the steps, each the function an existing path already uses.
//! - `ui`: the card drawn (components/start_free_card.slint) and what a press starts.
//!
//! Nothing here is on until the person presses: no consent is written, no mind is moved, no
//! request goes to any provider. The card names no provider, host or model of its own; it is made
//! from the accounts the catalogue lists.

pub mod offers;
pub mod press;
mod real;
pub mod ui;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_files;

use std::sync::Mutex;

pub use offers::{offers, sentence, Offer};

/// What the press did, for `describe shell`: nothing is written to disk for this, the choice
/// itself is on disk where the three existing paths keep it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pressed {
    pub account: String,
    /// "working", "answered", "unanswered" or "failed".
    pub outcome: &'static str,
}

static PRESSED: Mutex<Option<Pressed>> = Mutex::new(None);

pub(crate) fn note_pressed(account: &str, outcome: &'static str) {
    *PRESSED.lock().unwrap_or_else(|e| e.into_inner()) = Some(Pressed { account: account.to_string(), outcome });
}

fn pressed() -> Option<Pressed> {
    PRESSED.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// What `yos describe shell` shows under `start_free`: the card as offered (provider, sentence,
/// button) and whether it was pressed. No key, no address.
pub fn describe(offers: &[Offer], pressed: Option<&Pressed>) -> serde_json::Value {
    serde_json::json!({
        "offered": offers.iter().map(|o| serde_json::json!({
            "account": o.account,
            "provider": o.provider,
            "label": o.label,
            "sentence": sentence(&o.provider),
            "button": o.button,
        })).collect::<Vec<_>>(),
        "pressed": pressed.is_some(),
        "outcome": pressed.map(|p| p.outcome),
        "pressed_account": pressed.map(|p| p.account.as_str()),
    })
}

/// [`describe`] from what is known now.
pub fn for_describe() -> serde_json::Value {
    describe(&offers(&crate::ai_accounts::accounts(), &crate::ai_accounts::catalogue()), pressed().as_ref())
}
