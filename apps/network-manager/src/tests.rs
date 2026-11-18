//! Network Manager, driven headless through `yantrik-testing`: the window, the jobs and the
//! clock run when each test says, against a scripted network-service (`backend::fake`) and a
//! fake Mind. `describe` in each case, the refresh and the order readings land in, the buttons,
//! the rail; the intents are in `intent_tests.rs`, with these helpers.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use slint::Model as _;
use yantrik_app::mind::fake::FakeMind;
use yantrik_app::mind::Reach;
use yantrik_app::watch::Seen;
use yantrik_app::{App, Paths, Services};
use yantrik_testing::TestApp;

use crate::backend::fake::{self, FakeNetwork};
use crate::model::{Command, Taken};
use crate::reading::Reading;
use crate::NetworkManager;

pub(crate) fn start_with(net: &FakeNetwork, mind: FakeMind) -> (TestApp<NetworkManager>, Arc<FakeMind>) {
    let paths = Paths::temporary(NetworkManager::ID);
    fake::install(&paths, net);
    let mind = Arc::new(mind);
    let app = TestApp::<NetworkManager>::start_at_with(paths, Vec::new(), mind.clone(), Services::fakes());
    (app, mind)
}

pub(crate) fn start(net: &FakeNetwork) -> TestApp<NetworkManager> {
    start_with(net, FakeMind::new()).0
}

/// Started and read once.
pub(crate) fn read(net: &FakeNetwork) -> TestApp<NetworkManager> {
    let app = start(net);
    app.run_jobs();
    app
}

/// Runs jobs until none is left. A change that waited its turn starts only once the one before
/// it has landed, so one round does not run them all.
pub(crate) fn run_until_quiet(app: &TestApp<NetworkManager>) {
    const ROUNDS: usize = 16;
    for _ in 0..ROUNDS {
        if app.jobs_waiting() == 0 {
            return;
        }
        app.run_jobs();
    }
    panic!("jobs still waiting after {ROUNDS} rounds");
}

pub(crate) fn state(app: &TestApp<NetworkManager>) -> Value {
    app.describe()["state"].clone()
}

pub(crate) fn summary(app: &TestApp<NetworkManager>) -> String {
    app.describe()["summary"].as_str().unwrap_or_default().to_string()
}

/// A laptop on its radio: joined to HomeNet, the default route leaving by wlan0.
pub(crate) fn on_wifi() -> FakeNetwork {
    FakeNetwork::wired()
        .routing_by("wlan0")
        .answering("network.status", json!({ "connected": true, "type": "wifi", "ip_address": "192.168.1.34" }))
        .answering(
            "network.wifi_state",
            json!({
                "adapter_present": true, "device": "wlan0", "radio": "on", "connected_ssid": "HomeNet",
                "signal": 72, "rate": "270 Mbit/s", "ip_address": "192.168.1.34",
                "gateway": "192.168.1.1", "subnet": "255.255.255.0",
            }),
        )
}

/// A wired desktop with no Wi-Fi hardware.
pub(crate) fn no_radio() -> FakeNetwork {
    FakeNetwork::wired().routing_by("eth0").answering(
        "network.wifi_state",
        json!({ "adapter_present": false, "radio": "unknown", "reason": "this machine has no Wi-Fi adapter" }),
    )
}

// ── describe, in each case ──────────────────────────────────────────

#[test]
fn before_the_first_reading_nothing_is_claimed() {
    let app = start(&FakeNetwork::wired());
    assert_eq!(app.jobs_waiting(), 1, "the first refresh is a job, started at start");
    assert_eq!(summary(&app), "Network — not read yet");
    let s = state(&app);
    assert_eq!(s["connected"], json!(false));
    assert_eq!(s["source"]["answered"], json!(false));
    assert_eq!(s["source"]["read_at"], Value::Null);
    assert_eq!(s["wifi"]["radio"], json!("unknown"), "unknown, never off");
    assert_eq!(s["firewall"]["state"], json!("unknown"));
    assert_eq!(app.ui(|ui| ui.get_wifi_radio().to_string()), "unknown");
    assert_eq!(app.ui(|ui| ui.get_firewall_rule_count()), -1);
    assert!(app.model(|m| m.rail.context().is_empty()), "the rail says nothing about a machine nobody read");
}

