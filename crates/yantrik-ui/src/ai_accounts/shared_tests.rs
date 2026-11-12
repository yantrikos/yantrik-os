//! The gateway's real route (`gateway::route_for`, the code the server runs, over the live list and
//! card state), with a tier switched Off, its key removed, and switched back On. Fake keys only;
//! no vault is read (`Keys::NotRead`).

use std::collections::BTreeSet;
use std::sync::Arc;

use yantrik_ml::provider::pool::tiers::FREE_TIERS;

use crate::ai_accounts as aa;
use crate::bridge::{CompanionBridge, CompanionCommand};
use crate::test_home::Home;
use crate::wire::free_ai::{self, store::Op};
use aa::account::accounts;
use aa::consent::{self, Sharing};

const PICK: &str = "free-groq/openai/gpt-oss-20b";
const FAKE_KEY: &str = "gsk_FAKE0000000000000000000000000000000000000000000000";

/// The crate-wide HOME lock (`test_home`), a home of this test's own, and everyone shares for any
/// account, so the only reason a call can be refused is the tier itself.
fn home(tag: &str) -> Home {
    let home = Home::new(tag);
    let c = consent::Consent { default: Some(Sharing::Share), ..Default::default() };
    consent::save(&consent::path(), &c).unwrap();
    home
}

fn values() -> Vec<&'static str> {
    yantrik_ml::provider::pool::signup::signup("groq").unwrap().values.iter().map(|v| v.id).collect()
}

/// The list as a listing builds it from the card's state now (what `rebuild` does, less the vault).
fn list_now() {
    let (kept, off) = free_ai::kept_and_off();
    let list = accounts(&[], FREE_TIERS, &kept, &off);
    let catalogue = aa::store::Catalogue {
        accounts: list.iter().map(|a| aa::models::build(a, FREE_TIERS, None)).collect(),
        at: 0,
    };
    aa::install(list, catalogue);
}

fn route() -> Result<yantrik_gateway::server::Routed, yantrik_gateway::Refusal> {
    crate::gateway::route_for("lens", PICK)
}

fn offered() -> bool {
    aa::catalogue().models().any(|m| m.id == PICK)
}

fn setup(tag: &str) -> Home {
    let home = home(tag);
    free_ai::set_state_for_test(&values(), &[]);
    list_now();
    home
}

#[test]
fn a_tier_on_with_its_key_is_routed() {
    let _h = setup("a_tier_on_with_its_key_is_rout");
    assert!(offered());
    let routed = route().unwrap_or_else(|r| panic!("{}", r.message));
    assert_eq!(routed.target.account, "free-groq");
}

#[test]
fn toggling_a_tier_off_refuses_a_pick_on_it_with_no_restart_and_on_offers_it_again() {
    let _h = setup("toggling_a_tier_off_refuses_a_");
    free_ai::toggle_tier("groq");
    aa::shared::prune_to_shared();
    let refused = route().err().expect("a tier switched Off must be refused");
    // The reason is the true one even though the list no longer holds the account.
    assert_eq!((refused.status, refused.code), (403, "tier_off"));
    assert!(refused.message.contains("isn't offered any more"), "{}", refused.message);
    assert!(refused.message.contains("Nothing was sent"), "{}", refused.message);
    assert!(!offered(), "the picker's catalogue must drop it at once");
    assert!(aa::accounts().iter().all(|a| a.id != "free-groq"));

    free_ai::toggle_tier("groq");
    list_now(); // what the refresh that follows does
    assert!(offered());
    assert_eq!(route().unwrap_or_else(|r| panic!("{}", r.message)).target.account, "free-groq");
}

#[test]
fn removing_a_key_refuses_a_pick_on_it_with_no_restart() {
    let _h = setup("removing_a_key_refuses_a_pick_");
    free_ai::set_state_for_test(&[], &[]);
    aa::shared::prune_to_shared();
    let refused = route().err().expect("a removed key must be refused");
    assert_eq!((refused.status, refused.code), (503, "key_missing"));
    assert!(refused.message.contains("its key was removed"), "{}", refused.message);
    assert!(!offered());
}

#[test]
fn a_stale_list_with_the_tier_off_still_refuses_at_routing() {
    let _h = setup("a_stale_list_with_the_tier_off");
    free_ai::toggle_tier("groq"); // no prune, no refresh: the list is stale
    assert!(offered(), "the list is stale on purpose");
    let refused = route().err().expect("Off must refuse even when the list was not refreshed");
    assert_eq!((refused.status, refused.code), (403, "tier_off"));
    assert!(refused.message.contains("isn't offered any more"), "{}", refused.message);
    assert!(refused.message.contains("Nothing was sent"), "{}", refused.message);
}

