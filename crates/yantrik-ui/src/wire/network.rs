//! The network mark and its popover: the shell's one reader of the network.
//!
//! NetworkManager's reading arrives from `yantrik_os::network` whenever something changes (its
//! signals, not a timer), and this file is the only place it is turned into what the shell shows:
//! the bar's mark, the popover's list, Quick Settings' Wi-Fi tile, the System screen's network row,
//! Settings > Network and `describe shell`. They used to be four separate readings, which is how
//! the shell came to draw a Wi-Fi mark over NetworkManager's "Wired connection 1" on a machine with
//! no radio in it.
//!
//! What a person asks of the network from the bar comes back here too, and each ask is its own act:
//!   * the Wi-Fi tile is the **radio** (`set-radio`),
//!   * the connected row's Disconnect is **leave this network** (`disconnect`), and does not touch
//!     the radio. The Quick Settings tile did both under the caption "Tap to disconnect": it ran
//!     `nmcli radio wifi off`.
//!   * a row joins a network (`connect`).
//!
//! # The Wi-Fi password
//!
//! It is typed into the popover's field and goes `connect(ssid, secret)` -> [`ConnectRequest`] ->
//! NetworkManager's `AddAndActivateConnection` over D-Bus, and nowhere else. Never on a command
//! line (an `nmcli ... password X` is readable by every process through `/proc/<pid>/cmdline`),
//! never in a log line or a toast, never in a property that outlives the attempt: the field is
//! cleared as it submits, and the request's secret is overwritten when the attempt is over. No
//! action on the control surface carries one (see `control_network` and
//! `no_published_action_can_carry_a_passphrase`).

use std::rc::Rc;

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use yantrik_os::{ConnectRequest, NetKind, NetworkSnapshot, WifiSecret};

use crate::{App, NetworkRow, NetworkState};

/// Everything a screen shows about the network, as plain words, derived from one reading.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Readout {
    /// Something is up and carrying traffic. True on a wired machine, which is why nothing that
    /// means "am I online" may read the Wi-Fi flag.
    pub online: bool,
    /// `wifi`, `ethernet`, or empty when nothing is up.
    pub medium: &'static str,
    /// The label the System screen's network row stands under.
    pub label: &'static str,
    /// What stands beside it: the network's name, else the address, else "Connected".
    pub detail: String,
    /// The SSID, and only ever an SSID: empty on anything not wireless.
    pub ssid: String,
    pub ip: String,
}

pub(crate) fn readout(snapshot: &NetworkSnapshot) -> Readout {
    let ssid = if snapshot.kind == NetKind::Wifi { snapshot.ssid.clone().unwrap_or_default() } else { String::new() };
    let ip = snapshot.ip.clone().unwrap_or_default();
    let label = if snapshot.vpn && snapshot.online() {
        "VPN"
    } else {
        match snapshot.kind {
            NetKind::Wifi => "Wi-Fi",
            NetKind::Wired => "Ethernet",
            NetKind::None => "Network",
        }
    };
    let detail = if !snapshot.online() {
        "Offline".to_string()
    } else {
        [ssid.as_str(), ip.as_str()].into_iter().find(|c| !c.is_empty()).unwrap_or("Connected").to_string()
    };
    Readout { online: snapshot.online(), medium: snapshot.medium(), label, detail, ssid, ip }
}

thread_local! {
    /// The one model behind the popover's list. It is updated in place, never replaced: a Slint
    /// `for` keeps a row's element (and a password field's typed text) only while the model keeps
    /// the row, and a whole new model rebuilds every row. Each reading used to swap the list, so a
    /// rescan wiped a password half typed.
    static ROWS: Rc<VecModel<NetworkRow>> = Rc::new(VecModel::default());
}

/// The order the rows should be in. While the popover is open (`keep`) the rows already on screen
/// keep their places, whatever the signal does: a list that reshuffles under the pointer gets the
/// wrong network clicked, and a row that moves loses its element and what was typed in it. Rows
/// that have gone are dropped and new ones go at the end. Closed, the reading's own order (joined
/// first, then strongest) stands.
pub(crate) fn target_order(previous: &[String], desired: &[String], keep: bool) -> Vec<String> {
    if !keep {
        return desired.to_vec();
    }
    let mut order: Vec<String> = previous.iter().filter(|p| desired.contains(p)).cloned().collect();
    order.extend(desired.iter().filter(|d| !previous.contains(d)).cloned());
    order
}

