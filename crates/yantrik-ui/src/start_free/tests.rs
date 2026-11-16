use std::cell::RefCell;

use super::offers::{offers, sentence, LABEL};
use super::press::{press, Failed, Outcome, Steps};
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
    assert_eq!(card.lead, "What you tell your minds, and what they read on this computer, is sent to OVHcloud AI Endpoints, also when a mind works on its own.");
    let all = format!("{} {}", card.lead, card.detail);
    assert_eq!(all, sentence("OVHcloud AI Endpoints"), "the card, describe and the refusal take one text");
    assert!(all.contains("sent to") && all.contains("free and rate-limited"), "{all}");
    // The card has no "show more": the lead and the rest are both fields of the offer itself.
    let described = super::describe(&found, None);
    assert_eq!(described["offered"][0]["sentence"], all.as_str());
    assert_eq!(described["pressed"], false);
    assert_eq!(card.model, "free-ovh/gpt-oss-120b");
}

/// What the press grants: the consent is for every call a mind makes, and that is what a turn holds.
#[test]
fn the_sentence_names_each_kind_of_thing_a_mind_sends() {
    let all = sentence("X").to_lowercase();
    for word in [
        "what you tell your minds", "what they read on this computer", "also when a mind works on its own", "your messages and name", "the time",
        "what yantrik remembers about you", "a place you saved", "files", "pages", "command output", "saved password",
        "the screen and clipboard", "your network's name", "open apps", "every mind on yantrik models",
    ] {
        assert!(all.contains(word), "the card does not say {word:?}: {all}");
    }
}

/// A turn's system state, built with every reading filled in: the card has a word for each of its
/// lines, so a new line in the prompt fails here until the card names it.
#[test]
fn every_line_of_a_companions_system_state_has_a_word_in_the_sentence() {
    let snap = yantrik_os::SystemSnapshot {
        battery_available: true,
        battery_level: 50,
        network_connected: true,
        network_ssid: Some("Home".into()),
        cpu_usage_percent: 12.0,
        memory_used_bytes: 1 << 30,
        memory_total_bytes: 4 << 30,
        disk_total_bytes: 100 << 30,
        disk_available_bytes: 50 << 30,
        running_processes: vec![yantrik_os::ProcessInfo { name: "firefox".into(), pid: 1, cpu_percent: 1.0 }],
        idle_seconds: 600,
        user_idle: true,
        ..Default::default()
    };
    let state = crate::system_context::format_system_context(&snap);
    let all = sentence("X").to_lowercase();
    let said_as = |label: &str| match label {
        "Network" => Some("your network's name"),
        "Apps" => Some("open apps"),
        "Battery" => Some("battery"),
        "CPU" => Some("processor"),
        "RAM" => Some("memory"),
        "Disk" => Some("disk"),
        "User idle" => Some("how long the computer has been idle"),
        _ => None,
    };
    let lines: Vec<&str> = state.lines().collect();
    assert!(lines.len() >= 7, "every reading is in the state: {state}");
    for line in lines {
        let label = line.split(':').next().unwrap_or_default();
        let word = said_as(label).unwrap_or_else(|| panic!("the turn carries {label:?} and the card has no word for it: add one to the sentence and to this table"));
        assert!(all.contains(word), "{label}: {word:?} is not in the sentence");
    }
}

/// The README and the docs hold the same text as the card ("the provider" for its name: GitHub strips
/// `<provider>` from a page).
#[test]
fn the_readme_and_the_docs_say_what_the_card_says() {
    let lead = super::offers::LEAD.replace("{p}", "the provider");
    for (name, doc) in [
        ("README.md", include_str!("../../../../README.md")),
        ("docs/getting-started.md", include_str!("../../../../docs/getting-started.md")),
        ("docs/app-control.md", include_str!("../../../../docs/app-control.md")),
    ] {
        let doc = doc.replace("**", "");
        assert!(doc.contains(&lead), "{name} does not carry the lead: {lead}");
        assert!(doc.contains(super::offers::DETAIL), "{name} does not carry the rest");
    }
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
    /// Putting back fails too: the first undo that runs, or a step's own clean-up.
    undo_fails: bool,
    check: Result<String, String>,
}