#[test]
fn a_wired_machine_is_described_as_the_screen_shows_it() {
    let app = read(&FakeNetwork::wired().routing_by("eth0"));
    assert_eq!(summary(&app), "Network — online via Ethernet, 192.168.4.20");
    let s = state(&app);
    assert_eq!(s["connected"], json!(true));
    assert_eq!(s["type"], json!("Ethernet"));
    assert_eq!(s["ip_address"], json!("192.168.4.20"));
    assert_eq!(
        s["interfaces"],
        json!([{
            "name": "eth0", "type": "ethernet", "state": "UP", "ip": "192.168.4.20",
            "mac": "52:54:00:12:34:56", "speed": "1000 Mbit/s", "subnet": "255.255.255.0",
            "gateway": "192.168.4.1", "dns": "192.168.4.1",
        }])
    );
    assert_eq!(s["dns"], json!({ "nameservers": ["192.168.4.1"], "search_domains": ["lan"] }));
    assert_eq!(s["wifi"]["adapter_present"], json!(true));
    assert_eq!(s["wifi"]["device"], json!("wlan0"));
    assert_eq!(s["wifi"]["radio"], json!("on"));
    assert_eq!(s["wifi"]["connected_ssid"], Value::Null);
    assert_eq!(s["wifi"]["known"], json!([{ "ssid": "HomeNet", "active": false }]));
    assert_eq!(s["wifi"]["in_range"], json!([]));
    assert_eq!(s["firewall"]["kind"], Value::Null);
    assert_eq!(s["firewall"]["state"], json!("absent"));
    assert_eq!(s["firewall"]["rules"], Value::Null, "nobody counted, so not zero");
    assert_eq!(s["source"]["answered"], json!(true));
    assert!(s["source"]["read_at"].is_u64());
    assert_eq!(s["source"]["service"], json!("network-service"));
    assert_eq!(s["source"]["error"], Value::Null);
    assert_eq!(s["notice"], Value::Null);
    // The window draws the same reading.
    assert_eq!(app.ui(|ui| ui.get_status_state().to_string()), "connected");
    assert_eq!(app.ui(|ui| ui.get_status_connection_type().to_string()), "Ethernet");
    assert_eq!(app.ui(|ui| ui.get_status_signal_strength()), 0, "a cable has no signal bars");
    assert_eq!(app.ui(|ui| ui.get_wifi_dns().to_string()), "192.168.4.1");
    assert_eq!(app.ui(|ui| ui.get_ethernet_interfaces().row_data(0).unwrap().speed.to_string()), "1000 Mbit/s");
    assert_eq!(app.ui(|ui| ui.get_wifi_saved_networks().row_count()), 1);
    assert_eq!(app.ui(|ui| ui.get_firewall_state().to_string()), "absent");
    assert_eq!(app.ui(|ui| ui.get_notice().to_string()), "");
    // It opened on the section the traffic leaves by, once.
    assert_eq!(app.ui(|ui| ui.get_active_tab()), crate::opening::ETHERNET);
    app.ui(|ui| ui.set_active_tab(2));
    app.advance(crate::REFRESH_EVERY);
    app.run_jobs();
    assert_eq!(app.ui(|ui| ui.get_active_tab()), 2, "afterwards the section is the person's");
}

#[test]
fn a_machine_on_its_radio_is_online_via_the_network_it_joined() {
    let app = read(&on_wifi());
    assert_eq!(summary(&app), "Network — online via HomeNet, 192.168.1.34");
    assert_eq!(state(&app)["wifi"]["signal"], json!(72));
    assert_eq!(app.ui(|ui| ui.get_status_signal_strength()), 72);
    assert_eq!(app.ui(|ui| ui.get_wifi_current_ssid().to_string()), "HomeNet");
    assert_eq!(app.ui(|ui| ui.get_wifi_speed().to_string()), "270 Mbit/s");
    assert_eq!(app.ui(|ui| ui.get_active_tab()), crate::opening::WIFI);
}

