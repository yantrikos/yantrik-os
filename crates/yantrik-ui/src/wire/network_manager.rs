//! Network Manager wire module — WiFi, Ethernet, Bluetooth, VPN, Firewall.
//!
//! Runs system commands in background threads to avoid blocking the UI.
//! Refreshes network state on a 5-second timer.

use std::sync::{Arc, Mutex};

use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::{App, WifiNetwork, EthernetInterface, BluetoothDevice, VpnConnection, FirewallRule};

/// Shared network state — updated by background threads, read by UI timer.
struct NetState {
    // WiFi
    wifi_enabled: bool,
    wifi_current_ssid: String,
    wifi_signal: i32,
    wifi_ip: String,
    wifi_speed: String,
    wifi_networks: Vec<WifiNetworkData>,
    wifi_saved: Vec<WifiNetworkData>,
    wifi_scanning: bool,

    // Ethernet
    ethernet: Vec<EthernetData>,

    // Bluetooth
    bt_enabled: bool,
    bt_paired: Vec<BtDeviceData>,
    bt_available: Vec<BtDeviceData>,
    bt_scanning: bool,

    // VPN
    vpn_connections: Vec<VpnData>,

    // Firewall
    fw_enabled: bool,
    fw_backend: String,
    fw_rule_count: i32,
    fw_rules: Vec<FwRuleData>,
    fw_profile: String,

    // Dirty flag — set when data changes, cleared after UI sync
    dirty: bool,
}

#[derive(Clone)]
struct WifiNetworkData {
    ssid: String,
    signal: i32,
    security: String,
    is_connected: bool,
    is_saved: bool,
}

#[derive(Clone)]
struct EthernetData {
    name: String,
    status: String,
    ip_address: String,
    mac_address: String,
    speed: String,
    is_dhcp: bool,
    subnet: String,
    gateway: String,
    dns: String,
}

#[derive(Clone)]
struct BtDeviceData {
    name: String,
    address: String,
    device_type: String,
    is_paired: bool,
    is_connected: bool,
}

#[derive(Clone)]
struct VpnData {
    name: String,
    vpn_type: String,
    status: String,
    ip_address: String,
    traffic: String,
}

#[derive(Clone)]
struct FwRuleData {
    id: i32,
    description: String,
    chain: String,
    action: String,
}

impl NetState {
    fn new() -> Self {
        Self {
            wifi_enabled: false,
            wifi_current_ssid: String::new(),
            wifi_signal: 0,
            wifi_ip: String::new(),
            wifi_speed: String::new(),
            wifi_networks: Vec::new(),
            wifi_saved: Vec::new(),
            wifi_scanning: false,
            ethernet: Vec::new(),
            bt_enabled: false,
            bt_paired: Vec::new(),
            bt_available: Vec::new(),
            bt_scanning: false,
            vpn_connections: Vec::new(),
            fw_enabled: false,
            fw_backend: String::new(),
            fw_rule_count: 0,
            fw_rules: Vec::new(),
            fw_profile: "Custom".to_string(),
            dirty: true,
        }
    }
}

/// Run a command and return stdout, or empty string on failure.
fn cmd_output(cmd: &str, args: &[&str]) -> String {
    std::process::Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

/// Check if a command exists on the system.
fn cmd_exists(cmd: &str) -> bool {
    std::process::Command::new(cmd)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Detect WiFi backend: nmcli > iwctl > wpa_cli > none
fn wifi_backend() -> &'static str {
    if cmd_exists("nmcli") {
        "nmcli"
    } else if cmd_exists("iwctl") {
        "iwctl"
    } else if cmd_exists("wpa_cli") {
        "wpa_cli"
    } else {
        "none"
    }
}

/// Detect firewall backend: nft > iptables > none
fn fw_backend() -> &'static str {
    if std::process::Command::new("nft")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        "nft"
    } else if std::process::Command::new("iptables")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        "iptables"
    } else {
        "none"
    }
}

/// Refresh all network state (runs in background thread).
fn refresh_all(state: &Arc<Mutex<NetState>>) {
    refresh_wifi(state);
    refresh_ethernet(state);
    refresh_bluetooth(state);
    refresh_vpn(state);
    refresh_firewall(state);
}

