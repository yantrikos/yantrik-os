//! Changes to the machine reach the service one at a time, in the order they were asked for
//! (`lane.rs`). The jobs here run only when a test says, so "never both in flight" is shown
//! the way it would fail: by how many jobs are waiting while the first has not landed.

use serde_json::{json, Value};

use crate::backend::fake::FakeNetwork;
use crate::lane::MAX_WAITING;
use crate::tests::{on_wifi, read, run_until_quiet};

/// The calls that change the machine, in the order the service got them.
fn changes(net: &FakeNetwork) -> Vec<(String, Value)> {
    let changing = ["network.wifi_radio", "network.wifi_disconnect", "network.wifi_connect", "network.wifi_forget"];
    net.asked().into_iter().filter(|(m, _)| changing.contains(&m.as_str())).collect()
}

fn joined_nothing(radio: &str) -> Value {
    json!({ "adapter_present": true, "radio": radio })
}

#[test]
fn two_radio_flips_reach_the_service_one_after_the_other() {
    let net = on_wifi().answering("network.wifi_radio", joined_nothing("on"));
    let app = read(&net);
    app.ui(|ui| ui.invoke_wifi_radio_set(false));
    app.ui(|ui| ui.invoke_wifi_radio_set(true));
    assert_eq!(app.jobs_waiting(), 1, "the second waits; it is not in flight beside the first");
    app.run_jobs();
    assert_eq!(net.asked_for("network.wifi_radio"), vec![json!({ "enabled": false })]);
    assert_eq!(app.jobs_waiting(), 1, "the second starts once the first has landed");
    app.run_jobs();
    assert_eq!(
        net.asked_for("network.wifi_radio"),
        vec![json!({ "enabled": false }), json!({ "enabled": true })],
        "off, then on: the person's last word is the one the machine ends with"
    );
    assert_eq!(app.jobs_waiting(), 0);
    assert_eq!(app.model(|m| m.lane.in_flight()), None);
}

#[test]
fn only_the_last_radio_state_waits_behind_the_one_in_flight() {
    let net = on_wifi().answering("network.wifi_radio", joined_nothing("on"));
    let app = read(&net);
    for enabled in [false, true, false, true] {
        app.ui(move |ui| ui.invoke_wifi_radio_set(enabled));
    }
    assert_eq!(app.model(|m| m.lane.waiting()), 1);
    run_until_quiet(&app);
    assert_eq!(
        net.asked_for("network.wifi_radio"),
        vec![json!({ "enabled": false }), json!({ "enabled": true })],
        "the flips in between are not played back one by one"
    );
}

#[test]
fn a_disconnect_then_a_connect_run_in_the_order_they_were_pressed() {
    let net = on_wifi()
        .answering("network.wifi_disconnect", joined_nothing("on"))
        .answering("network.wifi_connect", json!({ "adapter_present": true, "radio": "on", "connected_ssid": "HomeNet" }));
    let app = read(&net);
    app.ui(|ui| ui.invoke_wifi_disconnect());
    app.ui(|ui| ui.invoke_wifi_connect("HomeNet".into(), "".into()));
    assert_eq!(app.jobs_waiting(), 1);
    app.run_jobs();
    assert_eq!(changes(&net), vec![("network.wifi_disconnect".to_string(), json!({}))]);
    run_until_quiet(&app);
    assert_eq!(
        changes(&net),
        vec![
            ("network.wifi_disconnect".to_string(), json!({})),
            ("network.wifi_connect".to_string(), json!({ "ssid": "HomeNet" })),
        ]
    );
    assert_eq!(app.ui(|ui| ui.get_wifi_connect_status().to_string()), "Connected to HomeNet");
}

#[test]
fn a_join_asked_for_while_a_change_is_in_flight_is_answered_and_runs_after_it() {
    let net = on_wifi().answering("network.wifi_disconnect", joined_nothing("on"));
    let app = read(&net);
    app.ui(|ui| ui.invoke_wifi_disconnect());
    // `wifi_connect` answers once accepted, so it can wait its turn.
    let reply = app.act("wifi_connect", json!({ "ssid": "HomeNet" })).unwrap().settle().unwrap();
    assert_eq!(reply, crate::connect::joining_answer("HomeNet"));
    assert_eq!(app.jobs_waiting(), 1);
    run_until_quiet(&app);
    let order: Vec<String> = changes(&net).into_iter().map(|(m, _)| m).collect();
    assert_eq!(order, ["network.wifi_disconnect", "network.wifi_connect"]);
}