#[test]
fn a_stale_list_with_the_key_removed_still_refuses_at_routing() {
    let _h = setup("a_stale_list_with_the_key_remo");
    free_ai::set_state_for_test(&[], &[]);
    let refused = route().err().expect("a removed key must refuse even when the list was not refreshed");
    assert_eq!((refused.status, refused.code), (503, "key_missing"));
}

#[test]
fn a_removed_saved_account_leaves_the_list_at_once() {
    let _h = setup("a_removed_saved_account_leaves");
    let mut list = aa::accounts();
    let mut mine = list[0].clone();
    mine.id = "mine".into();
    mine.source = aa::account::Source::Saved;
    mine.entry = Some("gone".into());
    list.push(mine);
    aa::install(list, aa::catalogue());
    aa::shared::prune_with_for_test(&BTreeSet::new());
    assert!(aa::accounts().iter().all(|a| a.id != "mine"));
}

#[test]
fn a_key_never_read_is_not_known_yet_and_is_not_said_removed() {
    let _h = setup("never_read");
    free_ai::set_state_for_test(&[], &[]);
    free_ai::forget_read_for_test();
    let refused = route().err().expect("nothing is known about the keys yet");
    assert_eq!((refused.status, refused.code), (503, "keys_not_known_yet"));
    assert!(!refused.message.contains("removed"), "{}", refused.message);
    assert!(refused.message.contains("Nothing was sent"), "{}", refused.message);
    // And a change made now does not drop what only was never read.
    aa::shared::prune_to_shared();
    assert!(offered(), "a tier is dropped for a missing key only when the vault was read");
}

/// A bridge with a stub worker that records what the companion is told, over a real vault store
/// holding a fake groq key. Holds the vault's test lock: the vault is process-wide.
struct Fixture {
    bridge: Arc<CompanionBridge>,
    told: crossbeam_channel::Receiver<CompanionCommand>,
    db: std::path::PathBuf,
    _vault: std::sync::MutexGuard<'static, ()>,
}

impl Fixture {
    fn new(home: &Home) -> Self {
        crate::provider_handoff::companion::note_on_saved(None);
        let vault = crate::vault_unlock::TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        yantrikdb_core::vault::lock();
        let db = home.path().join("memory.db");
        let conn = rusqlite::Connection::open(&db).expect("open the store");
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;").expect("pragmas");
        yantrikdb_core::vault::init_tables(&conn);
        drop(conn);
        let (cmd_tx, told) = crossbeam_channel::unbounded();
        let bridge = Arc::new(CompanionBridge::with_stub_worker(cmd_tx, db.to_str().unwrap()));
        for id in values() {
            assert!(bridge.provider_keys(Op::Store { id: id.into(), value: FAKE_KEY.into() }, std::time::Duration::from_secs(5)).is_ok());
        }
        Fixture { bridge, told, db, _vault: vault }
    }

    /// What every place that changes what is shared calls, and the listing it starts, waited for.
    fn changed(&self) {
        aa::shared::sharing_changed(self.bridge.clone());
        aa::wait_idle_for_test();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        crate::provider_handoff::companion::note_on_saved(None);
        let _ = std::fs::remove_file(&self.db);
    }
}

#[test]
fn sharing_changed_itself_takes_a_tier_off_at_once_and_back_on_and_follows_a_removed_key() {
    let h = home("sharing_changed");
    let f = Fixture::new(&h);
    free_ai::set_state_for_test(&[], &[]);
    f.changed();
    assert!(offered(), "listed from what the vault holds");
    assert_eq!(route().unwrap_or_else(|r| panic!("{}", r.message)).target.account, "free-groq");

    // Off: the account list is changed before sharing_changed returns, with the listing still to
    // come, and a turn is refused with the true reason.
    free_ai::toggle_tier("groq");
    aa::shared::sharing_changed(f.bridge.clone());
    assert!(!offered(), "dropped before the background listing ends");
    assert_eq!(route().err().expect("refused").code, "tier_off");
    aa::wait_idle_for_test();
    assert!(!offered(), "and the listing that follows does not bring it back");
    assert_eq!(route().err().expect("refused").code, "tier_off");

    // On again, its key still in the vault: offered and routed.
    free_ai::toggle_tier("groq");
    f.changed();
    assert!(offered());
    assert_eq!(route().unwrap_or_else(|r| panic!("{}", r.message)).target.account, "free-groq");

    // Its key removed from the vault: the listing reads the vault, and the turn is refused.
    for id in values() {
        f.bridge.provider_keys(Op::Remove { id: id.into() }, std::time::Duration::from_secs(5)).unwrap();
    }
    f.changed();
    assert!(!offered());
    let refused = route().err().expect("refused");
    assert_eq!((refused.status, refused.code), (503, "key_missing"));
    assert!(f.told.try_iter().next().is_none(), "no saved provider was involved: the companion was not touched");
}

