//! Yantrik Network Manager — standalone app binary.
//!
//! What this app is for: to say truthfully how this machine is connected, and to change it when
//! asked. Everything else it used to draw — Bluetooth, VPN, diagnostics, firewall profiles — has
//! come off the screen, because behind all eighteen of those controls was a `tracing::info!` and
//! nothing else.
//!
//! # What was wrong
//!
//! **The app called five methods the service did not have.** `network.wifi_toggle`,
//! `network.wifi_scan`, `network.wifi_connect`, `network.wifi_disconnect`,
//! `network.wifi_forget`; the service answered `network.interfaces`, `network.status` and
//! `network.dns`. Not one name in common. Three of the five went into a `let _ =`, so every press
//! returned "Unknown method" into a discarded `Result` and the app looked like it had worked —
//! the radio toggle even logged `Toggle WiFi: false -> true` on the way past.
//!
//! **Two readings were fabricated.** `wifi-enabled` and `firewall-enabled` were
//! `in property <bool> …: false` that Rust never wrote, and the screen drew both as measurements.
//! A security audit of this OS recorded "Firewall: Off" as a finding; the app had never looked at
//! a firewall in its life. Thirty-four `in` properties on this window were never set from Rust,
//! the worst count in the fleet.
//!
//! **There was no control surface and nowhere to report a failure.**
//!
//! # How it is put together now
//!
//! One typed contract (`yantrik_ipc_contracts::network`) that both this app and
//! `services/network-service` build and parse, so a rename stops compiling rather than emptying a
//! list. One function per mutation, returning `Result`, called by both the button and the control
//! action, so a person and a mind cannot be told different things about the same command. Every
//! failure lands in the `notice` strip on screen and in `describe.notice`.
//!
//! Scanning and connecting run off the UI thread and their actions declare `defers`: a rescan
//! sweeps two bands and an association waits on DHCP, and the control surface gives an action
//! three seconds before telling the caller the app did not answer.

use std::sync::{Arc, Mutex};

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use yantrik_app_runtime::prelude::*;
use yantrik_ipc_contracts::network::{
    method, ConnectionType, DnsConfig, FirewallState, FirewallStatus, KnownNetwork,
    NetworkInterfaceInfo, NetworkStatus, RadioState, ScannedNetwork, WifiConnectParams,
    WifiForgetParams, WifiForgetResult, WifiRadioParams, WifiScanParams, WifiState,
};
use yantrik_ipc_transport::SyncRpcClient;

/// The windowless half of connecting: the plan, the published action, the request the typed
/// secret travels in. Pure, so `tests/network-core` can exercise it without a window.
mod connect;

slint::include_modules!();

/// What the last refresh learned, beside the window.
///
/// `describe` has to answer questions the Slint properties cannot: whether the service answered
/// at all, and whether a value is unknown or merely empty. A string property is `""` for both,
/// and `""` read as a measurement is the disease this app was found with.
#[derive(Default)]
struct Reading {
    /// Whether the last refresh reached network-service.
    service_ok: bool,
    /// What it said when it did not.
    service_error: Option<String>,
    /// Unix seconds of the last refresh that reached the service.
    read_at: Option<u64>,
    wifi: WifiState,
    firewall: FirewallState,
    known: Vec<KnownNetwork>,
    dns: DnsConfig,
}

/// `Arc<Mutex<_>>` and not `Rc<RefCell<_>>`, which is what the rest of the fleet uses.
///
/// Scanning and connecting run on a worker thread and hand their result back through
/// `slint::Weak::upgrade_in_event_loop`, which takes a `Send` closure — so the closure that
/// settles the result cannot hold an `Rc`. Everything in `Reading` is `Send`, and every lock
/// taken below is held for one statement.
type State = Arc<Mutex<Reading>>;

fn main() {
    init_tracing("yantrik-network-manager");

    // One window per app: a second launch defers to the running one (the shell focuses it).
    let Some(_instance) = instance::claim("network-manager") else { return };

    let app = NetworkManagerApp::new().unwrap();
    // The window's title bar is the app's own (#256).
    yantrik_app_runtime::window_chrome!(app);

    // Same dark/accent choice as the shell, read from the shell's settings file.
    let theme = theme::load();
    app.global::<ThemeMode>().set_dark(theme.dark);
    app.global::<AccentPreset>().set_index(theme.accent_index);

    // Nothing has been asked yet, so nothing is known. The first refresh below decides, and until
    // it has run the window says "unknown" rather than "off".
    let state: State = Arc::new(Mutex::new(Reading::default()));

    wire(&app, &state);

    // What is true now, before the window is shown.
    refresh(&app, &state);

    // Published after the first read, so a describe arriving immediately reports the machine
    // rather than a half-built window.
    publish_control(&app, &state);

    // And every few seconds after. A cable pulled out while the window is open should show,
    // and three seconds is the same cadence the shell polls the rest of the system at.
    let timer = slint::Timer::default();
    {
        let weak = app.as_weak();
        let state = state.clone();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_secs(3),
            move || {
                if let Some(ui) = weak.upgrade() {
                    refresh(&ui, &state);
                }
            },
        );
    }

    // The rail follows the app's state on its own timer, for the same reason it does in
    // Containers: hooking every path that changes the window is how a refresh gets missed.
    let rail_timer = slint::Timer::default();
    {
        let weak = app.as_weak();
        let state = state.clone();
        rail_timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_secs(4),
            move || {
                if let Some(ui) = weak.upgrade() {
                    refresh_agent_rail(&ui, &state);
                }
            },
        );
    }
    refresh_agent_rail(&app, &state);

    run_until_closed(&app, "yantrik-network-manager");
}

