//! The built-in companion: it runs in this process on the backend the shell builds it with, so
//! "Use Yantrik models" is a small file of its own, ~/.config/yantrik/companion-models.json
//! (`base_url`, `model` — `picked`, so a pick in the ask bar is its next call's model — and
//! `api_key`, the companion's gateway token), applied to it at once and at every start. Revert
//! removes the file and puts it back on the primary provider from Settings.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use serde_json::{json, Value};

use super::{Handoff, Offer, Plan, Write};
use crate::bridge::CompanionBridge;
use crate::wire::settings::ProviderStoreEntry;

pub(crate) const ID: &str = crate::wire::harness::BUILTIN_ID;

pub(crate) struct Companion;

static BRIDGE: OnceLock<Arc<CompanionBridge>> = OnceLock::new();

/// The bridge reload goes through. Set once, when the gateway starts.
pub(crate) fn set_bridge(bridge: Arc<CompanionBridge>) {
    let _ = BRIDGE.set(bridge);
}

pub(crate) fn file(home: &Path) -> PathBuf {
    home.join(".config/yantrik/companion-models.json")
}

/// What the file says: (base_url, model, token). `None` when the companion keeps its own.
pub(crate) fn read(home: &Path) -> Option<(String, String, String)> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(file(home)).ok()?).ok()?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    Some((s("base_url")?, s("model")?, s("api_key")?))
}

/// Put the companion on what its file says, or back on the primary provider when there is none.
pub(crate) fn reload(home: &Path) {
    let Some(bridge) = BRIDGE.get() else { return };
    match read(home) {
        Some((base, model, token)) => {
            tracing::info!(model = %model, "the companion uses Yantrik models");
            bridge.reload_llm("api".into(), base, Some(token), model);
        }
        None => {
            if let Some(primary) = crate::wire::settings::ProviderStore::load().primary() {
                crate::wire::provider_panel::reload_primary(bridge, primary);
            }
        }
    }
}

impl Handoff for Companion {
    fn harness(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "Yantrik Companion"
    }

    fn files(&self, home: &Path) -> Vec<PathBuf> {
        vec![file(home)]
    }

    fn unit(&self) -> Option<&'static str> {
        None
    }

    fn plan(&self, _home: &Path, _provider: &ProviderStoreEntry) -> Result<Plan, String> {
        Err("The built-in companion runs on the primary provider under AI & Intelligence".into())
    }

    fn plan_gateway(&self, home: &Path, offer: &Offer) -> Result<Plan, String> {
        let text = serde_json::to_string_pretty(&json!({ "base_url": offer.base_url(), "model": yantrik_gateway::PICKED, "api_key": offer.token.0 }))
            .map_err(|e| e.to_string())?;
        Ok(super::gateway_plan(
            self,
            offer,
            vec![Write { path: file(home), content: format!("{text}\n"), what: "the model it runs on and its gateway token".into() }],
        ))
    }
}