#[test]
fn an_unconnected_machine_with_nothing_wired_is_offline() {
    let net = FakeNetwork::wired()
        .answering("network.interfaces", json!([]))
        .answering("network.status", json!({ "connected": false, "type": "none" }));
    let app = read(&net);
    assert_eq!(summary(&app), "Network — offline");
    assert_eq!(state(&app)["interfaces"], json!([]));
    assert_eq!(state(&app)["type"], Value::Null, "none is no type, not an empty one");
    assert_eq!(app.ui(|ui| ui.get_active_tab()), crate::opening::WIFI);
}

#[test]
fn a_service_that_does_not_answer_is_said_and_not_hidden() {
    let app = read(&FakeNetwork::unreachable());
    let unreachable = "Service at /run/user/1000/network.sock is unreachable (retrying in up to 5s)";
    assert_eq!(
        summary(&app),
        format!("Network — the network service did not answer: network.interfaces: {unreachable}"),
        "the interfaces' failure leads, as it always has"
    );
    let s = state(&app);
    assert_eq!(s["source"]["answered"], json!(false));
    assert_eq!(s["wifi"]["adapter_present"], json!(false));
    assert_eq!(s["wifi"]["radio"], json!("unknown"));
    assert_eq!(s["wifi"]["reason"], json!(format!("network.wifi_state: {unreachable}")));
    assert_eq!(s["firewall"]["state"], json!("unknown"));
    assert_eq!(s["firewall"]["reason"], json!(format!("network.firewall: {unreachable}")));
    let notice = format!("the network service did not answer: network.interfaces: {unreachable}");
    assert_eq!(s["notice"], json!(notice));
    assert_eq!(app.ui(|ui| ui.get_notice().to_string()), notice);
    assert_eq!(app.ui(|ui| ui.get_status_state().to_string()), "disconnected");
    assert_eq!(app.ui(|ui| ui.get_ethernet_interfaces().row_count()), 0);
}

#[test]
fn an_answer_in_a_shape_this_app_does_not_read_is_an_error_and_not_an_empty_list() {
    let net = FakeNetwork::wired().answering("network.interfaces", json!([{ "name": "eth0", "state": 17 }]));
    let app = read(&net);
    let error = state(&app)["source"]["error"].as_str().unwrap_or_default().to_string();
    assert!(error.starts_with("network.interfaces answered in a shape this app does not understand"), "{error}");
}

// ── refreshing, and the order readings land in ──────────────────────

#[test]
fn the_machine_is_read_every_three_seconds_but_never_twice_at_once() {
    let net = FakeNetwork::wired();
    let app = read(&net);
    assert_eq!(net.asked_for("network.dns").len(), 1);
    app.advance(crate::REFRESH_EVERY);
    assert_eq!(app.jobs_waiting(), 1);
    // A slow service: the next tick finds the read still in flight and starts no second one.
    app.advance(crate::REFRESH_EVERY);
    assert_eq!(app.jobs_waiting(), 1);
    app.run_jobs();
    assert_eq!(net.asked_for("network.dns").len(), 2);
    // A cable pulled out shows at the next tick.
    net.set("network.status", Ok(json!({ "connected": false, "type": "none" })));
    app.advance(crate::REFRESH_EVERY);
    app.run_jobs();
    assert_eq!(summary(&app), "Network — offline");
}

#[test]
fn a_reading_older_than_the_one_on_screen_is_dropped() {
    let app = read(&on_wifi());
    let shown = app.model(|m| m.shown);
    assert!(shown >= 1);
    // A refresh that started before the one on screen and landed after it.
    let older = Reading::default();
    app.run(Command::Read(Box::new(Taken { seq: shown - 1, reading: older.clone() })));
    assert_eq!(summary(&app), "Network — online via HomeNet, 192.168.1.34", "dropped");
    // A change's reading is held to the same rule; its own verdict still lands.
    app.run(Command::Changed(
        Box::new(crate::model::Outcome::Disconnect(Err("nmcli: not now".into()))),
        Box::new(Taken { seq: shown, reading: older }),
    ));
    assert_eq!(summary(&app), "Network — online via HomeNet, 192.168.1.34");
    assert_eq!(app.ui(|ui| ui.get_notice().to_string()), "nmcli: not now");
}

