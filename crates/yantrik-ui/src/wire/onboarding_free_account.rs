//! "Continue with my free account" on the first run's provider screen, and the first boot after an
//! install made that way: `prove_and_pick`'s free-account branch (#1168, finding 7).
//!
//! The key is already in the vault (`os:provider-key:<tier>`, kept by the Free AI card after its
//! own check) and is saved nowhere else: no provider entry, no `saved:` row, no file. The account
//! is proven the way the key path proves one (`check_key` on the tier's base URL, with the vault's
//! value and the tier's first model); only then is `provider_chosen` set and `free-<tier>/<model>`
//! made the one pick for every mind. "Connected" follows from that alone, and the screen goes on
//! (sharing; in the installer, `finished()`) only from "Connected", as it does for a key.
//!
//! Across an install the vault crosses in the copied system (`/opt/yantrik/data`, which the copy
//! does not leave out: `installer_carry`). The first boot then finds a pick naming a free account
//! whose value is kept, and skips the AI screens as it does for a saved provider.

use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use slint::ComponentHandle;
use yantrik_ml::provider::pool::signup;
use yantrik_ml::provider::pool::tiers::{FreeTier, FREE_TIERS};

use super::free_ai::store::{Op, Reply};
use super::onboarding::{load_saved, saved_path, set_up_default_mind, show_new_backend, update_saved};
use super::provider_models::KeyCheck;
use crate::ai_accounts::account::{KeyRef, Source};
use crate::app_context::AppContext;
use crate::bridge::CompanionBridge;
use crate::wire::settings::{ProviderStore, ProviderStoreEntry};
use crate::App;

/// What the Slint button chooses: not a provider card, the free account already set up.
pub const FREE_ACCOUNT: &str = "free-account";

/// How long the first boot waits for the vault to say which values are kept.
const VAULT_WAIT: Duration = Duration::from_secs(30);

static BRIDGE: OnceLock<Arc<CompanionBridge>> = OnceLock::new();

pub fn wire(ui: &App, ctx: &AppContext) {
    let _ = BRIDGE.set(ctx.bridge.clone());
    // A first boot that chose through these screens but saved no provider chose a free account:
    // `onboarding::wire` has just skipped the AI screens for a saved primary, and does not know this one.
    if load_saved(&saved_path()).provider_chosen && ProviderStore::load().primary().is_none() {
        skip_when_picked(ui);
    }
}

/// A free tier with every value kept and not switched off, as the Free AI card says "ready".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The AI-accounts id the pick is made under (`free-groq`).
    pub account: String,
    /// As the screens name it: "Groq (free)".
    pub name: String,
    pub base_url: String,
    /// The vault id of the provider's own value (`groq`); none for a tier that needs no key.
    pub key_id: Option<String>,
    /// The vault id of a part of the address only the person knows (Cloudflare's account id).
    pub fill: Option<String>,
    /// The tier's first model: what the account is proven with, and picked on.
    pub model: String,
}

/// The providers the person saved: they take their ids first in the catalogue, so a free tier's id
/// is worked out after them, as `ai_accounts::rebuild` does (a saved "Free Groq" makes the tier
/// `free-groq-2`).
fn saved_entries() -> Vec<ProviderStoreEntry> {
    ProviderStore::load().entries
}

/// The ready free accounts, in the tiers' order, under the ids the catalogue gives them.
fn plans(saved: &[ProviderStoreEntry], tiers: &[FreeTier], kept: &BTreeSet<String>, off: &BTreeSet<String>) -> Vec<Plan> {
    crate::ai_accounts::account::accounts(saved, tiers, kept, off)
        .into_iter()
        .filter_map(|a| {
            // The card lists the tiers that have a sign-up page; a tier without one is not a row
            // the person could have made ready.
            if a.source != Source::Free {
                return None;
            }
            signup::signup(&a.provider_type)?;
            let model = tiers.iter().find(|t| t.id == a.provider_type)?.models.first()?.id.to_string();
            let (key_id, fill) = match a.key {
                KeyRef::Vault { id, fill } => (Some(id), fill),
                _ => (None, None),
            };
            Some(Plan { account: a.id, name: a.name, base_url: a.base_url, key_id, fill, model })
        })
        .collect()
}

/// The account the button continues with: the first ready one.
pub fn plan(saved: &[ProviderStoreEntry], kept: &BTreeSet<String>, off: &BTreeSet<String>) -> Option<Plan> {
    plans(saved, FREE_TIERS, kept, off).into_iter().next()
}

