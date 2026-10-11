//! Network service contract — interfaces, connectivity, resolvers, Wi-Fi, firewall.
//!
//! # What these types exist to prevent
//!
//! Calendar's two ends drifted over parameter *names* while each file looked right on its own
//! page. Network's two ends never agreed on a method name at all. `apps/network-manager` called
//! `network.wifi_toggle`, `network.wifi_scan`, `network.wifi_connect`, `network.wifi_disconnect`
//! and `network.wifi_forget`; `services/network-service` implemented `network.interfaces`,
//! `network.status` and `network.dns` and answered everything else with "Unknown method". Not one
//! name in common. Three of the five calls were made into a `let _ =`, so the button logged that
//! it had toggled the radio and returned, and the app looked like it worked.
//!
//! The method names are constants here now, and every request and every response is a struct both
//! ends build and parse. A rename cannot land on one end alone: it stops compiling on the other.
//!
//! The response types moved here for the same reason. They lived in the service as "not in
//! contracts yet" and the app re-read them field by field out of a `serde_json::Value`, with
//! `serde_json::from_value(v).unwrap_or_default()` over the interface list — so a shape change on
//! the service side emptied the ethernet list in the window and said nothing. There is no
//! hand-written key left on either side of this wire.

use serde::{Deserialize, Serialize};

use crate::email::ServiceError;

/// The names of the network service's JSON-RPC methods.
pub mod method {
    // ── Reading ──
    pub const INTERFACES: &str = "network.interfaces";
    pub const STATUS: &str = "network.status";
    pub const DNS: &str = "network.dns";
    /// Whether this machine has a Wi-Fi adapter at all, and what its radio is doing.
    pub const WIFI_STATE: &str = "network.wifi_state";
    /// The networks saved on this machine, whether or not any of them is in range.
    pub const WIFI_KNOWN: &str = "network.wifi_known";
    /// Which firewall this machine runs, and whether it is running.
    pub const FIREWALL: &str = "network.firewall";

    // ── Changing ──
    /// Point this machine's resolvers somewhere else.
    ///
    /// Here rather than in the companion because the companion's `network_dns_set` wrote
    /// `/etc/resolv.conf` with `std::fs::write` and reported success. On a machine running
    /// NetworkManager — which is every machine this OS builds — that file is NetworkManager's,
    /// rewritten on the next carrier change or DHCP renew, so the change was either refused for
    /// want of privilege or undone without anybody being told. Resolvers belong to the connection
    /// profile, and the connection profile belongs to this service.
    pub const DNS_SET: &str = "network.dns_set";
    /// Turn the Wi-Fi radio on or off.
    pub const WIFI_RADIO: &str = "network.wifi_radio";
    /// Ask the adapter to look for access points, then read the list back.
    pub const WIFI_SCAN: &str = "network.wifi_scan";
    pub const WIFI_CONNECT: &str = "network.wifi_connect";
    pub const WIFI_DISCONNECT: &str = "network.wifi_disconnect";
    /// Delete a saved network, so this machine stops joining it by itself.
    pub const WIFI_FORGET: &str = "network.wifi_forget";
}

// ══════════════════════════════════════════════════════════════════════
// Requests
// ══════════════════════════════════════════════════════════════════════

/// Parameters for [`method::WIFI_RADIO`].
///
/// One field, and it is the whole state rather than a flip. `wifi_toggle` used to send
/// `{"enabled": !current}` computed from a UI property the service had never written, so the app
/// asked to turn on a radio that was already on as often as not.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiRadioParams {
    pub enabled: bool,
}

/// Parameters for [`method::WIFI_SCAN`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WifiScanParams {
    /// Ask the adapter to sweep the band before reading the list.
    ///
    /// `false` reads NetworkManager's cache, which is instant and may be minutes old. The app's
    /// Scan button sends `true`; a periodic refresh sends `false`, because rescanning every three
    /// seconds would keep the radio off the air.
    #[serde(default)]
    pub rescan: bool,
}