// ══════════════════════════════════════════════════════════════════════
// Talking to the service
// ══════════════════════════════════════════════════════════════════════

/// One call to network-service, with the answer parsed into the type the contract says it is.
///
/// The parse is the point. The old reader did
/// `serde_json::from_value(v).unwrap_or_default()` over the interface list, so a shape change on
/// the service side produced an empty ethernet pane and said nothing — the window reported "No
/// interface" on a machine with eth0 up. A mismatch is an error the person sees now. It cannot
/// happen while both ends build the same struct, and leaving the trap in place because it is
/// currently unreachable is how it becomes reachable again.
fn call<T: serde::de::DeserializeOwned>(
    method_name: &str,
    params: serde_json::Value,
) -> Result<T, String> {
    let client = SyncRpcClient::for_service("network");
    let value = client
        .call(method_name, params)
        .map_err(|e| format!("{method_name}: {}", e.message))?;
    serde_json::from_value(value).map_err(|e| {
        format!("{method_name} answered in a shape this app does not understand: {e}")
    })
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ══════════════════════════════════════════════════════════════════════
// Reading the network into the window
// ══════════════════════════════════════════════════════════════════════

/// Pull the current state out of the network service and put it on screen.
///
/// Every property this screen renders as a fact is written here, from something that was read.
/// The ones that cannot be read are written with the value that means unknown — `-1` for the
/// firewall rule count, `"unknown"` for the radio — never with a plausible default.
fn refresh(ui: &NetworkManagerApp, state: &State) {
    let mut reading = Reading::default();

    // Interfaces first: the ethernet list, and with it whether there is an interface at all.
    match call::<Vec<NetworkInterfaceInfo>>(method::INTERFACES, serde_json::json!({})) {
        Ok(rows) => {
            reading.service_ok = true;
            reading.read_at = Some(now_secs());
            let eth: Vec<EthernetInterface> = rows
                .iter()
                .filter(|i| matches!(i.conn_type, ConnectionType::Ethernet))
                .map(|i| EthernetInterface {
                    name: i.name.as_str().into(),
                    status: i.state.to_uppercase().into(),
                    ip_address: i.ip_address.clone().unwrap_or_default().into(),
                    mac_address: i.mac_address.as_str().into(),
                    // The service does not report link speed or per-interface DHCP, gateway and
                    // resolvers. Left empty rather than invented: an empty field reads as "not
                    // known", a made-up one reads as fact.
                    speed: "".into(),
                    is_dhcp: true,
                    subnet: "".into(),
                    gateway: "".into(),
                    dns: "".into(),
                })
                .collect();
            ui.set_ethernet_interfaces(ModelRc::new(VecModel::from(eth)));
        }
        Err(e) => {
            reading.service_error = Some(e);
            // The list is emptied deliberately and the notice below says why. Leaving the last
            // good list on screen would present a stale reading as a current one.
            ui.set_ethernet_interfaces(ModelRc::new(VecModel::<EthernetInterface>::default()));
        }
    }

    // The summary line the header shows.
    match call::<NetworkStatus>(method::STATUS, serde_json::json!({})) {
        Ok(status) => {
            reading.service_ok = true;
            ui.set_status_state(
                if status.connected { "connected" } else { "disconnected" }.into(),
            );
            ui.set_status_connection_type(
                match status.conn_type.as_str() {
                    "wifi" => "WiFi",
                    "ethernet" => "Ethernet",
                    "none" => "",
                    other => other,
                }
                .into(),
            );
            ui.set_status_ip_address(status.ip_address.clone().unwrap_or_default().into());
        }
        Err(e) => {
            reading.service_error.get_or_insert(e);
            ui.set_status_state("disconnected".into());
            ui.set_status_connection_type("".into());
            ui.set_status_ip_address("".into());
        }
    }

    // The resolvers. The property is called wifi-dns because that pane was built first; the
    // resolvers are the machine's, not the radio's.
    match call::<DnsConfig>(method::DNS, serde_json::json!({})) {
        Ok(dns) => {
            ui.set_wifi_dns(dns.nameservers.join(", ").into());
            reading.dns = dns;
        }
        Err(e) => {
            reading.service_error.get_or_insert(e);
            ui.set_wifi_dns("".into());
        }
    }

    // Wi-Fi. The three properties that replaced `wifi-enabled`, and everything that depends on
    // there being a radio.
    match call::<WifiState>(method::WIFI_STATE, serde_json::json!({})) {
        Ok(wifi) => {
            ui.set_wifi_adapter_present(wifi.adapter_present);
            ui.set_wifi_radio(wifi.radio.as_str().into());
            ui.set_wifi_state_reason(wifi.reason.clone().unwrap_or_default().into());
            ui.set_wifi_current_ssid(wifi.connected_ssid.clone().unwrap_or_default().into());
            ui.set_wifi_signal_strength(wifi.signal.unwrap_or(0));
            ui.set_wifi_ip_address(wifi.ip_address.clone().unwrap_or_default().into());
            ui.set_wifi_speed(wifi.rate.clone().unwrap_or_default().into());
            ui.set_wifi_gateway(wifi.gateway.clone().unwrap_or_default().into());
            ui.set_wifi_subnet(wifi.subnet.clone().unwrap_or_default().into());
            // The status bar's signal meter is the Wi-Fi one or nothing: an ethernet link has no
            // signal strength, and drawing four empty bars for it is a reading nobody took.
            ui.set_status_signal_strength(
                if ui.get_status_connection_type() == "WiFi" { wifi.signal.unwrap_or(0) } else { 0 },
            );
            reading.wifi = wifi;
        }
        Err(e) => {
            reading.service_error.get_or_insert(e.clone());
            ui.set_wifi_adapter_present(false);
            ui.set_wifi_radio("unknown".into());
            ui.set_wifi_state_reason(e.as_str().into());
            ui.set_wifi_current_ssid("".into());
            ui.set_wifi_signal_strength(0);
            ui.set_status_signal_strength(0);
            reading.wifi = WifiState {
                reason: Some(e),
                ..WifiState::default()
            };
        }
    }

    // The saved networks. Read whether or not the radio is on: this machine's list of networks it
    // will join is a fact about the machine, not about what is in range.
    match call::<Vec<KnownNetwork>>(method::WIFI_KNOWN, serde_json::json!({})) {
        Ok(known) => {
            let rows: Vec<WifiNetwork> = known
                .iter()
                .map(|k| WifiNetwork {
                    ssid: k.ssid.as_str().into(),
                    // A saved network's signal is only known if it is also in the scan list, and
                    // this list is not that list. Zero here means "not measured", and the Known
                    // Networks pane draws no signal bars for exactly that reason.
                    signal: 0,
                    security: "".into(),
                    is_connected: k.is_active,
                    is_saved: true,
                })
                .collect();
            ui.set_wifi_saved_networks(ModelRc::new(VecModel::from(rows)));
            reading.known = known;
        }
        Err(e) => {
            reading.service_error.get_or_insert(e);
            ui.set_wifi_saved_networks(ModelRc::new(VecModel::<WifiNetwork>::default()));
        }
    }

    // The firewall.
    match call::<FirewallState>(method::FIREWALL, serde_json::json!({})) {
        Ok(fw) => {
            apply_firewall(ui, &fw);
            reading.firewall = fw;
        }
        Err(e) => {
            reading.service_error.get_or_insert(e.clone());
            let fw = FirewallState {
                reason: Some(e),
                ..FirewallState::default()
            };
            apply_firewall(ui, &fw);
            reading.firewall = fw;
        }
    }

    // The state of the machine, said on screen. A command's own failure is more specific than
    // this and overwrites it in `settle`, which runs immediately after one.
    ui.set_notice(match &reading.service_error {
        Some(e) => format!("the network service did not answer: {e}").into(),
        None => SharedString::new(),
    });

    *state.lock().unwrap() = reading;
}

fn apply_firewall(ui: &NetworkManagerApp, fw: &FirewallState) {
    ui.set_firewall_state(fw.state.as_str().into());
    ui.set_firewall_backend_name(fw.kind.clone().unwrap_or_default().into());
    // -1, not 0. "Nobody counted" and "there are none" are different, and the screen draws the
    // first as "unknown". The property this replaces defaulted to 0 and was drawn as a number.
    ui.set_firewall_rule_count(fw.rule_count.map(|n| n as i32).unwrap_or(-1));
    ui.set_firewall_reason(fw.reason.clone().unwrap_or_default().into());
    let rules: Vec<FirewallRule> = fw
        .rules
        .iter()
        .map(|r| FirewallRule {
            description: r.text.as_str().into(),
            chain: r.chain.as_str().into(),
            action: r.action.as_str().into(),
        })
        .collect();
    ui.set_firewall_rules(ModelRc::new(VecModel::from(rules)));
}

/// Re-read the machine, then put this command's own verdict on screen.
///
/// The re-read comes first on purpose: `refresh` writes the notice about the machine, and the
/// command's message is the more specific of the two, so it has to be written last.
fn settle<T>(
    ui: &NetworkManagerApp,
    state: &State,
    result: Result<T, String>,
) -> Result<T, String> {
    refresh(ui, state);
    match &result {
        Ok(_) => ui.set_notice(SharedString::new()),
        Err(reason) => ui.set_notice(reason.as_str().into()),
    }
    result
}

/// Run one slow thing off the UI thread and settle it back on the UI thread when it is done.
///
/// Slint will not be touched from another thread, and the control surface will not wait more than
/// three seconds on this one. Scanning and connecting need both of those facts respected at once.
fn off_thread<T, W, A>(ui: &NetworkManagerApp, work: W, apply: A)
where
    T: Send + 'static,
    W: FnOnce() -> Result<T, String> + Send + 'static,
    A: FnOnce(&NetworkManagerApp, Result<T, String>) + Send + 'static,
{
    let back = ui.as_weak();
    std::thread::spawn(move || {
        let outcome = work();
        let _ = back.upgrade_in_event_loop(move |ui| apply(&ui, outcome));
    });
}

// ══════════════════════════════════════════════════════════════════════
// One path per mutation
// ══════════════════════════════════════════════════════════════════════
//
// Each function below is called by the button on screen and by the control action of the same
// name. There is nothing on the person's path that the mind's path skips, and no second
// implementation to drift. The service verifies each change against a re-read before answering;
// these check the answer again against the window, because `settle` has just refreshed it from
// the same service and a disagreement between the two is worth seeing.

/// Turn the radio on or off. `enabled` is the state being asked for, never a flip.
fn do_radio(ui: &NetworkManagerApp, state: &State, enabled: bool) -> Result<WifiState, String> {
    let params = serde_json::to_value(WifiRadioParams { enabled }).unwrap();
    let outcome = call::<WifiState>(method::WIFI_RADIO, params);
    settle(ui, state, outcome)
}

/// Disconnect from the current network.
fn do_disconnect(ui: &NetworkManagerApp, state: &State) -> Result<WifiState, String> {
    let outcome = call::<WifiState>(method::WIFI_DISCONNECT, serde_json::json!({}));
    settle(ui, state, outcome)
}

/// Delete a saved network. The service refuses the one that is currently in use.
fn do_forget(
    ui: &NetworkManagerApp,
    state: &State,
    ssid: &str,
) -> Result<WifiForgetResult, String> {
    let params = serde_json::to_value(WifiForgetParams { ssid: ssid.to_string() }).unwrap();
    let outcome = call::<WifiForgetResult>(method::WIFI_FORGET, params);
    settle(ui, state, outcome)
}

/// Ask the adapter to sweep the band, then read the list back. Runs off the UI thread.
fn do_scan(ui: &NetworkManagerApp, state: &State) {
    ui.set_wifi_scanning(true);
    let state = state.clone();
    off_thread(
        ui,
        || {
            let params = serde_json::to_value(WifiScanParams { rescan: true }).unwrap();
            call::<Vec<ScannedNetwork>>(method::WIFI_SCAN, params)
        },
        move |ui, outcome| {
            ui.set_wifi_scanning(false);
            match &outcome {
                Ok(found) => {
                    let rows: Vec<WifiNetwork> = found
                        .iter()
                        .map(|n| WifiNetwork {
                            // A hidden network has no name to show. Said rather than shown
                            // blank, and still listed: a row dropped from this list is how a
                            // pane comes to say "no networks in range" when there are some.
                            ssid: if n.hidden { "(hidden network)".into() } else { n.ssid.as_str().into() },
                            signal: n.signal,
                            security: n.security.as_str().into(),
                            is_connected: n.is_connected,
                            is_saved: n.is_saved,
                        })
                        .collect();
                    ui.set_wifi_networks(ModelRc::new(VecModel::from(rows)));
                }
                Err(_) => {
                    // The list is cleared rather than left standing: a stale list presented as
                    // the result of a scan that failed is a small version of the same lie.
                    ui.set_wifi_networks(ModelRc::new(VecModel::<WifiNetwork>::default()));
                }
            }
            let _ = settle(&ui, &state, outcome);
        },
    );
}

/// Put the window's own password prompt on screen for the person to type into.
///
/// It is the same dialog the Wi-Fi list opens for an unsaved row: same title, same box, same
/// Connect button that fires `on_wifi_connect`. The prompt is cleared first so a previous
/// attempt's status or half-typed secret is not sitting in it. A prompt nobody can see is a
/// connect that never happens, so the window comes out of minimized the way the other apps'
/// `show` actions do.
fn raise_password_prompt(ui: &NetworkManagerApp, ssid: &str) {
    ui.set_wifi_password_input("".into());
    ui.set_wifi_connect_status("".into());
    ui.set_wifi_password_ssid(ssid.into());
    ui.set_wifi_password_visible(true);
    ui.window().set_minimized(false);
}

/// The real backend: the socket call to network-service. `join_request` has already decided
/// what secret, if any, travels; the service hands it to nmcli over stdin with `--ask`, never
/// through a command line, and its errors never name it.
struct ServiceBackend;

impl connect::Backend for ServiceBackend {
    fn wifi_connect(&self, request: &WifiConnectParams) -> Result<WifiState, String> {
        let params = serde_json::to_value(request).unwrap();
        call::<WifiState>(method::WIFI_CONNECT, params)
    }
}

/// Join a network. Runs off the UI thread: an association waits on a handshake and on DHCP.
///
/// The request is moved into the worker and its password is never copied anywhere else — not
/// into the notice, not into `wifi-connect-status`, not into a log line. `wifi-password-input`
/// is cleared as the dialog closes so it does not sit in the window's model either.
fn do_connect(ui: &NetworkManagerApp, state: &State, request: WifiConnectParams) {
    ui.set_wifi_connect_status(format!("Connecting to {}\u{2026}", request.ssid).as_str().into());
    let state = state.clone();
    let named = request.ssid.clone();
    off_thread(
        ui,
        move || {
            // The same `submit` the tests drive with a recorder: one path from a typed
            // password to the backend, whether a person typed it or the action raised the
            // prompt for one.
            connect::submit(
                &ServiceBackend,
                &request.ssid,
                request.password.as_deref().unwrap_or(""),
            )
        },
        move |ui, outcome| {
            match &outcome {
                Ok(_) => {
                    ui.set_wifi_connect_status(format!("Connected to {named}").as_str().into());
                    ui.set_wifi_password_visible(false);
                    ui.set_wifi_password_input("".into());
                }
                Err(reason) => {
                    // The dialog stays open holding the SSID, the way Calendar's form stays open
                    // holding what was typed, with the reason above it. The password box is
                    // cleared: a wrong secret left in a text field gets tried again by accident.
                    ui.set_wifi_connect_status(reason.as_str().into());
                    ui.set_wifi_password_input("".into());
                }
            }
            let _ = settle(&ui, &state, outcome);
        },
    );
}

// ══════════════════════════════════════════════════════════════════════
// The agent rail
// ══════════════════════════════════════════════════════════════════════

/// Fill the rail from what this app actually holds.
///
/// Each row says where it came from. A section with nothing true to say does not draw, which on
/// this screen is most of the time: a machine with no Wi-Fi adapter has one interesting fact
/// about it and the rail says that one rather than four empty ones.
fn refresh_agent_rail(ui: &NetworkManagerApp, state: &State) {
    let reading = state.lock().unwrap();
    let mut context = Vec::new();

    context.push(AgentContextItem {
        id: "link".into(),
        label: if ui.get_status_state() == "connected" {
            format!(
                "{} — {}",
                ui.get_status_connection_type(),
                ui.get_status_ip_address()
            )
        } else {
            "Not connected".to_string()
        }
        .into(),
        detail: "from network-service".into(),
        source: "file".into(),
    });

    context.push(AgentContextItem {
        id: "wifi".into(),
        label: if !reading.wifi.adapter_present {
            "No Wi-Fi adapter".to_string()
        } else {
            match reading.wifi.connected_ssid.as_deref() {
                Some(ssid) => format!("Wi-Fi on {ssid}"),
                None => format!("Wi-Fi radio {}", reading.wifi.radio.as_str()),
            }
        }
        .into(),
        detail: reading
            .wifi
            .device
            .clone()
            .unwrap_or_else(|| "read from /sys/class/net".to_string())
            .into(),
        source: "file".into(),
    });

    context.push(AgentContextItem {
        id: "firewall".into(),
        label: match reading.firewall.state {
            FirewallStatus::Active => format!(
                "Firewall filtering ({})",
                reading.firewall.kind.clone().unwrap_or_default()
            ),
            FirewallStatus::Inactive => "Firewall installed, not filtering".to_string(),
            FirewallStatus::Absent => "No firewall installed".to_string(),
            FirewallStatus::Unknown => "Firewall state unknown".to_string(),
        }
        .into(),
        detail: reading
            .firewall
            .reason
            .clone()
            .unwrap_or_else(|| "read at this machine's privilege".to_string())
            .into(),
        source: "file".into(),
    });

    ui.set_agent_context(ModelRc::new(VecModel::from(context)));

    let reach = companion::reach();
    let mut suggestions: Vec<AgentSuggestion> = Vec::new();
    if reach == companion::Reach::Ready {
        suggestions.push(AgentSuggestion {
            id: "explain".into(),
            label: "Explain this machine's network".into(),
            detail: "reads what is on this screen".into(),
            icon: "spark".into(),
            running: ui.get_proposal_working(),
            proposes: false,
        });
    }
    ui.set_agent_suggestions(ModelRc::new(VecModel::from(suggestions)));
    ui.set_agent_unavailable(match reach.hint() {
        Some(hint) => hint.into(),
        None => SharedString::new(),
    });
}

/// What to ask the companion about what is on screen.
///
/// One place, because two controls ask it: the rail's suggestion and the header's Explain button.
/// Only what has been read is sent — no SSID this app has not seen, and never a password.
fn network_question(ui: &NetworkManagerApp, state: &State) -> String {
    let reading = state.lock().unwrap();
    let wifi = if !reading.wifi.adapter_present {
        "Wi-Fi: this machine has no Wi-Fi adapter.".to_string()
    } else {
        format!(
            "Wi-Fi: radio {}, joined to {}.",
            reading.wifi.radio.as_str(),
            reading.wifi.connected_ssid.as_deref().unwrap_or("nothing")
        )
    };
    let firewall = match reading.firewall.state {
        FirewallStatus::Unknown => format!(
            "Firewall: could not be determined ({}).",
            reading.firewall.reason.clone().unwrap_or_default()
        ),
        other => format!(
            "Firewall: {} ({}).",
            other.as_str(),
            reading.firewall.kind.clone().unwrap_or_else(|| "no tool".to_string())
        ),
    };
    format!(
        "This is what a machine's network settings report. In at most four short lines say how \
         this machine is connected and whether anything about it is worth attention. Use only \
         what is shown.\n\nLink: {} {}\n{wifi}\n{firewall}\nResolvers: {}",
        ui.get_status_connection_type(),
        ui.get_status_ip_address(),
        if reading.dns.nameservers.is_empty() {
            "none configured".to_string()
        } else {
            reading.dns.nameservers.join(", ")
        }
    )
}

// ══════════════════════════════════════════════════════════════════════
// The window's own controls
// ══════════════════════════════════════════════════════════════════════

fn wire(app: &NetworkManagerApp, state: &State) {
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_wifi_radio_set(move |enabled| {
            let Some(ui) = weak.upgrade() else { return };
            // The button drops the result because it has nowhere to return it to; what it cannot
            // drop is the notice, which `settle` has already put on screen.
            let _ = do_radio(&ui, &state, enabled);
        });
    }

    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_wifi_scan(move || {
            let Some(ui) = weak.upgrade() else { return };
            do_scan(&ui, &state);
        });
    }

    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_wifi_connect(move |ssid, password| {
            let Some(ui) = weak.upgrade() else { return };
            // An empty box is not a password; `join_request` decides that, and it is the same
            // function the control action's path goes through.
            do_connect(&ui, &state, connect::join_request(&ssid, &password));
        });
    }

    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_wifi_disconnect(move || {
            let Some(ui) = weak.upgrade() else { return };
            let _ = do_disconnect(&ui, &state);
        });
    }

    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_wifi_forget(move |ssid| {
            let Some(ui) = weak.upgrade() else { return };
            let _ = do_forget(&ui, &state, &ssid.to_string());
        });
    }

    // ── The agent layer ──
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_agent_suggestion_activated(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            if id != "explain" {
                return;
            }
            ask_companion(&ui, &state);
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_ai_explain_pressed(move || {
            let Some(ui) = weak.upgrade() else { return };
            ask_companion(&ui, &state);
        });
    }
    {
        let weak = app.as_weak();
        app.on_ai_dismiss(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_ai_response("".into());
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_proposal_dismissed(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_proposal(AgentProposal::default());
            }
        });
    }
    {
        // The card's verb is "Close": what it holds is an answer, not a change to apply. Both
        // buttons do the same thing, and they do it rather than being an empty closure — a
        // button that visibly does nothing is the fault this app was full of.
        let weak = app.as_weak();
        app.on_proposal_applied(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_proposal(AgentProposal::default());
            }
        });
    }
    {
        // A rail row goes to the section it is about. It is the smallest real thing this callback
        // can do and it is what a person pressing "Firewall state unknown" is asking for.
        let weak = app.as_weak();
        app.on_agent_context_activated(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            match id.as_str() {
                "wifi" => ui.set_active_tab(0),
                "link" => ui.set_active_tab(if ui.get_status_connection_type() == "WiFi" { 0 } else { 1 }),
                "firewall" => ui.set_active_tab(2),
                _ => {}
            }
        });
    }
}