/// Bring `model` to `desired` by removals, insertions and changes to a row's own data, so a row
/// that is still there is the same row (keyed by SSID), and keeps its element.
pub(crate) fn apply_rows(model: &VecModel<NetworkRow>, desired: Vec<NetworkRow>, keep: bool) {
    let previous: Vec<String> = (0..model.row_count()).filter_map(|i| model.row_data(i)).map(|r| r.ssid.to_string()).collect();
    let wanted: Vec<String> = desired.iter().map(|r| r.ssid.to_string()).collect();
    let order = target_order(&previous, &wanted, keep);
    let target: Vec<NetworkRow> = order
        .iter()
        .filter_map(|ssid| desired.iter().find(|r| r.ssid.as_str() == ssid).cloned())
        .collect();

    let mut i = 0;
    while i < model.row_count() {
        let here = model.row_data(i).map(|r| r.ssid.to_string()).unwrap_or_default();
        if order.contains(&here) {
            i += 1;
        } else {
            model.remove(i);
        }
    }
    for (i, row) in target.into_iter().enumerate() {
        if i < model.row_count() {
            let there = model.row_data(i).unwrap_or_default();
            if there.ssid == row.ssid {
                if there != row {
                    model.set_row_data(i, row);
                }
                continue;
            }
            // Moved: take it out of where it was, so there is one row for the name.
            if let Some(j) = (i + 1..model.row_count()).find(|&j| model.row_data(j).map_or(false, |r| r.ssid == row.ssid)) {
                model.remove(j);
            }
        }
        model.insert(i, row);
    }
}

/// Wire the popover's callbacks and start listening for NetworkManager's changes.
pub fn wire(ui: &App) {
    let state = ui.global::<NetworkState>();
    state.set_networks(ROWS.with(|m| ModelRc::from(m.clone())));

    // The popover opened: look for networks, once. Not repeated, not on a timer: a scan takes the
    // radio off the air for a moment.
    state.on_scan_requested(|| {
        std::thread::spawn(|| {
            if let Err(why) = yantrik_os::network::request_scan() {
                tracing::debug!(%why, "Wi-Fi scan not started");
            }
        });
    });

    // The Wi-Fi tile: the radio.
    state.on_set_radio(|on| {
        std::thread::spawn(move || match yantrik_os::network::set_wifi_enabled(on) {
            Ok(now) => tracing::info!(asked_for = on, now, "Wi-Fi radio set"),
            Err(why) => tracing::warn!(%why, "Wi-Fi radio could not be set"),
        });
    });

    // Disconnect: leave the network. The radio stays as it is.
    state.on_disconnect(|| {
        // `true`: it is the person at the machine, who can plug the cable back in.
        std::thread::spawn(|| match yantrik_os::network::disconnect(true) {
            Ok(device) => tracing::info!(%device, "Network disconnected"),
            Err(why) => tracing::warn!(%why, "Could not disconnect"),
        });
    });

    let weak = ui.as_weak();
    state.on_connect(move |ssid, secret| {
        // The secret moves straight into a type that overwrites itself on drop. `secret` here is
        // Slint's copy of the argument, which the field it came from has already been told to
        // clear.
        let request = ConnectRequest {
            ssid: ssid.to_string(),
            secret: if secret.is_empty() { None } else { Some(WifiSecret::new(secret.to_string())) },
            // The popover's buttons are a person's. A caller's join goes through
            // `control_network`, which never sets this.
            by_person: true,
        };
        // A refusal has already been put under the row for a person's press.
        let _ = start_join(weak.clone(), request);
    });

    let weak = ui.as_weak();
    state.on_settings_requested(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_network_open(false);
            // Settings > Network is category 3 of the settings sidebar.
            ui.set_settings_category(3);
            ui.set_current_screen(7);
            ui.invoke_navigate(7);
        }
    });

    // Everything the shell shows comes through here. Called now with the last reading if there
    // is one, then on each change, from the monitor's thread: hand it to the thread that draws.
    let weak = ui.as_weak();
    yantrik_os::network::subscribe(move |snapshot| {
        let weak = weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                publish(&ui, &snapshot);
            }
        });
    });
}

