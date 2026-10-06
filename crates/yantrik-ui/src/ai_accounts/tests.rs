use std::collections::BTreeSet;

use yantrik_ml::model_caps::Effort;
use yantrik_ml::provider::pool::tiers::FREE_TIERS;

use super::account::{accounts, slug, KeyRef, Source};
use super::keys::{resolve, Missing};
use super::models::{build, Listing};
use super::store::{self, Catalogue};
use crate::wire::provider_models::{ListError, ListedModel};
use crate::wire::settings::ProviderStoreEntry;

const KEY: &str = "sk-secret-0123456789abcdef";

fn saved(id: &str, name: &str, kind: &str, url: &str, key: Option<&str>, model: &str) -> ProviderStoreEntry {
    ProviderStoreEntry {
        id: id.into(),
        name: name.into(),
        provider_type: kind.into(),
        base_url: url.into(),
        api_key: key.map(str::to_string),
        auth_type: if key.is_some() { "bearer".into() } else { "none".into() },
        is_primary: false,
        is_fallback: false,
        model: model.into(),
    }
}

fn set(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

fn listed(ids: &[&str]) -> Vec<ListedModel> {
    ids.iter().map(|i| ListedModel { id: i.to_string(), name: i.to_string(), size_bytes: 0 }).collect()
}

#[test]
fn every_account_the_person_added_is_listed_once_with_a_readable_id() {
    let entries = vec![
        saved("a1", "Ollama Cloud", "ollama-cloud", "https://ollama.com/v1", Some(KEY), "deepseek-v4.1-flash"),
        saved("a2", "Home Ollama", "ollama", "http://192.168.1.20:11434", None, ""),
        saved("a3", "Ollama Cloud", "custom", "https://other.example/v1", Some(KEY), ""),
        // The gateway itself, saved as a provider: never an account, or it would loop.
        saved("a4", "Yantrik models", "custom", "http://127.0.0.1:7460/v1", Some("ygw-x"), ""),
    ];
    // Groq is kept; OpenRouter is switched off; Gemini has no key; Kilo needs none.
    let list = accounts(&entries, FREE_TIERS, &set(&["groq", "openrouter"]), &set(&["openrouter"]));
    let ids: Vec<&str> = list.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["ollama-cloud", "home-ollama", "ollama-cloud-2", "free-groq", "free-kilo", "free-ovh"]);
    assert_eq!(list[0].key, KeyRef::Saved { entry: "a1".into() });
    assert!(list[1].local && list[1].key == KeyRef::None, "a local runtime needs no key");
    assert_eq!(list[3].key, KeyRef::Vault { id: "groq".into(), fill: None });
    assert_eq!(list[3].source, Source::Free);
    assert!(list[4].trains_on_prompts, "the free card's training label travels with the account");
    assert_eq!(slug("  NVIDIA NIM!! "), "nvidia-nim");
}

#[test]
fn cloudflare_needs_its_account_id_as_well_as_its_token() {
    let none = accounts(&[], FREE_TIERS, &set(&["cloudflare"]), &set(&[]));
    assert!(!none.iter().any(|a| a.id == "free-cloudflare"), "half the values is not an account");
    let both = accounts(&[], FREE_TIERS, &set(&["cloudflare", "cloudflare_account"]), &set(&[]));
    let cf = both.iter().find(|a| a.id == "free-cloudflare").unwrap();
    assert_eq!(cf.key, KeyRef::Vault { id: "cloudflare".into(), fill: Some("cloudflare_account".into()) });
    let vault = |id: &str| match id {
        "cloudflare" => Some(KEY.to_string()),
        "cloudflare_account" => Some("abc123".to_string()),
        _ => None,
    };
    let r = resolve(cf, &[], &vault).unwrap();
    assert_eq!(r.base_url, "https://api.cloudflare.com/client/v4/accounts/abc123/ai/v1");
    let evil = |id: &str| if id == "cloudflare_account" { Some("x/../../evil".to_string()) } else { Some(KEY.to_string()) };
    assert_eq!(resolve(cf, &[], &evil).unwrap_err(), Missing::Address);
    assert!(!format!("{r:?}").contains(KEY), "a resolved key never prints");
}

