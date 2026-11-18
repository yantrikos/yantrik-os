//! Network Manager's intents through the real dispatch — the published contract, each intent's
//! answer and refusal, an agent in ask mode — and what happens when a job goes wrong.

use std::time::Duration;

use serde_json::{json, Value};
use yantrik_testing::Call;

use crate::backend::fake::FakeNetwork;
use crate::tests::{no_radio, on_wifi, read, start, state, summary};

// ── the intents ─────────────────────────────────────────────────────

/// The published contract, as `describe` lists it: the names, grades, settling and parameters
/// the app always had (#48: the two that delete or hand the machine over ask first; the two
/// that cut the channel are dangerous; reading is the floor).
#[test]
fn the_six_actions_are_published_as_they_were() {
    let app = start(&FakeNetwork::wired());
    let actions = app.describe()["actions"].as_array().cloned().unwrap_or_default();
    let find = |name: &str| actions.iter().find(|a| a["name"] == json!(name)).cloned().unwrap_or_else(|| panic!("no {name}"));
    let names: Vec<String> = actions.iter().filter_map(|a| a["name"].as_str().map(String::from)).collect();
    let own: Vec<&str> = names.iter().map(String::as_str).filter(|n| !["launch", "present"].contains(n)).collect();
    assert_eq!(own, ["refresh", "wifi_scan", "wifi_connect", "wifi_disconnect", "wifi_forget", "wifi_radio"]);
    for (name, grade, settles, params) in [
        ("refresh", "standard", "on return", json!([])),
        ("wifi_scan", "standard", "later", json!([])),
        ("wifi_connect", "sensitive", "later", json!(["ssid"])),
        ("wifi_disconnect", "dangerous", "on return", json!([])),
        ("wifi_forget", "sensitive", "on return", json!(["ssid"])),
        ("wifi_radio", "dangerous", "on return", json!(["state"])),
    ] {
        let a = find(name);
        assert_eq!(a["permission"], json!(grade), "{name}");
        assert_eq!(a["settles"], json!(settles), "{name}");
        assert_eq!(a["parameters"]["required"], params, "{name}");
    }
    assert!(find("wifi_disconnect")["description"].as_str().unwrap().contains("cannot be undone remotely"));
}

#[test]
fn refresh_answers_once_the_machine_has_been_read() {
    let net = FakeNetwork::wired();
    let app = read(&net);
    let pending = app.act("refresh", json!({})).unwrap();
    assert!(pending.is_owed());
    app.run_jobs();
    assert_eq!(
        pending.settle().unwrap(),
        json!({
            "connected": true, "type": "Ethernet", "wifi_adapter_present": true,
            "firewall_state": "absent", "service_answered": true,
        })
    );
}

#[test]
fn the_radio_intent_takes_on_or_off_and_names_anything_else() {
    let net = on_wifi().answering("network.wifi_radio", json!({ "adapter_present": true, "radio": "off" }));
    let app = read(&net);
    let e = app.act("wifi_radio", json!({ "state": "maybe" })).unwrap_err();
    assert!(e.contains("state must be \"on\" or \"off\"; got \"maybe\""), "{e}");
    assert!(net.asked_for("network.wifi_radio").is_empty(), "nothing is guessed at");
    let pending = app.act("wifi_radio", json!({ "state": " OFF " })).unwrap();
    app.run_jobs();
    assert_eq!(pending.settle().unwrap(), json!({ "radio": "off", "connected_ssid": null }));
    assert_eq!(net.asked_for("network.wifi_radio"), vec![json!({ "enabled": false })]);
    // The service's refusal is the intent's, and the screen's.
    net.set("network.wifi_radio", Err(crate::backend::RpcFailure { code: -32001, message: "polkit said no".into() }));
    let pending = app.act("wifi_radio", json!({ "state": "on" })).unwrap();
    app.run_jobs();
    assert_eq!(pending.settle().unwrap_err(), "network.wifi_radio: polkit said no");
    assert_eq!(app.ui(|ui| ui.get_notice().to_string()), "network.wifi_radio: polkit said no");
}

