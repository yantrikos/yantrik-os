//! Settings → AI's "Active AI" card: what the built-in companion talks to, and whether it answers.
//!
//! The card used to read three things that did not describe one fact. The name came from the
//! saved providers list, which a machine set up through config.yaml does not have, so it said "No
//! provider". The model came from config.yaml. And "Connected" was the bridge's `online` flag,
//! which starts true and only goes false after a chat has failed. So every machine read "No
//! provider · Connected · <whatever config.yaml names>" — a fresh install included, where
//! config.yaml named `yantrik-4b` at 127.0.0.1:8341 and nothing listened there.
//!
//! Now the card names what the companion actually uses — the saved primary provider, or else
//! the endpoint in config.yaml — and says what asking it found: the model is there, the model is
//! not among the ones it lists, the key was refused, nothing answered, or nothing is set up. It
//! asks with the one request the provider form's Connect uses (`provider_models::list_models`),
//! off the UI thread, and shows "Checking" until it has an answer.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use slint::{ComponentHandle, SharedString};

use super::provider_models::{list_models, ListError, ListedModel};
use super::settings::ProviderStore;
use crate::App;
use yantrik_ui_slint::AIStatusData;

/// The key config.yaml gives the companion, if any, so asking its endpoint authenticates the way
/// the companion does. Held in memory only, as the companion already holds it.
static CONFIG_KEY: OnceLock<Mutex<Option<String>>> = OnceLock::new();

/// Each refresh's number; an answer that arrives after a newer question was asked is dropped.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// What the companion was last resolved to, for the "what runs on what" map: the same answer the
/// card shows, never worked out a second way. No key in it.
static COMPANION: Mutex<Option<crate::runs_on::CompanionFact>> = Mutex::new(None);

/// The companion's address, model and where they are set, as last resolved.
pub(crate) fn companion() -> Option<crate::runs_on::CompanionFact> {
    COMPANION.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// What the companion was built from, as `bridge::build_companion` built it: the backend by the
/// bridge's own predicates (`runs_on::Backend::of`), the fallback said (`runs_on::fallback_label`),
/// and config.yaml's backend name. What the destination line may claim rests on these (security
/// review of #648, H1). Set once at start, from the config the companion is built from.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ConfigLlm {
    pub backend: crate::runs_on::Backend,
    pub fallback: Option<String>,
    pub kind: String,
}

static CONFIG_LLM: Mutex<Option<ConfigLlm>> = Mutex::new(None);

pub(crate) fn set_config_llm(config: ConfigLlm) {
    *CONFIG_LLM.lock().unwrap_or_else(|e| e.into_inner()) = Some(config);
}

/// The companion as the destination line needs it: the target, how it is reached, its fallback
/// and what it was set up as. A saved primary is applied as an API backend (`ReloadLLM`),
/// whatever config.yaml names, and its type is what it was set up as; a saved fallback provider
/// is named over config.yaml's. In-process and CLI backends are described even with no address.
pub(crate) fn companion_fact(
    target: Option<&Target>,
    config: &ConfigLlm,
    saved_primary_kind: Option<String>,
    saved_fallback: Option<String>,
) -> Option<crate::runs_on::CompanionFact> {
    use crate::runs_on::{Backend, CompanionFact};
    let backend = if saved_primary_kind.is_some() { Backend::Api } else { config.backend };
    let provider_kind = saved_primary_kind.unwrap_or_else(|| config.kind.clone());
    let fallback = saved_fallback.or_else(|| config.fallback.clone());
    match target {
        Some(t) => Some(CompanionFact {
            base_url: t.base_url.clone(),
            model: t.model.clone(),
            source: t.source.clone(),
            provider_name: t.name.clone(),
            backend,
            fallback,
            provider_kind,
        }),
        None if backend != Backend::Api => {
            Some(CompanionFact { source: "config.yaml".into(), backend, fallback, provider_kind, ..CompanionFact::default() })
        }
        None => None,
    }
}

pub(crate) fn set_config_key(key: Option<String>) {
    let slot = CONFIG_KEY.get_or_init(|| Mutex::new(None));
    *slot.lock().unwrap_or_else(|e| e.into_inner()) = key.filter(|k| !k.is_empty());
}

