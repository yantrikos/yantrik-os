//! The built-in companion: it runs in this process on the backend the shell builds it with, so
//! "Use Yantrik models" is a small file of its own, ~/.config/yantrik/companion-models.json
//! (`base_url`, `model` — `picked`, so a pick in the ask bar is its next call's model — and
//! `api_key`, the companion's gateway token), applied to it at once and at every start. Revert
//! removes the file and puts it back on the primary provider from Settings.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

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

/// Whether the built-in companion is on Yantrik models (its file says so). When it is not, it
/// talks to the saved primary provider itself. One answer for `adopt()` and for the accounts card,
/// so what the screen says cannot drift from what the code does.
pub(crate) fn companion_on_gateway(home: &Path) -> bool {
    read(home).is_some()
}

/// The id of the saved provider the companion was last put on by a reload, `None` when it was
/// put on Yantrik models, on no provider, or never moved. What [`primary_gone`] compares with the
/// saved providers.
static ON_SAVED: Mutex<Option<String>> = Mutex::new(None);

/// A reload put the companion on saved provider `id` (or on none of them).
pub(crate) fn note_on_saved(id: Option<String>) {
    *ON_SAVED.lock().unwrap_or_else(|e| e.into_inner()) = id;
}

/// Whether the companion is running on a saved provider that is no longer saved. Only then: a
/// companion on Yantrik models reaches providers through the gateway, which checks every call.
pub(crate) fn primary_gone(home: &Path, saved: &crate::wire::settings::ProviderStore) -> bool {
    if companion_on_gateway(home) {
        return false;
    }
    let on = ON_SAVED.lock().unwrap_or_else(|e| e.into_inner()).clone();
    on.is_some_and(|id| !saved.entries.iter().any(|e| e.id == id))
}

/// What the companion runs on when it has no provider: every turn is refused here, and nothing
/// leaves the machine. It holds no address and no key.
pub(crate) struct NoProvider;

const NO_PROVIDER: &str = "No AI provider is set up: the provider this companion used was removed. Add or choose one in Settings → AI & Intelligence.";

impl yantrik_ml::LLMBackend for NoProvider {
    fn chat(&self, _: &[yantrik_ml::ChatMessage], _: &yantrik_ml::GenerationConfig, _: Option<&[Value]>) -> anyhow::Result<yantrik_ml::LLMResponse> {
        anyhow::bail!(NO_PROVIDER)
    }

    fn chat_streaming(&self, _: &[yantrik_ml::ChatMessage], _: &yantrik_ml::GenerationConfig, _: Option<&[Value]>, _: &mut dyn FnMut(&str)) -> anyhow::Result<yantrik_ml::LLMResponse> {
        anyhow::bail!(NO_PROVIDER)
    }

    fn count_tokens(&self, text: &str) -> anyhow::Result<usize> {
        Ok(text.len() / 4)
    }

    fn backend_name(&self) -> &str {
        "none"
    }
}

/// The worker's side of `StopLLM`: no address, no key, and a backend that fails every call.
pub(crate) fn stop(companion: &mut yantrik_companion::CompanionService, online: &std::sync::atomic::AtomicBool) {
    tracing::info!("Stopping the LLM backend: no provider is set");
    companion.config.llm.api_base_url = None;
    companion.config.llm.api_key = None;
    companion.swap_llm(Arc::new(NoProvider));
    online.store(false, std::sync::atomic::Ordering::Relaxed);
}

/// Put the companion on the provider from Settings' own fields (nothing saved is primary): it no
/// longer runs on a saved provider, so none is noted.
pub(crate) fn reload_config(bridge: &CompanionBridge, base_url: String, key: Option<String>, model: String) {
    note_on_saved(None);
    bridge.reload_llm("api".into(), base_url, key, model);
}

/// The saved provider the companion ran on is gone. Stopped first, now and in this thread, so the
/// removed provider's key is out of the companion whatever follows; only then moved onto the
/// primary, when there is one. A move that cannot get its key leaves the companion stopped, not on
/// the removed provider. The channel keeps the order: a later `ReloadLLM` replaces the stop.
pub(crate) fn stop_then_move(bridge: &Arc<CompanionBridge>, primary: Option<&ProviderStoreEntry>) {
    note_on_saved(None);
    bridge.stop_llm();
    match primary {
        Some(primary) => crate::wire::provider_panel::reload_primary(bridge, primary),
        None => tracing::info!("the companion has no provider: it is stopped, not left on a removed one"),
    }
}

/// Put the companion on what its file says, or back on the primary provider when there is none,
/// or on no provider at all when there is no primary either.
pub(crate) fn reload(home: &Path) {
    let Some(bridge) = BRIDGE.get() else { return };
    reload_with(bridge, home);
}

pub(crate) fn reload_with(bridge: &Arc<CompanionBridge>, home: &Path) {
    match read(home) {
        Some((base, model, token)) => {
            tracing::info!(model = %model, "the companion uses Yantrik models");
            note_on_saved(None);
            bridge.reload_llm("api".into(), base, Some(token), model);
        }
        None => match crate::wire::settings::ProviderStore::load().primary() {
            Some(primary) => crate::wire::provider_panel::reload_primary(bridge, primary),
            None => stop_then_move(bridge, None),
        },
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

    /// Its file's `model`, while its `base_url` is the gateway's.
    fn asks_gateway_for(&self, home: &Path) -> Option<String> {
        let (base_url, model, _) = read(home)?;
        (super::is_gateway(&base_url) && !model.trim().is_empty()).then(|| model.trim().to_string())
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
