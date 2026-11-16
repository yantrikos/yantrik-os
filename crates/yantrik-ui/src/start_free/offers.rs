//! Which accounts "Start free, no account" is made from, and its words.

use yantrik_ml::provider::pool::tiers::tier;

use crate::ai_accounts::account::{Account, KeyRef, Source};
use crate::ai_accounts::store::Catalogue;

/// The card's own label, beside Groq's "Recommended".
pub const LABEL: &str = "No account needed";

/// One free, keyless account the person may start with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    /// The account's id in the catalogue.
    pub account: String,
    /// The provider's name, as the free card names it.
    pub provider: String,
    /// The account's one model, `<account>/<model>`.
    pub model: String,
    pub label: &'static str,
    /// Said on the card's face, never under "show more".
    pub sentence: String,
    pub button: String,
}

/// What is sent, to whom, and the cost of free, in plain words.
pub fn sentence(provider: &str) -> String {
    format!(
        "Your messages, and what Yantrik remembers about you, are sent to {provider}. No account is needed. \
         It is free and rate-limited, so it can be slow or busy. You can switch it off in Settings \u{2192} AI & Intelligence."
    )
}

/// The accounts the catalogue lists that are free and need no key, may be shared with (a free tier
/// that may train on what it is sent never is), and list exactly one model: with several, which to
/// start on would be chosen for the person. None when there is no such account, and then there is
/// no card.
pub fn offers(accounts: &[Account], catalogue: &Catalogue) -> Vec<Offer> {
    accounts
        .iter()
        .filter(|a| a.source == Source::Free && a.key == KeyRef::None && !a.local && !a.trains_on_prompts)
        .filter_map(|a| {
            let provider = tier(&a.provider_type)?.name.to_string();
            let listed = catalogue.accounts.iter().find(|g| g.account == a.id)?;
            let [only] = listed.models.as_slice() else { return None };
            Some(Offer {
                account: a.id.clone(),
                model: only.id.clone(),
                label: LABEL,
                sentence: sentence(&provider),
                button: format!("Start free with {provider}"),
                provider,
            })
        })
        .collect()
}