/// Parameters for [`method::WIFI_CONNECT`].
///
/// The password is `Option`, not `String`: a saved network and an open network are both joined
/// without one, and an empty string meant both "no password" and "the user cleared the box".
///
/// This struct is the only place a password appears in this crate, and nothing derives `Debug`
/// output that would print it — see the manual `Debug` below.
#[derive(Clone, Serialize, Deserialize)]
pub struct WifiConnectParams {
    pub ssid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

/// Written by hand so that a `{:?}` anywhere — a log line, a trace, an error built with
/// `format!("{params:?}")` — cannot put the password in a file.
impl std::fmt::Debug for WifiConnectParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WifiConnectParams")
            .field("ssid", &self.ssid)
            .field(
                "password",
                &if self.password.is_some() { "<given>" } else { "<none>" },
            )
            .finish()
    }
}

/// Parameters for [`method::WIFI_FORGET`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiForgetParams {
    /// The network name as [`KnownNetwork::ssid`] gives it.
    pub ssid: String,
}

/// Parameters for [`method::DNS_SET`].
///
/// One list rather than `primary` and `secondary`. The tool this replaces took those two names
/// and wrote two `nameserver` lines, which meant a machine that wanted three resolvers could not
/// say so and a machine that wanted one had to leave a field blank that also meant "unchanged".
/// A list of one is one resolver; an empty list is a request with nothing in it and is refused
/// rather than read as "clear them", because clearing the resolvers of the connection carrying
/// the default route is not something to do by omission.
///
/// Addresses are validated as addresses by the service before nmcli sees them. IPv4 and IPv6 are
/// both accepted and are applied to `ipv4.dns` and `ipv6.dns` respectively — a family with no
/// server in this list is left exactly as it was.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DnsSetParams {
    pub servers: Vec<String>,
}

// ══════════════════════════════════════════════════════════════════════
// Responses
// ══════════════════════════════════════════════════════════════════════

/// One network interface, as the service reads it out of `/proc` and `/sys`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkInterfaceInfo {
    pub name: String,
    pub mac_address: String,
    pub ip_address: Option<String>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    /// The kernel's own `operstate`: `up`, `down`, `unknown`.
    pub state: String,
    pub conn_type: ConnectionType,
    /// Link speed in Mbit/s, from `/sys/class/net/<if>/speed`. `None` when the driver does not
    /// report one — virtio answers -1 or refuses the read — and never a guessed figure.
    ///
    /// The three fields below are `default` so a service from before them still parses: its
    /// rows arrive with all three unknown, which is what that service knew.
    #[serde(default)]
    pub speed_mbps: Option<u32>,
    /// The address's netmask, as `255.255.255.0`: the same form as [`WifiState::subnet`], so the
    /// two panes' "Subnet" fields read alike. `None` with no address.
    #[serde(default)]
    pub subnet: Option<String>,
    /// The router this interface's default route goes through. `None` for an interface that
    /// carries no default route.
    #[serde(default)]
    pub gateway: Option<String>,
}

/// Overall connectivity: the one-line "am I online, and how".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkStatus {
    pub connected: bool,
    #[serde(rename = "type")]
    pub conn_type: String,
    pub ssid: Option<String>,
    pub ip_address: Option<String>,
}

/// The machine's resolvers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DnsConfig {
    pub nameservers: Vec<String>,
    pub search_domains: Vec<String>,
}

/// What [`method::DNS_SET`] observed after it changed the resolvers.
///
/// Two readings and not one. `/etc/resolv.conf` is what the C library resolves through and is the
/// thing a person means by "my DNS"; NetworkManager's own `IP4.DNS` for the device is what it
/// believes it applied. They agree on a plain NetworkManager machine, which is what this OS
/// builds. They do not agree on a machine running a stub resolver — `resolv.conf` there says
/// `127.0.0.53` and the real servers are only in the second reading — and a check written against
/// the first alone would call a change that worked a failure. Both are reported, and the service
/// accepts the change only if the servers asked for turn up in one of them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DnsSetResult {
    /// The connection profile whose resolvers were changed, by the name nmcli lists it under.
    pub connection: String,
    /// The interface that profile is up on.
    pub device: String,
    /// `/etc/resolv.conf`, re-read afterwards.
    pub resolv_conf: DnsConfig,
    /// `IP4.DNS` and `IP6.DNS` for the device, re-read afterwards, in NetworkManager's order.
    pub device_dns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionType {
    Ethernet,
    Wifi,
    Vpn,
    Bridge,
    Other(String),
}