/// The one path to the companion, shared by the header button and the rail suggestion.
fn ask_companion(ui: &NetworkManagerApp, state: &State) {
    let prompt = network_question(ui, state);
    ui.set_ai_is_working(true);
    ui.set_proposal_working(true);
    ui.set_proposal(AgentProposal {
        title: "Reading this machine's network".into(),
        source: "from what is on this screen".into(),
        ..Default::default()
    });
    let back = ui.as_weak();
    std::thread::spawn(move || {
        let outcome = companion::ask(&prompt);
        let _ = back.upgrade_in_event_loop(move |ui| {
            ui.set_ai_is_working(false);
            ui.set_proposal_working(false);
            match outcome {
                Ok(text) => {
                    ui.set_ai_response(text.as_str().into());
                    ui.set_proposal(AgentProposal {
                        title: "This machine's network".into(),
                        body: text.as_str().into(),
                        source: "from what is on this screen".into(),
                        verb: "Close".into(),
                        ..Default::default()
                    });
                }
                Err(e) => {
                    ui.set_ai_response(e.to_string().as_str().into());
                    ui.set_proposal(AgentProposal {
                        title: "The companion did not answer".into(),
                        body: format!("{e}").as_str().into(),
                        verb: "Close".into(),
                        ..Default::default()
                    });
                }
            }
        });
    });
}

