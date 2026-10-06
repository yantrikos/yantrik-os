//! Each account's models, with what each can do: from the provider's own list where it has one,
//! from a curated list where it does not (yantrik_ml::model_caps::curated, and the free tiers'
//! lists in the pool), and the saved model alone when neither answers. Pure: the listing is done
//! by the caller (`provider_models::list_models`) and handed in.

use serde::{Deserialize, Serialize};
use yantrik_ml::model_caps::{caps_for, curated, ModelCaps};
use yantrik_ml::provider::pool::tiers::FreeTier;

use super::account::{Account, Source};
use crate::wire::provider_models::{ListError, ListedModel};

/// One model of one account, as the picker and the gateway's `/v1/models` show it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogueModel {
    /// `<account>/<model>`: what a harness puts in a request's `model` for the gateway.
    pub id: String,
    pub account: String,
    /// What the provider calls it, sent upstream as `model`.
    pub model: String,
    pub name: String,
    pub caps: ModelCaps,
}

/// Where an account's list came from, and what went wrong when it did not come.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "from", content = "why")]
pub enum Listing {
    /// The provider's own list.
    Provider,
    /// A hand-kept list: the provider has none, or it is a free tier's.
    Curated,
    /// The list could not be had; the saved model is shown alone. A sentence for the person.
    Failed(String),
    /// Not asked yet.
    Pending,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountModels {
    pub account: String,
    pub name: String,
    pub listing: Listing,
    pub models: Vec<CatalogueModel>,
}

/// The gateway's id for a model of an account.
pub fn gateway_id(account: &str, model: &str) -> String {
    format!("{account}/{model}")
}

fn model(account: &Account, id: &str, name: &str, context: Option<u32>, tools: Option<bool>) -> CatalogueModel {
    CatalogueModel {
        id: gateway_id(&account.id, id),
        account: account.id.clone(),
        model: id.to_string(),
        name: name.to_string(),
        caps: caps_for(&account.provider_type, id, context, tools),
    }
}

/// Whether this account's models come from a hand-kept list rather than the network.
pub fn needs_no_listing(account: &Account, free: &[FreeTier]) -> bool {
    account.source == Source::Free && free.iter().any(|t| t.id == account.provider_type)
        || curated(&account.provider_type).is_some()
}

/// The account's models from what was found: `listed` is the provider's answer, `None` when it
/// was not asked (a free tier, or a provider with no list).
pub fn build(account: &Account, free: &[FreeTier], listed: Option<Result<Vec<ListedModel>, ListError>>) -> AccountModels {
    let mut out = AccountModels { account: account.id.clone(), name: account.name.clone(), listing: Listing::Pending, models: Vec::new() };
    if account.source == Source::Free {
        if let Some(t) = free.iter().find(|t| t.id == account.provider_type) {
            out.listing = Listing::Curated;
            out.models = t.models.iter().map(|m| model(account, m.id, m.id, Some(m.context), Some(m.tools))).collect();
            return out;
        }
    }
    if let Some(list) = curated(&account.provider_type) {
        out.listing = Listing::Curated;
        out.models = list.iter().map(|m| model(account, m.id, m.id, Some(m.context), None)).collect();
        return out;
    }
    match listed {
        Some(Ok(list)) => {
            out.listing = Listing::Provider;
            out.models = list.iter().map(|m| model(account, &m.id, &m.name, None, None)).collect();
        }
        Some(Err(e)) => out.listing = Listing::Failed(e.to_string()),
        None => {}
    }
    // The model the person chose in Settings is always there, listed or not.
    let saved = account.saved_model.trim();
    if !saved.is_empty() && !out.models.iter().any(|m| m.model == saved) {
        out.models.insert(0, model(account, saved, saved, None, None));
    }
    out
}