#[test]
fn disconnect_and_forget_answer_with_what_the_service_observed() {
    let net = on_wifi()
        .answering("network.wifi_disconnect", json!({ "adapter_present": true, "radio": "on" }))
        .answering("network.wifi_forget", json!({ "forgotten": "Old", "known": [{ "ssid": "HomeNet", "uuid": "u", "is_active": true }] }));
    let app = read(&net);
    let pending = app.act("wifi_disconnect", json!({})).unwrap();
    app.run_jobs();
    assert_eq!(pending.settle().unwrap(), json!({ "disconnected": true, "connected_ssid": null, "radio": "on" }));
    let e = app.act("wifi_forget", json!({ "ssid": "  " })).unwrap_err();
    assert!(e.contains("a network name is needed to forget one"), "{e}");
    let pending = app.act("wifi_forget", json!({ "ssid": " Old " })).unwrap();
    app.run_jobs();
    assert_eq!(pending.settle().unwrap(), json!({ "forgotten": "Old", "known": ["HomeNet"] }));
    assert_eq!(net.asked_for("network.wifi_forget"), vec![json!({ "ssid": "Old" })]);
}

#[test]
fn scan_and_connect_are_refused_on_a_machine_with_no_radio_and_say_so_on_screen() {
    let net = no_radio();
    let app = read(&net);
    for (name, args) in [("wifi_scan", json!({})), ("wifi_connect", json!({ "ssid": "HomeNet" }))] {
        let e = app.act(name, args).unwrap().settle().unwrap_err();
        assert_eq!(e, "this machine has no Wi-Fi adapter", "{name}");
        assert_eq!(app.ui(|ui| ui.get_notice().to_string()), "this machine has no Wi-Fi adapter");
    }
    assert!(net.asked_for("network.wifi_scan").is_empty());
    assert!(net.asked_for("network.wifi_connect").is_empty());
    // The button is not refused: it asks, and the service's own answer is what is said.
    app.ui(|ui| ui.invoke_wifi_scan());
    app.run_jobs();
    assert_eq!(net.asked_for("network.wifi_scan").len(), 1);
}

#[test]
fn the_scan_intent_answers_at_once_and_the_list_arrives_in_describe() {
    let net = on_wifi().answering(
        "network.wifi_scan",
        json!([{ "ssid": "Neighbour", "hidden": false, "signal": 44, "security": "WPA2", "rate": "130 Mbit/s", "is_connected": false, "is_saved": false }]),
    );
    let app = read(&net);
    let pending = app.act("wifi_scan", json!({})).unwrap();
    assert_eq!(pending.envelope()["settled"], json!(false), "it only started the sweep");
    assert_eq!(
        pending.settle().unwrap(),
        json!({ "scanning": true, "settles": "wifi.in_range and notice in describe" })
    );
    assert!(app.ui(|ui| ui.get_wifi_scanning()));
    app.run_jobs();
    assert_eq!(state(&app)["wifi"]["in_range"][0]["ssid"], json!("Neighbour"));
}

#[test]
fn connect_joins_a_saved_network_and_asks_the_person_for_any_other() {
    let net = on_wifi().answering("network.wifi_connect", json!({ "adapter_present": true, "radio": "on", "connected_ssid": "HomeNet" }));
    let app = read(&net);
    let e = app.act("wifi_connect", json!({ "ssid": " " })).unwrap_err();
    assert!(e.contains("a network name is needed to connect"), "{e}");
    // Saved: joined with no secret, and answered at once.
    let reply = app.act("wifi_connect", json!({ "ssid": "HomeNet" })).unwrap().settle().unwrap();
    assert_eq!(reply, crate::connect::joining_answer("HomeNet"));
    assert_eq!(app.ui(|ui| ui.get_wifi_connect_status().to_string()), "Connecting to HomeNet\u{2026}");
    assert!(!app.ui(|ui| ui.get_wifi_password_visible()), "a saved network needs no prompt");
    app.run_jobs();
    assert_eq!(net.asked_for("network.wifi_connect"), vec![json!({ "ssid": "HomeNet" })]);
    // Not saved: the window's own prompt goes up, empty, and nothing is sent until the person types.
    app.ui(|ui| ui.set_wifi_password_input("left over".into()));
    let reply = app.act("wifi_connect", json!({ "ssid": "Cafe: Free" })).unwrap().settle().unwrap();
    assert_eq!(reply, crate::connect::waiting_answer("Cafe: Free"));
    assert!(app.ui(|ui| ui.get_wifi_password_visible()));
    assert_eq!(app.ui(|ui| ui.get_wifi_password_ssid().to_string()), "Cafe: Free");
    assert_eq!(app.ui(|ui| ui.get_wifi_password_input().to_string()), "");
    assert_eq!(app.ui(|ui| ui.get_wifi_connect_status().to_string()), "");
    app.run_jobs();
    assert_eq!(net.asked_for("network.wifi_connect").len(), 1);
    // A redraw of everything does not put the prompt back once the person has closed it.
    app.ui(|ui| ui.set_wifi_password_visible(false));
    app.advance(crate::REFRESH_EVERY);
    app.run_jobs();
    assert!(!app.ui(|ui| ui.get_wifi_password_visible()));
}