/// Join a network off the thread that draws: association waits on a handshake and on DHCP.
///
/// The request is moved into the worker and nowhere else. Its secret is not copied into the
/// notice, the status text or a log line, and it is overwritten when the attempt ends.
///
/// One join runs at a time, and a mind's are spaced: the slot is taken HERE, before the thread
/// exists, so a caller that loops this cannot start threads faster than joins finish. A refusal is
/// returned to the caller and, for a person's press, shown under the row.
pub(crate) fn start_join(weak: slint::Weak<App>, request: ConnectRequest) -> Result<(), String> {
    let ssid = request.ssid.clone();
    let slot = match yantrik_os::network::begin_join(request.by_person) {
        Ok(slot) => slot,
        Err(why) => {
            if request.by_person {
                if let Some(ui) = weak.upgrade() {
                    let state = ui.global::<NetworkState>();
                    state.set_notice_ssid(ssid.as_str().into());
                    state.set_notice(why.as_str().into());
                }
            }
            return Err(why);
        }
    };
    if let Some(ui) = weak.upgrade() {
        let state = ui.global::<NetworkState>();
        state.set_joining_ssid(ssid.as_str().into());
        state.set_notice_ssid(SharedString::default());
        state.set_notice(SharedString::default());
    }
    std::thread::spawn(move || {
        let outcome = yantrik_os::network::connect(request, true, slot);
        match &outcome {
            Ok(_) => tracing::info!(%ssid, "Wi-Fi joined"),
            // The reason is words for a person and never contains the password.
            Err(why) => tracing::info!(%ssid, %why, "Wi-Fi join did not complete"),
        }
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            let state = ui.global::<NetworkState>();
            state.set_joining_ssid(SharedString::default());
            match outcome {
                Ok(_) => {
                    // Joined: whatever was asked for is answered.
                    if state.get_requested_ssid().as_str() == ssid {
                        state.set_requested_ssid(SharedString::default());
                    }
                }
                Err(why) => {
                    // Under the row, in words. The field is NOT reopened here: a failed join can
                    // be a mind's, and a field appearing under the person's hands would take
                    // their next keystrokes. Clicking the row opens it again.
                    state.set_notice_ssid(ssid.as_str().into());
                    state.set_notice(why.as_str().into());
                }
            }
        });
    });
    Ok(())
}