impl Default for ConnectionType {
    fn default() -> Self {
        ConnectionType::Other(String::new())
    }
}

impl ConnectionType {
    /// The short word the rest of the UI and `describe` use.
    pub fn as_str(&self) -> &'static str {
        match self {
            ConnectionType::Wifi => "wifi",
            ConnectionType::Ethernet => "ethernet",
            ConnectionType::Vpn => "vpn",
            ConnectionType::Bridge => "bridge",
            ConnectionType::Other(_) => "other",
        }
    }
}

/// What the Wi-Fi radio is doing, when there is a radio.
///
/// Three states and not a `bool`, because the `bool` was the bug. `wifi-enabled` was an unset
/// Slint property that defaulted to `false`, and the window drew "Wi-Fi: Off" on a machine that
/// has no Wi-Fi adapter in it — a measurement nobody had taken, rendered as a fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RadioState {
    On,
    Off,
    /// The adapter is there but its state could not be read; [`WifiState::reason`] says why.
    Unknown,
}

impl RadioState {
    pub fn as_str(&self) -> &'static str {
        match self {
            RadioState::On => "on",
            RadioState::Off => "off",
            RadioState::Unknown => "unknown",
        }
    }
}

/// The Wi-Fi half of this machine, including the case where there isn't one.
///
/// `adapter_present: false` is the answer for the wired test machine, and it is a different
/// statement from `radio: Off`. A caller must be able to tell "this machine cannot do Wi-Fi" from
/// "the radio is switched off", because the second invites a button press and the first does not.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiState {
    pub adapter_present: bool,
    /// The interface name, `wlan0` or `wlp3s0`, when there is one.
    pub device: Option<String>,
    pub radio: RadioState,
    /// The SSID this machine is joined to, or `None`. Never `""` — an empty SSID is a hidden
    /// network's, and reporting it as "not connected" would be wrong.
    pub connected_ssid: Option<String>,
    /// 0-100, as NetworkManager reports it. `None` when not connected or not readable.
    pub signal: Option<i32>,
    /// Link rate for the current association, as nmcli words it (`270 Mbit/s`).
    pub rate: Option<String>,
    pub ip_address: Option<String>,
    pub gateway: Option<String>,
    /// The CIDR prefix of [`Self::ip_address`], as `255.255.255.0`.
    pub subnet: Option<String>,
    /// Why the answer above is as thin as it is: no adapter, nmcli absent, NetworkManager down,
    /// polkit refused. `None` when nothing is wrong. Never a sentence when nothing is wrong.
    pub reason: Option<String>,
}

impl Default for WifiState {
    fn default() -> Self {
        Self {
            adapter_present: false,
            device: None,
            radio: RadioState::Unknown,
            connected_ssid: None,
            signal: None,
            rate: None,
            ip_address: None,
            gateway: None,
            subnet: None,
            reason: None,
        }
    }
}

/// One access point a scan found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScannedNetwork {
    /// The network name. Empty for a hidden network, which is reported as hidden rather than
    /// dropped: a list that silently omits rows is how "there is nothing here" gets said wrongly.
    pub ssid: String,
    pub hidden: bool,
    /// 0-100.
    pub signal: i32,
    /// `WPA2`, `WPA3`, `--` for an open network, as nmcli words it.
    pub security: String,
    pub rate: String,
    pub is_connected: bool,
    /// Whether this machine already has a saved connection for this SSID.
    pub is_saved: bool,
}

/// One saved connection — a network this machine will join by itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownNetwork {
    /// The connection's name, which for a Wi-Fi connection NetworkManager makes is the SSID.
    pub ssid: String,
    pub uuid: String,
    /// Whether this saved connection is the one currently up.
    pub is_active: bool,
}

