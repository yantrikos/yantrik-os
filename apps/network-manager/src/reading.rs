//! What one refresh learns from network-service, and the words the window draws it in.
//!
//! Every value this screen renders as a fact comes from here, from something that was read. The
//! ones that could not be read keep the value that means unknown — `-1` for the firewall rule
//! count, `"unknown"` for the radio — never a plausible default. Pure but for the six calls, so
//! the job that runs it is the only thing that waits on the service.

use yantrik_ipc_contracts::network::{
    method, ConnectionType, DnsConfig, FirewallState, KnownNetwork, NetworkInterfaceInfo,
    NetworkStatus, WifiState,
};

use crate::backend::{call, NetworkApi};

/// What the last refresh learned.
///
/// `describe` has to answer questions a string on screen cannot: whether the service answered at
/// all, and whether a value is unknown or merely empty. A string property is `""` for both, and
/// `""` read as a measurement is the disease this app was found with.
#[derive(Debug, Clone, Default)]
pub struct Reading {
    /// Whether the refresh reached network-service.
    pub service_ok: bool,
    /// What it said when it did not.
    pub service_error: Option<String>,
    /// Unix seconds of the refresh, when it reached the service.
    pub read_at: Option<u64>,
    pub dns: DnsConfig,
    /// The resolvers as one line (`wifi-dns`); empty when they could not be read.
    pub resolvers: String,
    /// The wired interfaces, in the service's order.
    pub ethernet: Vec<NetworkInterfaceInfo>,
    pub status: Status,
    pub wifi: WifiState,
    /// Whether `network.wifi_state` answered. When it did not, the Wi-Fi link details on screen
    /// (address, rate, gateway, mask) are left as they were, as they always have been.
    pub wifi_answered: bool,
    pub known: Vec<KnownNetwork>,
    pub firewall: FirewallState,
    /// The kernel's route table, read with the readings taken before the window chose its
    /// opening section, and not after.
    pub route_table: Option<String>,
}

/// The header's one line: whether this machine is online, how, and at what address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// `connected` or `disconnected`.
    pub state: String,
    /// `WiFi`, `Ethernet`, empty for none, or the service's own word for anything else.
    pub kind: String,
    pub ip: String,
}

impl Default for Status {
    /// What the window shows before anything was read, and when the status could not be.
    fn default() -> Status {
        Status {
            state: "disconnected".into(),
            kind: String::new(),
            ip: String::new(),
        }
    }
}

impl Status {
    pub fn connected(&self) -> bool {
        self.state == "connected"
    }

    pub fn is_wifi(&self) -> bool {
        self.kind == "WiFi"
    }
}

impl Reading {
    /// The status bar's signal meter: the Wi-Fi one or nothing. An ethernet link has no signal
    /// strength, and drawing four empty bars for it is a reading nobody took.
    pub fn status_signal(&self) -> i32 {
        if self.status.is_wifi() {
            self.wifi.signal.unwrap_or(0)
        } else {
            0
        }
    }

    /// The state of the machine, said on screen. A command's own verdict is more specific and is
    /// written over it when one lands.
    pub fn notice(&self) -> String {
        match &self.service_error {
            Some(e) => format!("the network service did not answer: {e}"),
            None => String::new(),
        }
    }

    /// The sentence a Wi-Fi action is refused with on a machine with no adapter. It names the
    /// absence rather than the SSID, because "no network called X was found" on a machine that
    /// cannot look for one at all is the wrong thing to tell somebody.
    pub fn no_adapter(&self) -> String {
        self.wifi
            .reason
            .clone()
            .unwrap_or_else(|| "this machine has no Wi-Fi adapter".to_string())
    }
}