impl Stub {
    fn new(fail: Option<&'static str>) -> Stub {
        Stub { log: RefCell::new(Vec::new()), fail, undo_fails: false, check: Ok("It said OK.".into()) }
    }
    fn step(&self, name: &'static str) -> Result<(), Failed> {
        self.log.borrow_mut().push(name);
        if self.fail == Some(name) {
            Err("it broke".to_string().into())
        } else {
            Ok(())
        }
    }
    fn undo(&self, name: &'static str) -> Result<(), String> {
        self.log.borrow_mut().push(name);
        if self.undo_fails {
            Err(format!("{name} failed"))
        } else {
            Ok(())
        }
    }
}

impl Steps for Stub {
    fn allow_sharing(&self) -> Result<(), Failed> {
        self.step("allow")
    }
    fn give_models(&self) -> Result<(), Failed> {
        self.step("give")
    }
    fn pick_model(&self) -> Result<(), Failed> {
        self.step("pick")
    }
    fn undo_sharing(&self) -> Result<(), String> {
        self.undo("undo-allow")
    }
    fn undo_models(&self) -> Result<(), String> {
        self.undo("undo-give")
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
fn each_failed_step_is_said_with_why_and_undoes_only_the_steps_that_succeeded() {
    for (fail, expect_log, words) in [
        ("allow", vec!["allow"], "Allowing sharing with OVHcloud did not work: it broke. Nothing was changed."),
        ("give", vec!["allow", "give", "undo-allow"], "Giving Yantrik Companion Yantrik models did not work: it broke. Nothing was changed."),
        ("pick", vec!["allow", "give", "pick", "undo-give", "undo-allow"], "Choosing OVHcloud's model did not work: it broke. Nothing was changed."),
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

/// "Nothing was changed" is said only when it is true: an undo that fails, or a step that could not
/// put back what it half wrote, is named instead.
#[test]
fn nothing_was_changed_is_not_said_when_something_could_not_be_put_back() {
    // The press's own undo fails: both undos still run, and both are named.
    let mut s = Stub::new(Some("pick"));
    s.undo_fails = true;
    let Outcome::Failed(words) = press(&s, "OVHcloud") else { panic!("failed") };
    assert_eq!(*s.log.borrow(), ["allow", "give", "pick", "undo-give", "undo-allow"], "the second undo runs after the first failed");
    assert!(!words.contains("Nothing was changed"), "{words}");
    assert!(words.starts_with("Choosing OVHcloud's model did not work: it broke. Putting it back also failed: undo-give failed; undo-allow failed."), "{words}");
    assert!(words.contains("Settings \u{2192} Minds"), "{words}");
    // A step that could not clean up after itself says so too.
    struct Leaves;
    impl Steps for Leaves {
        fn allow_sharing(&self) -> Result<(), Failed> {
            Ok(())
        }
        fn give_models(&self) -> Result<(), Failed> {
            Err(Failed { why: "the disk is full".into(), left: vec!["the companion's file was not put back".into()] })
        }
        fn pick_model(&self) -> Result<(), Failed> {
            Ok(())
        }
        fn undo_sharing(&self) -> Result<(), String> {
            Ok(())
        }
        fn undo_models(&self) -> Result<(), String> {
            Ok(())
        }
        fn check(&self) -> Result<String, String> {
            Ok(String::new())
        }
    }
    let Outcome::Failed(words) = press(&Leaves, "OVHcloud") else { panic!("failed") };
    assert!(!words.contains("Nothing was changed") && words.contains("the companion's file was not put back"), "{words}");
}

/// The preview's own copy of the card's words and of the Hermes line is the shell's text: what it
/// measures is what the shell draws.
#[test]
fn the_preview_checks_draw_the_text_the_shell_draws() {
    let preview = include_str!("../../../../tests/ui-preview/src/start_free_tests.rs").replace("\\u{2192}", "\u{2192}");
    assert!(preview.contains(&super::offers::lead("OVHcloud AI Endpoints")), "the preview's lead is not the card's");
    assert!(preview.contains(super::offers::DETAIL), "the preview's detail is not the card's");
    assert!(preview.contains(crate::default_mind::BEFORE_NEXT), "the preview's Hermes line is not the shell's");
}
