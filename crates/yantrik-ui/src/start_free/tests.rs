use std::cell::RefCell;

use super::offers::{offers, sentence, LABEL};
use super::press::{press, Outcome, Steps};
use crate::ai_accounts::account::{Account, KeyRef, Source};
use crate::ai_accounts::models::{AccountModels, CatalogueModel, Listing};
use crate::ai_accounts::store::Catalogue;

/// A free account as the catalogue lists one: `provider` is a free tier's catalogue id.
pub(super) fn free(provider: &str, id: &str) -> Account {
    Account {
        id: id.into(),
        name: format!("{id} (free)"),
        provider_type: provider.into(),
        base_url: "https://example.invalid/v1".into(),
        key: KeyRef::None,
        entry: None,
        local: false,
        source: Source::Free,
        trains_on_prompts: false,
        saved_model: String::new(),
    }
}

pub(super) fn listing(account: &str, models: &[&str]) -> AccountModels {
    AccountModels {
        account: account.into(),
        name: account.into(),
        listing: Listing::Curated,
        models: models
            .iter()
            .map(|m| CatalogueModel {
                id: format!("{account}/{m}"),
                account: account.into(),
                model: m.to_string(),
                name: m.to_string(),
                caps: yantrik_ml::model_caps::caps_for("ovh", m, None, None),
            })
            .collect(),
    }
}

fn catalogue(groups: Vec<AccountModels>) -> Catalogue {
    Catalogue { accounts: groups, at: 0 }
}

#[test]
fn the_card_names_the_provider_and_says_what_is_sent_on_its_face() {
    let found = offers(&[free("ovh", "free-ovh")], &catalogue(vec![listing("free-ovh", &["gpt-oss-120b"])]));
    let [card] = found.as_slice() else { panic!("one card: {found:?}") };
    assert_eq!(card.provider, "OVHcloud AI Endpoints", "the provider's own name, from the free-tier list");
    assert_eq!(card.label, LABEL);
    assert_eq!(card.label, "No account needed");
    assert_eq!(card.button, "Start free with OVHcloud AI Endpoints");
    assert!(card.sentence.contains("OVHcloud AI Endpoints") && card.sentence.contains("sent to"), "{}", card.sentence);
    assert_eq!(
        card.sentence,
        "Your messages, and what Yantrik remembers about you, are sent to OVHcloud AI Endpoints. No account is needed. \
         It is free and rate-limited, so it can be slow or busy. You can switch it off in Settings \u{2192} AI & Intelligence."
    );
    assert_eq!(sentence("X"), sentence("X"));
    assert_eq!(card.model, "free-ovh/gpt-oss-120b");
    // The sentence is a field of the card, not of a "show more": describe carries it, collapsed or not.
    let described = super::describe(&found, None);
    assert_eq!(described["offered"][0]["sentence"], card.sentence.as_str());
    assert_eq!(described["pressed"], false);
}

#[test]
fn no_free_keyless_account_in_the_catalogue_is_no_card() {
    let one = catalogue(vec![listing("free-ovh", &["gpt-oss-120b"]), listing("free-kilo", &["a"]), listing("groq", &["b"]), listing("free-groq", &["b"])]);
    assert!(offers(&[], &one).is_empty(), "no accounts");
    // A free tier that needs a key.
    let mut keyed = free("groq", "free-groq");
    keyed.key = KeyRef::Vault { id: "groq".into(), fill: None };
    // One that may train on what it is sent can never be shared with.
    let mut trains = free("kilo", "free-kilo");
    trains.trains_on_prompts = true;
    // A saved account, a local one.
    let mut saved = free("ovh", "groq");
    saved.source = Source::Saved;
    let mut local = free("ovh", "free-ovh");
    local.local = true;
    assert!(offers(&[keyed, trains, saved, local], &one).is_empty());
    // Listed as an account, but the catalogue has not listed its models yet.
    assert!(offers(&[free("ovh", "free-ovh")], &catalogue(vec![])).is_empty());
    // Several models: which to start on would be chosen for the person.
    assert!(offers(&[free("ovh", "free-ovh")], &catalogue(vec![listing("free-ovh", &["a", "b"])])).is_empty());
}