pub(crate) fn config_key() -> Option<String> {
    CONFIG_KEY.get().and_then(|m| m.lock().ok().and_then(|k| k.clone()))
}

/// What the companion is pointed at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    pub name: String,
    /// Where the card says this came from: "saved provider" or "config.yaml".
    pub source: String,
    pub base_url: String,
    pub model: String,
    pub key: Option<String>,
    pub auth_type: String,
}

/// The saved primary provider when there is one — Settings points the companion at it — or
/// else the endpoint config.yaml gives it. `None` when neither names an address.
pub(crate) fn target(store: &ProviderStore, config_url: &str, config_model: &str, key: Option<String>) -> Option<Target> {
    if let Some(p) = store.primary() {
        return Some(Target {
            name: p.name.clone(),
            source: "saved provider".into(),
            base_url: p.base_url.clone(),
            model: if p.model.is_empty() { config_model.to_string() } else { p.model.clone() },
            key: p.api_key.clone(),
            auth_type: p.auth_type.clone(),
        });
    }
    let url = config_url.trim();
    if url.is_empty() {
        return None;
    }
    Some(Target {
        name: name_for(url),
        source: "config.yaml".into(),
        base_url: url.to_string(),
        model: config_model.trim().to_string(),
        key,
        auth_type: "bearer".into(),
    })
}

/// A provider's own name when the address is one the catalogue knows, else the address itself.
/// What an address is called: the catalogue's name for it, or its host. The same answer every
/// other surface gives (runs_on::identity).
fn name_for(url: &str) -> String {
    use crate::runs_on::identity::{from_url, host, ProviderRef};
    match from_url(url) {
        ProviderRef::Known(p) => p.display_name.to_string(),
        ProviderRef::Custom(h) | ProviderRef::Local(h) => h,
        ProviderRef::NotReported => host(url),
    }
}

/// What asking the target found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Health {
    Checking,
    Connected,
    /// It answered, and the model the companion asks for is not among the ones it lists.
    ModelMissing { listed: usize },
    KeyRefused,
    Unreachable(String),
    /// It answered, but not with a model list: its models could not be checked.
    Unverified(String),
    NotSetUp,
}

/// Whether `model` is one of `listed`, as the provider names them. Ollama lists `name:latest`
/// for a model asked for as `name`.
pub(crate) fn serves(listed: &[ListedModel], model: &str) -> bool {
    let want = model.trim();
    !want.is_empty()
        && listed.iter().any(|m| {
            m.id == want || m.id.strip_suffix(":latest") == Some(want) || want.strip_suffix(":latest") == Some(m.id.as_str())
        })
}

pub(crate) fn judge(result: Result<Vec<ListedModel>, ListError>, model: &str) -> Health {
    match result {
        Ok(listed) if serves(&listed, model) => Health::Connected,
        Ok(listed) => Health::ModelMissing { listed: listed.len() },
        Err(ListError::KeyRefused) => Health::KeyRefused,
        Err(e @ (ListError::Unreachable { .. } | ListError::TimedOut { .. })) => Health::Unreachable(e.to_string()),
        Err(ListError::BadUrl) => Health::NotSetUp,
        Err(e) => Health::Unverified(e.to_string()),
    }
}

/// The card, from a target and what asking it found. Never carries a key.
pub(crate) fn card(target: Option<&Target>, health: &Health, fallback: Option<&str>) -> AIStatusData {
    let (status, detail) = match (target, health) {
        (None, _) | (_, Health::NotSetUp) => (
            "unset",
            "No model is set up for the built-in companion. Add a provider below.".to_string(),
        ),
        (Some(_), Health::Checking) => ("checking", String::new()),
        (Some(_), Health::Connected) => ("connected", String::new()),
        (Some(t), Health::ModelMissing { listed }) => (
            "missing",
            if t.model.is_empty() {
                format!("No model is chosen. {} lists {listed} — pick one under Providers.", t.name)
            } else {
                format!("“{}” is not one of the {listed} models {} lists.", t.model, t.name)
            },
        ),
        (Some(_), Health::KeyRefused) => ("refused", "The key was refused.".to_string()),
        (Some(_), Health::Unreachable(why)) => ("unreachable", why.clone()),
        (Some(_), Health::Unverified(why)) => ("error", why.clone()),
    };
    AIStatusData {
        provider_name: target.map(|t| t.name.clone()).unwrap_or_default().into(),
        provider_type: target.map(|t| format!("from {}", t.source)).unwrap_or_default().into(),
        model_name: target.map(|t| t.model.clone()).unwrap_or_default().into(),
        model_tier: SharedString::default(),
        status: status.into(),
        detail: detail.into(),
        latency_ms: -1,
        tokens_per_sec: 0.0,
        tokens_today: 0,
        using_fallback: false,
        fallback_provider: fallback.unwrap_or_default().into(),
        fallback_model: SharedString::default(),
    }
}

