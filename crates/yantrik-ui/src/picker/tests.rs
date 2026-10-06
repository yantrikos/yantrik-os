use std::collections::BTreeSet;

use yantrik_gateway::log::Call;
use yantrik_ml::provider::pool::tiers::FREE_TIERS;

use super::attach;
use super::choices::Choices;
use super::menu::{caps_words, mind_rows, model_rows, MindSeen};
use super::status::{judge, Fix, Input, Outcome, Probe};
use crate::ai_accounts::account::accounts;
use crate::ai_accounts::consent::Consent;
use crate::ai_accounts::models::build;
use crate::ai_accounts::store::Catalogue;
use crate::wire::provider_models::ListedModel;
use crate::wire::settings::ProviderStoreEntry;

fn mind(id: &str, on_gateway: bool, private_context: bool) -> MindSeen {
    MindSeen { id: id.into(), name: id.to_uppercase(), attached: true, active: true, state: String::new(), on_gateway, private_context }
}

fn fixture() -> (Catalogue, Vec<crate::ai_accounts::account::Account>) {
    let saved = vec![
        ProviderStoreEntry {
            id: "a1".into(),
            name: "OpenAI".into(),
            provider_type: "openai".into(),
            base_url: "https://api.openai.com/v1".into(),
            api_key: Some("sk-x".into()),
            auth_type: "bearer".into(),
            is_primary: false,
            is_fallback: false,
            model: String::new(),
        },
        ProviderStoreEntry {
            id: "a2".into(),
            name: "Home".into(),
            provider_type: "ollama".into(),
            base_url: "http://127.0.0.1:11434".into(),
            api_key: None,
            auth_type: "none".into(),
            is_primary: false,
            is_fallback: false,
            model: String::new(),
        },
    ];
    let kept: BTreeSet<String> = ["groq".to_string()].into_iter().collect();
    let list = accounts(&saved, FREE_TIERS, &kept, &BTreeSet::new());
    let listed = |ids: &[&str]| ids.iter().map(|i| ListedModel { id: i.to_string(), name: i.to_string(), size_bytes: 0 }).collect::<Vec<_>>();
    let catalogue = Catalogue {
        accounts: vec![
            build(&list[0], FREE_TIERS, Some(Ok(listed(&["o4-mini", "gpt-4o"])))),
            build(&list[1], FREE_TIERS, Some(Ok(listed(&["qwen3:8b"])))),
            build(list.iter().find(|a| a.id == "free-groq").unwrap(), FREE_TIERS, None),
            build(list.iter().find(|a| a.id == "free-kilo").unwrap(), FREE_TIERS, None),
        ],
        at: 0,
    };
    (catalogue, list)
}

#[test]
fn the_mind_menu_puts_the_answering_mind_first_and_says_what_the_others_need() {
    let mut minds = vec![mind("pi", true, false), mind("mind", false, true), mind("hermes", false, true)];
    minds[0].active = false;
    minds[2].attached = false;
    minds[2].active = false;
    minds[2].state = "Not installed".into();
    let rows = mind_rows(&minds);
    assert_eq!(rows[0].id, "mind");
    assert!(rows[0].current && rows[0].words.contains("its own models"));
    assert_eq!(rows[1].words, "ready · Yantrik models");
    assert!(!rows[2].selectable && rows[2].words.starts_with("not installed"));
}

#[test]
fn the_model_menu_groups_by_account_lists_recent_first_and_searches() {
    let (catalogue, accounts) = fixture();
    let pi = mind("pi", true, false);
    let rows = model_rows(&pi, &catalogue, &accounts, &Consent::default(), false, "openai/o4-mini", &["free-groq/openai/gpt-oss-120b".to_string()], "");
    let headers: Vec<&str> = rows.iter().filter(|r| r.kind == "header").map(|r| r.name.as_str()).collect();
    assert_eq!(headers, ["RECENT", "OPENAI", "HOME", "GROQ (FREE)", "KILO GATEWAY (FREE)"]);
    assert_eq!(rows[1].id, "free-groq/openai/gpt-oss-120b", "the recent pick comes first");
    assert!(rows.iter().any(|r| r.id == "openai/o4-mini" && r.current));
    assert!(rows.iter().all(|r| !r.disabled), "a mind that sends no private context may use any");
    let found = model_rows(&pi, &catalogue, &accounts, &Consent::default(), false, "", &[], "qwen 8b");
    let ids: Vec<&str> = found.iter().filter(|r| r.kind == "model").map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["home/qwen3:8b"]);
    let none = model_rows(&pi, &catalogue, &accounts, &Consent::default(), false, "", &[], "zzz");
    assert!(none.iter().any(|r| r.kind == "note" && r.name.contains("zzz")));
}

