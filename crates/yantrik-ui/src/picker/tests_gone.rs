//! #1162, the 520 crash: a pick no account offers made the shell overflow its stack about a second
//! after it started. Saying the pick had gone named its mind from the host's listing; the listing
//! asks the shell for each mind's detail line; and that line is what said the pick had gone.
//!
//! These run the real path with the real hook, on the host the rest of the shell finds, on stacks
//! small enough that a loop fails at once. A loop aborts the whole test binary, which the harness
//! reports as a failure: it cannot hang.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use yantrik_gateway::Refusal;

use super::menu::MindSeen;
use super::status::Fix;
use super::*;
use crate::ai_accounts::account::{Account, KeyRef, Source};
use crate::ai_accounts::models::{AccountModels, CatalogueModel, Listing};
use crate::ai_accounts::store::Catalogue;
use crate::wire::harness::test_host::{host_with_a_mind, MIND, NAME};

/// Everything here is process-wide (HOME, the pick, the accounts, the shell's line hook), so these
/// tests take turns.
static WORLD: Mutex<()> = Mutex::new(());

/// A stack that a loop through the line hook fills in a moment, and that the real work needs a
/// fraction of.
const SMALL_STACK: usize = 512 * 1024;

const ISNT_OFFERED: &str = "isn't offered by any of your accounts any more";

struct World {
    _turn: MutexGuard<'static, ()>,
    home: PathBuf,
    previous: Option<std::ffi::OsString>,
}