/// Put one reading on every surface that states it.
///
/// Every write is guarded by a comparison, because a Slint property set marks its dependents dirty
/// whether or not the value moved: an unconditional write repaints the bar on each reading.
fn publish(ui: &App, snapshot: &NetworkSnapshot) {
    let r = readout(snapshot);
    let state = ui.global::<NetworkState>();

    let (mark, bars) = match snapshot.mark() {
        yantrik_os::Mark::Offline => ("offline", 0),
        yantrik_os::Mark::Wired => ("wired", 0),
        yantrik_os::Mark::Wifi(n) => ("wifi", n as i32),
        yantrik_os::Mark::Vpn => ("vpn", 0),
    };
    if state.get_mark().as_str() != mark {
        state.set_mark(mark.into());
    }
    if state.get_bars() != bars {
        state.set_bars(bars);
    }
    let tooltip = snapshot.tooltip();
    if state.get_tooltip().as_str() != tooltip {
        state.set_tooltip(tooltip.as_str().into());
    }
    if state.get_kind().as_str() != snapshot.kind.as_str() {
        state.set_kind(snapshot.kind.as_str().into());
    }
    if state.get_online() != r.online {
        state.set_online(r.online);
    }
    let connecting = snapshot.state == yantrik_os::NetState::Connecting;
    if state.get_connecting() != connecting {
        state.set_connecting(connecting);
    }
    if state.get_ssid().as_str() != r.ssid {
        state.set_ssid(r.ssid.as_str().into());
    }
    if state.get_ip().as_str() != r.ip {
        state.set_ip(r.ip.as_str().into());
    }
    if state.get_vpn() != snapshot.vpn {
        state.set_vpn(snapshot.vpn);
    }
    if state.get_no_internet() != snapshot.no_internet() {
        state.set_no_internet(snapshot.no_internet());
    }
    if state.get_portal() != snapshot.portal() {
        state.set_portal(snapshot.portal());
    }
    if state.get_wifi_present() != snapshot.wifi_present {
        state.set_wifi_present(snapshot.wifi_present);
    }
    if state.get_radio_on() != snapshot.radio_on {
        state.set_radio_on(snapshot.radio_on);
    }
    let rows: Vec<NetworkRow> = snapshot
        .access_points
        .iter()
        .map(|p| NetworkRow {
            ssid: p.ssid.as_str().into(),
            strength: p.strength as i32,
            bars: yantrik_os::network_model::bars(p.strength) as i32,
            secured: p.secured,
            known: p.known,
            connected: p.connected,
            enterprise: p.enterprise,
        })
        .collect();
    // In place, keyed by name: see `ROWS`.
    let keep = ui.get_network_open() || !state.get_asking_ssid().is_empty();
    ROWS.with(|model| apply_rows(model, rows, keep));
    // The grades of the actions that could end the machine's only connection follow it.
    crate::control_network::sync_grades(Some(snapshot));

    // The properties the System screen, Settings > Network and the older readers stand on. One
    // owner, so nothing goes on showing an address after the connection has gone.
    if ui.get_network_online() != r.online {
        ui.set_network_online(r.online);
    }
    if ui.get_network_medium().as_str() != r.medium {
        ui.set_network_medium(r.medium.into());
    }
    if ui.get_network_label().as_str() != r.label {
        ui.set_network_label(r.label.into());
    }
    if ui.get_network_detail().as_str() != r.detail {
        ui.set_network_detail(r.detail.as_str().into());
    }
    let wireless = r.online && r.medium == "wifi";
    if ui.get_wifi_connected() != wireless {
        ui.set_wifi_connected(wireless);
    }
    if ui.get_sys_wifi_ssid().as_str() != r.ssid {
        ui.set_sys_wifi_ssid(r.ssid.as_str().into());
    }
    if !r.online {
        ui.set_settings_ip_address(Default::default());
    } else if !r.ip.is_empty() {
        ui.set_settings_ip_address(r.ip.as_str().into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_os::{AccessPoint, Connectivity, NetState};

    fn wired() -> NetworkSnapshot {
        NetworkSnapshot {
            kind: NetKind::Wired,
            state: NetState::Connected,
            ip: Some("192.168.4.44".into()),
            connectivity: Connectivity::Full,
            ..NetworkSnapshot::default()
        }
    }

    fn laptop() -> NetworkSnapshot {
        NetworkSnapshot {
            kind: NetKind::Wifi,
            state: NetState::Connected,
            ssid: Some("Wombat".into()),
            strength: Some(80),
            ip: Some("10.0.0.8".into()),
            connectivity: Connectivity::Full,
            wifi_present: true,
            radio_on: true,
            access_points: vec![AccessPoint {
                ssid: "Wombat".into(),
                strength: 80,
                secured: true,
                known: true,
                connected: true,
                enterprise: false,
            }],
            ..NetworkSnapshot::default()
        }
    }

    /// The whole of #50's second screen: "WiFi" stood over NetworkManager's connection name on a
    /// machine with no wireless device.
    #[test]
    fn a_wired_machine_is_online_and_is_not_wifi() {
        let r = readout(&wired());
        assert_eq!(r.label, "Ethernet");
        assert_eq!(r.medium, "ethernet");
        assert_eq!(r.ssid, "");
        // Online is what the bar's mark means; deriving it from the Wi-Fi flag drew a wired
        // machine as offline.
        assert!(r.online);
        assert_eq!(r.ip, "192.168.4.44");
        assert_eq!(r.detail, "192.168.4.44");
    }

    #[test]
    fn a_wireless_machine_shows_its_ssid() {
        let r = readout(&laptop());
        assert_eq!(r.label, "Wi-Fi");
        assert_eq!(r.detail, "Wombat");
        assert_eq!(r.ssid, "Wombat");
        assert!(r.online);
    }

    #[test]
    fn a_connection_with_no_address_still_says_connected() {
        let mut s = wired();
        s.ip = None;
        assert_eq!(readout(&s).detail, "Connected");
    }

    #[test]
    fn nothing_up_says_so() {
        let r = readout(&NetworkSnapshot::default());
        assert!(!r.online);
        assert_eq!(r.label, "Network");
        assert_eq!(r.detail, "Offline");
        assert_eq!(r.medium, "");
    }

    /// The code above the tests, which is what the shell runs.
    fn code() -> String {
        include_str!("network.rs").split("#[cfg(test)]").next().unwrap().to_string()
    }

    /// The handler `on_<name>(` to the end of its call.
    fn handler(name: &str) -> String {
        let src = code();
        let from = src.find(&format!("state.on_{name}(")).unwrap_or_else(|| panic!("no handler on_{name}"));
        let body = &src[from..];
        body[..body.find("\n    });").expect("the handler ends")].to_string()
    }

    /// The bug this story fixed: the Quick Settings Wi-Fi tile ran `nmcli radio wifi off` under the
    /// caption "Tap to disconnect". The radio's action is the radio, Disconnect's action is
    /// disconnect, and neither does the other's work.
    #[test]
    fn the_radio_tile_switches_the_radio_and_disconnect_only_disconnects() {
        let radio = handler("set_radio");
        assert!(radio.contains("set_wifi_enabled(on)"), "the tile is the radio. As written:\n{radio}");
        assert!(!radio.contains("disconnect"), "the radio tile does not disconnect. As written:\n{radio}");

        let leave = handler("disconnect");
        assert!(leave.contains("network::disconnect(true)"), "Disconnect disconnects. As written:\n{leave}");
        assert!(!leave.contains("set_wifi_enabled"), "Disconnect must not switch the radio off. As written:\n{leave}");
    }

    /// The same two acts, as the Slint says them: the tiles call the radio, the button calls
    /// disconnect, and the Quick Settings tile no longer runs nmcli.
    #[test]
    fn the_tiles_and_the_button_are_wired_to_the_right_acts() {
        let popover = include_str!("../../../yantrik-ui-slint/ui/components/popovers/network_popover.slint");
        let toggle = &popover[popover.find("YToggleTile {").unwrap()..];
        let toggle = &toggle[..toggle.find("\n    }").unwrap()];
        assert!(toggle.contains("NetworkState.set-radio("), "the popover's Wi-Fi tile is the radio:\n{toggle}");
        assert!(!toggle.contains("disconnect"), "the popover's Wi-Fi tile does not disconnect:\n{toggle}");
        let leave_button = &popover[popover.find("label: \"Disconnect\"").unwrap()..];
        let leave_button = &leave_button[..leave_button.find('}').unwrap()];
        assert!(leave_button.contains("root.leave()"), "the Disconnect button leaves the network:\n{leave_button}");
        assert!(popover.contains("leave => { NetworkState.disconnect(); }"), "and leaving is disconnect, not the radio");

        let overlays = include_str!("../../../yantrik-ui-slint/ui/components/shell_overlays.slint");
        assert!(
            overlays.contains("toggle-wifi => { NetworkState.set-radio(!NetworkState.radio-on); }"),
            "the Quick Settings tile is the radio, through the same callback as the popover's"
        );

        let callbacks = include_str!("callbacks.rs");
        assert!(!callbacks.contains("\"radio\""), "nothing in callbacks.rs shells out to `nmcli radio` any more");
        let qs = include_str!("../../../yantrik-ui-slint/ui/components/quick_settings.slint");
        assert!(!qs.contains("Tap to disconnect"), "the tile does not promise a disconnect it never did");
        // The tile says the radio's state in words ("Off", the network's name), presses the radio,
        // and its chevron is the list: the body and the chevron are separate targets.
        assert!(qs.contains("root.toggle-wifi()"), "pressing the Wi-Fi tile switches the radio");
        assert!(qs.contains("details-requested => { root.network-details(); }"), "its chevron opens the network list");
        assert!(qs.contains("!root.wifi-radio-on ? \"Off\""), "the tile says the radio is off when it is");
    }

    fn row(ssid: &str, strength: i32) -> NetworkRow {
        NetworkRow { ssid: ssid.into(), strength, bars: 2, secured: true, known: false, connected: false, enterprise: false }
    }
    fn names(m: &VecModel<NetworkRow>) -> Vec<String> {
        (0..m.row_count()).map(|i| m.row_data(i).unwrap().ssid.to_string()).collect()
    }

    /// A rescan used to swap the whole list, rebuilding the row with the password field in it and
    /// losing what was typed. Open, the rows stay where they are, keyed by name, and a changed
    /// signal is a change to that row's data.
    #[test]
    fn a_new_reading_keeps_the_rows_on_screen_where_they_are() {
        let model = VecModel::from(vec![row("Home", 70), row("Cafe", 50), row("Guest", 30)]);
        // Cafe is now the strongest and a new network appeared; Guest left.
        apply_rows(&model, vec![row("Cafe", 90), row("Home", 70), row("Newcomer", 40)], true);
        assert_eq!(names(&model), ["Home", "Cafe", "Newcomer"], "kept in place, the new one at the end, the gone one dropped");
        assert_eq!(model.row_data(1).unwrap().strength, 90, "the row's own data moved with the reading");
        // Closed, the reading's own order stands.
        apply_rows(&model, vec![row("Cafe", 90), row("Home", 70), row("Newcomer", 40)], false);
        assert_eq!(names(&model), ["Cafe", "Home", "Newcomer"]);
    }

    #[test]
    fn rows_are_updated_in_place_and_the_model_is_never_replaced() {
        let code = code();
        let publish = &code[code.find("fn publish(").unwrap()..];
        assert!(!publish.contains("set_networks("), "publish must not swap the list: a new model rebuilds every row");
        assert!(publish.contains("apply_rows(model, rows, keep)"));
        // The model is handed to the window once.
        assert_eq!(code.matches("set_networks(").count(), 1);
    }

    /// A failed join may be a mind's, and a password field opening under the person's hands would
    /// take their next keystrokes: nothing in the join path opens one.
    #[test]
    fn nothing_but_a_click_opens_a_password_row() {
        let code = code();
        assert!(!code.contains("set_asking_ssid"), "only the person's click in the popover sets the row");
    }

    /// A password on a command line is readable by every process on the machine, and one in a log
    /// line is readable by anyone who can read the log. Neither is possible from this file: it
    /// spawns no process, and no log line here names the secret.
    #[test]
    fn the_password_is_never_logged_and_no_process_is_spawned_for_it() {
        // Comments say why, and may name what went wrong; only the code is checked.
        let src: String = code().lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("
");
        assert!(!src.contains("Command::new"), "the join goes over D-Bus, not through nmcli");
        assert!(!src.contains("nmcli"), "no nmcli");
        for (n, line) in src.lines().enumerate().filter(|(_, l)| l.contains("tracing::")) {
            assert!(
                !line.to_lowercase().contains("secret") && !line.to_lowercase().contains("password") && !line.contains("request"),
                "line {} logs something that could be the password: {line}",
                n + 1
            );
        }
    }

    /// The Slint state holds no property for a password, so there is nowhere to read one from.
    #[test]
    fn the_network_state_has_no_property_that_could_hold_a_password() {
        let state = include_str!("../../../yantrik-ui-slint/ui/components/bar/network_state.slint");
        for line in state.lines().filter(|l| !l.trim_start().starts_with("//") && l.contains("property")) {
            let lower = line.to_lowercase();
            for word in ["password", "secret", "psk", "passphrase"] {
                assert!(!lower.contains(word), "NetworkState declares a property that reads as a secret: {line}");
            }
        }
    }

    /// The field that takes the password clears itself as it submits, on both of its routes.
    #[test]
    fn the_password_field_is_cleared_as_it_submits() {
        let popover = include_str!("../../../yantrik-ui-slint/ui/components/popovers/network_popover.slint");
        assert!(popover.contains("accepted(text) => { root.submit(text); self.value = \"\"; }"), "Enter clears the field");
        assert!(popover.contains("clicked => { root.submit(pw.value); pw.value = \"\"; }"), "Join clears the field");
        // And it only exists while a password is being asked for.
        assert!(popover.contains("if root.asking : HorizontalLayout"), "the field is not instantiated unless asked for");
    }
}
