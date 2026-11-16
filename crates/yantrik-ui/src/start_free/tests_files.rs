//! The press against a scratch home: the files it leaves are the ones the three existing paths
//! leave, and nothing is written, and no provider is called, before it.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::offers::offers;
use super::press::{press, Outcome};
use super::real::Real;
use super::tests::{free, listing};
use crate::ai_accounts::account::Account;
use crate::ai_accounts::consent::{self, Sharing};
use crate::ai_accounts::store::Catalogue;
use crate::provider_handoff;
use crate::test_home::Home;
use crate::wire::provider_models::{check_key, KeyCheck};

/// A provider on the loopback that counts the connections made to it and answers every request.
struct Stub {
    url: String,
    hits: Arc<AtomicUsize>,
}

fn stub() -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let url = format!("http://{}/v1", listener.local_addr().expect("its address"));
    let hits = Arc::new(AtomicUsize::new(0));
    let counted = hits.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { continue };
            counted.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0u8; 4096];
            let _ = conn.read(&mut buf);
            let body = r#"{"choices":[{"message":{"role":"assistant","content":"OK"}}]}"#;
            let _ = write!(conn, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        }
    });
    Stub { url, hits }
}

fn world(stub: &Stub) -> (Account, Catalogue) {
    let mut a = free("ovh", "free-ovh");
    a.base_url = stub.url.clone();
    let catalogue = Catalogue { accounts: vec![listing("free-ovh", &["gpt-oss-120b"])], at: 0 };
    crate::ai_accounts::set_listing_for_test(vec![a.clone()], catalogue.clone());
    (a, catalogue)
}

fn real(a: &Account, model: &str, stub: &Stub) -> Real {
    let url = stub.url.clone();
    let id = format!("free-ovh/{model}");
    let model = model.to_string();
    let check = move || match check_key(&url, "", &model) {
        KeyCheck::Works => Ok("It answered.".to_string()),
        other => Err(format!("{other:?}")),
    };
    Real::new(a.clone(), id, Box::new(check))
}

fn bytes(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

/// The companion's file with its gateway token (minted at each Apply) taken out.
fn without_token(path: &Path) -> Option<serde_json::Value> {
    let mut v: serde_json::Value = serde_json::from_slice(&bytes(path)?).ok()?;
    v["api_key"] = "<token>".into();
    Some(v)
}

fn files(home: &Path) -> (Option<Vec<u8>>, Option<Vec<u8>>, Option<serde_json::Value>) {
    (
        bytes(&consent::path()),
        bytes(&crate::picker::choices::path()),
        without_token(&provider_handoff::companion::file(home)),
    )
}

/// What Settings → AI accounts' switch, Settings → Minds' Apply and the ask bar's model menu leave,
/// done by hand in that order.
fn by_hand(a: &Account) {
    consent::set_for_account(&consent::path(), a, Sharing::Share).expect("the switch");
    let plan = crate::wire::harness_provider::plan(provider_handoff::companion::ID, provider_handoff::GATEWAY_ID).expect("the plan");
    provider_handoff::apply(&crate::wire::harness_provider::home_dir(), &plan).expect("Apply");
    crate::picker::pick_model_for(provider_handoff::companion::ID, "free-ovh/gpt-oss-120b");
}

#[test]
fn before_the_press_nothing_is_written_and_no_provider_is_asked() {
    let home = Home::new("start-free-before");
    crate::picker::restore_picks(Default::default());
    let s = stub();
    let (a, catalogue) = world(&s);
    // Everything the page does while it is shown: the card is made, described, and the steps are set up.
    let shown = offers(&[a.clone()], &catalogue);
    assert_eq!(shown.len(), 1);
    let _ = super::describe(&shown, None);
    let _steps = real(&a, "gpt-oss-120b", &s);
    assert_eq!(files(home.path()), (None, None, None), "no sharing, no pick, no Yantrik models");
    assert_eq!(consent::load(&consent::path()).sharing_for(&a), Sharing::Off, "every default stays as it is");
    assert_eq!(s.hits.load(Ordering::SeqCst), 0, "no connection to the provider");
}

#[test]
fn after_the_press_the_files_are_what_the_three_paths_leave_by_hand() {
    let home = Home::new("start-free-after");
    crate::picker::restore_picks(Default::default());
    let s = stub();
    let (a, _) = world(&s);
    let out = press(&real(&a, "gpt-oss-120b", &s), "OVHcloud");
    assert!(matches!(out, Outcome::Answered(_)), "{out:?}");
    assert!(s.hits.load(Ordering::SeqCst) >= 1, "the check reached the provider");
    let pressed = files(home.path());
    assert!(pressed.0.is_some() && pressed.1.is_some() && pressed.2.is_some(), "all three are written");
    assert_eq!(consent::load(&consent::path()).sharing_for(&a), Sharing::Share);

    // The same, by hand, from the same start.
    provider_handoff::revert(home.path(), provider_handoff::companion::ID).expect("revert");
    crate::picker::restore_picks(Default::default());
    let _ = std::fs::remove_file(consent::path());
    assert_eq!(files(home.path()), (None, None, None));
    by_hand(&a);
    assert_eq!(files(home.path()), pressed, "byte for byte (the gateway token, minted at each Apply, aside)");
    // Left clean for the tests that follow.
    provider_handoff::revert(home.path(), provider_handoff::companion::ID).expect("revert");
    crate::picker::restore_picks(Default::default());
}

#[test]
fn a_step_that_fails_leaves_the_earlier_steps_undone_on_disk() {
    let home = Home::new("start-free-fails");
    crate::picker::restore_picks(Default::default());
    let s = stub();
    let (a, _) = world(&s);
    // A saved sharing file the person already had: it is put back as it was, not deleted.
    let had = b"{\n  \"default\": \"off\"\n}\n".to_vec();
    std::fs::create_dir_all(consent::path().parent().expect("a folder")).expect("the config folder");
    std::fs::write(consent::path(), &had).expect("an earlier file");

    // Step 3 fails: the model is not one the catalogue lists.
    let steps = Real::new(a.clone(), "free-ovh/not-a-model".into(), Box::new(|| Ok(String::new())));
    let out = press(&steps, "OVHcloud");
    let Outcome::Failed(words) = out else { panic!("{out:?}") };
    assert!(words.starts_with("Choosing OVHcloud's model did not work:") && words.ends_with("Nothing was changed."), "{words}");
    assert_eq!(bytes(&consent::path()), Some(had.clone()), "the person's sharing file is as it was");
    assert_eq!(without_token(&provider_handoff::companion::file(home.path())), None, "the companion is off Yantrik models again");
    assert_eq!(bytes(&crate::picker::choices::path()), None, "no pick was written");
    assert_eq!(s.hits.load(Ordering::SeqCst), 0, "the provider was never asked");

    // Step 1 fails: a local account has no such choice.
    let mut local = a.clone();
    local.local = true;
    let steps = Real::new(local, "free-ovh/gpt-oss-120b".into(), Box::new(|| Ok(String::new())));
    let Outcome::Failed(words) = press(&steps, "OVHcloud") else { panic!("refused") };
    assert!(words.starts_with("Allowing sharing with OVHcloud did not work:"), "{words}");
    assert_eq!(bytes(&consent::path()), Some(had), "still as it was");
    assert_eq!(without_token(&provider_handoff::companion::file(home.path())), None);
}