/// Six calls, in the order the window has always made them, each failure kept in words.
pub fn read(api: &dyn NetworkApi, with_route_table: bool) -> Reading {
    let mut reading = Reading::default();
    let none = || serde_json::json!({});

    // The resolvers. The property is called wifi-dns because that pane was built first; the
    // resolvers are the machine's, not the radio's. Read before the interfaces, whose DNS field
    // is the same list.
    match call::<DnsConfig>(api, method::DNS, none()) {
        Ok(dns) => {
            reading.resolvers = dns.nameservers.join(", ");
            reading.dns = dns;
        }
        Err(e) => {
            reading.service_error.get_or_insert(e);
        }
    }

    // The interfaces: the ethernet list, and with it whether there is an interface at all.
    match call::<Vec<NetworkInterfaceInfo>>(api, method::INTERFACES, none()) {
        Ok(rows) => {
            reading.service_ok = true;
            reading.read_at = Some(now_secs());
            reading.ethernet = rows
                .into_iter()
                .filter(|i| matches!(i.conn_type, ConnectionType::Ethernet))
                .collect();
        }
        Err(e) => {
            // The list stays empty deliberately and the notice says why. Leaving the last good
            // list on screen would present a stale reading as a current one.
            reading.service_error = Some(e);
        }
    }

    // The summary line the header shows.
    match call::<NetworkStatus>(api, method::STATUS, none()) {
        Ok(status) => {
            reading.service_ok = true;
            reading.status = Status {
                state: if status.connected { "connected" } else { "disconnected" }.into(),
                kind: match status.conn_type.as_str() {
                    "wifi" => "WiFi",
                    "ethernet" => "Ethernet",
                    "none" => "",
                    other => other,
                }
                .into(),
                ip: status.ip_address.unwrap_or_default(),
            };
        }
        Err(e) => {
            reading.service_error.get_or_insert(e);
        }
    }

    // Wi-Fi: the three facts that replaced `wifi-enabled`, and everything that depends on there
    // being a radio.
    match call::<WifiState>(api, method::WIFI_STATE, none()) {
        Ok(wifi) => {
            reading.wifi = wifi;
            reading.wifi_answered = true;
        }
        Err(e) => {
            reading.service_error.get_or_insert(e.clone());
            reading.wifi = WifiState {
                reason: Some(e),
                ..WifiState::default()
            };
        }
    }

    // The saved networks. Read whether or not the radio is on: this machine's list of networks
    // it will join is a fact about the machine, not about what is in range.
    match call::<Vec<KnownNetwork>>(api, method::WIFI_KNOWN, none()) {
        Ok(known) => reading.known = known,
        Err(e) => {
            reading.service_error.get_or_insert(e);
        }
    }

    match call::<FirewallState>(api, method::FIREWALL, none()) {
        Ok(fw) => reading.firewall = fw,
        Err(e) => {
            reading.service_error.get_or_insert(e.clone());
            reading.firewall = FirewallState {
                reason: Some(e),
                ..FirewallState::default()
            };
        }
    }

    if with_route_table {
        reading.route_table = Some(api.route_table());
    }
    reading
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A link speed in the words nmcli uses for a Wi-Fi rate (`1000 Mbit/s`), so the two panes'
/// "Speed" read alike; empty when the link reports none.
pub fn speed_text(mbps: Option<u32>) -> String {
    mbps.map(|n| format!("{n} Mbit/s")).unwrap_or_default()
}

/// A drawn string as JSON, with the empty string as `null`.
///
/// System Monitor's lesson, one app over: an empty `cpu_model` and an empty `ip` read to an
/// auditor as measurements of an empty thing. `null` is the value that means "this app does not
/// know", and `""` is not it.
pub fn text_or_null(text: &str) -> serde_json::Value {
    if text.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::Value::String(text.to_string())
    }
}

/// One wired interface as the Ethernet pane draws it, and as `describe` reports what it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EthernetRow {
    pub name: String,
    pub status: String,
    pub ip_address: String,
    pub mac_address: String,
    /// What the service could not read stays empty, and the window says "not reported" for it:
    /// an empty field reads as unknown, a made-up one as fact.
    pub speed: String,
    pub subnet: String,
    pub gateway: String,
    /// The machine's resolvers: there is one list, whichever link carries a lookup.
    pub dns: String,
}

impl Reading {
    pub fn ethernet_rows(&self) -> Vec<EthernetRow> {
        let dns = self.dns.nameservers.join(", ");
        self.ethernet
            .iter()
            .map(|i| EthernetRow {
                name: i.name.clone(),
                status: i.state.to_uppercase(),
                ip_address: i.ip_address.clone().unwrap_or_default(),
                mac_address: i.mac_address.clone(),
                speed: speed_text(i.speed_mbps),
                subnet: i.subnet.clone().unwrap_or_default(),
                gateway: i.gateway.clone().unwrap_or_default(),
                dns: dns.clone(),
            })
            .collect()
    }
}