#[test]
fn a_panicked_change_releases_the_lane() {
    let net = on_wifi()
        .panicking("network.wifi_disconnect")
        .answering("network.wifi_radio", joined_nothing("off"));
    let app = read(&net);
    app.ui(|ui| ui.invoke_wifi_disconnect());
    app.ui(|ui| ui.invoke_wifi_radio_set(false));
    app.run_jobs();
    assert_eq!(app.ui(|ui| ui.get_notice().to_string()), "The disconnect did not finish; try again.");
    assert_eq!(app.jobs_waiting(), 1, "the change behind it starts from the failed command");
    app.run_jobs();
    assert_eq!(net.asked_for("network.wifi_radio"), vec![json!({ "enabled": false })]);
    assert_eq!(app.model(|m| m.lane.in_flight()), None);
}

#[test]
fn an_intent_that_waits_on_its_change_is_refused_while_another_is_in_flight() {
    let net = on_wifi()
        .answering("network.wifi_disconnect", joined_nothing("on"))
        .answering("network.wifi_radio", joined_nothing("off"));
    let app = read(&net);
    app.ui(|ui| ui.invoke_wifi_disconnect());
    let busy = "The disconnect is still being made, so nothing was changed; ask again once it has finished.";
    for (name, args) in [
        ("wifi_radio", json!({ "state": "off" })),
        ("wifi_disconnect", json!({})),
        ("wifi_forget", json!({ "ssid": "HomeNet" })),
    ] {
        assert_eq!(app.act(name, args).unwrap().settle().unwrap_err(), busy, "{name}");
    }
    assert_eq!(app.ui(|ui| ui.get_notice().to_string()), busy, "said on screen too");
    assert_eq!(app.jobs_waiting(), 1, "nothing was queued for them");
    run_until_quiet(&app);
    assert_eq!(changes(&net).len(), 1, "refused means not made, now or later");
    // Once it has landed, the same intent goes through and is answered by its own change.
    let pending = app.act("wifi_radio", json!({ "state": "off" })).unwrap();
    app.run_jobs();
    assert_eq!(pending.settle().unwrap(), json!({ "radio": "off", "connected_ssid": null }));
}

#[test]
fn an_intent_s_change_in_flight_gets_its_own_answer_and_a_button_behind_it_then_runs() {
    let net = on_wifi()
        .answering("network.wifi_disconnect", joined_nothing("on"))
        .answering("network.wifi_radio", joined_nothing("off"));
    let app = read(&net);
    let pending = app.act("wifi_disconnect", json!({})).unwrap();
    app.ui(|ui| ui.invoke_wifi_radio_set(false));
    assert_eq!(app.jobs_waiting(), 1);
    app.run_jobs();
    assert_eq!(
        pending.settle().unwrap(),
        json!({ "disconnected": true, "connected_ssid": null, "radio": "on" }),
        "the disconnect's own answer, read before the radio change ran"
    );
    assert_eq!(app.settlements_open(), 0, "the button's change is owed to nobody");
    assert_eq!(app.jobs_waiting(), 1, "the radio change started once the disconnect landed");
    app.run_jobs();
    assert_eq!(net.asked_for("network.wifi_radio"), vec![json!({ "enabled": false })]);
}

#[test]
fn a_full_lane_says_so_and_a_typed_password_it_could_not_take_goes_nowhere() {
    let net = on_wifi()
        .answering("network.wifi_disconnect", joined_nothing("on"))
        .answering("network.wifi_forget", json!({ "forgotten": "HomeNet", "known": [] }));
    let app = read(&net);
    app.ui(|ui| ui.invoke_wifi_disconnect());
    for _ in 0..MAX_WAITING {
        app.ui(|ui| ui.invoke_wifi_forget("HomeNet".into()));
    }
    let full = format!(
        "{MAX_WAITING} changes are already waiting, so this one was not made; try again once they have finished."
    );
    app.ui(|ui| ui.invoke_wifi_forget("HomeNet".into()));
    assert_eq!(app.ui(|ui| ui.get_notice().to_string()), full);
    app.ui(|ui| {
        ui.set_wifi_password_input("hunter2-the-real-one".into());
        ui.invoke_wifi_connect("Cafe".into(), "hunter2-the-real-one".into());
    });
    assert_eq!(app.ui(|ui| ui.get_wifi_connect_status().to_string()), full);
    assert_eq!(app.ui(|ui| ui.get_wifi_password_input().to_string()), "", "the box is cleared");
    assert_eq!(app.model(|m| m.lane.waiting()), MAX_WAITING);
    run_until_quiet(&app);
    assert_eq!(net.asked_for("network.wifi_forget").len(), MAX_WAITING);
    assert!(net.asked_for("network.wifi_connect").is_empty());
}