/// What [`method::WIFI_FORGET`] observed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiForgetResult {
    pub forgotten: String,
    /// The saved list re-read after the delete, so a caller can check the network is gone from it
    /// rather than take the exit status for an answer.
    pub known: Vec<KnownNetwork>,
}

/// Which firewall this machine has, and what it is doing — or why that is not knowable.
///
/// The property this replaces was `in property <bool> firewall-enabled: false`, never written
/// from Rust, drawn on screen as "Off" in warning colour. A security audit recorded "Firewall:
/// Off" as a finding. Nothing had ever looked.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirewallState {
    /// `nftables`, `ufw`, `firewalld`, or `None` when no firewall tool is installed.
    pub kind: Option<String>,
    pub state: FirewallStatus,
    /// How many rules are loaded. `None` whenever the ruleset could not be read — including when
    /// the firewall is known to be active, because "active with an unknown ruleset" is a real
    /// state and reporting it as zero rules would be the old bug wearing a number.
    pub rule_count: Option<i64>,
    pub rules: Vec<FirewallRuleInfo>,
    /// Why the state is `Unknown`, or why the rules are absent. `None` when nothing is wrong.
    pub reason: Option<String>,
}

impl Default for FirewallState {
    fn default() -> Self {
        Self {
            kind: None,
            state: FirewallStatus::Unknown,
            rule_count: None,
            rules: Vec::new(),
            reason: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FirewallStatus {
    /// A firewall is installed and filtering.
    Active,
    /// A firewall is installed and is not filtering. Only said when that was actually read.
    Inactive,
    /// No firewall tool on this machine at all. Different from `Inactive`, which claims a tool
    /// looked and found nothing loaded.
    Absent,
    /// It could not be determined. [`FirewallState::reason`] says why, and there is always a
    /// reason: this variant is never returned bare.
    Unknown,
}

impl FirewallStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            FirewallStatus::Active => "active",
            FirewallStatus::Inactive => "inactive",
            FirewallStatus::Absent => "absent",
            FirewallStatus::Unknown => "unknown",
        }
    }
}

/// One rule, as the firewall's own listing words it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirewallRuleInfo {
    /// The chain or nftables chain name this rule sits in.
    pub chain: String,
    /// `accept`, `drop`, `reject`, or whatever verdict the tool printed.
    pub action: String,
    /// The rule, in the tool's own words. Not reworded: a person comparing this against
    /// `nft list ruleset` has to see the same text.
    pub text: String,
}

/// Network service operations, as a trait for an in-process implementation.
///
/// The wire is the method constants above; this is the same surface for a caller that holds the
/// service rather than a socket. Kept in step with the constants by hand — there is no caller of
/// it today, and the day there is, the compiler will not be the thing that notices a gap.
pub trait NetworkService: Send + Sync {
    fn interfaces(&self) -> Result<Vec<NetworkInterfaceInfo>, ServiceError>;
    fn status(&self) -> Result<NetworkStatus, ServiceError>;
    fn dns(&self) -> Result<DnsConfig, ServiceError>;
    fn dns_set(&self, params: &DnsSetParams) -> Result<DnsSetResult, ServiceError>;
    fn wifi_state(&self) -> Result<WifiState, ServiceError>;
    fn wifi_known(&self) -> Result<Vec<KnownNetwork>, ServiceError>;
    fn wifi_scan(&self, params: &WifiScanParams) -> Result<Vec<ScannedNetwork>, ServiceError>;
    fn wifi_radio(&self, params: &WifiRadioParams) -> Result<WifiState, ServiceError>;
    fn wifi_connect(&self, params: &WifiConnectParams) -> Result<WifiState, ServiceError>;
    fn wifi_disconnect(&self) -> Result<WifiState, ServiceError>;
    fn wifi_forget(&self, params: &WifiForgetParams) -> Result<WifiForgetResult, ServiceError>;
    fn firewall(&self) -> Result<FirewallState, ServiceError>;
}