// ══════════════════════════════════════════════════════════════════════
// The control surface
// ══════════════════════════════════════════════════════════════════════
//
// The id is `network`, which is what `crates/yantrik-ui/src/wire/dock.rs` routes
// `network` and `network_manager` to (`Launch::Program { id: "network", bin:
// "yantrik-network-manager" }`). The socket is `app-network.sock`, beside the service's own
// `network.sock`: the service stores nothing and answers about the machine, the app is the window
// someone is looking at, and `yos` tries `app-network` first.
//
// ## The grades, and why
//
// This machine is reached over the network. That fact decides three of these five.
//
// `wifi_disconnect` and `wifi_radio` are **dangerous**, not sensitive. The ladder's own rule is
// "anything that destroys work or state a person cannot get back". What these destroy is not
// data, it is the channel: a caller driving this machine from somewhere else and turning the
// radio off has cut the wire it would have sent the undo down. Nothing in the surface can put it
// back, and no amount of care at the call site makes it recoverable — it needs someone in the
// room. `stop` on a container is `sensitive` because a start is one call away; there is no such
// call here. `wifi_radio` carries the grade for its worst argument, because the ladder grades
// actions and not argument values, and `off` is the worst argument.
//
// `wifi_connect` is **sensitive**. It hands this machine to whatever is answering to that
// SSID, which is worth a deliberate decision; it does not take the machine off the network it
// is on — a wired link is untouched, and a failed association leaves the previous one standing.
// The password is not an argument and cannot become one (#178): a network this machine has not
// saved opens the window's own prompt and waits for a person to type into it.
//
// `wifi_forget` is **sensitive**, and it is only honestly sensitive because the service refuses
// to forget the network currently in use: `nmcli connection delete` on the active profile takes
// the link down as a side effect, which would have made this a `dangerous` thing wearing a
// `sensitive` grade. What it does destroy is a stored credential nobody may have written down.
//
// `wifi_scan` and `refresh` are **standard**, the floor. A scan puts the radio off the air for a
// few seconds and changes nothing.
//
// No password is echoed by any of this: not in a result, not in `describe`, not in the notice.
// None is taken in either — the only road a secret travels is the window's prompt to the
// service, and no action argument, card or audit line is on it.