#[test]
fn several_such_accounts_are_each_a_card_and_none_is_picked_for_the_person() {
    let both = catalogue(vec![listing("free-ovh", &["a"]), listing("free-other", &["b"])]);
    let found = offers(&[free("ovh", "free-ovh"), free("ovh", "free-other")], &both);
    assert_eq!(found.iter().map(|o| o.account.as_str()).collect::<Vec<_>>(), ["free-ovh", "free-other"]);
}

/// Steps that record what ran, and fail where told to.
struct Stub {
    log: RefCell<Vec<&'static str>>,
    fail: Option<&'static str>,
    check: Result<String, String>,
}

impl Stub {
    fn new(fail: Option<&'static str>) -> Stub {
        Stub { log: RefCell::new(Vec::new()), fail, check: Ok("It said OK.".into()) }
    }
    fn step(&self, name: &'static str) -> Result<(), String> {
        self.log.borrow_mut().push(name);
        if self.fail == Some(name) {
            Err("it broke".into())
        } else {
            Ok(())
        }
    }
}

impl Steps for Stub {
    fn allow_sharing(&self) -> Result<(), String> {
        self.step("allow")
    }
    fn give_models(&self) -> Result<(), String> {
        self.step("give")
    }
    fn pick_model(&self) -> Result<(), String> {
        self.step("pick")
    }
    fn undo_sharing(&self) {
        self.log.borrow_mut().push("undo-allow");
    }
    fn undo_models(&self) {
        self.log.borrow_mut().push("undo-give");
    }
    fn undo_pick(&self) {
        self.log.borrow_mut().push("undo-pick");
    }
    fn check(&self) -> Result<String, String> {
        self.log.borrow_mut().push("check");
        self.check.clone()
    }
}

#[test]
fn the_press_goes_in_the_order_the_three_paths_go_and_ends_on_the_check() {
    let s = Stub::new(None);
    let out = press(&s, "OVHcloud");
    assert_eq!(*s.log.borrow(), ["allow", "give", "pick", "check"]);
    assert_eq!(out, Outcome::Answered("OVHcloud answered. It said OK.".into()));
}

#[test]
fn each_failed_step_is_said_with_why_and_leaves_the_earlier_steps_undone() {
    for (fail, expect_log, words) in [
        ("allow", vec!["allow", "undo-allow"], "Allowing sharing with OVHcloud did not work: it broke. Nothing was changed."),
        ("give", vec!["allow", "give", "undo-give", "undo-allow"], "Giving Yantrik Companion Yantrik models did not work: it broke. Nothing was changed."),
        ("pick", vec!["allow", "give", "pick", "undo-pick", "undo-give", "undo-allow"], "Choosing OVHcloud's model did not work: it broke. Nothing was changed."),
    ] {
        let s = Stub::new(Some(fail));
        let out = press(&s, "OVHcloud");
        assert_eq!(*s.log.borrow(), expect_log, "{fail}");
        assert_eq!(out, Outcome::Failed(words.into()), "{fail}");
    }
}

#[test]
fn a_provider_that_does_not_answer_keeps_the_choice_and_says_so() {
    let mut s = Stub::new(None);
    s.check = Err("it is not set up to think yet".into());
    let out = press(&s, "OVHcloud");
    assert_eq!(*s.log.borrow(), ["allow", "give", "pick", "check"], "nothing is undone");
    let Outcome::Unanswered(words) = out else { panic!("{out:?}") };
    assert!(words.starts_with("OVHcloud did not answer:") && words.contains("Your choice is kept") && words.contains("try again"), "{words}");
}