#[test]
fn models_a_mind_cannot_use_are_disabled_with_the_reason() {
    let (catalogue, accounts) = fixture();
    let the_mind = mind("mind", true, true);
    let rows = model_rows(&the_mind, &catalogue, &accounts, &Consent::default(), false, "", &[], "");
    let o4 = rows.iter().find(|r| r.id == "openai/o4-mini").unwrap();
    assert!(o4.disabled && o4.reason.contains("allow private context for OpenAI"), "{}", o4.reason);
    let home = rows.iter().find(|r| r.id == "home/qwen3:8b").unwrap();
    assert!(!home.disabled, "a model on this network takes private context");
    let groq = rows.iter().find(|r| r.id.starts_with("free-groq/")).unwrap();
    assert!(groq.disabled && groq.reason.contains("allow private context for Groq"), "{}", groq.reason);
    let kilo = rows.iter().find(|r| r.id.starts_with("free-kilo/")).unwrap();
    assert!(kilo.disabled && kilo.reason.contains("may train"), "a free tier that trains on prompts is never allowed: {}", kilo.reason);
    let allowed = Consent { private_context: ["openai".to_string()].into_iter().collect() };
    let rows = model_rows(&the_mind, &catalogue, &accounts, &allowed, false, "", &[], "");
    assert!(!rows.iter().find(|r| r.id == "openai/o4-mini").unwrap().disabled);
    // Private mode: only what is on this network.
    let rows = model_rows(&mind("pi", true, false), &catalogue, &accounts, &allowed, true, "", &[], "");
    assert!(rows.iter().find(|r| r.id == "openai/o4-mini").unwrap().reason.starts_with("Private mode"));
    // Its own models: every model says how to change that.
    let rows = model_rows(&mind("hermes", false, true), &catalogue, &accounts, &allowed, false, "", &[], "");
    assert!(rows[0].kind == "note" && rows[0].name.contains("uses its own models"));
    assert!(rows.iter().filter(|r| r.kind == "model").all(|r| r.disabled && r.reason.contains("Use Yantrik models")));
    assert_eq!(caps_words(&catalogue.find("openai/o4-mini").unwrap().caps), "thinks · 200K context");
}

