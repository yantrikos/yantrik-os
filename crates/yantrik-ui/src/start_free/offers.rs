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
    /// Said on the card's face, never under "show more": the lead (bold) and the rest.
    pub lead: String,
    pub detail: String,
    pub button: String,
}

/// The card's words, in the one place every surface takes them from: the card, `describe`, the
/// control surface's refusal, and (as `<provider>`) the README, docs/getting-started.md and
/// docs/app-control.md, which a test holds to this text. `{p}` is the provider's name.
///
/// What a minds' call carries is what `yantrik-companion`'s context builds each turn: the message
/// and history, memories (including what the shell notes by itself), the system state (network
/// name, running apps, battery, memory, disk, idle time), the person's name and time, and every tool
/// result (files, pages, screen, clipboard, terminal). The press gives that consent to every mind
/// on Yantrik models, since the pick is one for all of them.
pub const LEAD: &str = "What you type, and what your minds read on this computer, is sent to {p}.";
pub const DETAIL: &str = "That includes your messages, what Yantrik remembers about you, files and pages a mind opens for you, \
    the screen and clipboard when a mind looks at them, and the name of your network, your open apps and how the computer is doing \
    (battery, memory, disk). It applies to every mind you give Yantrik models. \
    No account is needed. It is free and rate-limited, so it can be slow or busy. \
    You can switch it off in Settings \u{2192} AI & Intelligence.";

/// The bold first line.
pub fn lead(provider: &str) -> String {
    LEAD.replace("{p}", provider)
}

/// All of it, as one sentence for `describe`, the refusal and a screen reader.
pub fn sentence(provider: &str) -> String {
    format!("{} {DETAIL}", lead(provider))
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
                lead: lead(&provider),
                detail: DETAIL.to_string(),
                button: format!("Start free with {provider}"),
                provider,
            })
        })
        .collect()
}
