//! An account whose allowance is tight: a free tier that allows a few requests a minute or a day
//! (`yantrik_ml::provider::pool::tiers::FreeTier::is_tight`, the one place that decides it). The
//! gateway keeps what the companion starts by itself from it, and the account says so.

use yantrik_ml::provider::pool::tiers::{self, allows_words};

use super::account::{Account, Source};

impl Account {
    /// The request limit that makes this account tight: only a free tier has one.
    fn tight_limit(&self) -> Option<&'static tiers::Limit> {
        if self.source != Source::Free {
            return None;
        }
        tiers::tier(&self.provider_type)?.tight_limit()
    }

    /// Whether the companion's own calls are kept from this account.
    pub fn tight(&self) -> bool {
        self.tight_limit().is_some()
    }
}

/// The sentence a tight account's detail gains, in the tier's own number; `None` when it is not
/// tight.
pub fn words(account: &Account) -> Option<String> {
    let allows = allows_words(account.tight_limit()?)?;
    Some(format!(
        "It allows {allows}, so the companion uses it only for what you ask and does not learn from conversations or think by itself on it."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_accounts::account::{accounts, Account};
    use std::collections::BTreeSet;
    use yantrik_ml::provider::pool::tiers::FREE_TIERS;

    fn free(id: &str) -> Account {
        // The tiers asked for here are keyless: nothing needs to be kept for them to be listed.
        accounts(&[], FREE_TIERS, &BTreeSet::new(), &BTreeSet::new())
            .into_iter()
            .find(|a| a.provider_type == id)
            .unwrap_or_else(|| panic!("{id} is not listed"))
    }

    #[test]
    fn the_keyless_tier_is_tight_and_says_so_with_its_own_number() {
        let ovh = free("ovh");
        assert!(ovh.tight());
        assert_eq!(
            words(&ovh).as_deref(),
            Some("It allows 2 requests a minute, so the companion uses it only for what you ask and does not learn from conversations or think by itself on it.")
        );
    }

    #[test]
    fn a_tier_that_is_not_tight_says_nothing() {
        let kilo = free("kilo");
        assert!(!kilo.tight());
        assert_eq!(words(&kilo), None);
    }

    #[test]
    fn a_saved_account_is_never_tight() {
        let mut saved = free("ovh");
        saved.source = Source::Saved;
        assert!(!saved.tight(), "only a free tier has a limit the OS knows");
        assert_eq!(words(&saved), None);
    }
}