#[test]
fn a_missing_key_or_a_shut_vault_is_said_not_guessed() {
    let entries = vec![saved("a1", "OpenAI", "openai", "https://api.openai.com/v1", Some(KEY), "")];
    let list = accounts(&entries, FREE_TIERS, &set(&["groq"]), &set(&[]));
    let shut = |_: &str| None;
    assert_eq!(resolve(&list[0], &[], &shut).unwrap_err(), Missing::Key, "the entry was removed since");
    let groq = list.iter().find(|a| a.id == "free-groq").unwrap();
    assert_eq!(resolve(groq, &entries, &shut).unwrap_err(), Missing::Vault);
    assert_eq!(resolve(&list[0], &entries, &shut).unwrap().key.as_deref(), Some(KEY));
}

#[test]
fn models_come_from_the_provider_a_curated_list_or_the_free_tier() {
    let entries = vec![
        saved("a1", "OpenAI", "openai", "https://api.openai.com/v1", Some(KEY), "gpt-4o-mini"),
        saved("a2", "Perplexity", "perplexity", "https://api.perplexity.ai", Some(KEY), ""),
    ];
    let list = accounts(&entries, FREE_TIERS, &set(&["groq"]), &set(&[]));

    let openai = build(&list[0], FREE_TIERS, Some(Ok(listed(&["gpt-4o-mini", "o4-mini", "text-embedding-3-small"]))));
    assert_eq!(openai.listing, Listing::Provider);
    let o4 = openai.models.iter().find(|m| m.model == "o4-mini").unwrap();
    assert_eq!(o4.id, "openai/o4-mini");
    assert_eq!(o4.caps.efforts, vec![Effort::Easy, Effort::Medium, Effort::High]);
    assert!(!openai.models.iter().find(|m| m.model == "gpt-4o-mini").unwrap().caps.thinks());

    let pplx = build(&list[1], FREE_TIERS, None);
    assert_eq!(pplx.listing, Listing::Curated);
    assert!(pplx.models.iter().any(|m| m.id == "perplexity/sonar"));

    let groq = build(list.iter().find(|a| a.id == "free-groq").unwrap(), FREE_TIERS, None);
    assert_eq!(groq.listing, Listing::Curated);
    let oss = groq.models.iter().find(|m| m.model == "openai/gpt-oss-120b").unwrap();
    assert_eq!(oss.id, "free-groq/openai/gpt-oss-120b", "the model keeps its own slash");
    assert_eq!(oss.caps.context, 131_072);
    assert!(oss.caps.thinks());
}

#[test]
fn a_list_that_failed_still_shows_the_model_chosen_in_settings() {
    let entries = vec![saved("a1", "NIM", "nvidia-nim", "https://integrate.api.nvidia.com/v1", Some(KEY), "nvidia/nemotron-3-super-120b-a12b")];
    let list = accounts(&entries, FREE_TIERS, &set(&[]), &set(&[]));
    let built = build(&list[0], FREE_TIERS, Some(Err(ListError::KeyRefused)));
    assert!(matches!(&built.listing, Listing::Failed(why) if why.contains("refused")));
    assert_eq!(built.models.len(), 1);
    assert_eq!(built.models[0].id, "nim/nvidia/nemotron-3-super-120b-a12b");
}

#[test]
fn the_cache_file_never_holds_a_key() {
    let dir = std::env::temp_dir().join(format!("ai-accounts-{}", std::process::id()));
    let path = dir.join("model-catalogue.json");
    let entries = vec![saved("a1", "OpenAI", "openai", "https://api.openai.com/v1", Some(KEY), "gpt-4o-mini")];
    let list = accounts(&entries, FREE_TIERS, &set(&["groq"]), &set(&[]));
    let catalogue = Catalogue {
        accounts: list.iter().map(|a| build(a, FREE_TIERS, Some(Ok(listed(&["gpt-4o-mini"]))))).collect(),
        at: 1,
    };
    store::save(&path, &catalogue).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains(KEY) && !text.contains("sk-"), "{text}");
    assert_eq!(store::load(&path), catalogue);
    assert!(catalogue.find("openai/gpt-4o-mini").is_some());
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn private_context_is_off_for_every_account_until_the_person_switches_it_on() {
    use super::consent;
    let dir = std::env::temp_dir().join(format!("ai-consent-{}", std::process::id()));
    let path = dir.join("ai-accounts.json");
    assert!(consent::load(&path).private_context.is_empty(), "nothing is allowed by default");
    assert!(consent::toggle(&path, "ollama-cloud").unwrap());
    assert!(consent::load(&path).private_context.contains("ollama-cloud"));
    assert!(!consent::toggle(&path, "ollama-cloud").unwrap());
    assert!(consent::load(&path).private_context.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}