fn refresh_wifi(state: &Arc<Mutex<NetState>>) {
    let backend = wifi_backend();

    // Check if WiFi interface exists
    let has_wifi = std::path::Path::new("/sys/class/net/wlan0").exists()
        || cmd_output("sh", &["-c", "ls /sys/class/net/*/wireless 2>/dev/null"])
            .trim()
            .len()
            > 0;

    let mut current_ssid = String::new();
    let mut signal = 0i32;
    let mut ip = String::new();
    let mut speed = String::new();
    let mut networks = Vec::new();

    if has_wifi {
        match backend {
            "nmcli" => {
                // Current connection
                let status = cmd_output("nmcli", &["-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device", "status"]);
                for line in status.lines() {
                    let parts: Vec<&str> = line.split(':').collect();
                    if parts.len() >= 4 && parts[1] == "wifi" && parts[2] == "connected" {
                        current_ssid = parts[3].to_string();
                    }
                }

                // Signal strength from /proc/net/wireless
                if let Ok(content) = std::fs::read_to_string("/proc/net/wireless") {
                    for line in content.lines().skip(2) {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.len() >= 4 {
                            let qual = parts[2].trim_end_matches('.').parse::<f32>().unwrap_or(0.0);
                            // Convert quality (typically 0-70) to percentage
                            signal = ((qual / 70.0) * 100.0).min(100.0) as i32;
                        }
                    }
                }

                // IP address
                let ip_out = cmd_output("sh", &["-c", "ip -4 addr show scope global | grep inet | head -1"]);
                if let Some(addr) = ip_out.split_whitespace().nth(1) {
                    ip = addr.split('/').next().unwrap_or(addr).to_string();
                }

                // Scan results
                let scan = cmd_output("nmcli", &["-t", "-f", "SSID,SIGNAL,SECURITY", "device", "wifi", "list"]);
                let mut seen_ssids = std::collections::HashSet::new();
                for line in scan.lines() {
                    let parts: Vec<&str> = line.split(':').collect();
                    if parts.len() >= 3 && !parts[0].is_empty() {
                        let ssid = parts[0].to_string();
                        if seen_ssids.contains(&ssid) {
                            continue;
                        }
                        seen_ssids.insert(ssid.clone());
                        let sig = parts[1].parse::<i32>().unwrap_or(0);
                        let sec = parts[2].to_string();
                        let connected = ssid == current_ssid;
                        networks.push(WifiNetworkData {
                            ssid,
                            signal: sig,
                            security: if sec.is_empty() { "Open".to_string() } else { sec },
                            is_connected: connected,
                            is_saved: false, // nmcli doesn't distinguish in scan
                        });
                    }
                }
            }
            "iwctl" | "wpa_cli" => {
                // Basic status via /proc/net/wireless
                if let Ok(content) = std::fs::read_to_string("/proc/net/wireless") {
                    for line in content.lines().skip(2) {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.len() >= 4 {
                            let qual = parts[2].trim_end_matches('.').parse::<f32>().unwrap_or(0.0);
                            signal = ((qual / 70.0) * 100.0).min(100.0) as i32;
                        }
                    }
                }

                // Try to get SSID from iw
                let iw_out = cmd_output("sh", &["-c", "iw dev wlan0 link 2>/dev/null | grep SSID"]);
                if let Some(ssid_part) = iw_out.split("SSID:").nth(1) {
                    current_ssid = ssid_part.trim().to_string();
                }

                // IP
                let ip_out = cmd_output("sh", &["-c", "ip -4 addr show scope global | grep inet | head -1"]);
                if let Some(addr) = ip_out.split_whitespace().nth(1) {
                    ip = addr.split('/').next().unwrap_or(addr).to_string();
                }
            }
            _ => {}
        }
    }

    // Sort networks: connected first, then by signal descending
    networks.sort_by(|a, b| {
        b.is_connected
            .cmp(&a.is_connected)
            .then(b.signal.cmp(&a.signal))
    });

    if let Ok(mut s) = state.lock() {
        s.wifi_enabled = has_wifi && !current_ssid.is_empty() || has_wifi;
        s.wifi_current_ssid = current_ssid;
        s.wifi_signal = signal;
        s.wifi_ip = ip;
        s.wifi_speed = speed;
        s.wifi_networks = networks;
        s.dirty = true;
    }
}

fn refresh_ethernet(state: &Arc<Mutex<NetState>>) {
    let mut interfaces = Vec::new();

    // Read from ip -br addr
    let br_addr = cmd_output("ip", &["-br", "addr"]);
    let br_link = cmd_output("ip", &["-br", "link"]);

    // Get gateway
    let gateway = cmd_output("sh", &["-c", "ip route show default | awk '{print $3}' | head -1"])
        .trim()
        .to_string();

    // Get DNS
    let dns = std::fs::read_to_string("/etc/resolv.conf")
        .ok()
        .and_then(|c| {
            c.lines()
                .find(|l| l.starts_with("nameserver"))
                .and_then(|l| l.split_whitespace().nth(1))
                .map(String::from)
        })
        .unwrap_or_default();

    // Build MAC map from link output
    let mut mac_map = std::collections::HashMap::new();
    for line in br_link.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 {
            mac_map.insert(parts[0].to_string(), parts[2].to_string());
        }
    }

    for line in br_addr.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        let name = parts[0].to_string();
        // Skip loopback and WiFi
        if name == "lo" || name.starts_with("wlan") || name.starts_with("wl") {
            continue;
        }
        // Skip virtual/tunnel interfaces
        if name.starts_with("tun") || name.starts_with("wg") || name.starts_with("docker") || name.starts_with("br-") || name.starts_with("veth") {
            continue;
        }

        let status = parts[1].to_string();
        let ip_address = if parts.len() > 2 {
            parts[2].split('/').next().unwrap_or("").to_string()
        } else {
            String::new()
        };

        let subnet = if parts.len() > 2 {
            let full = parts[2];
            if let Some(prefix) = full.split('/').nth(1) {
                format!("/{}", prefix)
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        // Try to get speed from /sys
        let speed_str = std::fs::read_to_string(format!("/sys/class/net/{}/speed", name))
            .ok()
            .and_then(|s| {
                let mbps = s.trim().parse::<i32>().ok()?;
                if mbps > 0 {
                    Some(format!("{} Mbps", mbps))
                } else {
                    None
                }
            })
            .unwrap_or_default();

        let mac = mac_map.get(&name).cloned().unwrap_or_default();

        // Check DHCP — look for dhclient or dhcpcd process
        let is_dhcp = cmd_output("sh", &["-c", &format!("pgrep -a 'dhclient|dhcpcd' 2>/dev/null | grep -q {} && echo yes", name)])
            .trim()
            .contains("yes")
            || std::path::Path::new(&format!("/var/lib/dhcpcd/{}.lease", name)).exists();

        interfaces.push(EthernetData {
            name,
            status,
            ip_address,
            mac_address: mac,
            speed: speed_str,
            is_dhcp,
            subnet,
            gateway: gateway.clone(),
            dns: dns.clone(),
        });
    }

    if let Ok(mut s) = state.lock() {
        s.ethernet = interfaces;
        s.dirty = true;
    }
}