#[test]
fn deleting_the_primary_leaves_a_companion_off_yantrik_models_with_no_key_and_no_backend_that_can_send() {
    let h = home("primary_deleted");
    let f = Fixture::new(&h);
    free_ai::set_state_for_test(&values(), &[]);
    // The companion is put on a saved primary the way a reload does: it holds the key in memory.
    let entry = crate::wire::settings::ProviderStoreEntry {
        id: "p1".into(),
        name: "Mine".into(),
        provider_type: "openai".into(),
        base_url: "https://api.example.test/v1".into(),
        api_key: Some(FAKE_KEY.into()),
        api_key_ref: None,
        unmigrated_key: None,
        auth_type: "bearer".into(),
        is_primary: true,
        is_fallback: false,
        model: "m".into(),
    };
    crate::wire::provider_panel::reload_now(&entry, &*crate::wire::provider_panel::bridge_sender(&f.bridge)).unwrap();
    match f.told.try_iter().collect::<Vec<_>>().as_slice() {
        [CompanionCommand::ReloadLLM { api_key, .. }] => assert_eq!(api_key.as_deref(), Some(FAKE_KEY), "it holds the key now"),
        _ => panic!("the companion was not put on the provider"),
    }
    assert!(!crate::provider_handoff::companion::companion_on_gateway(h.path()), "and it is not on Yantrik models");

    // The provider is deleted in Settings (nothing is saved in this home), which calls this.
    f.changed();
    let told: Vec<CompanionCommand> = f.told.try_iter().collect();
    assert!(told.iter().any(|c| matches!(c, CompanionCommand::StopLLM)), "its LLM was stopped");
    assert!(!told.iter().any(|c| matches!(c, CompanionCommand::ReloadLLM { .. })), "and not put on anything");
    assert!(!f.bridge.is_online());

    // What it runs on cannot send, and holds no key.
    use yantrik_ml::LLMBackend;
    let none = crate::provider_handoff::companion::NoProvider;
    let cfg = yantrik_ml::GenerationConfig::default();
    let msgs = [yantrik_ml::ChatMessage::user("hello")];
    assert!(none.chat(&msgs, &cfg, None).is_err());
    assert!(none.chat_streaming(&msgs, &cfg, None, &mut |_| panic!("a token")).is_err());
    assert_eq!(none.backend_name(), "none");

    // Said once: the next change finds it already stopped.
    f.changed();
    assert!(f.told.try_iter().next().is_none());
}

#[test]
fn a_companion_on_yantrik_models_is_left_alone_when_a_saved_provider_goes() {
    let h = home("on_gateway");
    let f = Fixture::new(&h);
    crate::provider_handoff::companion::note_on_saved(Some("p1".into())); // as a reload on the saved primary leaves it
    let file = crate::provider_handoff::companion::file(h.path());
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, r#"{"base_url":"http://127.0.0.1:7460/v1","model":"x","api_key":"yk-fake"}"#).unwrap();
    f.changed();
    assert!(f.told.try_iter().next().is_none(), "the gateway checks every call it makes");
}

fn saved(id: &str, primary: bool, key_ref: Option<&str>) -> crate::wire::settings::ProviderStoreEntry {
    crate::wire::settings::ProviderStoreEntry {
        id: id.into(),
        name: id.into(),
        provider_type: "openai".into(),
        base_url: format!("https://{id}.example.test/v1"),
        api_key: None,
        api_key_ref: key_ref.map(str::to_string),
        unmigrated_key: None,
        auth_type: "bearer".into(),
        is_primary: primary,
        is_fallback: false,
        model: "m".into(),
    }
}

fn save_providers(entries: Vec<crate::wire::settings::ProviderStoreEntry>) {
    crate::wire::settings::ProviderStore { entries }.save().expect("save the providers");
}

/// What the companion was told, waiting up to `wait` for the reload thread's word.
fn told_within(f: &Fixture, wait: std::time::Duration) -> Vec<CompanionCommand> {
    std::thread::sleep(wait);
    f.told.try_iter().collect()
}

#[test]
fn a_move_to_the_new_primary_that_cannot_get_its_key_leaves_the_companion_stopped() {
    let h = home("move_fails");
    let f = Fixture::new(&h);
    free_ai::set_state_for_test(&values(), &[]);
    // The companion runs on A; B is primary but its key cannot be had (a reference the vault does not hold).
    crate::provider_handoff::companion::note_on_saved(Some("a".into()));
    save_providers(vec![saved("b", true, Some("no-such-ref"))]);
    f.changed();
    let told = told_within(&f, std::time::Duration::from_millis(800));
    assert!(matches!(told.first(), Some(CompanionCommand::StopLLM)), "stopped first, before anything is tried");
    assert!(!told.iter().any(|c| matches!(c, CompanionCommand::ReloadLLM { .. })), "no reload: not for A, and B's key was not had");
    assert!(!f.bridge.is_online());
}

