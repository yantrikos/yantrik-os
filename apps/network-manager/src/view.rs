//! The only code that writes the window: the model, drawn, for the parts that changed.

use slint::ComponentHandle as _;
use yantrik_app::app::Changed;
use yantrik_app::rail::{self, RailView as _};
use yantrik_app::rows;

use crate::model::*;
use crate::{EthernetInterface, FirewallRule, NetworkManagerApp, WifiNetwork};

/// Whether a command named `part`. The parts that write the window's in-out properties are drawn
/// only then, never on a redraw of everything (see `model::PART_PROMPT_OPEN`).
fn named(changed: &Changed, part: &str) -> bool {
    matches!(changed, Changed::Parts(parts) if parts.contains(&part))
}

pub fn render(ui: &NetworkManagerApp, m: &Model, changed: &Changed) {
    if changed.has(PART_READING) {
        reading(ui, m);
    }
    if changed.has(PART_NOTICE) {
        ui.set_notice(m.notice.as_str().into());
    }
    if changed.has(PART_SCAN) {
        ui.set_wifi_scanning(m.scanning);
        let networks: Vec<WifiNetwork> = m
            .in_range
            .iter()
            .map(|n| WifiNetwork {
                ssid: n.ssid.as_str().into(),
                signal: n.signal,
                security: n.security.as_str().into(),
                is_connected: n.is_connected,
                is_saved: n.is_saved,
            })
            .collect();
        rows::set(ui.get_wifi_networks(), networks, |list| ui.set_wifi_networks(list));
    }
    if changed.has(PART_PROMPT_STATUS) {
        ui.set_wifi_connect_status(m.prompt_status.as_str().into());
    }
    // The window's own password prompt, put up for the person to type into: the same dialog the
    // Wi-Fi list opens for an unsaved row, cleared first so a previous attempt's half-typed
    // secret is not sitting in it.
    if named(changed, PART_PROMPT_CLEAR) {
        ui.set_wifi_password_input("".into());
    }
    if named(changed, PART_PROMPT_OPEN) {
        ui.set_wifi_password_ssid(m.prompt_ssid.as_str().into());
        ui.set_wifi_password_visible(true);
        ui.window().set_minimized(false);
    }
    if named(changed, PART_PROMPT_CLOSE) {
        ui.set_wifi_password_visible(false);
    }
    if named(changed, PART_SECTION) {
        ui.set_active_tab(m.section);
    }
    if changed.has(PART_AI) {
        ui.set_ai_is_working(m.ai.working);
        ui.set_ai_response(m.ai.response.as_str().into());
    }
    if changed.has(rail::PART) {
        ui.render_rail(&m.rail);
    }
}

/// Everything the last refresh read, on screen. A reading lands every three seconds and is
/// usually the one before it: every property is set to its value, which Slint compares, and every
/// list through `rows::set`, so an unchanged network draws nothing.
fn reading(ui: &NetworkManagerApp, m: &Model) {
    let r = &m.reading;
    ui.set_wifi_dns(r.resolvers.as_str().into());

    let eth: Vec<EthernetInterface> = r
        .ethernet_rows()
        .into_iter()
        .map(|i| EthernetInterface {
            name: i.name.into(),
            status: i.status.into(),
            ip_address: i.ip_address.into(),
            mac_address: i.mac_address.into(),
            speed: i.speed.into(),
            // Per-interface DHCP is still not reported; the badge is the old assumption.
            is_dhcp: true,
            subnet: i.subnet.into(),
            gateway: i.gateway.into(),
            dns: i.dns.into(),
        })
        .collect();
    rows::set(ui.get_ethernet_interfaces(), eth, |list| ui.set_ethernet_interfaces(list));

    ui.set_status_state(r.status.state.as_str().into());
    ui.set_status_connection_type(r.status.kind.as_str().into());
    ui.set_status_ip_address(r.status.ip.as_str().into());

    // Wi-Fi: the three properties that replaced `wifi-enabled`. When the service could not say,
    // the reading is "no adapter, radio unknown" with the reason, which is what is drawn.
    ui.set_wifi_adapter_present(r.wifi.adapter_present);
    ui.set_wifi_radio(r.wifi.radio.as_str().into());
    ui.set_wifi_state_reason(r.wifi.reason.clone().unwrap_or_default().into());
    ui.set_wifi_current_ssid(r.wifi.connected_ssid.clone().unwrap_or_default().into());
    ui.set_wifi_signal_strength(r.wifi.signal.unwrap_or(0));
    if r.wifi_answered {
        ui.set_wifi_ip_address(r.wifi.ip_address.clone().unwrap_or_default().into());
        ui.set_wifi_speed(r.wifi.rate.clone().unwrap_or_default().into());
        ui.set_wifi_gateway(r.wifi.gateway.clone().unwrap_or_default().into());
        ui.set_wifi_subnet(r.wifi.subnet.clone().unwrap_or_default().into());
    }
    ui.set_status_signal_strength(r.status_signal());

    let saved: Vec<WifiNetwork> = r
        .known
        .iter()
        .map(|k| WifiNetwork {
            ssid: k.ssid.as_str().into(),
            // A saved network's signal is only known if it is also in the scan list, and this
            // list is not that list. Zero here means "not measured", and the Known Networks pane
            // draws no signal bars for exactly that reason.
            signal: 0,
            security: "".into(),
            is_connected: k.is_active,
            is_saved: true,
        })
        .collect();
    rows::set(ui.get_wifi_saved_networks(), saved, |list| ui.set_wifi_saved_networks(list));

    let fw = &r.firewall;
    ui.set_firewall_state(fw.state.as_str().into());
    ui.set_firewall_backend_name(fw.kind.clone().unwrap_or_default().into());
    // -1, not 0. "Nobody counted" and "there are none" are different, and the screen draws the
    // first as "unknown".
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
    rows::set(ui.get_firewall_rules(), rules, |list| ui.set_firewall_rules(list));
}