#[test]
fn a_command_s_verdict_is_said_over_the_reading_until_the_next_one() {
    let net = on_wifi().failing("network.wifi_radio", -32001, "Not authorized to enable/disable WiFi device");
    let app = read(&net);
    app.ui(|ui| ui.invoke_wifi_radio_set(false));
    app.run_jobs();
    let said = "network.wifi_radio: Not authorized to enable/disable WiFi device";
    assert_eq!(app.ui(|ui| ui.get_notice().to_string()), said);
    assert_eq!(state(&app)["notice"], json!(said));
    // The next refresh says the state of the machine again, which here is nothing wrong.
    app.advance(crate::REFRESH_EVERY);
    app.run_jobs();
    assert_eq!(app.ui(|ui| ui.get_notice().to_string()), "");
}

// ── the buttons ─────────────────────────────────────────────────────

#[test]
fn every_button_sends_what_it_says_and_reads_the_machine_after() {
    let net = on_wifi();
    let app = read(&net);
    app.ui(|ui| ui.invoke_wifi_radio_set(true));
    app.ui(|ui| ui.invoke_wifi_disconnect());
    app.ui(|ui| ui.invoke_wifi_forget("HomeNet".into()));
    net.set(
        "network.wifi_forget",
        Ok(json!({ "forgotten": "HomeNet", "known": [] })),
    );
    net.set("network.wifi_radio", Ok(json!({ "adapter_present": true, "radio": "on" })));
    net.set("network.wifi_disconnect", Ok(json!({ "adapter_present": true, "radio": "on" })));
    let before = net.asked_for("network.dns").len();
    // One at a time (`lane.rs`): each starts once the one before it has landed.
    run_until_quiet(&app);
    assert_eq!(net.asked_for("network.wifi_radio"), vec![json!({ "enabled": true })], "the state asked for, not a flip");
    assert_eq!(net.asked_for("network.wifi_disconnect"), vec![json!({})]);
    assert_eq!(net.asked_for("network.wifi_forget"), vec![json!({ "ssid": "HomeNet" })]);
    assert_eq!(net.asked_for("network.dns").len(), before + 3, "each change re-reads the machine");
    assert_eq!(app.ui(|ui| ui.get_notice().to_string()), "");
}

#[test]
fn the_scan_button_lists_what_is_in_range_and_a_hidden_network_is_said() {
    let net = on_wifi();
    let app = read(&net);
    // A mind swept seconds ago: the service refuses a second rescan, and the list it left is read.
    net.once("network.wifi_scan", Err(crate::backend::RpcFailure { code: crate::backend::RESCAN_TOO_SOON, message: "too soon".into() }));
    net.set(
        "network.wifi_scan",
        Ok(json!([
            { "ssid": "HomeNet", "hidden": false, "signal": 81, "security": "WPA2", "rate": "270 Mbit/s", "is_connected": true, "is_saved": true },
            { "ssid": "", "hidden": true, "signal": 55, "security": "WPA2", "rate": "54 Mbit/s", "is_connected": false, "is_saved": false },
        ])),
    );
    app.ui(|ui| ui.invoke_wifi_scan());
    assert!(app.ui(|ui| ui.get_wifi_scanning()));
    app.run_jobs();
    assert!(!app.ui(|ui| ui.get_wifi_scanning()));
    assert_eq!(
        net.asked_for("network.wifi_scan"),
        vec![json!({ "rescan": true }), json!({ "rescan": false })]
    );
    assert_eq!(app.ui(|ui| ui.get_wifi_networks().row_data(1).unwrap().ssid.to_string()), "(hidden network)");
    assert_eq!(
        state(&app)["wifi"]["in_range"][1],
        json!({ "ssid": "(hidden network)", "signal": 55, "security": "WPA2", "connected": false, "saved": false })
    );
    // A scan that fails clears the list rather than leave a stale one standing as its result.
    net.set("network.wifi_scan", Err(crate::backend::RpcFailure { code: -32000, message: "radio busy".into() }));
    app.ui(|ui| ui.invoke_wifi_scan());
    app.run_jobs();
    assert_eq!(app.ui(|ui| ui.get_wifi_networks().row_count()), 0);
    assert_eq!(app.ui(|ui| ui.get_notice().to_string()), "network.wifi_scan: radio busy");
}

