//! The Model menu's header when every model is held back for one reason (#1042).

use super::blocked::{blocker, Blocker, ALLOW_SHARING, PRIVATE_LANE};
use super::menu::FIX_AI_ACCOUNTS;
use super::{section_for_fix, AI_SECTION};
use super::menu::{model_rows, ModelRow};
use super::tests::{fixture, mind};
use crate::ai_accounts::account::Account;
use crate::ai_accounts::consent::{off_for_every_account, Consent, Sharing};
use crate::ai_accounts::store::Catalogue;

/// The fixture without its account on the local network: cloud accounts only.
fn cloud_only(keep: &[&str]) -> (Catalogue, Vec<Account>) {
    let (mut catalogue, accounts) = fixture();
    catalogue.accounts.retain(|g| keep.is_empty() && g.account != "home" || keep.contains(&g.account.as_str()));
    (catalogue, accounts)
}

fn rows_for(catalogue: &Catalogue, accounts: &[Account], consent: &Consent, private_mode: bool) -> Vec<ModelRow> {
    model_rows(&mind("mind", true, true), catalogue, accounts, consent, private_mode, "", &[], "")
}

fn kinds(rows: &[ModelRow]) -> Vec<&str> {
    rows.iter().map(|r| r.kind).collect()
}

#[test]
fn every_model_held_back_for_sharing_gives_one_header_and_its_actions() {
    let (catalogue, accounts) = cloud_only(&[]);
    let rows = rows_for(&catalogue, &accounts, &Consent::default(), false);
    assert_eq!(&kinds(&rows)[..3], ["banner", "action", "action"]);
    assert!(rows[0].name.starts_with("Sharing is off for every account, so no model can be used yet."), "{}", rows[0].name);
    assert_eq!((rows[1].id.as_str(), rows[1].name.as_str()), (ALLOW_SHARING, "Allow sharing\u{2026}"));
    assert_eq!((rows[2].id.as_str(), rows[2].name.as_str()), (PRIVATE_LANE, "Set up a private lane"));
    let models: Vec<_> = rows.iter().filter(|r| r.kind == "model").collect();
    assert!(!models.is_empty() && models.iter().all(|r| r.disabled), "still not pickable");
    assert!(models.iter().all(|r| r.reason.is_empty()), "the reason is said once, not on every row");
}

#[test]
fn one_account_is_named_and_a_private_lane_already_set_is_not_offered_again() {
    let (catalogue, accounts) = cloud_only(&["openai"]);
    let consent = Consent { lane: Some("home/qwen3:8b".into()), ..Consent::default() };
    let rows = rows_for(&catalogue, &accounts, &consent, false);
    assert!(rows[0].name.starts_with("Sharing is off for OpenAI,"), "{}", rows[0].name);
    assert_eq!(rows[1].name, "Allow sharing with OpenAI\u{2026}");
    assert!(rows.iter().all(|r| r.id != PRIVATE_LANE));
}

#[test]
fn private_mode_for_every_model_is_said_once_with_no_press() {
    let (catalogue, accounts) = cloud_only(&[]);
    let allowed = Consent { default: Some(Sharing::Share), ..Consent::default() };
    let rows = rows_for(&catalogue, &accounts, &allowed, true);
    assert_eq!(rows[0].kind, "banner");
    assert!(rows[0].name.starts_with("Private mode is on"));
    assert!(rows.iter().all(|r| r.kind != "action"));
}

#[test]
fn mixed_reasons_or_a_usable_model_give_no_header_and_each_row_keeps_its_reason() {
    // A model on the local network is usable: some held back, some not.
    let (catalogue, accounts) = fixture();
    let rows = rows_for(&catalogue, &accounts, &Consent::default(), false);
    assert!(rows.iter().all(|r| r.kind != "banner" && r.kind != "action"));
    let held: Vec<_> = rows.iter().filter(|r| r.kind == "model" && r.disabled).collect();
    assert!(!held.is_empty() && held.iter().all(|r| r.reason.contains("allow sharing with")));
    // Different reasons for different rows.
    let (catalogue, accounts) = cloud_only(&[]);
    let (mind, _) = (mind("mind", true, true), ());
    let mixed = Consent { sharing: [("openai".to_string(), Sharing::Share)].into_iter().collect(), ..Consent::default() };
    let rows = model_rows(&mind, &catalogue, &accounts, &mixed, false, "", &[], "");
    assert!(rows.iter().all(|r| r.kind != "banner"), "OpenAI is usable, the others are not");
    assert_eq!(blocker(&mind, &catalogue, &accounts, &Consent::default(), false).map(|b| matches!(b, Blocker::SharingOff { .. })), Some(true));
    assert_eq!(blocker(&mind, &catalogue, &accounts, &mixed, false), None);
    // A mind with its own models has its own note, not this header.
    let own = model_rows(&super::tests::mind("hermes", false, true), &catalogue, &accounts, &Consent::default(), false, "", &[], "");
    assert!(own.iter().all(|r| r.kind != "banner"));
}

#[test]
fn the_actions_take_the_path_of_a_disabled_rows_press_to_ai_accounts() {
    // `fix-model(fix)` -> `section_for_fix`: the same door as a greyed row, so the consent
    // choice is made where AI accounts holds it and nowhere else.
    let (catalogue, accounts) = cloud_only(&[]);
    let rows = rows_for(&catalogue, &accounts, &Consent::default(), false);
    let actions: Vec<_> = rows.iter().filter(|r| r.kind == "action").collect();
    assert_eq!(actions.len(), 2);
    assert!(actions.iter().all(|r| r.fix == FIX_AI_ACCOUNTS && section_for_fix(r.fix) == Some(AI_SECTION)));
}

#[test]
fn the_lens_cause_is_sharing_off_for_every_account_that_lists_a_model() {
    let (cloud, accounts) = cloud_only(&[]);
    assert!(off_for_every_account(&cloud, &accounts, &Consent::default(), false));
    let shared = Consent { sharing: [("openai".to_string(), Sharing::Share)].into_iter().collect(), ..Consent::default() };
    assert!(!off_for_every_account(&cloud, &accounts, &shared, false));
    let (all, accounts) = fixture();
    assert!(!off_for_every_account(&all, &accounts, &Consent::default(), false), "a model on the local network is usable");
    assert!(!off_for_every_account(&Catalogue::default(), &accounts, &Consent::default(), false), "no models is not a consent matter");
}

#[test]
fn the_lens_does_not_blame_sharing_when_a_lane_is_kept_or_private_mode_is_the_cause() {
    let (cloud, accounts) = cloud_only(&[]);
    let lane = Consent { lane: Some("home/qwen3:8b".into()), ..Consent::default() };
    assert!(!off_for_every_account(&cloud, &accounts, &lane, false), "a private lane is a way through");
    assert!(!off_for_every_account(&cloud, &accounts, &Consent::default(), true), "Private mode explains it, not sharing");
    // A lane naming no local account is not a lane.
    let stray = Consent { lane: Some("nowhere/x".into()), ..Consent::default() };
    assert!(off_for_every_account(&cloud, &accounts, &stray, false));
}