/// `(account id, its models)`; every account is on this machine's network.
type Accounts<'a> = &'a [(&'a str, &'a [&'a str])];

impl World {
    /// A shell whose accounts are `accounts` and whose default pick is `pick`, with `MIND` on
    /// Yantrik models and attached to the host.
    fn new(tag: &str, accounts: Accounts, pick: &str) -> World {
        let turn = WORLD.lock().unwrap_or_else(|e| e.into_inner());
        let home = std::env::temp_dir().join(format!("yantrik-gone-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let marker = home.join(".config/yantrik/handoff");
        std::fs::create_dir_all(&marker).unwrap();
        let on_gateway = serde_json::json!({ "provider_id": "yantrik-models", "provider_name": "Yantrik models", "model": "picked", "at": "", "files": [] });
        std::fs::write(marker.join(format!("{MIND}.json")), on_gateway.to_string()).unwrap();
        let previous = std::env::var_os("HOME");
        std::env::set_var("HOME", &home);
        host_with_a_mind();
        yantrik_harness::host::detail::set_model_line(crate::answering::model_line);
        let world = World { _turn: turn, home, previous };
        world.list(accounts);
        world.pick(pick);
        world
    }

    /// The accounts the shell lists from now on.
    fn list(&self, accounts: Accounts) {
        let account = |id: &str| Account {
            id: id.into(),
            name: format!("Box {id}"),
            provider_type: "openai".into(),
            base_url: "http://192.168.4.2:8000/v1".into(),
            key: KeyRef::None,
            entry: None,
            local: true,
            source: Source::Saved,
            trains_on_prompts: false,
            saved_model: String::new(),
        };
        let built = |id: &str, models: &[&str]| AccountModels {
            account: id.into(),
            name: format!("Box {id}"),
            listing: Listing::Curated,
            models: models
                .iter()
                .map(|m| CatalogueModel {
                    id: format!("{id}/{m}"),
                    account: id.into(),
                    model: m.to_string(),
                    name: m.to_string(),
                    caps: yantrik_ml::model_caps::caps_for("openai", m, None, None),
                })
                .collect(),
        };
        crate::ai_accounts::set_listing_for_test(
            accounts.iter().map(|(id, _)| account(id)).collect(),
            Catalogue { accounts: accounts.iter().map(|(id, models)| built(id, models)).collect(), at: 0 },
        );
        yantrik_harness::host::detail::refresh_model_lines();
    }

    /// The default pick changes, as the picker does.
    fn pick(&self, model: &str) {
        lock(&CHOICES).model = model.to_string();
        yantrik_harness::host::detail::refresh_model_lines();
    }
}

impl Drop for World {
    fn drop(&mut self) {
        lock(&CHOICES).model.clear();
        crate::ai_accounts::set_listing_for_test(Vec::new(), Catalogue::default());
        yantrik_harness::host::detail::refresh_model_lines();
        match &self.previous {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// `work` on a thread with a small stack and a name, as the gateway's connection threads are.
fn on_a_small_stack<T: Send + 'static>(name: &str, work: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new().name(name.into()).stack_size(SMALL_STACK).spawn(work).unwrap().join().expect("the work finished")
}

/// What the shell says about the mind, asked every way it is asked: the answer, the gateway's own
/// routing of a call, the mind's detail line in the host's listing, and the status the panel and
/// the picker judge from those (`status_now` less the gateway's own listening), with its one fix.
struct Said {
    answering: Result<crate::answering::Answer, Refusal>,
    call: Result<yantrik_gateway::server::Routed, Refusal>,
    detail: String,
    status: super::status::Status,
}

fn said() -> Said {
    on_a_small_stack("main", || {
        let host = crate::wire::harness::host().expect("the host");
        let detail = host.list().into_iter().find(|e| e.id == MIND).and_then(|e| e.detail).unwrap_or_default();
        let mind = MindSeen { id: MIND.into(), name: NAME.into(), attached: true, active: true, state: String::new(), on_gateway: true, private_context: true, builtin_runs_on: None };
        // `status_now`, but for the gateway that is not listening under test: its other inputs.
        let (answer, refusal) = match crate::answering::answering(MIND) {
            Ok(a) => (Some(a), None),
            Err(r) => (None, Some(r)),
        };
        let input = super::status::Input { now: 1_000, on_gateway: true, answer, refusal, fallback: fallback_words(&mind), ..Default::default() };
        Said {
            answering: crate::answering::answering(MIND),
            // What a connection thread of the gateway does with a call.
            call: on_a_small_stack("gateway-conn", || crate::gateway::route_for(MIND, yantrik_gateway::PICKED)),
            detail,
            status: super::status::judge(&input),
        }
    })
}

fn assert_said_as_gone(said: &Said) {
    let refusal = said.answering.as_ref().expect_err("a gone pick answers nothing");
    assert_eq!(refusal.code, "pick_gone");
    assert!(refusal.message.contains(ISNT_OFFERED) && refusal.message.contains(NAME), "{}", refusal.message);
    assert_eq!(said.call.as_ref().expect_err("the gateway refuses it").message, refusal.message, "the call is refused in the words the panel says");
    assert_eq!(said.detail, "Hermes 0.14.0 \u{b7} the gateway refuses its calls");
    assert_eq!((said.status.state, said.status.fix), ("pick-gone", Fix::PickAgain));
    assert!(said.status.words.contains(ISNT_OFFERED), "{}", said.status.words);
    assert_eq!(Fix::PickAgain.label(), "Pick again");
}

/// The 520 case, as the gate reproduced it: the pick is `nope/nope`, which no account offers.
#[test]
fn a_pick_no_account_offers_is_said_and_never_recurses() {
    let _world = World::new("nope", &[("box1", &["qwen3:8b"])], "nope/nope");
    assert_said_as_gone(&said());
}

/// An account taken away while a mind is on one of its models: a pick that was offered a moment ago.
#[test]
fn a_pick_whose_account_was_removed_is_said_and_never_recurses() {
    let world = World::new("removed", &[("box1", &["qwen3:8b"]), ("box2", &["gpt-oss"])], "box2/gpt-oss");
    assert_eq!(said().answering.as_ref().expect("offered").model, "gpt-oss");
    world.list(&[("box1", &["qwen3:8b"])]);
    let after = said();
    assert_said_as_gone(&after);
    assert!(!after.status.words.contains("box2"), "a slug is never said: {}", after.status.words);
}

/// A call the gateway refuses as `pick_gone` reaches the panel as the sentence and the one press,
/// with the host's listing asked in the same breath.
#[test]
fn a_pick_the_gateway_refuses_is_said_with_pick_again() {
    let world = World::new("refused", &[("box1", &["qwen3:8b"])], "box1/qwen3:8b");
    assert!(said().call.is_ok(), "an offered pick is routed");
    world.pick("box1/retired");
    let refused = said();
    assert_said_as_gone(&refused);
    assert_eq!(refused.call.expect_err("refused").status, 404);
}

/// A pick that is offered still names its model, on the answer and on the mind's own line.
#[test]
fn an_offered_pick_still_shows_its_model() {
    let _world = World::new("offered", &[("box1", &["qwen3:8b", "gpt-oss"])], "box1/gpt-oss");
    let said = said();
    let answer = said.answering.expect("offered");
    assert_eq!((answer.account.as_str(), answer.model.as_str()), ("Box box1", "gpt-oss"));
    assert_eq!(said.detail, "Hermes 0.14.0 \u{b7} gpt-oss");
    assert!(said.call.is_ok());
    assert_ne!(said.status.fix, Fix::PickAgain);
}
