//! The gateway's real route (`gateway::route_for`, the code the server runs, over the live list and
//! card state), with a tier switched Off, its key removed, and switched back On. Fake keys only;
//! no vault is read (`Keys::NotRead`).

use std::collections::BTreeSet;
use std::sync::Mutex;

use yantrik_ml::provider::pool::tiers::FREE_TIERS;

use crate::ai_accounts as aa;
use aa::account::accounts;
use aa::consent::{self, Sharing};
use crate::wire::free_ai;

/// The list, the card's state and HOME are process-wide: one test at a time.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

const PICK: &str = "free-groq/openai/gpt-oss-20b";

struct Home(std::path::PathBuf, Option<std::ffi::OsString>);

impl Home {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("yantrik-shared-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let previous = std::env::var_os("HOME");
        std::env::set_var("HOME", &dir);
        // Share for any account: the only reason a call can be refused is the tier itself.
        let c = consent::Consent { default: Some(Sharing::Share), ..Default::default() };
        consent::save(&consent::path(), &c).unwrap();
        Home(dir, previous)
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        match &self.1 {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
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

fn setup() -> (std::sync::MutexGuard<'static, ()>, Home) {
    let guard = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    free_ai::set_state_for_test(&values(), &[]);
    list_now();
    (guard, home)
}

#[test]
fn a_tier_on_with_its_key_is_routed() {
    let (_g, _h) = setup();
    assert!(offered());
    let routed = route().unwrap_or_else(|r| panic!("{}", r.message));
    assert_eq!(routed.target.account, "free-groq");
}

#[test]
fn toggling_a_tier_off_refuses_a_pick_on_it_with_no_restart_and_on_offers_it_again() {
    let (_g, _h) = setup();
    free_ai::toggle_tier("groq");
    aa::shared::prune_to_shared();
    let refused = route().err().expect("a tier switched Off must be refused");
    assert_eq!(refused.code, "account_not_found");
    assert!(!offered(), "the picker's catalogue must drop it at once");
    assert!(aa::accounts().iter().all(|a| a.id != "free-groq"));

    free_ai::toggle_tier("groq");
    list_now(); // what the refresh that follows does
    assert!(offered());
    assert_eq!(route().unwrap_or_else(|r| panic!("{}", r.message)).target.account, "free-groq");
}

#[test]
fn removing_a_key_refuses_a_pick_on_it_with_no_restart() {
    let (_g, _h) = setup();
    free_ai::set_state_for_test(&[], &[]);
    aa::shared::prune_to_shared();
    assert!(route().is_err());
    assert!(!offered());
}

#[test]
fn a_stale_list_with_the_tier_off_still_refuses_at_routing() {
    let (_g, _h) = setup();
    free_ai::toggle_tier("groq"); // no prune, no refresh: the list is stale
    assert!(offered(), "the list is stale on purpose");
    let refused = route().err().expect("Off must refuse even when the list was not refreshed");
    assert_eq!((refused.status, refused.code), (403, "tier_off"));
    assert!(refused.message.contains("isn't offered any more"), "{}", refused.message);
    assert!(refused.message.contains("Nothing was sent"), "{}", refused.message);
}

#[test]
fn a_stale_list_with_the_key_removed_still_refuses_at_routing() {
    let (_g, _h) = setup();
    free_ai::set_state_for_test(&[], &[]);
    let refused = route().err().expect("a removed key must refuse even when the list was not refreshed");
    assert_eq!((refused.status, refused.code), (503, "key_missing"));
}

#[test]
fn a_removed_saved_account_leaves_the_list_at_once() {
    let (_g, _h) = setup();
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