/// `free-groq` from the pick `free-groq/openai/gpt-oss-120b`; none for any other pick.
pub fn free_account_of_pick(pick: &str) -> Option<&str> {
    pick.split_once('/').map(|(account, _)| account).filter(|a| a.starts_with("free-"))
}

/// Prove the account with the vault's value and the tier's first model, through `check`
/// (`provider_models::check_key`). The value is read and sent, never kept anywhere here.
pub fn prove(plan: &Plan, value_of: &dyn Fn(&str) -> Option<String>, check: &dyn Fn(&str, &str, &str) -> KeyCheck) -> Result<(), String> {
    let read = |id: &str| value_of(id).ok_or_else(|| format!("the {} key could not be read from the vault (if the vault is locked, unlock it first)", plan.name));
    let key = match &plan.key_id {
        Some(id) => read(id)?,
        None => String::new(),
    };
    let base = match &plan.fill {
        Some(fill) => plan.base_url.replace("{account_id}", &read(fill)?),
        None => plan.base_url.clone(),
    };
    match check(&base, &key, &plan.model) {
        KeyCheck::Works => Ok(()),
        KeyCheck::Refused => Err(format!("{} refused the key it was given; paste a new one on the card above", plan.name)),
        KeyCheck::Unconfirmed(why) => Err(why.trim_end_matches('.').to_string()),
    }
}

/// Make `<account>/<model>` the one pick for every mind (#675), the catalogue listed again first
/// so an account made ready a moment ago is in it. The key path's `prove_and_pick` picks through
/// here too.
pub(super) fn pick_for_every_mind(account: &str, model: &str) -> Result<(), String> {
    if let Some(bridge) = BRIDGE.get() {
        crate::ai_accounts::refresh_now(bridge);
    }
    crate::ai_accounts::onboarding::choose_model(&crate::ai_accounts::models::gateway_id(account, model))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Connected,
    /// It answered, but the pick did not take.
    PickFailed(String),
    /// It did not answer, or could not be asked.
    Attention(String),
}

/// The branch in order: proven, then chosen (`provider_chosen`), then picked. Nothing is recorded
/// for an account that did not answer.
pub fn run(
    plan: &Plan,
    value_of: &dyn Fn(&str) -> Option<String>,
    check: &dyn Fn(&str, &str, &str) -> KeyCheck,
    chosen: &dyn Fn(),
    pick: &dyn Fn(&str, &str) -> Result<(), String>,
) -> Outcome {
    if let Err(why) = prove(plan, value_of, check) {
        return Outcome::Attention(why);
    }
    chosen();
    match pick(&plan.account, &plan.model) {
        Ok(()) => Outcome::Connected,
        Err(why) => Outcome::PickFailed(why),
    }
}

/// The button was pressed (through `choose-provider`, as every card is chosen): off the UI
/// thread, prove the first ready account, record the choice, make the pick, then say so. The
/// Slint has already put the screen in connect mode `free`; a failure brings the list back, with
/// the reason above it.
pub fn continue_with(ui: &App) {
    let (kept, off) = super::free_ai::kept_and_off();
    let Some(plan) = plan(&saved_entries(), &kept, &off) else {
        return attention(ui, "no free account is ready yet; paste a key on the card above first".into());
    };
    let Some(bridge) = BRIDGE.get().cloned() else {
        return attention(ui, "the vault cannot be reached yet".into());
    };
    ui.set_fr_chosen_name(plan.name.clone().into());
    ui.set_fr_chosen_model(plan.model.clone().into());
    ui.set_fr_connect_status("checking".into());
    ui.set_fr_connect_words(format!("Checking (started {})", super::onboarding::clock(chrono::Local::now())).into());
    let weak = ui.as_weak();
    let spawned = std::thread::Builder::new().name("free-account-continue".into()).spawn(move || {
        let outcome = run(
            &plan,
            &|id| crate::ai_accounts::vault_value(&bridge, id),
            &super::provider_models::check_key,
            &|| update_saved(&saved_path(), |s| s.provider_chosen = true),
            &pick_for_every_mind,
        );
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            match outcome {
                Outcome::Connected => {
                    show_new_backend(&ui);
                    ui.set_fr_connect_status("connected".into());
                    ui.set_fr_connect_words(
                        format!("Connected to {}. Yantrik Companion and your Minds that use Yantrik models will use {}.", plan.name, plan.model).into(),
                    );
                    set_up_default_mind(&ui);
                }
                Outcome::PickFailed(why) => {
                    tracing::warn!(error = %why, model = %plan.model, "first run: the free account's model was not made the pick");
                    attention(&ui, format!("{} answered, but your Minds that use Yantrik models could not be set to use it ({why}); press Continue with my free account again", plan.name));
                }
                Outcome::Attention(why) => attention(&ui, why),
            }
        });
    });
    if spawned.is_err() {
        attention(ui, "the check could not be started".into());
    }
}