#[test]
fn a_pick_is_kept_per_mind_and_recent_first() {
    let mut c = Choices::default();
    c.pick_model("pi", "openai/o4-mini");
    c.pick_model("mind", "home/qwen3:8b");
    c.pick_effort("pi", "high");
    c.pick_model("pi", "openai/o4-mini");
    assert_eq!(c.of("pi").model, "openai/o4-mini");
    assert_eq!(c.of("pi").effort, "high");
    assert_eq!(c.of("mind").model, "home/qwen3:8b", "switching mind keeps each one's own");
    assert_eq!(c.recent, ["openai/o4-mini", "home/qwen3:8b"], "no repeats, newest first");
    assert_eq!(c.of("nobody"), Default::default());
    let dir = std::env::temp_dir().join(format!("picker-{}", std::process::id()));
    let path = dir.join("picker.json");
    super::choices::save(&path, &c).unwrap();
    assert_eq!(super::choices::load(&path), c);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_handed_over_file_carries_its_provenance_its_hash_and_small_content() {
    let dir = std::env::temp_dir().join(format!("attach-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let small = dir.join("plan.md");
    std::fs::write(&small, b"hello").unwrap();
    let big = dir.join("big.bin");
    std::fs::write(&big, vec![0u8; (attach::INLINE_EACH + 1) as usize]).unwrap();
    let mut left = attach::INLINE_TOTAL;
    let a = attach::read(&small, "lens", "2026-10-06T10:00:00Z", &mut left).unwrap();
    assert_eq!((a.handed_over_by.as_str(), a.via.as_str()), ("person", "lens"));
    assert_eq!(a.sha256, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
    assert_eq!(a.content_b64, "aGVsbG8=");
    assert_eq!((a.mime.as_str(), a.size), ("text/markdown", 5));
    assert_eq!(left, attach::INLINE_TOTAL - 5);
    let b = attach::read(&big, "lens", "now", &mut left).unwrap();
    assert!(b.content_b64.is_empty(), "over 1 MiB: the path and the hash, not the content");
    let mut spent = 0;
    assert!(attach::read(&small, "lens", "now", &mut spent).unwrap().content_b64.is_empty(), "the message's allowance is spent");
    assert!(attach::read(&dir, "lens", "now", &mut left).is_err(), "a folder is not handed over");
    assert_eq!(attach::dropped_paths("file:///home/p/My%20Notes.md\r\n# comment\nrelative.txt\n/tmp/x"), [std::path::PathBuf::from("/home/p/My Notes.md"), "/tmp/x".into()]);
    let listed = attach::list(&dir).unwrap();
    assert_eq!(listed.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["big.bin", "plan.md"]);
    assert_eq!(attach::resolve_dir(&format!("{}/..", dir.display()), &dir), std::fs::canonicalize(std::env::temp_dir()).unwrap());
    assert_eq!(attach::resolve_dir("/no/such/place", &dir), dir);
    let _ = std::fs::remove_dir_all(dir);
}

fn call(at: i64, status: u16, outcome: &str) -> Call {
    Call { at, harness: "pi".into(), account: "openai".into(), model: "o4-mini".into(), effort: None, status, outcome: outcome.into(), prompt_tokens: 0, completion_tokens: 0, ms: 1 }
}

#[test]
fn connected_is_said_only_after_a_real_call_or_a_probe() {
    let base = Input { now: 1_000, on_gateway: true, model: "openai/o4-mini".into(), ..Input::default() };
    let s = judge(&base);
    assert_eq!((s.state, s.words.as_str()), ("checking", "checking · o4-mini"), "configured is not connected");
    let s = judge(&Input { last_call: Some(call(990, 200, "")), ..base.clone() });
    assert_eq!((s.state, s.words.as_str(), s.fix), ("connected", "connected · o4-mini", Fix::None));
    let s = judge(&Input { probe: Some(Probe { at: 950, outcome: Outcome::Ok }), ..base.clone() });
    assert_eq!(s.state, "connected");
    let stale = judge(&Input { probe: Some(Probe { at: 100, outcome: Outcome::Ok }), ..base.clone() });
    assert_eq!(stale.state, "checking", "a probe goes stale and is asked again");
    let s = judge(&Input { last_call: Some(call(990, 401, "upstream_401")), ..base.clone() });
    assert_eq!((s.state, s.fix), ("key-missing", Fix::AddKey));
    let s = judge(&Input { last_call: Some(call(990, 502, "unreachable")), ..base.clone() });
    assert_eq!((s.state, s.fix), ("unreachable", Fix::Retry));
    let s = judge(&Input { last_call: Some(call(990, 403, "private_context_not_allowed")), ..base.clone() });
    assert_eq!((s.state, s.fix), ("not-allowed", Fix::Allow));
    let s = judge(&Input { probe: Some(Probe { at: 990, outcome: Outcome::KeyMissing }), ..base.clone() });
    assert_eq!(s.words, "key missing");
    let s = judge(&Input { gateway_down: Some("127.0.0.1:7460 is in use".into()), ..base.clone() });
    assert_eq!(s.state, "unreachable");
    let s = judge(&Input { model: String::new(), ..base.clone() });
    assert_eq!((s.state, s.fix), ("not-set-up", Fix::AddKey));
    // A call for another model says nothing about this one.
    let mut other = call(990, 200, "");
    other.model = "gpt-4o".into();
    assert_eq!(judge(&Input { last_call: Some(other), ..base.clone() }).state, "checking");
}

#[test]
fn a_mind_with_its_own_models_is_connected_only_once_it_answered() {
    let own = Input { now: 1_000, on_gateway: false, ..Input::default() };
    let s = judge(&own);
    assert_eq!((s.state, s.words.as_str(), s.fix), ("own", "uses its own models", Fix::SetUp));
    let s = judge(&Input { own_answered_at: Some(900), own_model: "kimi-k3".into(), ..own.clone() });
    assert_eq!(s.words, "connected · kimi-k3");
    assert_eq!(judge(&Input { own_answered_at: Some(1), ..own }).state, "own", "long ago is not now");
}

#[test]
fn switching_mind_never_relabels_what_an_earlier_mind_said() {
    use slint::Model as _;
    let transcript = slint::VecModel::<crate::MessageData>::default();
    let mind = crate::streaming::Speaker { mind: "Yantrik Mind".into(), model: "deepseek-v4.1-flash".into() };
    transcript.push(crate::streaming::reply_bubble(&mind));
    // `use_harness companion`: the next reply is the companion's, and only the next one.
    transcript.push(crate::streaming::reply_bubble(&crate::streaming::Speaker::companion()));
    let first = transcript.row_data(0).unwrap();
    assert_eq!((first.mind.as_str(), first.model.as_str()), ("Yantrik Mind", "deepseek-v4.1-flash"));
    assert_eq!(transcript.row_data(1).unwrap().mind, crate::wire::harness::BUILTIN_NAME);
    // The bubble names the message's own mind, and the one answering now only for a message
    // from before minds were kept on it.
    let bubble = include_str!("../../../yantrik-ui-slint/ui/components/chat_message.slint");
    assert!(bubble.contains("root.data.mind != \"\" ? root.data.mind : root.mind-name"), "the label is the message's own");
    let lens = include_str!("../../../yantrik-ui-slint/ui/components/intent_lens.slint");
    assert!(lens.contains("root.messages[idx - 1].mind != msg.mind"), "a new mind starts a new group");
}