#[test]
fn a_working_move_replaces_the_stop_with_the_new_primary() {
    let h = home("move_works");
    let f = Fixture::new(&h);
    free_ai::set_state_for_test(&values(), &[]);
    crate::provider_handoff::companion::note_on_saved(Some("a".into()));
    save_providers(vec![saved("b", true, None)]);
    f.changed();
    let told = told_within(&f, std::time::Duration::from_millis(800));
    assert!(matches!(told.first(), Some(CompanionCommand::StopLLM)), "the removed provider is out first: {}", told.len());
    match told.get(1) {
        Some(CompanionCommand::ReloadLLM { base_url, .. }) => assert_eq!(base_url, "https://b.example.test/v1"),
        _ => panic!("the companion was not moved onto the new primary"),
    }
    assert!(!told.iter().any(|c| matches!(c, CompanionCommand::ReloadLLM { base_url, .. } if base_url.contains("//a."))));
}

#[test]
fn a_reload_for_a_provider_deleted_while_its_key_was_read_is_not_sent() {
    let h = home("reload_races_delete");
    let f = Fixture::new(&h);
    let send = crate::wire::provider_panel::bridge_sender(&f.bridge);
    let reload = |id: &str| crate::wire::provider_panel::Reload { entry_id: id.into(), provider_type: "openai".into(), base_url: "https://x.example.test/v1".into(), key: None, model: "m".into() };
    save_providers(vec![saved("kept", false, None)]);
    send(reload("gone"));
    assert!(f.told.try_iter().next().is_none(), "nothing is sent for a provider that is no longer saved");
    send(reload("kept"));
    assert!(matches!(f.told.try_iter().collect::<Vec<_>>().as_slice(), [CompanionCommand::ReloadLLM { .. }]));
}

#[test]
fn a_companion_put_on_the_config_provider_is_not_stopped_when_the_old_saved_one_goes() {
    let h = home("config_provider");
    let f = Fixture::new(&h);
    free_ai::set_state_for_test(&values(), &[]);
    crate::provider_handoff::companion::note_on_saved(Some("old".into()));
    // The primary is cleared in Settings: the companion goes to the config provider.
    crate::provider_handoff::companion::reload_config(&f.bridge, "https://config.example.test/v1".into(), None, "m".into());
    assert!(matches!(f.told.try_iter().collect::<Vec<_>>().as_slice(), [CompanionCommand::ReloadLLM { .. }]));
    // The old saved provider is then deleted: the companion is not on it, so it is left alone.
    save_providers(vec![]);
    f.changed();
    assert!(f.told.try_iter().next().is_none(), "a working config provider is not stopped");
}

#[test]
fn a_tier_switched_off_clears_its_probe_and_is_asked_the_gateways_question() {
    let h = home("probe_off");
    let f = Fixture::new(&h);
    free_ai::set_state_for_test(&values(), &[]);
    list_now();
    crate::picker::probe_for_test("free-groq", Some(crate::picker::status::Outcome::Ok));
    free_ai::toggle_tier("groq");
    aa::shared::sharing_changed(f.bridge.clone());
    assert!(!crate::picker::probe_for_test("free-groq", None), "the older success is dropped with the account");
    aa::wait_idle_for_test();
    // Not in the list any more, and still asked first: switched Off, not "key missing" or "checking".
    assert_eq!(crate::picker::probe_outcome(&f.bridge, "free-groq", "openai/gpt-oss-20b"), Some(crate::picker::status::Outcome::SwitchedOff));
    assert_eq!(crate::picker::probe_outcome(&f.bridge, "no-such-account", "m"), Some(crate::picker::status::Outcome::Gone));
}

#[test]
fn a_read_that_began_before_a_removal_does_not_write_the_key_back() {
    let h = home("kept_race");
    let _f = Fixture::new(&h);
    free_ai::set_state_for_test(&values(), &[]);
    let tails = |ids: &[&'static str]| ids.iter().map(|k| (k.to_string(), "…abcd".to_string())).collect::<std::collections::BTreeMap<_, _>>();
    let gone = values()[0];
    let began = free_ai::read_began();
    free_ai::forget_kept(&[gone]); // removed while the listing's read was out
    free_ai::note_kept(tails(&values()), began); // the read comes back holding it
    assert!(!free_ai::kept_and_off().0.contains(gone), "not written back as kept");
    // A read that began after the removal is believed.
    let after = free_ai::read_began();
    free_ai::note_kept(tails(&values()), after);
    assert!(free_ai::kept_and_off().0.contains(gone), "a later read can say it is kept again");
}