fn publish_control(app: &NetworkManagerApp, state: &State) {
    use yantrik_app_runtime::control::{Action, App, Param, View};

    let describe = {
        let weak = app.as_weak();
        let state = state.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                return View::new("Network — closing");
            };
            let reading = state.lock().unwrap();

            let interfaces: Vec<serde_json::Value> = {
                let model = ui.get_ethernet_interfaces();
                (0..model.row_count())
                    .filter_map(|i| model.row_data(i))
                    .map(|i| {
                        serde_json::json!({
                            "name": i.name.to_string(),
                            "type": "ethernet",
                            "state": i.status.to_string(),
                            "ip": text_or_null(&i.ip_address),
                            "mac": text_or_null(&i.mac_address),
                        })
                    })
                    .collect()
            };

            let known: Vec<serde_json::Value> = reading
                .known
                .iter()
                .map(|k| serde_json::json!({ "ssid": k.ssid, "active": k.is_active }))
                .collect();

            let in_range: Vec<serde_json::Value> = {
                let model = ui.get_wifi_networks();
                (0..model.row_count())
                    .filter_map(|i| model.row_data(i))
                    .map(|n| {
                        serde_json::json!({
                            "ssid": n.ssid.to_string(),
                            "signal": n.signal,
                            "security": n.security.to_string(),
                            "connected": n.is_connected,
                            "saved": n.is_saved,
                        })
                    })
                    .collect()
            };

            // Trouble leads the summary, the way a failed service leads the shell's: it is the
            // reason to look. "No Wi-Fi adapter" is a fact about the machine and not trouble, so
            // it does not lead — but it is never rendered as "Wi-Fi off" either.
            let summary = if let Some(e) = &reading.service_error {
                format!("Network — the network service did not answer: {e}")
            } else if ui.get_status_state() == "connected" {
                let via = if ui.get_status_connection_type() == "WiFi" {
                    reading
                        .wifi
                        .connected_ssid
                        .clone()
                        .unwrap_or_else(|| "WiFi".to_string())
                } else {
                    ui.get_status_connection_type().to_string()
                };
                format!(
                    "Network — online via {via}, {}",
                    ui.get_status_ip_address()
                )
            } else {
                "Network — offline".to_string()
            };

            View::new(summary)
                .with("connected", ui.get_status_state() == "connected")
                .with("type", text_or_null(&ui.get_status_connection_type()))
                .with("ip_address", text_or_null(&ui.get_status_ip_address()))
                .with("interfaces", serde_json::Value::Array(interfaces))
                .with(
                    "dns",
                    serde_json::json!({
                        "nameservers": reading.dns.nameservers,
                        "search_domains": reading.dns.search_domains,
                    }),
                )
                .with(
                    "wifi",
                    serde_json::json!({
                        "adapter_present": reading.wifi.adapter_present,
                        "device": reading.wifi.device,
                        // "on", "off" or "unknown" — never a bool, because a bool cannot say
                        // "there is no radio" and this app used to answer false for that.
                        "radio": reading.wifi.radio.as_str(),
                        "connected_ssid": reading.wifi.connected_ssid,
                        "signal": reading.wifi.signal,
                        "reason": reading.wifi.reason,
                        "known": serde_json::Value::Array(known),
                        "in_range": serde_json::Value::Array(in_range),
                    }),
                )
                .with(
                    "firewall",
                    serde_json::json!({
                        "kind": reading.firewall.kind,
                        "state": reading.firewall.state.as_str(),
                        // null, not 0. An active firewall whose ruleset this session may not read
                        // has an unknown number of rules, and zero is a number somebody counted.
                        "rules": reading.firewall.rule_count,
                        "reason": reading.firewall.reason,
                    }),
                )
                .with(
                    "source",
                    serde_json::json!({
                        "service": "network-service",
                        "answered": reading.service_ok,
                        "read_at": reading.read_at,
                        // What each half of this view was read with, so a caller can judge it.
                        "interfaces": "/proc/net/dev and SIOCGIFADDR, via network-service",
                        "dns": "/etc/resolv.conf, via network-service",
                        "wifi": "/sys/class/net for the adapter, nmcli for the rest",
                        "firewall": "nft, ufw or firewall-cmd, read unprivileged",
                        "error": reading.service_error,
                    }),
                )
                // Said twice: the strip under the header is the person's half of this.
                .with("notice", text_or_null(&ui.get_notice()))
        }
    };

    let weak = app.as_weak();
    let ui_for = move || weak.upgrade().ok_or_else(|| "Network window is gone".to_string());

    let refresh_ui = ui_for.clone();
    let scan_ui = ui_for.clone();
    let connect_ui = ui_for.clone();
    let disconnect_ui = ui_for.clone();
    let forget_ui = ui_for.clone();
    let radio_ui = ui_for;

    let refresh_state = state.clone();
    let scan_state = state.clone();
    let connect_state = state.clone();
    let disconnect_state = state.clone();
    let forget_state = state.clone();
    let radio_state = state.clone();

    App::new("network")
        .describe(describe)
        .action(
            Action::new("refresh", "Re-read interfaces, Wi-Fi, resolvers and the firewall"),
            move |_| {
                let ui = refresh_ui()?;
                refresh(&ui, &refresh_state);
                let reading = refresh_state.lock().unwrap();
                Ok(serde_json::json!({
                    "connected": ui.get_status_state() == "connected",
                    "type": ui.get_status_connection_type().to_string(),
                    "wifi_adapter_present": reading.wifi.adapter_present,
                    "firewall_state": reading.firewall.state.as_str(),
                    "service_answered": reading.service_ok,
                }))
            },
        )
        .action(
            // Defers: a rescan sweeps 2.4 and 5 GHz and the service allows it ten seconds, which
            // is more than the three this surface waits before telling the caller the app did not
            // answer. The list arrives in `describe` under `wifi.in_range`.
            Action::new("wifi_scan", "Look for Wi-Fi networks in range").defers(),
            move |_| {
                let ui = scan_ui()?;
                // Bound to a local first: the lock guard of a `.lock()` written inside an
                // `if` condition lives to the end of the whole `if`, and `no_adapter` locks
                // again — which would be a deadlock on the UI thread.
                let present = scan_state.lock().unwrap().wifi.adapter_present;
                if !present {
                    // Refused before anything is started, naming the reason. A deferred action
                    // that accepts and then fails silently is worse than one that says no.
                    return Err(no_adapter(&ui, &scan_state));
                }
                do_scan(&ui, &scan_state);
                Ok(serde_json::json!({
                    "scanning": true,
                    "settles": "wifi.in_range and notice in describe",
                }))
            },
        )
        .action(
            // Sensitive, and deferred: an association waits on a WPA handshake and on DHCP, which
            // the service allows twenty-five seconds. The published parameter list — and the
            // reason there is no `password` in it — lives in `connect::wifi_connect_action`.
            connect::wifi_connect_action(),
            move |args| {
                let ui = connect_ui()?;
                let ssid = args["ssid"].as_str().unwrap_or_default().trim().to_string();
                if ssid.is_empty() {
                    return Err("a network name is needed to connect".to_string());
                }
                // Bound to locals first: the lock guard of a `.lock()` written inside an `if`
                // condition lives to the end of the whole `if`, and `no_adapter` locks again —
                // which would be a deadlock on the UI thread.
                let (present, plan) = {
                    let reading = connect_state.lock().unwrap();
                    (reading.wifi.adapter_present, connect::plan(&ssid, &reading.known))
                };
                if !present {
                    return Err(no_adapter(&ui, &connect_state));
                }
                match plan {
                    connect::Plan::Join => {
                        // Saved on this machine: NetworkManager holds the credential, so no
                        // secret is needed and none is asked for.
                        do_connect(&ui, &connect_state, connect::join_request(&ssid, ""));
                        Ok(connect::joining_answer(&ssid))
                    }
                    connect::Plan::AskPerson => {
                        // Nothing is saved here and a password is not an argument this action
                        // can carry (#178). The window's own prompt goes up, the person types
                        // into it, and what they type reaches the service through the same
                        // `on_wifi_connect` path their click would.
                        raise_password_prompt(&ui, &ssid);
                        Ok(connect::waiting_answer(&ssid))
                    }
                }
            },
        )
        .action(
            // Dangerous. See the note above this function: on a machine reached over the network
            // this takes away the channel the undo would travel on.
            Action::new(
                "wifi_disconnect",
                "Leave the Wi-Fi network this machine is on. On a machine reached over Wi-Fi \
                 this ends that connection and cannot be undone remotely.",
            )
            .risk("dangerous"),
            move |_| {
                let ui = disconnect_ui()?;
                let state = do_disconnect(&ui, &disconnect_state)?;
                Ok(serde_json::json!({
                    "disconnected": true,
                    "connected_ssid": state.connected_ssid,
                    "radio": state.radio.as_str(),
                }))
            },
        )
        .action(
            // Sensitive: a stored credential, gone. The service refuses the active network, so
            // this cannot take the link down as a side effect.
            Action::new("wifi_forget", "Delete a saved Wi-Fi network from this machine")
                .risk("sensitive")
                .arg(Param::text("ssid").describe("The saved network to delete")),
            move |args| {
                let ui = forget_ui()?;
                let ssid = args["ssid"].as_str().unwrap_or_default().trim().to_string();
                if ssid.is_empty() {
                    return Err("a network name is needed to forget one".to_string());
                }
                let result = do_forget(&ui, &forget_state, &ssid)?;
                Ok(serde_json::json!({
                    "forgotten": result.forgotten,
                    "known": result.known.iter().map(|k| k.ssid.clone()).collect::<Vec<_>>(),
                }))
            },
        )
        .action(
            // Dangerous, for the same reason as disconnect and one worse: a machine whose radio
            // is off cannot be told to turn it back on over the radio.
            Action::new(
                "wifi_radio",
                "Turn the Wi-Fi radio on or off. Turning it off on a machine reached over Wi-Fi \
                 ends that connection and cannot be undone remotely.",
            )
            .risk("dangerous")
            .arg(Param::text("state").describe("\"on\" or \"off\"")),
            move |args| {
                let ui = radio_ui()?;
                let want = args["state"].as_str().unwrap_or_default().trim().to_lowercase();
                let enabled = match want.as_str() {
                    "on" => true,
                    "off" => false,
                    // Named rather than guessed at. A `dangerous` action must not decide for
                    // itself what an unrecognised word meant.
                    other => {
                        return Err(format!(
                            "state must be \"on\" or \"off\"; got \"{other}\""
                        ))
                    }
                };
                let state = do_radio(&ui, &radio_state, enabled)?;
                Ok(serde_json::json!({
                    "radio": state.radio.as_str(),
                    "connected_ssid": state.connected_ssid,
                }))
            },
        )
        .serve();
}

/// The refusal for a machine with no Wi-Fi adapter, said on screen as well as returned.
///
/// The sentence names the absence rather than the SSID, because "no network called X was found"
/// on a machine that cannot look for one at all is the wrong thing to tell somebody.
fn no_adapter(ui: &NetworkManagerApp, state: &State) -> String {
    let reason = state
        .lock()
        .unwrap()
        .wifi
        .reason
        .clone()
        .unwrap_or_else(|| "this machine has no Wi-Fi adapter".to_string());
    ui.set_notice(reason.as_str().into());
    reason
}

/// A string property as JSON, with the empty string as `null`.
///
/// System Monitor's lesson, one app over: an empty `cpu_model` and an empty `ip` read to an
/// auditor as measurements of an empty thing. `null` is the value that means "this app does not
/// know", and `""` is not it.
fn text_or_null(text: &SharedString) -> serde_json::Value {
    if text.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::Value::String(text.to_string())
    }
}
