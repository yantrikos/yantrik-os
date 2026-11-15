//! The worker's reload arms and cuts the backend, and drops a reload a stop has overtaken. Each
//! test fails without the piece it names.

use std::io::Read as _;
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use yantrik_ml::{GenerationConfig, LlmSwitch, Stopped};

use super::reload::{armed_reload, Reload};
use crate::bridge::CompanionBridge;

const FAKE_KEY: &str = "fake-key-for-tests";

/// A server that takes a request and never answers, counting the connections it gets.
fn silent_server() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let connections = Arc::new(AtomicUsize::new(0));
    let count = connections.clone();
    std::thread::spawn(move || {
        while let Ok((mut conn, _)) = listener.accept() {
            count.fetch_add(1, Ordering::SeqCst);
            std::thread::spawn(move || {
                let mut sink = [0u8; 4096];
                while conn.read(&mut sink).is_ok_and(|n| n > 0) {}
            });
        }
    });
    (url, connections)
}

fn reload<'a>(url: &'a str, epoch: u64) -> Reload<'a> {
    Reload { provider_type: "custom", base_url: url, key: Some(FAKE_KEY), model: "stub", epoch }
}

fn bridge(dir: &tempfile::TempDir) -> (Arc<CompanionBridge>, crossbeam_channel::Receiver<crate::bridge::CompanionCommand>) {
    let db = dir.path().join("memory.db");
    drop(rusqlite::Connection::open(&db).expect("open the store"));
    let (cmd_tx, told) = crossbeam_channel::unbounded();
    (Arc::new(CompanionBridge::with_stub_worker(cmd_tx, db.to_str().unwrap())), told)
}

/// H1: A is chosen, a turn is queued, A is removed, and only then does the worker reach A's reload.
#[test]
fn a_reload_asked_for_before_a_stop_arms_nothing_after_it() {
    let (url, connections) = silent_server();
    let switch = LlmSwitch::new();
    let asked_at = switch.epoch();
    switch.cut(); // the press on Remove

    assert!(armed_reload(&switch, &reload(&url, asked_at)).is_none(), "dropped before anything is built");
    assert_eq!(connections.load(Ordering::SeqCst), 0);
    // And the switch holds nothing to cut, so the dropped reload left no backend behind.
    assert!(armed_reload(&switch, &reload(&url, switch.epoch())).is_some(), "a reload asked for after the stop is armed");
}

#[test]
fn a_reload_the_bridge_queues_carries_the_epoch_it_was_asked_under() {
    let dir = tempfile::tempdir().unwrap();
    let (bridge, told) = bridge(&dir);
    let before = bridge.llm_switch_for_test().epoch();
    bridge.reload_llm("custom".into(), "http://127.0.0.1:1/v1".into(), None, "m".into());
    bridge.stop_llm();
    bridge.reload_llm("custom".into(), "http://127.0.0.1:1/v1".into(), None, "m".into());
    let epochs: Vec<u64> = told
        .try_iter()
        .filter_map(|c| match c {
            crate::bridge::CompanionCommand::ReloadLLM { epoch, .. } => Some(epoch),
            _ => None,
        })
        .collect();
    assert_eq!(epochs, vec![before, before + 1], "the second was asked for after the press");
}

/// M2: what the worker arms is cut by the bridge's own `stop_llm`, a call already on the socket
/// ends, and the backend is built with the cut (a bare `ApiLLM` would sit for its 300 s timeout).
#[test]
fn the_backend_the_worker_arms_is_cut_by_the_bridges_stop_with_its_call_on_the_socket() {
    let (url, connections) = silent_server();
    let dir = tempfile::tempdir().unwrap();
    let (bridge, _told) = bridge(&dir);
    let armed = armed_reload(&bridge.llm_switch_for_test(), &reload(&url, bridge.llm_switch_for_test().epoch()))
        .expect("armed");
    let during = armed.clone();
    let (done, result) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(during.chat(&[], &GenerationConfig::default(), None));
    });
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(connections.load(Ordering::SeqCst), 1, "the call is on the socket");

    bridge.stop_llm();
    let ended = result.recv_timeout(Duration::from_secs(2)).expect("the call ended within two seconds of the press");
    assert!(ended.expect_err("cut").downcast_ref::<Stopped>().is_some());
    assert!(armed.chat(&[], &GenerationConfig::default(), None).unwrap_err().downcast_ref::<Stopped>().is_some());
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(connections.load(Ordering::SeqCst), 1, "nothing was sent after the press");
}
