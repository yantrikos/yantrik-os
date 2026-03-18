//! Yantrik Network Manager — standalone app binary.
//!
//! Network management with WiFi, Ethernet, Bluetooth, VPN, Firewall, Diagnostics.
//! Uses IPC service "network" for operations, with basic stubs as fallback.

use slint::{ComponentHandle, Model, ModelRc, VecModel};
use yantrik_app_runtime::prelude::*;
use yantrik_ipc_transport::SyncRpcClient;

slint::include_modules!();

fn main() {
    init_tracing("yantrik-network-manager");

    let app = NetworkManagerApp::new().unwrap();
    wire(&app);
    app.run().unwrap();
}

// ── Service wrappers ─────────────────────────────────────────────────

fn call_network(method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    let client = SyncRpcClient::for_service("network");
    client.call(method, params).map_err(|e| e.message)
}

fn wire(app: &NetworkManagerApp) {
    // ── WiFi ──
    {
        let weak = app.as_weak();
        app.on_toggle_wifi(move || {
            let Some(ui) = weak.upgrade() else { return };
            let enabled = ui.get_wifi_enabled();
            let _ = call_network("network.wifi_toggle", serde_json::json!({ "enabled": !enabled }));
            tracing::info!("Toggle WiFi: {} -> {}", enabled, !enabled);
        });
    }

    {
        let weak = app.as_weak();
        app.on_wifi_scan(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_wifi_scanning(true);
            match call_network("network.wifi_scan", serde_json::json!({})) {
                Ok(result) => {
                    if let Ok(networks) = serde_json::from_value::<Vec<serde_json::Value>>(result) {
                        let wifi_list: Vec<WifiNetwork> = networks.iter().map(|n| {
                            WifiNetwork {
                                ssid: n.get("ssid").and_then(|v| v.as_str()).unwrap_or("").into(),
                                signal: n.get("signal").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                                security: n.get("security").and_then(|v| v.as_str()).unwrap_or("").into(),
                                is_connected: n.get("is_connected").and_then(|v| v.as_bool()).unwrap_or(false),
                                is_saved: n.get("is_saved").and_then(|v| v.as_bool()).unwrap_or(false),
                            }
                        }).collect();
                        ui.set_wifi_networks(ModelRc::new(VecModel::from(wifi_list)));
                    }
                }
                Err(e) => tracing::warn!("WiFi scan failed: {e}"),
            }
            ui.set_wifi_scanning(false);
        });
    }

    {
        let weak = app.as_weak();
        app.on_wifi_connect(move |ssid, password| {
            let Some(ui) = weak.upgrade() else { return };
            let ssid_str = ssid.to_string();
            let pass_str = password.to_string();
            match call_network("network.wifi_connect", serde_json::json!({
                "ssid": ssid_str,
                "password": pass_str
            })) {
                Ok(_) => {
                    ui.set_wifi_connect_status("Connected".into());
                    ui.set_wifi_password_visible(false);
                    tracing::info!("Connected to WiFi: {ssid_str}");
                }
                Err(e) => {
                    ui.set_wifi_connect_status(format!("Failed: {e}").into());
                    tracing::warn!("WiFi connect failed: {e}");
                }
            }
        });
    }

    app.on_wifi_disconnect(|| {
        let _ = call_network("network.wifi_disconnect", serde_json::json!({}));
        tracing::info!("WiFi disconnect");
    });

    app.on_wifi_forget(|ssid| {
        let _ = call_network("network.wifi_forget", serde_json::json!({ "ssid": ssid.to_string() }));
        tracing::info!("WiFi forget: {ssid}");
    });

    // ── Bluetooth ──
    app.on_toggle_bluetooth(|| { tracing::info!("Toggle Bluetooth"); });
    app.on_bt_scan(|| { tracing::info!("Bluetooth scan"); });
    app.on_bt_pair(|addr| { tracing::info!("Bluetooth pair: {addr}"); });
    app.on_bt_connect(|addr| { tracing::info!("Bluetooth connect: {addr}"); });
    app.on_bt_disconnect(|addr| { tracing::info!("Bluetooth disconnect: {addr}"); });

    // ── VPN ──
    app.on_vpn_connect(|name| { tracing::info!("VPN connect: {name}"); });
    app.on_vpn_disconnect(|name| { tracing::info!("VPN disconnect: {name}"); });
    app.on_net_vpn_import_config(|| { tracing::info!("VPN import config"); });

    // ── Firewall ──
    app.on_toggle_firewall(|| { tracing::info!("Toggle firewall"); });
    app.on_firewall_allow_port(|port| { tracing::info!("Firewall allow port: {port}"); });
    app.on_firewall_block_port(|port| { tracing::info!("Firewall block port: {port}"); });
    app.on_firewall_apply_profile(|profile| { tracing::info!("Firewall apply profile: {profile}"); });

    // ── Diagnostics ──
    app.on_diag_run_test(|test| { tracing::info!("Run diagnostic test: {test}"); });
    app.on_diag_run_all_tests(|| { tracing::info!("Run all diagnostic tests"); });
    app.on_test_ai_connectivity(|| { tracing::info!("Test AI provider connectivity"); });
    app.on_net_diag_run_ping(|| { tracing::info!("Run ping"); });
    app.on_net_diag_run_traceroute(|| { tracing::info!("Run traceroute"); });

    // ── AI assist ──
    app.on_ai_explain_pressed(|| { tracing::info!("AI explain requested (standalone mode)"); });
    app.on_ai_dismiss(|| { tracing::info!("AI dismiss"); });
}