fn refresh_bluetooth(state: &Arc<Mutex<NetState>>) {
    let mut enabled = false;
    let mut paired = Vec::new();
    let mut available = Vec::new();

    // Check adapter status
    let show = cmd_output("bluetoothctl", &["show"]);
    for line in show.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Powered:") && trimmed.contains("yes") {
            enabled = true;
        }
    }

    if enabled {
        // Paired devices
        let paired_out = cmd_output("bluetoothctl", &["paired-devices"]);
        for line in paired_out.lines() {
            if let Some(dev) = parse_bt_device(line) {
                // Check if connected
                let info = cmd_output("bluetoothctl", &["info", &dev.address]);
                let connected = info.lines().any(|l| l.trim().starts_with("Connected:") && l.contains("yes"));
                let icon = info
                    .lines()
                    .find(|l| l.trim().starts_with("Icon:"))
                    .map(|l| l.trim().trim_start_matches("Icon:").trim().to_string())
                    .unwrap_or_default();

                paired.push(BtDeviceData {
                    name: dev.name,
                    address: dev.address,
                    device_type: classify_bt_icon(&icon),
                    is_paired: true,
                    is_connected: connected,
                });
            }
        }

        // Available (non-paired) devices
        let devices_out = cmd_output("bluetoothctl", &["devices"]);
        let paired_addrs: std::collections::HashSet<String> =
            paired.iter().map(|p| p.address.clone()).collect();
        for line in devices_out.lines() {
            if let Some(dev) = parse_bt_device(line) {
                if !paired_addrs.contains(&dev.address) {
                    available.push(BtDeviceData {
                        name: dev.name,
                        address: dev.address,
                        device_type: "unknown".to_string(),
                        is_paired: false,
                        is_connected: false,
                    });
                }
            }
        }
    }

    if let Ok(mut s) = state.lock() {
        s.bt_enabled = enabled;
        s.bt_paired = paired;
        s.bt_available = available;
        s.dirty = true;
    }
}

struct ParsedBtDevice {
    name: String,
    address: String,
}

fn parse_bt_device(line: &str) -> Option<ParsedBtDevice> {
    // Format: "Device XX:XX:XX:XX:XX:XX Name Here"
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() >= 3 && parts[0] == "Device" {
        let addr = parts[1].to_string();
        let name = parts[2..].join(" ");
        Some(ParsedBtDevice {
            name: if name.is_empty() { addr.clone() } else { name },
            address: addr,
        })
    } else {
        None
    }
}

fn classify_bt_icon(icon: &str) -> String {
    if icon.contains("audio-headset") || icon.contains("audio-headphones") {
        "headphones".to_string()
    } else if icon.contains("input-keyboard") {
        "keyboard".to_string()
    } else if icon.contains("input-mouse") {
        "mouse".to_string()
    } else if icon.contains("phone") {
        "phone".to_string()
    } else if icon.contains("audio-speakers") || icon.contains("audio-card") {
        "speaker".to_string()
    } else {
        "unknown".to_string()
    }
}

fn refresh_vpn(state: &Arc<Mutex<NetState>>) {
    let mut connections = Vec::new();

    // WireGuard
    let wg_out = cmd_output("wg", &["show"]);
    if !wg_out.trim().is_empty() {
        let mut current_iface = String::new();
        let mut endpoint = String::new();
        let mut transfer = String::new();

        for line in wg_out.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("interface:") {
                if !current_iface.is_empty() {
                    connections.push(VpnData {
                        name: current_iface.clone(),
                        vpn_type: "WireGuard".to_string(),
                        status: "connected".to_string(),
                        ip_address: endpoint.clone(),
                        traffic: transfer.clone(),
                    });
                }
                current_iface = trimmed.split(':').nth(1).unwrap_or("").trim().to_string();
                endpoint.clear();
                transfer.clear();
            } else if trimmed.starts_with("endpoint:") {
                endpoint = trimmed.split(':').nth(1).unwrap_or("").trim().to_string();
            } else if trimmed.starts_with("transfer:") {
                transfer = trimmed.trim_start_matches("transfer:").trim().to_string();
            }
        }
        if !current_iface.is_empty() {
            // Get IP from the wg interface
            let wg_ip = cmd_output("sh", &["-c", &format!("ip -4 addr show {} 2>/dev/null | grep inet | awk '{{print $2}}'", current_iface)])
                .trim()
                .to_string();

            connections.push(VpnData {
                name: current_iface,
                vpn_type: "WireGuard".to_string(),
                status: "connected".to_string(),
                ip_address: wg_ip,
                traffic: transfer,
            });
        }
    }

    // OpenVPN — check if running
    let ovpn = cmd_output("sh", &["-c", "pgrep -a openvpn 2>/dev/null"]);
    if !ovpn.trim().is_empty() {
        // Try to get config name from process args
        let name = ovpn
            .lines()
            .next()
            .and_then(|l| {
                l.split_whitespace()
                    .find(|w| w.ends_with(".conf") || w.ends_with(".ovpn"))
                    .map(|p| {
                        std::path::Path::new(p)
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("OpenVPN")
                            .to_string()
                    })
            })
            .unwrap_or_else(|| "OpenVPN".to_string());

        // Check tun interface
        let tun_ip = cmd_output("sh", &["-c", "ip -4 addr show tun0 2>/dev/null | grep inet | awk '{print $2}'"])
            .trim()
            .to_string();

        connections.push(VpnData {
            name,
            vpn_type: "OpenVPN".to_string(),
            status: "connected".to_string(),
            ip_address: tun_ip,
            traffic: String::new(),
        });
    }

    if let Ok(mut s) = state.lock() {
        s.vpn_connections = connections;
        s.dirty = true;
    }
}