#[test]
fn what_the_person_types_reaches_the_service_and_nothing_readable_carries_it() {
    let net = on_wifi().answering("network.wifi_connect", json!({ "adapter_present": true, "radio": "on", "connected_ssid": "Cafe: Free" }));
    let app = read(&net);
    app.ui(|ui| {
        ui.set_wifi_password_input("hunter2-the-real-one".into());
        ui.invoke_wifi_connect("Cafe: Free".into(), "hunter2-the-real-one".into());
        // As the dialog's Connect does.
        ui.set_wifi_password_visible(false);
    });
    assert_eq!(app.ui(|ui| ui.get_wifi_connect_status().to_string()), "Connecting to Cafe: Free\u{2026}");
    app.run_jobs();
    assert_eq!(
        net.asked_for("network.wifi_connect"),
        vec![json!({ "ssid": "Cafe: Free", "password": "hunter2-the-real-one" })]
    );
    assert_eq!(app.ui(|ui| ui.get_wifi_connect_status().to_string()), "Connected to Cafe: Free");
    assert_eq!(app.ui(|ui| ui.get_wifi_password_input().to_string()), "", "the box is cleared");
    assert!(!app.ui(|ui| ui.get_wifi_password_visible()));
    let described = app.describe().to_string();
    assert!(!described.contains("hunter2"), "{described}");
    // A saved row joins with no secret at all; a refusal stays in the prompt, the box cleared.
    net.set("network.wifi_connect", Err(crate::backend::RpcFailure { code: -32010, message: "the network refused the password".into() }));
    app.ui(|ui| ui.invoke_wifi_connect("HomeNet".into(), "".into()));
    app.run_jobs();
    assert_eq!(net.asked_for("network.wifi_connect")[1], json!({ "ssid": "HomeNet" }));
    assert_eq!(
        app.ui(|ui| ui.get_wifi_connect_status().to_string()),
        "network.wifi_connect: the network refused the password"
    );
}

// ── the rail and the AI panel ───────────────────────────────────────