#[test]
fn an_agent_in_ask_mode_is_refused_what_changes_the_machine() {
    let net = on_wifi();
    let app = read(&net);
    let agent = |name: &str, args: Value| Call::new(name, args).as_agent("mind").in_mode("ask");
    for (name, args) in [
        ("wifi_forget", json!({ "ssid": "HomeNet" })),
        ("wifi_connect", json!({ "ssid": "HomeNet" })),
        ("wifi_disconnect", json!({})),
        ("wifi_radio", json!({ "state": "off" })),
    ] {
        assert!(app.act_as(agent(name, args)).is_err(), "{name} needs a person's word in ask mode");
    }
    // Nothing was started, and running what is queued reaches nothing: without the run, a change
    // that had been accepted would still be sitting unrun, and the asserts below could not fail.
    assert_eq!(app.jobs_waiting(), 0, "no change was started");
    app.run_jobs();
    assert!(net.asked_for("network.wifi_forget").is_empty());
    assert!(net.asked_for("network.wifi_connect").is_empty());
    assert!(net.asked_for("network.wifi_disconnect").is_empty());
    assert!(net.asked_for("network.wifi_radio").is_empty());
    // Reading, and looking, are not.
    assert!(app.act_as(agent("refresh", json!({}))).is_ok());
    assert!(app.act_as(agent("wifi_scan", json!({}))).is_ok());
}

// ── what goes wrong in a job ────────────────────────────────────────

#[test]
fn a_change_whose_worker_panics_is_said_and_the_app_keeps_reading() {
    let net = on_wifi().panicking("network.wifi_disconnect");
    let app = read(&net);
    let pending = app.act("wifi_disconnect", json!({})).unwrap();
    app.run_jobs();
    let e = pending.settle().unwrap_err();
    assert!(e.starts_with("the work failed"), "{e}");
    assert_eq!(app.ui(|ui| ui.get_notice().to_string()), "The disconnect did not finish; try again.");
    // A scan whose worker panics is not left scanning.
    let net = on_wifi().panicking("network.wifi_scan");
    let app = read(&net);
    app.ui(|ui| ui.invoke_wifi_scan());
    app.run_jobs();
    assert!(!app.ui(|ui| ui.get_wifi_scanning()));
    app.advance(crate::REFRESH_EVERY);
    assert_eq!(app.jobs_waiting(), 1, "the next refresh still starts");
}

#[test]
fn a_refresh_whose_worker_panics_does_not_stop_the_next_one() {
    let net = FakeNetwork::wired().panicking("network.dns");
    let app = start(&net);
    app.run_jobs();
    assert_eq!(summary(&app), "Network — not read yet");
    assert_eq!(app.model(|m| m.refreshing), 0);
    app.advance(crate::REFRESH_EVERY);
    assert_eq!(app.jobs_waiting(), 1);
}

#[test]
fn a_shutdown_with_a_change_in_flight_answers_it_and_does_not_hang() {
    let app = read(&on_wifi());
    let owed = app.act("wifi_disconnect", json!({})).unwrap();
    assert!(owed.is_owed());
    let began = std::time::Instant::now();
    let _ = app.shutdown();
    assert!(began.elapsed() < Duration::from_secs(2));
    assert_eq!(owed.settle().unwrap_err(), "the app is closing");
}

#[test]
fn the_app_keeps_no_settings_file() {
    let app = read(&FakeNetwork::wired());
    app.run_writes();
    let _ = app.shutdown();
    assert!(!app.paths().config_file().exists(), "Network has never written a file of its own");
}