fn refresh_firewall(state: &Arc<Mutex<NetState>>) {
    let backend = fw_backend();
    let mut enabled = false;
    let mut rule_count = 0;
    let mut rules = Vec::new();

    match backend {
        "nft" => {
            let tables = cmd_output("nft", &["list", "tables"]);
            enabled = !tables.trim().is_empty();

            if enabled {
                let ruleset = cmd_output("nft", &["list", "ruleset"]);
                let mut id_counter = 0;
                let mut current_chain = String::new();
                for line in ruleset.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("chain ") {
                        current_chain = trimmed
                            .trim_start_matches("chain ")
                            .trim_end_matches(" {")
                            .trim()
                            .to_uppercase();
                    }
                    // Count actual rules (not chain/table definitions)
                    if (trimmed.starts_with("tcp") || trimmed.starts_with("udp")
                        || trimmed.starts_with("ip") || trimmed.starts_with("ct")
                        || trimmed.starts_with("iif") || trimmed.starts_with("icmp")
                        || trimmed.starts_with("meta"))
                        && !trimmed.starts_with("table")
                    {
                        id_counter += 1;
                        let action = if trimmed.contains("accept") {
                            "ACCEPT"
                        } else if trimmed.contains("drop") {
                            "DROP"
                        } else if trimmed.contains("reject") {
                            "REJECT"
                        } else {
                            "OTHER"
                        };

                        rules.push(FwRuleData {
                            id: id_counter,
                            description: trimmed.to_string(),
                            chain: current_chain.clone(),
                            action: action.to_string(),
                        });
                    }
                }
                rule_count = id_counter;
            }
        }
        "iptables" => {
            let output = cmd_output("iptables", &["-L", "-n", "--line-numbers"]);
            enabled = !output.trim().is_empty();
            if enabled {
                let mut current_chain = String::new();
                let mut id_counter = 0;
                for line in output.lines() {
                    if line.starts_with("Chain ") {
                        current_chain = line
                            .split_whitespace()
                            .nth(1)
                            .unwrap_or("")
                            .to_string();
                    } else if line.starts_with(|c: char| c.is_ascii_digit()) {
                        id_counter += 1;
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        let action = parts.get(1).unwrap_or(&"").to_string();
                        let desc = parts.get(1..).map(|p| p.join(" ")).unwrap_or_default();
                        rules.push(FwRuleData {
                            id: id_counter,
                            description: desc,
                            chain: current_chain.clone(),
                            action,
                        });
                    }
                }
                rule_count = id_counter;
            }
        }
        _ => {}
    }

    if let Ok(mut s) = state.lock() {
        s.fw_enabled = enabled;
        s.fw_backend = backend.to_string();
        s.fw_rule_count = rule_count;
        s.fw_rules = rules;
        s.dirty = true;
    }
}

/// Sync state from Arc<Mutex<NetState>> to Slint UI.
fn sync_to_ui(ui: &App, state: &Arc<Mutex<NetState>>) {
    let s = match state.lock() {
        Ok(mut s) => {
            if !s.dirty {
                return;
            }
            s.dirty = false;
            // Clone the data we need
            NetStateSnapshot {
                wifi_enabled: s.wifi_enabled,
                wifi_current_ssid: s.wifi_current_ssid.clone(),
                wifi_signal: s.wifi_signal,
                wifi_ip: s.wifi_ip.clone(),
                wifi_speed: s.wifi_speed.clone(),
                wifi_networks: s.wifi_networks.clone(),
                wifi_saved: s.wifi_saved.clone(),
                wifi_scanning: s.wifi_scanning,
                ethernet: s.ethernet.clone(),
                bt_enabled: s.bt_enabled,
                bt_paired: s.bt_paired.clone(),
                bt_available: s.bt_available.clone(),
                bt_scanning: s.bt_scanning,
                vpn_connections: s.vpn_connections.clone(),
                fw_enabled: s.fw_enabled,
                fw_backend: s.fw_backend.clone(),
                fw_rule_count: s.fw_rule_count,
                fw_rules: s.fw_rules.clone(),
                fw_profile: s.fw_profile.clone(),
            }
        }
        Err(_) => return,
    };

    // WiFi
    ui.set_net_wifi_enabled(s.wifi_enabled);
    ui.set_net_wifi_current_ssid(s.wifi_current_ssid.into());
    ui.set_net_wifi_signal_strength(s.wifi_signal);
    ui.set_net_wifi_ip_address(s.wifi_ip.into());
    ui.set_net_wifi_speed(s.wifi_speed.into());
    ui.set_net_wifi_scanning(s.wifi_scanning);

    let wifi_items: Vec<WifiNetwork> = s
        .wifi_networks
        .iter()
        .map(|n| WifiNetwork {
            ssid: n.ssid.clone().into(),
            signal: n.signal,
            security: n.security.clone().into(),
            is_connected: n.is_connected,
            is_saved: n.is_saved,
        })
        .collect();
    ui.set_net_wifi_networks(ModelRc::new(VecModel::from(wifi_items)));

    let saved_items: Vec<WifiNetwork> = s
        .wifi_saved
        .iter()
        .map(|n| WifiNetwork {
            ssid: n.ssid.clone().into(),
            signal: n.signal,
            security: n.security.clone().into(),
            is_connected: n.is_connected,
            is_saved: n.is_saved,
        })
        .collect();
    ui.set_net_wifi_saved_networks(ModelRc::new(VecModel::from(saved_items)));

    // Ethernet
    let eth_items: Vec<EthernetInterface> = s
        .ethernet
        .iter()
        .map(|e| EthernetInterface {
            name: e.name.clone().into(),
            status: e.status.clone().into(),
            ip_address: e.ip_address.clone().into(),
            mac_address: e.mac_address.clone().into(),
            speed: e.speed.clone().into(),
            is_dhcp: e.is_dhcp,
            subnet: e.subnet.clone().into(),
            gateway: e.gateway.clone().into(),
            dns: e.dns.clone().into(),
        })
        .collect();
    ui.set_net_ethernet_interfaces(ModelRc::new(VecModel::from(eth_items)));

    // Bluetooth
    ui.set_net_bt_enabled(s.bt_enabled);
    ui.set_net_bt_scanning(s.bt_scanning);

    let bt_paired_items: Vec<BluetoothDevice> = s
        .bt_paired
        .iter()
        .map(|d| BluetoothDevice {
            name: d.name.clone().into(),
            address: d.address.clone().into(),
            device_type: d.device_type.clone().into(),
            is_paired: d.is_paired,
            is_connected: d.is_connected,
        })
        .collect();
    ui.set_net_bt_paired_devices(ModelRc::new(VecModel::from(bt_paired_items)));

    let bt_avail_items: Vec<BluetoothDevice> = s
        .bt_available
        .iter()
        .map(|d| BluetoothDevice {
            name: d.name.clone().into(),
            address: d.address.clone().into(),
            device_type: d.device_type.clone().into(),
            is_paired: d.is_paired,
            is_connected: d.is_connected,
        })
        .collect();
    ui.set_net_bt_available_devices(ModelRc::new(VecModel::from(bt_avail_items)));

    // VPN
    let vpn_items: Vec<VpnConnection> = s
        .vpn_connections
        .iter()
        .map(|v| VpnConnection {
            name: v.name.clone().into(),
            vpn_type: v.vpn_type.clone().into(),
            status: v.status.clone().into(),
            ip_address: v.ip_address.clone().into(),
            traffic: v.traffic.clone().into(),
        })
        .collect();
    ui.set_net_vpn_connections(ModelRc::new(VecModel::from(vpn_items)));

    // Firewall
    ui.set_net_firewall_enabled(s.fw_enabled);
    ui.set_net_firewall_backend_name(s.fw_backend.into());
    ui.set_net_firewall_rule_count(s.fw_rule_count);
    ui.set_net_firewall_profile(s.fw_profile.into());

    let fw_items: Vec<FirewallRule> = s
        .fw_rules
        .iter()
        .take(50) // Limit to 50 rules in UI
        .map(|r| FirewallRule {
            id: r.id,
            description: r.description.clone().into(),
            chain: r.chain.clone().into(),
            action: r.action.clone().into(),
        })
        .collect();
    ui.set_net_firewall_rules(ModelRc::new(VecModel::from(fw_items)));
}

