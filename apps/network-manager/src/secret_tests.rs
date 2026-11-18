//! The road a Wi-Fi password travels (#178): a prompt the person types into, then the
//! service — and never an action argument.
//!
//! `wifi_connect` used to declare `password` as an optional parameter. An action's args are
//! what the approval card draws, what `record_unasked_action` writes into `mind-audit.jsonl`,
//! what a grant binds to and what the answer echoes, so the secret was handed to every
//! recording surface in the system before anything connected — and `yos check network`
//! failed on the parameter's name alone.

use crate::connect::*;
use yantrik_ipc_contracts::network::{KnownNetwork, WifiConnectParams, WifiState};

/// The same two lists `deploy/yantrik-os/yos` and
/// `crates/yantrik-ui/src/control_approvals.rs` carry, spelled out here so this file fails
/// if the app's published action would fail either of them on a live machine.
const SECRET_WORDS: [&str; 7] =
    ["passphrase", "password", "passwd", "pin", "secret", "credential", "unlock"];
const SECRET_PERMITTED: [&str; 1] = ["pinned"];

fn named_like_a_secret(name: &str) -> bool {
    let lower = name.to_lowercase();
    SECRET_WORDS.iter().any(|w| lower.contains(w)) && !SECRET_PERMITTED.contains(&lower.as_str())
}

#[test]
fn wifi_connect_declares_no_parameter_named_like_a_secret() {
    let action = wifi_connect_action();
    assert_eq!(action.name, "wifi_connect");
    // The only argument is the name. Written out: a `password` slipping back in beside
    // `ssid`, under any of the seven words, is the regression this file exists for.
    let names: Vec<&str> = action.params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["ssid"], "{names:?}");
    for p in &action.params {
        assert!(!named_like_a_secret(&p.name), "`{}` is named like a secret", p.name);
    }
    // The JSON Schema a model gets handed says the same thing, and its property keys are
    // exactly what `yos check` reads when it fails a surface on a secret-named parameter.
    let schema = action.schema();
    for key in schema["parameters"]["properties"].as_object().unwrap().keys() {
        assert!(!named_like_a_secret(key), "`{key}` is named like a secret");
    }
    assert_eq!(schema["parameters"]["required"], serde_json::json!(["ssid"]));
    // Still the grade and the settling the connect needs: an association waits on a
    // handshake and on DHCP, and joining a network is a deliberate decision.
    assert_eq!(schema["permission"], serde_json::json!("sensitive"));
    assert_eq!(schema["settles"], serde_json::json!("later"));
}

#[test]
fn the_card_the_audit_line_and_both_answers_of_a_connect_carry_no_password() {
    let secret = "hunter2-the-real-one";
    let action = wifi_connect_action();
    // A connect call, shaped the way the shell records one: the published schema beside
    // the args of the call, which is what an approval card draws and what
    // `record_unasked_action` writes into `mind-audit.jsonl`.
    let record = serde_json::json!({
        "app": "network",
        "action": action.schema(),
        "args": { "ssid": "Cafe: Free" },
        "answer_joining": joining_answer("Cafe: Free"),
        "answer_waiting": waiting_answer("Cafe: Free"),
    })
    .to_string();
    assert!(!record.contains(secret), "{record}");
    assert!(!record.contains("hunter2"), "{record}");
    // Neither answer the action can give even has a field a secret could sit in: one names
    // the join, the other names the wait and the prompt. Both must stay readable in a log.
    for answer in [joining_answer("Cafe: Free"), waiting_answer("Cafe: Free")] {
        assert_eq!(answer.get("password"), None, "{answer}");
        assert_eq!(answer.get("secret"), None, "{answer}");
    }
    assert_eq!(waiting_answer("Cafe: Free")["waiting_on_person"], serde_json::json!("Cafe: Free"));
}

#[test]
fn what_the_person_types_reaches_the_backend_and_nothing_readable_carries_it() {
    use std::sync::Mutex;

    struct Recorder(Mutex<Vec<WifiConnectParams>>);

    impl Backend for Recorder {
        fn wifi_connect(&self, request: &WifiConnectParams) -> Result<WifiState, String> {
            self.0.lock().unwrap().push(request.clone());
            Ok(WifiState {
                connected_ssid: Some(request.ssid.clone()),
                ..WifiState::default()
            })
        }
    }

    let recorder = Recorder(Mutex::new(Vec::new()));
    // The dialog's Connect button and the action's saved-network path both hand `submit`
    // what the prompt holds; this is the typed-password one.
    let state = submit(&recorder, " Cafe: Free ", "hunter2-the-real-one").unwrap();
    let seen = recorder.0.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].ssid, "Cafe: Free");
    assert_eq!(
        seen[0].password.as_deref(),
        Some("hunter2-the-real-one"),
        "the typed secret reaches the backend in the one field built for it"
    );
    // The caller reads back the join, not the secret — and a trace printing the request or
    // the state finds nothing either, which is what `WifiConnectParams`' hand-written
    // `Debug` is for.
    assert_eq!(state.connected_ssid.as_deref(), Some("Cafe: Free"));
    let printed = format!("{state:?} {:?}", seen[0]);
    assert!(!printed.contains("hunter2"), "{printed}");
    drop(seen);

    // An empty box is not a password: the open-network path sends no secret field at all,
    // so nothing downstream can mistake "" for one.
    submit(&recorder, "OpenCafe", "").unwrap();
    let seen = recorder.0.lock().unwrap();
    assert_eq!(seen[1].password, None);
    let wire = serde_json::to_string(&seen[1]).unwrap();
    assert!(!wire.contains("password"), "{wire}");
}

#[test]
fn a_saved_network_joins_and_any_other_asks_the_person() {
    // The rule the person's own click follows in `network_manager.slint`: a saved row
    // joins outright, any other raises the password dialog. The action takes the same
    // branch, so the mind cannot skip a prompt the person would have seen.
    let known = vec![KnownNetwork {
        ssid: "HomeNet".into(),
        uuid: "11111111-1111-1111-1111-111111111111".into(),
        is_active: false,
    }];
    assert_eq!(plan("HomeNet", &known), Plan::Join);
    assert_eq!(plan("Cafe: Free", &known), Plan::AskPerson);
    // A saved name is matched whole: a prefix of one is a different network.
    assert_eq!(plan("Home", &known), Plan::AskPerson);
    assert_eq!(plan("HomeNet2", &known), Plan::AskPerson);
    // A machine with nothing saved asks, whatever the name.
    assert_eq!(plan("HomeNet", &[]), Plan::AskPerson);
}
