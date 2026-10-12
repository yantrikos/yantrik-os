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
fn private_context_is_off_for_every_account_until_the_person_chooses() {
    use super::consent::{self, Sharing};
    let dir = std::env::temp_dir().join(format!("ai-consent-{}", std::process::id()));
    let path = dir.join("ai-accounts.json");
    let _ = std::fs::remove_file(&path);
    let cloud = saved_account("ollama-cloud", false, false);
    assert_eq!(consent::load(&path).sharing_for(&cloud), Sharing::Off, "nothing is shared by default");
    consent::set(&path, "ollama-cloud", Sharing::Lane).unwrap();
    assert_eq!(consent::load(&path).sharing_for(&cloud), Sharing::Lane);
    consent::agree_at_onboarding(&path, true).unwrap();
    let c = consent::load(&path);
    assert_eq!(c.sharing_for(&cloud), Sharing::Lane, "a choice already made stays");
    assert_eq!(c.sharing_for(&saved_account("openai", false, false)), Sharing::Share, "agreed at onboarding: the rest share");
    assert_eq!(c.sharing_for(&saved_account("free-kilo", false, true)), Sharing::Lane, "a tier that may train is never shared with");
    assert_eq!(c.sharing_for(&saved_account("home", true, false)), Sharing::Share, "a local account: nothing leaves");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_earlier_allow_list_reads_as_share_and_is_never_deleted() {
    use super::consent::{self, Sharing};
    let dir = std::env::temp_dir().join(format!("ai-consent-old-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ai-accounts.json");
    std::fs::write(&path, r#"{"private_context": ["openai"]}"#).unwrap();
    let c = consent::load(&path);
    assert_eq!(c.sharing_for(&saved_account("openai", false, false)), Sharing::Share, "Allow → Share");
    assert_eq!(c.sharing_for(&saved_account("groq", false, false)), Sharing::Off, "Stop → Off");
    consent::set(&path, "groq", Sharing::Share).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["private_context"], serde_json::json!(["openai"]), "the stored list is kept as written: {text}");
    assert_eq!(v["sharing"]["openai"], "share");
    assert_eq!(v["sharing"]["groq"], "share");
    // A newer choice wins over the old list.
    consent::set(&path, "openai", Sharing::Off).unwrap();
    assert_eq!(consent::load(&path).sharing_for(&saved_account("openai", false, false)), Sharing::Off);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_private_lane_is_only_ever_a_local_account() {
    use super::consent::{self, Consent};
    let accounts = [saved_account("home", true, false), saved_account("openai", false, false)];
    let on = |lane: &str| consent::lane(&Consent { lane: Some(lane.into()), ..Consent::default() }, &accounts);
    assert_eq!(on("home/qwen3:8b").as_deref(), Some("home/qwen3:8b"));
    assert_eq!(on("openai/gpt-4o"), None);
    assert_eq!(on("gone/x"), None);
    assert_eq!(consent::lane(&Consent::default(), &accounts), None);
}

fn saved_account(id: &str, local: bool, trains: bool) -> super::account::Account {
    super::account::Account {
        id: id.into(),
        name: id.into(),
        provider_type: "openai".into(),
        base_url: "https://example.invalid/v1".into(),
        key: super::account::KeyRef::None,
        local,
        source: super::account::Source::Saved,
        trains_on_prompts: trains,
        saved_model: String::new(),
    }
}

#[test]
fn the_isos_local_model_account_is_named_honestly_and_its_id_is_the_slug_of_its_name() {
    let script = include_str!("../../../../deploy/yantrik-os/build-debian-iso.sh");
    let value = |key: &str| {
        let line = script.lines().map(str::trim).find(|l| l.starts_with(&format!("{key}="))).unwrap_or_else(|| panic!("{key} is set"));
        line.split_once('=').unwrap().1.trim_matches('"').to_string()
    };
    let name = value("LOCAL_ACCOUNT_NAME");
    assert_eq!(name, "On this computer (Qwen 3.5 4B)");
    assert_eq!(super::account::slug(&name), value("LOCAL_ACCOUNT_ID"), "the lane and the pick name the account by this id");
    assert_eq!(value("LOCAL_MODEL"), "qwen3.5-4b");
    assert!(script.contains("--alias qwen3.5-4b"), "llama-server serves it under that name");
    // The build fills the config's empty model lines; they are there to fill.
    let config = include_str!("../../../../deploy/yantrik-os/config-default.yaml");
    assert!(config.contains("\n  api_base_url: \"\"\n") && config.contains("\n  api_model: \"\"\n"));
    let local = crate::runs_on::identity::is_local("http://127.0.0.1:8341/v1");
    assert!(local, "an account on this computer keeps personal things here");
}
