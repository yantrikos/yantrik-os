//! `memory_validate` and what a turn carries (#447, #1176).

use super::*;
use yantrik_app_runtime::control::{Caller, CallerScope};
use yantrik_harness::{protocol, Host};

/// The mind account, as these tests pretend it is: no test machine has one.
const MIND_UID: u32 = 990;
fn is_mind(uid: u32) -> bool {
    uid == MIND_UID
}

fn person() -> u32 {
    unsafe { libc::geteuid() }
}

/// The pid the Mind has. Whether a peer was the Mind is decided at accept and carried as
/// `the_mind`; a Hermes under the same account (#1076) has another pid and is not.
const MIND_PID: i32 = 700;

/// A host with `id` attached by a process running as `uid`, and one agent's memory
/// credential digest.
fn host_with(id: &str, uid: u32) -> (Host, String) {
    let host = Host::new(vec![]);
    host.handle_from(
        protocol::ATTACH,
        &json!({ "id": id, "name": id, "conversations": true }),
        Some(std::process::id()),
        Some(uid),
    )
    .unwrap();
    let agent = host.start_agent(id).unwrap();
    let credential = host.memory_credential(&agent, yantrik_ipc_transport::reach::token_digest).unwrap().unwrap();
    (host, yantrik_ipc_transport::reach::token_digest(&credential))
}

fn ask(caller: Option<Caller>, digest: &str, host: Option<&Host>, choices: Option<Store>) -> Result<Value, String> {
    let _who = CallerScope::enter(caller);
    validate(&json!({ "memory_sha256": digest }), host, is_mind, || choices)
}

fn caller(uid: u32) -> Option<Caller> {
    Some(Caller { pid: MIND_PID, uid, gid: uid, the_mind: uid == MIND_UID })
}

#[test]
fn only_the_mind_account_is_answered_and_it_needs_no_agent_token() {
    let unknown = "0".repeat(64);
    assert!(ask(None, &unknown, None, Some(Store::default())).is_err(), "nobody the kernel named");
    let err = ask(caller(person()), &unknown, None, Some(Store::default())).unwrap_err();
    assert!(err.contains("memory server"), "the person is refused too: {err}");
    // The memory server: no agent token anywhere in this call, and it is answered.
    assert_eq!(ask(caller(MIND_UID), &unknown, None, Some(Store::default())), Ok(Value::Null));
}

#[test]
fn a_known_credential_is_answered_with_its_mind_its_attach_and_its_grants() {
    let (host, digest) = host_with("pi", person());
    let mut store = Store::default();
    store.minds.insert("pi".into(), Grants { remember: true, ..Grants::default() });
    let answered = ask(caller(MIND_UID), &digest, Some(&host), Some(store)).unwrap();
    assert_eq!(answered["mind"], "pi");
    assert_eq!(answered["grants"], json!(["remember"]));
    assert_eq!(answered["person_uid"], person());
    let attach = answered["attach"].as_str().unwrap();
    assert!(attach.starts_with("pi:") && attach.contains("@s1-"), "{attach}");
    // A third party the person never enabled holds a credential that grants nothing.
    assert_eq!(ask(caller(MIND_UID), &digest, Some(&host), Some(Store::default())), Ok(Value::Null));
}

#[test]
fn the_first_party_is_decided_by_the_account_that_attached() {
    let (real, digest) = host_with(FIRST_PARTY_MIND, MIND_UID);
    let answered = ask(caller(MIND_UID), &digest, Some(&real), Some(Store::default())).unwrap();
    assert_eq!(answered["grants"], json!(["recall_ordinary", "remember", "believe"]));

    // The same name from the person's own account earns nothing, even what the person
    // listed for their Mind by name.
    let (borrowed, digest) = host_with(FIRST_PARTY_MIND, person());
    let mut store = Store::default();
    store.minds.insert(FIRST_PARTY_MIND.into(), Grants::ordinary());
    assert_eq!(ask(caller(MIND_UID), &digest, Some(&borrowed), Some(store)), Ok(Value::Null));
}

#[test]
fn a_grants_file_that_cannot_be_trusted_answers_nobody() {
    let (real, digest) = host_with(FIRST_PARTY_MIND, MIND_UID);
    assert_eq!(ask(caller(MIND_UID), &digest, Some(&real), None), Ok(Value::Null));
}

/// #1176: the memory server is the Mind, in its unit. A mind-uid process outside it (a Hermes
/// under the shared account) is refused.
#[test]
fn a_mind_uid_asker_outside_the_minds_unit_is_refused() {
    let unknown = "0".repeat(64);
    let hermes = Some(Caller { pid: 5151, uid: MIND_UID, gid: MIND_UID, the_mind: false });
    let err = ask(hermes, &unknown, None, Some(Store::default())).unwrap_err();
    assert!(err.contains("memory server"), "{err}");
    // Even for a credential it holds, it learns nothing about whether it is good.
    let (host, digest) = host_with("hermes", MIND_UID);
    assert!(ask(hermes, &digest, Some(&host), Some(Store::default())).is_err());
    assert_eq!(ask(caller(MIND_UID), &unknown, None, Some(Store::default())), Ok(Value::Null), "the real unit");
}

/// The turn a host hands `id` after it attached from `uid`, judged as the shell judges it, with no
/// `memory-grants.json`.
fn turn_for(id: &str, uid: u32) -> Value {
    let host = Host::new(vec![])
        .with_liveness(|pid| pid == 4242)
        .with_memory(
            |harness, uid| carries_memory(Some(&Store::default()), harness, uid, is_mind),
            |s: &str| format!("{:0>64}", s.len()),
        )
        .with_memory_url(|| Some("unix:/run/yantrik-mind/1000/memory.sock".to_string()));
    let session = host
        .handle_from(protocol::ATTACH, &json!({ "id": id, "name": id }), Some(4242), Some(uid))
        .unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string();
    let agent = yantrik_harness::AgentId::new(id, yantrik_harness::AgentId::MAIN);
    let _answer = host.send_to(&agent, yantrik_harness::Turn::new("hi")).unwrap();
    let turn = host.handle_from(protocol::POLL, &json!({ "session": session }), Some(4242), Some(uid)).unwrap();
    assert_eq!(turn["text"], "hi");
    turn
}

/// #1176, pinned: the first-party defaults are keyed by the id, the uid only unlocks that id, so
/// a Hermes under the mind uid is granted nothing and its own memory is its own. The positive
/// control is on the same host: `mind` attached from the mind uid gets both, so a poll that
/// stopped carrying memory at all would fail here.
#[test]
fn hermes_under_the_mind_uid_carries_no_memory() {
    assert!(!carries_memory(Some(&Store::default()), "hermes", Some(MIND_UID), is_mind));
    assert!(carries_memory(Some(&Store::default()), FIRST_PARTY_MIND, Some(MIND_UID), is_mind));
    assert!(!carries_memory(Some(&Store::default()), FIRST_PARTY_MIND, Some(person()), is_mind));

    let hermes = turn_for("hermes", MIND_UID);
    assert!(hermes.get("memory_credential").is_none(), "{hermes}");
    assert!(hermes.get("memory_url").is_none(), "{hermes}");

    let mind = turn_for(FIRST_PARTY_MIND, MIND_UID);
    assert!(mind.get("memory_credential").is_some(), "{mind}");
    assert!(mind.get("memory_url").is_some(), "{mind}");
}