#[test]
fn the_rail_says_what_was_read_and_both_explain_controls_ask_one_question() {
    let (app, mind) = start_with(
        &FakeNetwork::wired().routing_by("eth0"),
        FakeMind::new().answers([Ok("Wired, nothing to worry about.".into()), Ok("Still wired.".into())]),
    );
    mind.set_reach(Reach::Ready);
    app.seen(Seen::Reach(Reach::Ready));
    app.run_jobs();
    app.advance(Duration::from_millis(300));
    let labels = app.model(|m| m.rail.context().iter().map(|c| c.label.clone()).collect::<Vec<_>>());
    assert_eq!(
        labels,
        [
            "Ethernet — 192.168.4.20",
            "Wi-Fi radio on",
            "No supported firewall manager detected; active filtering rules not checked",
        ]
    );
    assert_eq!(app.model(|m| m.rail.suggestions()[0].id.clone()), "explain");
    // The rail's suggestion: a card to read, and the panel says the same.
    app.ui(|ui| ui.invoke_agent_suggestion_activated("explain".into()));
    assert!(app.ui(|ui| ui.get_ai_is_working()));
    assert_eq!(app.ui(|ui| ui.get_proposal().title.to_string()), "Reading this machine's network");
    app.run_jobs();
    let card = app.model(|m| m.rail.proposal().cloned().unwrap());
    assert_eq!((card.title.as_str(), card.verb.as_str()), ("This machine's network", "Close"));
    assert_eq!(card.source, "from what is on this screen");
    assert!(card.applies.is_none(), "an answer to read, nothing to apply");
    assert_eq!(app.ui(|ui| ui.get_ai_response().to_string()), "Wired, nothing to worry about.");
    assert!(!app.ui(|ui| ui.get_ai_is_working()));
    let prompt = mind.prompts()[0].clone();
    assert!(prompt.contains("Link: Ethernet 192.168.4.20\nWi-Fi: radio on, joined to nothing."), "{prompt}");
    assert!(prompt.ends_with("Resolvers: 192.168.4.1"), "{prompt}");
    // The header's Explain asks the same question.
    app.ui(|ui| ui.invoke_ai_explain_pressed());
    app.run_jobs();
    assert_eq!(mind.prompts()[1], prompt);
    assert_eq!(app.ui(|ui| ui.get_ai_response().to_string()), "Still wired.");
    app.ui(|ui| ui.invoke_ai_dismiss());
    assert_eq!(app.ui(|ui| ui.get_ai_response().to_string()), "");
    app.ui(|ui| ui.invoke_proposal_dismissed());
    assert_eq!(app.ui(|ui| ui.get_proposal().title.to_string()), "");
    // A rail row goes to the section it is about.
    app.ui(|ui| ui.invoke_agent_context_activated("firewall".into()));
    assert_eq!(app.ui(|ui| ui.get_active_tab()), 2);
    app.ui(|ui| ui.invoke_agent_context_activated("link".into()));
    assert_eq!(app.ui(|ui| ui.get_active_tab()), 1, "the link is a cable");
}

#[test]
fn with_no_companion_reachable_the_header_still_asks_and_says_what_came_back() {
    let (app, mind) = start_with(&FakeNetwork::wired(), FakeMind::new());
    app.run_jobs();
    app.ui(|ui| ui.invoke_ai_explain_pressed());
    app.run_jobs();
    assert_eq!(mind.prompts().len(), 1);
    assert_eq!(
        app.ui(|ui| ui.get_ai_response().to_string()),
        "The companion did not answer: the fake has no answer left"
    );
    assert!(!app.ui(|ui| ui.get_ai_is_working()));
    assert!(app.model(|m| m.rail.suggestions().is_empty()), "nothing that asks is offered");
}

#[test]
fn the_header_explain_before_the_first_reading_says_so_and_asks_nothing() {
    let (app, mind) = start_with(&FakeNetwork::wired(), FakeMind::new().answers([Ok("Too soon to say.".into())]));
    app.ui(|ui| ui.invoke_ai_explain_pressed());
    assert_eq!(
        app.ui(|ui| ui.get_ai_response().to_string()),
        "the network has not been read yet; try again in a moment"
    );
    assert!(!app.ui(|ui| ui.get_ai_is_working()));
    assert_eq!(app.jobs_waiting(), 1, "only the first refresh: no question was started");
    app.run_jobs();
    assert!(mind.prompts().is_empty(), "the companion was never handed an empty reading");
}

/// Nothing drawn does nothing: every callback the window declares is bound, by the app or by
/// the framework's rail.
#[test]
fn every_callback_the_window_declares_is_wired() {
    let window = include_str!("../ui/app.slint");
    let lib = include_str!("lib.rs");
    let by_rail = ["agent-suggestion-activated", "proposal-applied", "proposal-dismissed"];
    let mut unwired = Vec::new();
    for line in window.lines().map(str::trim).filter(|l| l.starts_with("callback ")) {
        let name = line["callback ".len()..].split(|c: char| c == ' ' || c == '(' || c == ';').next().unwrap_or_default();
        let binding = format!("on_{}(", name.replace('-', "_"));
        if !lib.contains(&binding) && !by_rail.contains(&name) {
            unwired.push(name.to_string());
        }
    }
    assert!(unwired.is_empty(), "callbacks nothing answers: {unwired:?}");
    assert!(include_str!("../ui/network_manager.slint").contains("context-opens: true"), "the rows are answered");
}