/// Show what the companion is pointed at as "Checking", then ask it, and show the answer.
pub(crate) fn refresh(ui: &App, store: &ProviderStore) {
    let target = target(
        store,
        &ui.get_settings_llm_api_url(),
        &ui.get_settings_llm_api_model(),
        config_key(),
    );
    let fallback = store.fallback().map(|f| f.name.clone());
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    // Before the config is known, nothing is claimed for it: an API, which is never "stays".
    let config = CONFIG_LLM
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| ConfigLlm { kind: ui.get_settings_llm_backend().to_string(), ..ConfigLlm::default() });
    *COMPANION.lock().unwrap_or_else(|e| e.into_inner()) = companion_fact(
        target.as_ref(),
        &config,
        store.primary().map(|p| p.provider_type.clone()),
        fallback.clone(),
    );
    super::runs_on_card::publish(ui);
    let Some(target) = target else {
        ui.set_settings_ai_status(card(None, &Health::NotSetUp, fallback.as_deref()));
        return;
    };
    ui.set_settings_ai_status(card(Some(&target), &Health::Checking, fallback.as_deref()));
    let weak = ui.as_weak();
    let spawned = std::thread::Builder::new().name("ai-status".into()).spawn(move || {
        let health = judge(list_models(&target.base_url, target.key.as_deref(), &target.auth_type), &target.model);
        let _ = slint::invoke_from_event_loop(move || {
            if GENERATION.load(Ordering::SeqCst) != generation {
                return; // a newer question was asked; its answer is the one to show
            }
            if let Some(ui) = weak.upgrade() {
                ui.set_settings_ai_status(card(Some(&target), &health, fallback.as_deref()));
            }
        });
    });
    if spawned.is_err() {
        tracing::warn!("no thread to check the companion's model");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::settings::ProviderStoreEntry;

    fn listed(ids: &[&str]) -> Vec<ListedModel> {
        ids.iter().map(|i| ListedModel { id: i.to_string(), name: i.to_string(), size_bytes: 0 }).collect()
    }

    /// What the destination line rests on: a saved primary is an API backend whatever config.yaml
    /// says, config.yaml's in-process and CLI backends are described with no address, and a
    /// fallback is carried — the saved one over config.yaml's.
    #[test]
    fn the_companion_fact_carries_its_backend_and_fallback() {
        use crate::runs_on::Backend;
        let cfg = |backend: Backend, fallback: Option<&str>, kind: &str| ConfigLlm {
            backend,
            fallback: fallback.map(String::from),
            kind: kind.into(),
        };
        let t = target(&ProviderStore::default(), "http://localhost:11434/v1", "qwen3.5:9b", None).unwrap();
        let f = companion_fact(Some(&t), &cfg(Backend::Api, None, "api"), None, None).unwrap();
        assert_eq!((f.backend, f.fallback.as_deref(), f.provider_kind.as_str()), (Backend::Api, None, "api"));
        let f = companion_fact(Some(&t), &cfg(Backend::ClaudeCli, None, "claude-cli"), None, None).unwrap();
        assert_eq!(f.backend, Backend::ClaudeCli);
        let saved = companion_fact(Some(&t), &cfg(Backend::ClaudeCli, None, "claude-cli"), Some("litellm".into()), None).unwrap();
        assert_eq!((saved.backend, saved.provider_kind.as_str()), (Backend::Api, "litellm"), "a saved primary is applied as the API, as what it was saved as");
        let f = companion_fact(None, &cfg(Backend::InProcess, Some("Ollama at localhost:8341"), "llamacpp"), None, None).unwrap();
        assert_eq!((f.backend, f.fallback.as_deref()), (Backend::InProcess, Some("Ollama at localhost:8341")));
        assert!(companion_fact(None, &cfg(Backend::Api, None, "api"), None, None).is_none());
        let f = companion_fact(Some(&t), &cfg(Backend::Api, Some("x"), "api"), None, Some("Saved".into())).unwrap();
        assert_eq!(f.fallback.as_deref(), Some("Saved"));
    }

    #[test]
    fn a_machine_with_only_config_yaml_names_its_endpoint_not_no_provider() {
        let t = target(&ProviderStore::default(), "http://192.168.4.35:11434/v1", "qwen3.5:9b", None).unwrap();
        assert_eq!((t.name.as_str(), t.source.as_str(), t.model.as_str()), ("192.168.4.35:11434", "config.yaml", "qwen3.5:9b"));
        let local = target(&ProviderStore::default(), "http://localhost:11434/v1", "x", None).unwrap();
        assert_eq!(local.name, "Ollama", "an address the catalogue knows is called by its name");
        assert!(target(&ProviderStore::default(), "  ", "x", None).is_none());
    }

    #[test]
    fn a_saved_primary_is_what_the_card_names() {
        let mut store = ProviderStore::default();
        store.entries.push(ProviderStoreEntry {
            id: "p".into(),
            name: "NVIDIA NIM".into(),
            provider_type: "nvidia-nim".into(),
            base_url: "https://integrate.api.nvidia.com/v1".into(),
            api_key: Some("nvapi-x".into()),
            auth_type: "bearer".into(),
            is_primary: true,
            is_fallback: false,
            model: "nvidia/nemotron-3-super-120b-a12b".into(),
        });
        let t = target(&store, "http://127.0.0.1:8341/v1", "yantrik-4b", None).unwrap();
        assert_eq!((t.name.as_str(), t.model.as_str()), ("NVIDIA NIM", "nvidia/nemotron-3-super-120b-a12b"));
    }

    #[test]
    fn connected_means_the_model_asked_for_is_one_it_serves() {
        assert_eq!(judge(Ok(listed(&["qwen3.5:9b", "bge-m3:latest"])), "qwen3.5:9b"), Health::Connected);
        assert_eq!(judge(Ok(listed(&["llama3:latest"])), "llama3"), Health::Connected, "Ollama's :latest");
        assert_eq!(judge(Ok(listed(&["a", "b"])), "yantrik-4b"), Health::ModelMissing { listed: 2 });
        assert_eq!(judge(Err(ListError::KeyRefused), "m"), Health::KeyRefused);
        assert!(matches!(judge(Err(ListError::Unreachable { host: "127.0.0.1:8341".into() }), "m"), Health::Unreachable(_)));
    }

    #[test]
    fn the_fresh_install_card_no_longer_says_connected() {
        // config-default.yaml's endpoint, with nothing listening there.
        let t = target(&ProviderStore::default(), "http://127.0.0.1:8341/v1", "yantrik-4b", None).unwrap();
        let health = judge(Err(ListError::Unreachable { host: "127.0.0.1:8341".into() }), &t.model);
        let c = card(Some(&t), &health, None);
        assert_eq!(c.status, "unreachable");
        assert_eq!(c.detail, "Could not reach 127.0.0.1:8341.");
        assert_eq!(c.provider_name, "127.0.0.1:8341");
    }

    #[test]
    fn nothing_on_the_card_carries_a_key() {
        let t = Target {
            name: "X".into(),
            source: "saved provider".into(),
            base_url: "https://x/v1".into(),
            model: "m".into(),
            key: Some("sk-secret-123".into()),
            auth_type: "bearer".into(),
        };
        for h in [Health::Checking, Health::Connected, Health::KeyRefused, Health::ModelMissing { listed: 3 }] {
            let c = card(Some(&t), &h, None);
            let all = format!("{}{}{}{}{}", c.provider_name, c.provider_type, c.model_name, c.status, c.detail);
            assert!(!all.contains("sk-secret"), "{all}");
        }
    }
}