struct NetStateSnapshot {
    wifi_enabled: bool,
    wifi_current_ssid: String,
    wifi_signal: i32,
    wifi_ip: String,
    wifi_speed: String,
    wifi_networks: Vec<WifiNetworkData>,
    wifi_saved: Vec<WifiNetworkData>,
    wifi_scanning: bool,
    ethernet: Vec<EthernetData>,
    bt_enabled: bool,
    bt_paired: Vec<BtDeviceData>,
    bt_available: Vec<BtDeviceData>,
    bt_scanning: bool,
    vpn_connections: Vec<VpnData>,
    fw_enabled: bool,
    fw_backend: String,
    fw_rule_count: i32,
    fw_rules: Vec<FwRuleData>,
    fw_profile: String,
}

/// Wire all Network Manager callbacks.
pub fn wire(ui: &App, _ctx: &AppContext) {
    let state = Arc::new(Mutex::new(NetState::new()));

    // Initial refresh in background
    {
        let state_clone = state.clone();
        std::thread::spawn(move || {
            refresh_all(&state_clone);
        });
    }

    // 5-second refresh timer
    let refresh_timer = Timer::default();
    {
        let state_clone = state.clone();
        let ui_weak = ui.as_weak();
        refresh_timer.start(TimerMode::Repeated, std::time::Duration::from_secs(5), move || {
            // Sync to UI on timer tick
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &state_clone);
            }

            // Trigger background refresh
            let state_bg = state_clone.clone();
            std::thread::spawn(move || {
                refresh_all(&state_bg);
            });
        });
    }
    // Keep timer alive (leak it — timers self-manage in Slint)
    std::mem::forget(refresh_timer);

    // Also do an immediate sync after short delay for initial data
    {
        let state_clone = state.clone();
        let ui_weak = ui.as_weak();
        let init_timer = Timer::default();
        init_timer.start(TimerMode::Repeated, std::time::Duration::from_millis(200), move || {
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &state_clone);
            }
        });
        // Leak this timer too
        std::mem::forget(init_timer);
    }

    // ── WiFi callbacks ──

    // Toggle WiFi (bring interface up/down)
    {
        let state_clone = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_net_toggle_wifi(move || {
            let st = state_clone.clone();
            std::thread::spawn(move || {
                let enabled = st.lock().map(|s| s.wifi_enabled).unwrap_or(false);
                if enabled {
                    let _ = std::process::Command::new("ip")
                        .args(["link", "set", "wlan0", "down"])
                        .output();
                } else {
                    let _ = std::process::Command::new("ip")
                        .args(["link", "set", "wlan0", "up"])
                        .output();
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
                refresh_wifi(&st);
            });
        });
    }

    // WiFi scan
    {
        let state_clone = state.clone();
        ui.on_net_wifi_scan(move || {
            let st = state_clone.clone();
            if let Ok(mut s) = st.lock() {
                s.wifi_scanning = true;
                s.dirty = true;
            }
            std::thread::spawn(move || {
                match wifi_backend() {
                    "nmcli" => {
                        let _ = std::process::Command::new("nmcli")
                            .args(["device", "wifi", "rescan"])
                            .output();
                        std::thread::sleep(std::time::Duration::from_secs(3));
                    }
                    "iwctl" => {
                        let _ = std::process::Command::new("iwctl")
                            .args(["station", "wlan0", "scan"])
                            .output();
                        std::thread::sleep(std::time::Duration::from_secs(3));
                    }
                    _ => {
                        let _ = std::process::Command::new("wpa_cli")
                            .args(["-i", "wlan0", "scan"])
                            .output();
                        std::thread::sleep(std::time::Duration::from_secs(3));
                    }
                }
                refresh_wifi(&st);
                if let Ok(mut s) = st.lock() {
                    s.wifi_scanning = false;
                    s.dirty = true;
                }
            });
        });
    }

    // WiFi connect
    {
        let state_clone = state.clone();
        ui.on_net_wifi_connect(move |ssid, password| {
            let ssid_str = ssid.to_string();
            let pw_str = password.to_string();
            let st = state_clone.clone();
            std::thread::spawn(move || {
                match wifi_backend() {
                    "nmcli" => {
                        let mut cmd = std::process::Command::new("nmcli");
                        cmd.args(["device", "wifi", "connect", &ssid_str]);
                        if !pw_str.is_empty() {
                            cmd.args(["password", &pw_str]);
                        }
                        let _ = cmd.output();
                    }
                    "iwctl" => {
                        let mut cmd = std::process::Command::new("iwctl");
                        if pw_str.is_empty() {
                            cmd.args(["station", "wlan0", "connect", &ssid_str]);
                        } else {
                            cmd.args(["--passphrase", &pw_str, "station", "wlan0", "connect", &ssid_str]);
                        }
                        let _ = cmd.output();
                    }
                    _ => {
                        tracing::warn!("WiFi connect not supported with wpa_cli backend from UI");
                    }
                }
                std::thread::sleep(std::time::Duration::from_secs(2));
                refresh_wifi(&st);
            });
        });
    }

    // WiFi disconnect
    {
        let state_clone = state.clone();
        ui.on_net_wifi_disconnect(move || {
            let st = state_clone.clone();
            std::thread::spawn(move || {
                match wifi_backend() {
                    "nmcli" => {
                        let _ = std::process::Command::new("nmcli")
                            .args(["device", "disconnect", "wlan0"])
                            .output();
                    }
                    "iwctl" => {
                        let _ = std::process::Command::new("iwctl")
                            .args(["station", "wlan0", "disconnect"])
                            .output();
                    }
                    _ => {}
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
                refresh_wifi(&st);
            });
        });
    }

    // WiFi forget network
    {
        let state_clone = state.clone();
        ui.on_net_wifi_forget(move |ssid| {
            let ssid_str = ssid.to_string();
            let st = state_clone.clone();
            std::thread::spawn(move || {
                match wifi_backend() {
                    "nmcli" => {
                        let _ = std::process::Command::new("nmcli")
                            .args(["connection", "delete", &ssid_str])
                            .output();
                    }
                    "iwctl" => {
                        let _ = std::process::Command::new("iwctl")
                            .args(["known-networks", &ssid_str, "forget"])
                            .output();
                    }
                    _ => {}
                }
                refresh_wifi(&st);
            });
        });
    }

    // ── Bluetooth callbacks ──

    // Toggle Bluetooth
    {
        let state_clone = state.clone();
        ui.on_net_toggle_bluetooth(move || {
            let st = state_clone.clone();
            std::thread::spawn(move || {
                let enabled = st.lock().map(|s| s.bt_enabled).unwrap_or(false);
                if enabled {
                    let _ = std::process::Command::new("bluetoothctl")
                        .args(["power", "off"])
                        .output();
                } else {
                    let _ = std::process::Command::new("bluetoothctl")
                        .args(["power", "on"])
                        .output();
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
                refresh_bluetooth(&st);
            });
        });
    }

    // Bluetooth scan
    {
        let state_clone = state.clone();
        ui.on_net_bt_scan(move || {
            let st = state_clone.clone();
            if let Ok(mut s) = st.lock() {
                s.bt_scanning = true;
                s.dirty = true;
            }
            std::thread::spawn(move || {
                let _ = std::process::Command::new("bluetoothctl")
                    .args(["scan", "on"])
                    .output();
                std::thread::sleep(std::time::Duration::from_secs(5));
                let _ = std::process::Command::new("bluetoothctl")
                    .args(["scan", "off"])
                    .output();
                refresh_bluetooth(&st);
                if let Ok(mut s) = st.lock() {
                    s.bt_scanning = false;
                    s.dirty = true;
                }
            });
        });
    }

    // Bluetooth pair
    {
        let state_clone = state.clone();
        ui.on_net_bt_pair(move |addr| {
            let addr_str = addr.to_string();
            let st = state_clone.clone();
            std::thread::spawn(move || {
                let _ = std::process::Command::new("bluetoothctl")
                    .args(["pair", &addr_str])
                    .output();
                std::thread::sleep(std::time::Duration::from_secs(2));
                refresh_bluetooth(&st);
            });
        });
    }

    // Bluetooth connect
    {
        let state_clone = state.clone();
        ui.on_net_bt_connect(move |addr| {
            let addr_str = addr.to_string();
            let st = state_clone.clone();
            std::thread::spawn(move || {
                let _ = std::process::Command::new("bluetoothctl")
                    .args(["connect", &addr_str])
                    .output();
                std::thread::sleep(std::time::Duration::from_secs(2));
                refresh_bluetooth(&st);
            });
        });
    }

    // Bluetooth disconnect
    {
        let state_clone = state.clone();
        ui.on_net_bt_disconnect(move |addr| {
            let addr_str = addr.to_string();
            let st = state_clone.clone();
            std::thread::spawn(move || {
                let _ = std::process::Command::new("bluetoothctl")
                    .args(["disconnect", &addr_str])
                    .output();
                std::thread::sleep(std::time::Duration::from_secs(1));
                refresh_bluetooth(&st);
            });
        });
    }

    // ── VPN callbacks ──

    {
        let state_clone = state.clone();
        ui.on_net_vpn_connect(move |name| {
            let name_str = name.to_string();
            let st = state_clone.clone();
            std::thread::spawn(move || {
                // Try wg-quick first
                let _ = std::process::Command::new("wg-quick")
                    .args(["up", &name_str])
                    .output();
                std::thread::sleep(std::time::Duration::from_secs(2));
                refresh_vpn(&st);
            });
        });
    }

    {
        let state_clone = state.clone();
        ui.on_net_vpn_disconnect(move |name| {
            let name_str = name.to_string();
            let st = state_clone.clone();
            std::thread::spawn(move || {
                let _ = std::process::Command::new("wg-quick")
                    .args(["down", &name_str])
                    .output();
                std::thread::sleep(std::time::Duration::from_secs(1));
                refresh_vpn(&st);
            });
        });
    }

    // ── Firewall callbacks ──

    {
        let state_clone = state.clone();
        ui.on_net_toggle_firewall(move || {
            let st = state_clone.clone();
            std::thread::spawn(move || {
                let enabled = st.lock().map(|s| s.fw_enabled).unwrap_or(false);
                let backend = fw_backend();
                if enabled {
                    // Disable
                    match backend {
                        "nft" => {
                            let _ = std::process::Command::new("nft")
                                .args(["flush", "ruleset"])
                                .output();
                        }
                        "iptables" => {
                            let _ = std::process::Command::new("iptables").args(["-P", "INPUT", "ACCEPT"]).output();
                            let _ = std::process::Command::new("iptables").args(["-P", "FORWARD", "ACCEPT"]).output();
                            let _ = std::process::Command::new("iptables").args(["-P", "OUTPUT", "ACCEPT"]).output();
                            let _ = std::process::Command::new("iptables").args(["-F"]).output();
                        }
                        _ => {}
                    }
                } else {
                    // Enable with sane defaults
                    match backend {
                        "nft" => {
                            let ruleset = "table inet filter {\n  chain input {\n    type filter hook input priority 0; policy drop;\n    ct state established,related accept\n    iif lo accept\n    icmp type echo-request accept\n    tcp dport 22 accept\n  }\n  chain forward {\n    type filter hook forward priority 0; policy drop;\n  }\n  chain output {\n    type filter hook output priority 0; policy accept;\n  }\n}\n";
                            let _ = std::process::Command::new("nft").args(["flush", "ruleset"]).output();
                            if let Ok(mut child) = std::process::Command::new("nft")
                                .args(["-f", "-"])
                                .stdin(std::process::Stdio::piped())
                                .spawn()
                            {
                                use std::io::Write;
                                if let Some(ref mut stdin) = child.stdin {
                                    let _ = stdin.write_all(ruleset.as_bytes());
                                }
                                let _ = child.wait();
                            }
                        }
                        "iptables" => {
                            let _ = std::process::Command::new("iptables").args(["-P", "INPUT", "DROP"]).output();
                            let _ = std::process::Command::new("iptables").args(["-P", "FORWARD", "DROP"]).output();
                            let _ = std::process::Command::new("iptables").args(["-P", "OUTPUT", "ACCEPT"]).output();
                            let _ = std::process::Command::new("iptables").args(["-A", "INPUT", "-m", "conntrack", "--ctstate", "ESTABLISHED,RELATED", "-j", "ACCEPT"]).output();
                            let _ = std::process::Command::new("iptables").args(["-A", "INPUT", "-i", "lo", "-j", "ACCEPT"]).output();
                            let _ = std::process::Command::new("iptables").args(["-A", "INPUT", "-p", "icmp", "-j", "ACCEPT"]).output();
                            let _ = std::process::Command::new("iptables").args(["-A", "INPUT", "-p", "tcp", "--dport", "22", "-j", "ACCEPT"]).output();
                        }
                        _ => {}
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
                refresh_firewall(&st);
            });
        });
    }

    // Firewall allow port
    {
        let state_clone = state.clone();
        ui.on_net_firewall_allow_port(move |port_str| {
            let port = port_str.to_string();
            let st = state_clone.clone();
            std::thread::spawn(move || {
                if port.parse::<u16>().is_ok() {
                    match fw_backend() {
                        "nft" => {
                            let _ = std::process::Command::new("nft")
                                .args(["add", "rule", "inet", "filter", "input", "tcp", "dport", &port, "accept"])
                                .output();
                        }
                        "iptables" => {
                            let _ = std::process::Command::new("iptables")
                                .args(["-A", "INPUT", "-p", "tcp", "--dport", &port, "-j", "ACCEPT"])
                                .output();
                        }
                        _ => {}
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
                refresh_firewall(&st);
            });
        });
    }

    // Firewall block port
    {
        let state_clone = state.clone();
        ui.on_net_firewall_block_port(move |port_str| {
            let port = port_str.to_string();
            let st = state_clone.clone();
            std::thread::spawn(move || {
                if port.parse::<u16>().is_ok() {
                    match fw_backend() {
                        "nft" => {
                            let _ = std::process::Command::new("nft")
                                .args(["add", "rule", "inet", "filter", "input", "tcp", "dport", &port, "drop"])
                                .output();
                        }
                        "iptables" => {
                            let _ = std::process::Command::new("iptables")
                                .args(["-A", "INPUT", "-p", "tcp", "--dport", &port, "-j", "DROP"])
                                .output();
                        }
                        _ => {}
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
                refresh_firewall(&st);
            });
        });
    }

    // Firewall apply profile
    {
        let state_clone = state.clone();
        ui.on_net_firewall_apply_profile(move |profile| {
            let profile_str = profile.to_string();
            let st = state_clone.clone();
            std::thread::spawn(move || {
                let backend = fw_backend();

                // Flush existing rules first
                match backend {
                    "nft" => { let _ = std::process::Command::new("nft").args(["flush", "ruleset"]).output(); }
                    "iptables" => { let _ = std::process::Command::new("iptables").args(["-F"]).output(); }
                    _ => {}
                }

                match profile_str.as_str() {
                    "home" => {
                        // Home: allow established, SSH, HTTP, HTTPS, DNS, mDNS
                        apply_base_rules(backend);
                        for port in &["80", "443", "53", "5353", "8080"] {
                            allow_port(backend, port);
                        }
                    }
                    "public" => {
                        // Public: strict, only established + SSH
                        apply_base_rules(backend);
                    }
                    "restrictive" => {
                        // Restrictive: block everything except established
                        match backend {
                            "nft" => {
                                let ruleset = "table inet filter {\n  chain input {\n    type filter hook input priority 0; policy drop;\n    ct state established,related accept\n    iif lo accept\n  }\n  chain forward {\n    type filter hook forward priority 0; policy drop;\n  }\n  chain output {\n    type filter hook output priority 0; policy accept;\n  }\n}\n";
                                if let Ok(mut child) = std::process::Command::new("nft")
                                    .args(["-f", "-"])
                                    .stdin(std::process::Stdio::piped())
                                    .spawn()
                                {
                                    use std::io::Write;
                                    if let Some(ref mut stdin) = child.stdin {
                                        let _ = stdin.write_all(ruleset.as_bytes());
                                    }
                                    let _ = child.wait();
                                }
                            }
                            "iptables" => {
                                let _ = std::process::Command::new("iptables").args(["-P", "INPUT", "DROP"]).output();
                                let _ = std::process::Command::new("iptables").args(["-P", "FORWARD", "DROP"]).output();
                                let _ = std::process::Command::new("iptables").args(["-P", "OUTPUT", "ACCEPT"]).output();
                                let _ = std::process::Command::new("iptables").args(["-A", "INPUT", "-m", "conntrack", "--ctstate", "ESTABLISHED,RELATED", "-j", "ACCEPT"]).output();
                                let _ = std::process::Command::new("iptables").args(["-A", "INPUT", "-i", "lo", "-j", "ACCEPT"]).output();
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }

                // Update profile name
                if let Ok(mut s) = st.lock() {
                    s.fw_profile = match profile_str.as_str() {
                        "home" => "Home".to_string(),
                        "public" => "Public".to_string(),
                        "restrictive" => "Restrictive".to_string(),
                        _ => "Custom".to_string(),
                    };
                }

                std::thread::sleep(std::time::Duration::from_millis(500));
                refresh_firewall(&st);
            });
        });
    }
}

/// Apply base firewall rules (established, loopback, ICMP, SSH).
fn apply_base_rules(backend: &str) {
    match backend {
        "nft" => {
            let ruleset = "table inet filter {\n  chain input {\n    type filter hook input priority 0; policy drop;\n    ct state established,related accept\n    iif lo accept\n    icmp type echo-request accept\n    tcp dport 22 accept\n  }\n  chain forward {\n    type filter hook forward priority 0; policy drop;\n  }\n  chain output {\n    type filter hook output priority 0; policy accept;\n  }\n}\n";
            if let Ok(mut child) = std::process::Command::new("nft")
                .args(["-f", "-"])
                .stdin(std::process::Stdio::piped())
                .spawn()
            {
                use std::io::Write;
                if let Some(ref mut stdin) = child.stdin {
                    let _ = stdin.write_all(ruleset.as_bytes());
                }
                let _ = child.wait();
            }
        }
        "iptables" => {
            let _ = std::process::Command::new("iptables").args(["-P", "INPUT", "DROP"]).output();
            let _ = std::process::Command::new("iptables").args(["-P", "FORWARD", "DROP"]).output();
            let _ = std::process::Command::new("iptables").args(["-P", "OUTPUT", "ACCEPT"]).output();
            let _ = std::process::Command::new("iptables").args(["-A", "INPUT", "-m", "conntrack", "--ctstate", "ESTABLISHED,RELATED", "-j", "ACCEPT"]).output();
            let _ = std::process::Command::new("iptables").args(["-A", "INPUT", "-i", "lo", "-j", "ACCEPT"]).output();
            let _ = std::process::Command::new("iptables").args(["-A", "INPUT", "-p", "icmp", "-j", "ACCEPT"]).output();
            let _ = std::process::Command::new("iptables").args(["-A", "INPUT", "-p", "tcp", "--dport", "22", "-j", "ACCEPT"]).output();
        }
        _ => {}
    }
}

/// Allow a TCP port in the firewall.
fn allow_port(backend: &str, port: &str) {
    match backend {
        "nft" => {
            let _ = std::process::Command::new("nft")
                .args(["add", "rule", "inet", "filter", "input", "tcp", "dport", port, "accept"])
                .output();
        }
        "iptables" => {
            let _ = std::process::Command::new("iptables")
                .args(["-A", "INPUT", "-p", "tcp", "--dport", port, "-j", "ACCEPT"])
                .output();
        }
        _ => {}
    }
}