/// Said above the list, which comes back for another go.
fn attention(ui: &App, why: String) {
    ui.set_fr_connect_mode("".into());
    ui.set_fr_connect_status("attention".into());
    ui.set_fr_connect_words(format!("Needs attention: {why}").into());
}

/// A late answer from the vault skips the AI screens only for a person still at Welcome (step 1,
/// the step the skip passes over). One who has already moved on is on the AI screens by their own
/// choice, and is not moved underneath.
fn skip_still_applies(step: i32) -> bool {
    step == 1
}

/// The first boot's skip of the AI screens for a pick that names a free account, once the vault
/// says its value is kept. The vault is asked off the UI thread; the screen is told when it
/// answers, which is before the person is past Welcome.
fn skip_when_picked(ui: &App) {
    let pick = crate::picker::remembered_model("").unwrap_or_default();
    let Some(account) = free_account_of_pick(&pick).map(str::to_string) else { return };
    let Some(bridge) = BRIDGE.get().cloned() else { return };
    let weak = ui.as_weak();
    let _ = std::thread::Builder::new().name("free-account-first-boot".into()).spawn(move || {
        let Ok(Reply::Tails(tails)) = bridge.provider_keys(Op::Tails, VAULT_WAIT) else { return };
        let kept: BTreeSet<String> = tails.into_keys().collect();
        let off = super::free_ai::kept_and_off().1;
        let Some(ready) = plans(&saved_entries(), FREE_TIERS, &kept, &off).into_iter().find(|p| p.account == account) else { return };
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            if !skip_still_applies(ui.get_fr_step()) {
                return;
            }
            ui.set_fr_installer_chose(true);
            ui.set_fr_chosen_name(ready.name.clone().into());
            ui.set_fr_installer_choice(ready.name.into());
            // As for a saved provider: the model step was done, so the default mind's install
            // starts here, once per machine.
            if !crate::onboarding::marker_path().exists() {
                crate::default_mind::start(crate::default_mind::When::FirstRun);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const GROQ_MODEL: &str = "openai/gpt-oss-120b";

    fn kept(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    fn groq() -> Plan {
        plan(&[], &kept(&["groq"]), &BTreeSet::new()).expect("Groq is ready")
    }

    #[test]
    fn the_first_ready_tier_is_the_plan_and_an_unready_or_off_one_is_not() {
        let p = groq();
        assert_eq!((p.account.as_str(), p.name.as_str(), p.key_id.as_deref(), p.model.as_str()), ("free-groq", "Groq (free)", Some("groq"), GROQ_MODEL));
        assert_eq!(p.base_url, "https://api.groq.com/openai/v1");
        assert_eq!(p.fill, None);
        assert_ne!(plan(&[], &kept(&[]), &BTreeSet::new()).map(|p| p.account), Some("free-groq".into()), "no key kept: not Groq");
        assert_ne!(plan(&[], &kept(&["groq"]), &kept(&["groq"])).map(|p| p.account), Some("free-groq".into()), "switched off: not Groq");
    }

    /// Proven with the vault's value and the tier's first model; the value is read, never stored.
    #[test]
    fn the_account_is_proven_with_the_vaults_value_and_the_tiers_first_model() {
        let reads = RefCell::new(Vec::new());
        let asked = RefCell::new(None);
        let value_of = |id: &str| {
            reads.borrow_mut().push(id.to_string());
            Some("gsk_value".to_string())
        };
        let check = |base: &str, key: &str, model: &str| {
            *asked.borrow_mut() = Some((base.to_string(), key.to_string(), model.to_string()));
            KeyCheck::Works
        };
        assert_eq!(prove(&groq(), &value_of, &check), Ok(()));
        assert_eq!(*reads.borrow(), ["groq"], "one read, of the provider's own value");
        assert_eq!(asked.into_inner(), Some(("https://api.groq.com/openai/v1".into(), "gsk_value".into(), GROQ_MODEL.into())));
    }

    #[test]
    fn a_refused_key_and_an_unreadable_vault_are_said() {
        let some = |_: &str| Some("gsk_value".to_string());
        let refused = prove(&groq(), &some, &|_, _, _| KeyCheck::Refused).unwrap_err();
        assert!(refused.starts_with("Groq (free) refused the key"), "{refused}");
        let unconfirmed = prove(&groq(), &some, &|_, _, _| KeyCheck::Unconfirmed("it did not answer in time.".into())).unwrap_err();
        assert_eq!(unconfirmed, "it did not answer in time");
        let never_asked = RefCell::new(true);
        let locked = prove(&groq(), &|_| None, &|_, _, _| { *never_asked.borrow_mut() = false; KeyCheck::Works }).unwrap_err();
        assert!(locked.contains("could not be read from the vault"), "{locked}");
        assert!(never_asked.into_inner(), "nothing is sent without the key");
    }

    /// The address with a part only the person knows is filled from the vault before the proof.
    #[test]
    fn an_address_with_the_persons_account_id_is_filled_from_the_vault() {
        let p = Plan { base_url: "https://x/{account_id}/v1".into(), key_id: Some("cloudflare".into()), fill: Some("cloudflare_account".into()), ..groq() };
        let asked = RefCell::new(None);
        let value_of = |id: &str| Some(if id == "cloudflare_account" { "acct1".to_string() } else { "key1".to_string() });
        let check = |base: &str, key: &str, _: &str| {
            *asked.borrow_mut() = Some((base.to_string(), key.to_string()));
            KeyCheck::Works
        };
        assert_eq!(prove(&p, &value_of, &check), Ok(()));
        assert_eq!(asked.into_inner(), Some(("https://x/acct1/v1".into(), "key1".into())));
    }

    /// Decision (finding 7): proven, then `provider_chosen`, then the pick `free-groq/<model>`; a
    /// refused account records nothing and picks nothing.
    #[test]
    fn the_choice_and_the_pick_follow_the_proof_and_only_the_proof() {
        let order = RefCell::new(Vec::new());
        let some = |_: &str| Some("gsk_value".to_string());
        let chosen = || order.borrow_mut().push("chosen".to_string());
        let pick = |account: &str, model: &str| {
            order.borrow_mut().push(format!("pick {account}/{model}"));
            Ok(())
        };
        assert_eq!(run(&groq(), &some, &|_, _, _| KeyCheck::Works, &chosen, &pick), Outcome::Connected);
        assert_eq!(*order.borrow(), ["chosen", &format!("pick free-groq/{GROQ_MODEL}")]);

        order.borrow_mut().clear();
        let outcome = run(&groq(), &some, &|_, _, _| KeyCheck::Refused, &chosen, &pick);
        assert!(matches!(outcome, Outcome::Attention(_)));
        assert!(order.borrow().is_empty(), "nothing recorded for an account that did not answer");

        let failing = |_: &str, _: &str| Err("no catalogue".to_string());
        assert_eq!(run(&groq(), &some, &|_, _, _| KeyCheck::Works, &chosen, &failing), Outcome::PickFailed("no catalogue".into()));
        assert_eq!(*order.borrow(), ["chosen"], "chosen, since it answered; the pick is what failed");
    }

    fn saved_named(name: &str) -> ProviderStoreEntry {
        ProviderStoreEntry {
            id: "p1".into(),
            name: name.into(),
            provider_type: "custom".into(),
            base_url: "https://example.test/v1".into(),
            api_key: None,
            api_key_ref: None,
            unmigrated_key: None,
            auth_type: "none".into(),
            is_primary: false,
            is_fallback: false,
            model: String::new(),
        }
    }

    /// The pick names the id the catalogue gives the proven tier: with a saved "Free Groq" taking
    /// `free-groq`, the tier is `free-groq-2`, and that is what is proven, picked and matched at
    /// first boot (never the saved entry's id).
    #[test]
    fn the_pick_names_the_account_that_was_proven() {
        let saved = [saved_named("Free Groq")];
        let p = plan(&saved, &kept(&["groq"]), &BTreeSet::new()).expect("Groq is ready");
        assert_eq!(p.account, "free-groq-2");
        let catalogue = crate::ai_accounts::account::accounts(&saved, FREE_TIERS, &kept(&["groq"]), &BTreeSet::new());
        let tier = catalogue.iter().find(|a| a.provider_type == "groq" && a.source == Source::Free).expect("the tier");
        assert_eq!(p.account, tier.id, "the plan's id is the catalogue's id for the tier");
        let saved_one = catalogue.iter().find(|a| a.source == Source::Saved).expect("the saved entry");
        assert_eq!(saved_one.id, "free-groq", "the saved entry keeps the bare id");
        let picked = RefCell::new(String::new());
        let some = |_: &str| Some("gsk_value".to_string());
        let pick = |account: &str, _: &str| {
            *picked.borrow_mut() = account.to_string();
            Ok(())
        };
        assert_eq!(run(&p, &some, &|_, _, _| KeyCheck::Works, &|| {}, &pick), Outcome::Connected);
        assert_eq!(*picked.borrow(), tier.id);
        assert!(plans(&saved, FREE_TIERS, &kept(&["groq"]), &BTreeSet::new()).iter().all(|q| q.account != "free-groq"), "the saved entry is never a free plan");
    }

    /// A vault answer that comes late moves nobody who has already left Welcome.
    #[test]
    fn a_late_skip_applies_only_to_a_person_still_at_welcome() {
        assert!(skip_still_applies(1));
        for step in 2..=5 {
            assert!(!skip_still_applies(step), "step {step}: the person moved on");
        }
        let src = include_str!("onboarding_free_account.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        let late = &code[code.find("skip_still_applies(ui.get_fr_step())").expect("the late skip asks first")..];
        assert!(late.find("skip_still_applies").unwrap() < late.find("set_fr_installer_chose(true)").unwrap());
        assert!(late.find("skip_still_applies").unwrap() < late.find("default_mind::start").unwrap(), "no default mind is started for a skip that does not apply");
    }

    #[test]
    fn the_first_boot_knows_a_pick_that_names_a_free_account() {
        assert_eq!(free_account_of_pick(&format!("free-groq/{GROQ_MODEL}")), Some("free-groq"));
        assert_eq!(free_account_of_pick("anthropic-onboarding/claude-sonnet-4-5"), None);
        assert_eq!(free_account_of_pick("free-groq"), None, "no model: not a pick");
        assert_eq!(free_account_of_pick(""), None);
    }

    /// Decision (finding 7): the key stays where the card put it. This branch names no provider
    /// store, saves no primary and stores nothing in the vault: providers.yaml is unchanged and the
    /// vault keeps its one `os:provider-key:groq` row and gains no `saved:` row.
    #[test]
    fn the_branch_saves_the_key_nowhere() {
        let src = include_str!("onboarding_free_account.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        for forbidden in ["save_primary", "adopt(", "Op::Store", "api_key", "unmigrated_key", ".save()"] {
            assert!(!code.contains(forbidden), "the free branch must not {forbidden}");
        }
        // Two provider-store reads: the first boot's "no saved primary" test, and the saved ids.
        assert_eq!(code.matches("ProviderStore::").count(), 2);
        assert_eq!(code.matches("provider_chosen = true").count(), 1, "chosen once, in `run`'s order");
        // "Connected" only from `Outcome::Connected`, and the default mind only from there.
        let body = &code[code.find("pub fn continue_with").unwrap()..code.find("fn attention").unwrap()];
        assert_eq!(body.matches("\"connected\"").count(), 1);
        assert_eq!(body.matches("set_up_default_mind(&ui);").count(), 1);
        let arm = &body[body.find("Outcome::Connected =>").unwrap()..body.find("Outcome::PickFailed").unwrap()];
        assert!(arm.contains("\"connected\"") && arm.contains("set_up_default_mind(&ui);"));
    }

    /// Decision (finding 7): the Slint button only calls a callback; step 3 and, in the installer,
    /// `finished()` follow "connected" alone, through the same Continue the key path uses.
    #[test]
    fn the_button_only_asks_and_the_screen_goes_on_from_connected_alone() {
        let slint = include_str!("../../../yantrik-ui-slint/ui/onboarding_first_run.slint");
        let button = slint.find("label: \"Continue with my free account\";").expect("the button");
        let block = &slint[button..button + slint[button..].find("\n                            }").unwrap()];
        assert!(block.contains(&format!("root.choose-provider(\"{FREE_ACCOUNT}\")")), "{block}");
        assert!(block.contains("root.connect-mode = \"free\""), "the list steps aside while it is checked: {block}");
        for not_here in ["finished()", "step = 3", "step = 4"] {
            assert!(!block.contains(not_here), "the button itself must not {not_here}: {block}");
        }
        let enabled = "enabled: root.connect-status == \"connected\" && root.connect-mode != \"\";";
        assert_eq!(slint.matches(enabled).count(), 1, "the one Continue under the connect words, for the free mode too");
    }
}
