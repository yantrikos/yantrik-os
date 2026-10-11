//! What the machine knows about its network, as plain data — no D-Bus in here.
//!
//! `network.rs` reads NetworkManager and fills a [`NetworkSnapshot`]; everything that decides
//! what a person is told (which mark the bar draws, how many bars, what the tooltip says, which
//! networks the popover lists and in what order) is a pure function of that snapshot, so it is
//! tested here without a bus. The bar, the popover, the System screen and `describe shell` all
//! read the one snapshot, which is how they stop disagreeing: the shell used to draw a Wi-Fi mark
//! over NetworkManager's "Wired connection 1" on a machine with no radio in it.
//!
//! The Wi-Fi secret has one type, [`WifiSecret`], and one way in, [`ConnectRequest`]. Neither
//! prints its contents under `{:?}`, and the secret is overwritten when it is dropped. See the
//! tests at the bottom for the failures that matters for: a request in a log line, an error built
//! with `format!("{request:?}")`.

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// What carries the traffic. `None` is "nothing is up", which is a different statement from a
/// wired machine whose cable is out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetKind {
    Wired,
    Wifi,
    None,
}

impl NetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NetKind::Wired => "wired",
            NetKind::Wifi => "wifi",
            NetKind::None => "none",
        }
    }
}

/// The link, in words. `Connecting` is its own state: a person who just pressed Connect should see
/// that something is happening, not the old network for a few seconds and then the new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetState {
    Connected,
    Connecting,
    Disconnected,
}

impl NetState {
    pub fn as_str(self) -> &'static str {
        match self {
            NetState::Connected => "connected",
            NetState::Connecting => "connecting",
            NetState::Disconnected => "disconnected",
        }
    }
}

/// NetworkManager's own connectivity check, which is the only thing that can say "linked but the
/// internet is not there". `Unknown` when it has not checked or checking is off, and then the
/// shell says nothing about the internet rather than guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Connectivity {
    Full,
    /// Linked, but the check could not reach the internet.
    Limited,
    /// Behind a login page.
    Portal,
    None,
    Unknown,
}

impl Connectivity {
    /// `NMConnectivityState`: 0 unknown, 1 none, 2 portal, 3 limited, 4 full.
    pub fn from_nm(code: u32) -> Self {
        match code {
            1 => Connectivity::None,
            2 => Connectivity::Portal,
            3 => Connectivity::Limited,
            4 => Connectivity::Full,
            _ => Connectivity::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Connectivity::Full => "full",
            Connectivity::Limited => "limited",
            Connectivity::Portal => "portal",
            Connectivity::None => "none",
            Connectivity::Unknown => "unknown",
        }
    }
}

/// One visible Wi-Fi network. Several access points sharing a name are folded into one row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessPoint {
    pub ssid: String,
    /// 0-100, as NetworkManager reports it.
    pub strength: u8,
    /// Needs a key to join.
    pub secured: bool,
    /// A saved NetworkManager profile exists for this name, so joining needs no secret.
    pub known: bool,
    /// This machine is on it now.
    pub connected: bool,
    /// Needs a username and a certificate (802.1X). The popover cannot join one with a single
    /// password field, and says so rather than send a wrong kind of secret.
    pub enterprise: bool,
}

/// Everything the shell shows about the network, from one reading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkSnapshot {
    pub kind: NetKind,
    pub state: NetState,
    /// The network joined, for Wi-Fi. Never a wired connection's profile name.
    pub ssid: Option<String>,
    /// 0-100 for the joined access point; `None` unless on Wi-Fi.
    pub strength: Option<u8>,
    pub ip: Option<String>,
    /// A VPN is up over whatever is carrying it.
    pub vpn: bool,
    pub connectivity: Connectivity,
    /// There is a Wi-Fi device at all. A machine without one gets no Wi-Fi row, no radio tile.
    pub wifi_present: bool,
    /// The radio is switched on. Meaningless when `wifi_present` is false.
    pub radio_on: bool,
    /// Visible networks, joined one first, then strongest. Empty when there is no Wi-Fi device.
    pub access_points: Vec<AccessPoint>,
    /// How many wired or Wi-Fi connections are up. One means leaving it leaves the machine with
    /// no network at all, which is what grades `disconnect_network` and `set_wifi` off.
    pub links: u8,
}

impl Default for NetworkSnapshot {
    fn default() -> Self {
        NetworkSnapshot {
            kind: NetKind::None,
            state: NetState::Disconnected,
            ssid: None,
            strength: None,
            ip: None,
            vpn: false,
            connectivity: Connectivity::Unknown,
            wifi_present: false,
            radio_on: false,
            access_points: Vec::new(),
            links: 0,
        }
    }
}

/// What the bar draws for the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// Nothing is up.
    Offline,
    Wired,
    /// 1 to 4 bars lit.
    Wifi(u8),
    Vpn,
}

/// Signal as bars. A joined network always shows at least one: zero bars would read as "not
/// connected" on a link that is up.
pub fn bars(strength: u8) -> u8 {
    match strength {
        75..=u8::MAX => 4,
        50..=74 => 3,
        25..=49 => 2,
        _ => 1,
    }
}

impl NetworkSnapshot {
    /// Online in the sense the shell has always meant by it: something is up and carrying traffic.
    /// True on a wired machine, which is why nothing that means "am I online" may read the
    /// Wi-Fi flag.
    pub fn online(&self) -> bool {
        self.state == NetState::Connected
    }

    /// Taking down the connection that carries traffic would leave the machine with none: the one
    /// case where disconnecting is more than a reversible convenience, because every mind that
    /// reaches the machine over the network is cut off with it.
    pub fn is_only_connection(&self) -> bool {
        self.online() && self.links <= 1
    }

    /// Joined, but NetworkManager's check says the internet is not reachable.
    pub fn no_internet(&self) -> bool {
        self.online() && matches!(self.connectivity, Connectivity::Limited | Connectivity::None)
    }

    /// Behind a captive portal: the person has to sign in somewhere before anything works.
    pub fn portal(&self) -> bool {
        self.online() && self.connectivity == Connectivity::Portal
    }

    /// The mark. A VPN outranks the medium under it: that is the fact a person reaches for the
    /// glyph to check.
    pub fn mark(&self) -> Mark {
        if !self.online() {
            return Mark::Offline;
        }
        if self.vpn {
            return Mark::Vpn;
        }
        match self.kind {
            NetKind::Wifi => Mark::Wifi(bars(self.strength.unwrap_or(0))),
            NetKind::Wired => Mark::Wired,
            NetKind::None => Mark::Offline,
        }
    }

    /// The medium's word for the System screen and Settings: `wifi`, `ethernet`, or empty.
    pub fn medium(&self) -> &'static str {
        match self.kind {
            NetKind::Wifi => "wifi",
            NetKind::Wired => "ethernet",
            NetKind::None => "",
        }
    }

    /// One line for the tooltip: the name, the address and the state in words.
    pub fn tooltip(&self) -> String {
        let how = match self.kind {
            NetKind::Wifi => match self.ssid.as_deref().filter(|s| !s.is_empty()) {
                Some(ssid) => format!("Wi-Fi {ssid}"),
                None => "Wi-Fi".to_string(),
            },
            NetKind::Wired => "Wired".to_string(),
            NetKind::None => String::new(),
        };
        let mut line = match self.state {
            NetState::Disconnected => {
                return if self.wifi_present && !self.radio_on {
                    "Offline, Wi-Fi is off".to_string()
                } else {
                    "Offline".to_string()
                };
            }
            NetState::Connecting => {
                return match self.ssid.as_deref().filter(|s| !s.is_empty()) {
                    Some(ssid) => format!("Connecting to {ssid}"),
                    None => "Connecting".to_string(),
                };
            }
            NetState::Connected => how,
        };
        if line.is_empty() {
            line.push_str("Connected");
        } else {
            line.push_str(", connected");
        }
        if self.portal() {
            line.push_str(", sign-in needed");
        } else if self.no_internet() {
            line.push_str(", no internet");
        }
        if self.vpn {
            line.push_str(", VPN up");
        }
        if let Some(ip) = self.ip.as_deref().filter(|ip| !ip.is_empty()) {
            line.push_str(", ");
            line.push_str(ip);
        }
        line
    }

    /// Fold access points that share a name into one row, put the joined network first and the
    /// rest strongest-first, and drop hidden networks (no name to show or to join).
    pub fn arrange(mut points: Vec<AccessPoint>) -> Vec<AccessPoint> {
        points.retain(|p| !p.ssid.is_empty());
        let mut folded: Vec<AccessPoint> = Vec::new();
        for p in points {
            match folded.iter_mut().find(|f| f.ssid == p.ssid) {
                Some(f) => {
                    f.strength = f.strength.max(p.strength);
                    f.known |= p.known;
                    f.connected |= p.connected;
                    f.secured |= p.secured;
                    f.enterprise |= p.enterprise;
                }
                None => folded.push(p),
            }
        }
        folded.sort_by(|a, b| {
            b.connected
                .cmp(&a.connected)
                .then(b.strength.cmp(&a.strength))
                .then(a.ssid.cmp(&b.ssid))
        });
        folded
    }

    /// The visible network with this name, if any.
    pub fn access_point(&self, ssid: &str) -> Option<&AccessPoint> {
        self.access_points.iter().find(|p| p.ssid == ssid)
    }
}

/// A Wi-Fi secret. It is overwritten when dropped and says `<secret>` however it is printed.
///
/// Held for as long as one connection attempt and not a moment longer: the popover's field hands
/// it over, the attempt consumes it, and what remains in memory is zeros.
pub struct WifiSecret(String);

impl WifiSecret {
    pub fn new(secret: String) -> Self {
        WifiSecret(secret)
    }

    /// The text, for the one place that puts it in the D-Bus message. Not for logging: nothing
    /// that formats a request should call this.
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Drop for WifiSecret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for WifiSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<secret>")
    }
}

/// A request to join a network. `Debug` shows the name and whether a secret came with it, never
/// the secret. Deliberately not `Clone` and not `Serialize`: a secret that cannot be copied or
/// written out is held in exactly one place.
pub struct ConnectRequest {
    pub ssid: String,
    pub secret: Option<WifiSecret>,
    /// A person pressed Join on this network. Only then may a profile made for an OPEN network
    /// autoconnect: an open network is the easiest one to impersonate, and a profile saved for an
    /// attacker's access point that the machine rejoins by itself is how a join becomes permanent.
    pub by_person: bool,
}

impl std::fmt::Debug for ConnectRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectRequest")
            .field("ssid", &self.ssid)
            .field("secret", &if self.secret.is_some() { "<given>" } else { "<none>" })
            .field("by_person", &self.by_person)
            .finish()
    }
}

/// Why a request was turned away before anything was sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The network is not in range.
    NotVisible,
    /// 802.1X: needs more than one password.
    Enterprise,
    /// A secured network this machine has no profile for, and no secret came.
    NeedsSecret,
    /// A WPA passphrase is 8 to 63 characters (or 64 hex digits for a raw key).
    BadSecretLength,
}

impl Refusal {
    /// Words for a person. None of them contains the secret or its length.
    pub fn say(&self, ssid: &str) -> String {
        match self {
            Refusal::NotVisible => format!("{ssid} is not in range"),
            Refusal::Enterprise => format!(
                "{ssid} is a work or school network that needs a username and certificate; set it up in Network settings"
            ),
            Refusal::NeedsSecret => format!("{ssid} needs a password"),
            Refusal::BadSecretLength => "Wi-Fi passwords are 8 to 63 characters".to_string(),
        }
    }
}

/// How a request will be carried out, decided from what is visible and what is saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// A saved profile exists: ask NetworkManager to activate it. No secret travels.
    UseSaved,
    /// Open network, no profile: create one with no security section.
    JoinOpen,
    /// Secured, no profile, a secret in hand: create a profile carrying it.
    JoinSecured { key_mgmt: &'static str },
}

/// A passphrase NetworkManager will accept: 8 to 63 characters, or exactly 64 hex digits.
pub fn secret_length_ok(secret: &str) -> bool {
    let n = secret.chars().count();
    (8..=63).contains(&n) || (n == 64 && secret.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Decide what to do with a request. Pure: nothing is sent, and the secret is only measured.
///
/// `rsn_sae_only` is true when the access point offers WPA3-SAE and not WPA2-PSK, which needs a
/// different key-management word.
pub fn plan(
    snapshot: &NetworkSnapshot,
    request: &ConnectRequest,
    sae_only: bool,
) -> Result<Plan, Refusal> {
    let ap = snapshot.access_point(&request.ssid).ok_or(Refusal::NotVisible)?;
    if ap.known {
        return Ok(Plan::UseSaved);
    }
    if ap.enterprise {
        return Err(Refusal::Enterprise);
    }
    if !ap.secured {
        return Ok(Plan::JoinOpen);
    }
    match request.secret.as_ref() {
        None => Err(Refusal::NeedsSecret),
        Some(s) if !secret_length_ok(s.expose()) => Err(Refusal::BadSecretLength),
        Some(_) => Ok(Plan::JoinSecured { key_mgmt: if sae_only { "sae" } else { "wpa-psk" } }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ap(ssid: &str, strength: u8, secured: bool, known: bool, connected: bool) -> AccessPoint {
        AccessPoint { ssid: ssid.into(), strength, secured, known, connected, enterprise: false }
    }

    fn laptop() -> NetworkSnapshot {
        NetworkSnapshot {
            kind: NetKind::Wifi,
            state: NetState::Connected,
            ssid: Some("Home".into()),
            strength: Some(80),
            ip: Some("192.168.1.20".into()),
            connectivity: Connectivity::Full,
            wifi_present: true,
            radio_on: true,
            access_points: NetworkSnapshot::arrange(vec![
                ap("Cafe", 40, true, false, false),
                ap("Home", 80, true, true, true),
                ap("Guest", 90, false, false, false),
            ]),
            ..NetworkSnapshot::default()
        }
    }

    #[test]
    fn bars_run_one_to_four_and_a_live_link_is_never_zero_bars() {
        assert_eq!(bars(0), 1, "a joined network with no reading still shows one bar");
        assert_eq!(bars(24), 1);
        assert_eq!(bars(25), 2);
        assert_eq!(bars(49), 2);
        assert_eq!(bars(50), 3);
        assert_eq!(bars(74), 3);
        assert_eq!(bars(75), 4);
        assert_eq!(bars(100), 4);
    }

    #[test]
    fn the_mark_names_the_medium_and_a_vpn_outranks_it() {
        assert_eq!(laptop().mark(), Mark::Wifi(4));
        let wired = NetworkSnapshot { kind: NetKind::Wired, state: NetState::Connected, ..NetworkSnapshot::default() };
        assert_eq!(wired.mark(), Mark::Wired);
        let vpn = NetworkSnapshot { vpn: true, ..laptop() };
        assert_eq!(vpn.mark(), Mark::Vpn);
        assert_eq!(NetworkSnapshot::default().mark(), Mark::Offline);
        // A VPN profile that is up with nothing under it carrying traffic is not "online".
        let dangling = NetworkSnapshot { vpn: true, ..NetworkSnapshot::default() };
        assert_eq!(dangling.mark(), Mark::Offline);
    }

    /// The bug the shell shipped with: a wired machine drew the Wi-Fi arcs because the profile
    /// was called "Wired connection 1" and something took any connection name for an SSID.
    #[test]
    fn a_wired_machine_has_no_ssid_and_no_wifi_row() {
        let wired = NetworkSnapshot {
            kind: NetKind::Wired,
            state: NetState::Connected,
            ip: Some("192.168.4.30".into()),
            connectivity: Connectivity::Full,
            ..NetworkSnapshot::default()
        };
        assert_eq!(wired.medium(), "ethernet");
        assert!(wired.ssid.is_none());
        assert!(!wired.wifi_present);
        assert!(wired.access_points.is_empty());
        assert_eq!(wired.tooltip(), "Wired, connected, 192.168.4.30");
    }

    #[test]
    fn the_tooltip_says_the_name_the_state_and_the_address_in_words() {
        assert_eq!(laptop().tooltip(), "Wi-Fi Home, connected, 192.168.1.20");
        let limited = NetworkSnapshot { connectivity: Connectivity::Limited, ..laptop() };
        assert_eq!(limited.tooltip(), "Wi-Fi Home, connected, no internet, 192.168.1.20");
        assert!(limited.no_internet());
        let portal = NetworkSnapshot { connectivity: Connectivity::Portal, ..laptop() };
        assert!(portal.tooltip().contains("sign-in needed"));
        assert_eq!(NetworkSnapshot::default().tooltip(), "Offline");
        let radio_off = NetworkSnapshot { wifi_present: true, radio_on: false, ..NetworkSnapshot::default() };
        assert_eq!(radio_off.tooltip(), "Offline, Wi-Fi is off");
        let joining = NetworkSnapshot { state: NetState::Connecting, ssid: Some("Cafe".into()), ..NetworkSnapshot::default() };
        assert_eq!(joining.tooltip(), "Connecting to Cafe");
    }

    #[test]
    fn unknown_connectivity_claims_nothing_about_the_internet() {
        let unknown = NetworkSnapshot { connectivity: Connectivity::Unknown, ..laptop() };
        assert!(!unknown.no_internet());
        assert!(!unknown.tooltip().contains("internet"));
        assert_eq!(Connectivity::from_nm(3), Connectivity::Limited);
        assert_eq!(Connectivity::from_nm(4), Connectivity::Full);
        assert_eq!(Connectivity::from_nm(0), Connectivity::Unknown);
    }

    #[test]
    fn networks_list_joined_first_then_strongest_and_one_row_per_name() {
        let rows = NetworkSnapshot::arrange(vec![
            ap("Cafe", 40, true, false, false),
            ap("Guest", 90, false, false, false),
            ap("Home", 30, true, true, true),
            // A second access point for the same name, stronger, from the same network.
            ap("Cafe", 70, true, false, false),
            // Hidden networks have no name to show or to join.
            ap("", 99, true, false, false),
        ]);
        let names: Vec<&str> = rows.iter().map(|r| r.ssid.as_str()).collect();
        assert_eq!(names, ["Home", "Guest", "Cafe"]);
        assert_eq!(rows[2].strength, 70, "the strongest access point speaks for the name");
    }

    // ── the secret ──

    #[test]
    fn a_request_does_not_show_its_secret_when_debug_printed() {
        let request = ConnectRequest {
            ssid: "Cafe".into(),
            secret: Some(WifiSecret::new("hunter2-correct-horse".into())),
            by_person: true,
        };
        for printed in [format!("{request:?}"), format!("{request:#?}"), format!("{:?}", request.secret)] {
            assert!(!printed.contains("hunter2"), "the secret is in a Debug print: {printed}");
            assert!(!printed.contains("correct-horse"), "the secret is in a Debug print: {printed}");
        }
        assert!(format!("{request:?}").contains("Cafe"), "the name is the part worth logging");
    }

    #[test]
    fn a_refusal_states_the_rule_and_not_the_length_that_was_typed() {
        assert_eq!(Refusal::BadSecretLength.say("Cafe"), "Wi-Fi passwords are 8 to 63 characters");
    }

    #[test]
    fn the_secret_is_overwritten_when_it_is_dropped() {
        let mut s = WifiSecret::new("topsecret-passphrase".into());
        let before = s.0.as_ptr();
        let len = s.0.len();
        // Drive the Drop body without freeing, so the buffer can be read afterwards.
        s.0.zeroize();
        // SAFETY: the String is alive; zeroize leaves its capacity in place and sets len to 0.
        let after = unsafe { std::slice::from_raw_parts(before, len) };
        assert!(after.iter().all(|&b| b == 0), "the bytes are zeros once the secret is cleared");
        assert!(s.0.is_empty());
    }

    // ── planning a connection ──

    fn request(ssid: &str, secret: Option<&str>) -> ConnectRequest {
        ConnectRequest { ssid: ssid.into(), secret: secret.map(|s| WifiSecret::new(s.into())), by_person: true }
    }

    #[test]
    fn a_saved_network_is_joined_without_a_secret() {
        assert_eq!(plan(&laptop(), &request("Home", None), false), Ok(Plan::UseSaved));
    }

    #[test]
    fn an_open_unknown_network_is_joined_open() {
        assert_eq!(plan(&laptop(), &request("Guest", None), false), Ok(Plan::JoinOpen));
    }

    #[test]
    fn a_secured_unknown_network_needs_a_secret_and_a_sane_one() {
        assert_eq!(plan(&laptop(), &request("Cafe", None), false), Err(Refusal::NeedsSecret));
        assert_eq!(plan(&laptop(), &request("Cafe", Some("short")), false), Err(Refusal::BadSecretLength));
        assert_eq!(
            plan(&laptop(), &request("Cafe", Some("long-enough-1")), false),
            Ok(Plan::JoinSecured { key_mgmt: "wpa-psk" })
        );
        assert_eq!(
            plan(&laptop(), &request("Cafe", Some("long-enough-1")), true),
            Ok(Plan::JoinSecured { key_mgmt: "sae" })
        );
    }

    #[test]
    fn a_network_out_of_range_or_enterprise_is_refused_before_anything_is_sent() {
        assert_eq!(plan(&laptop(), &request("Nowhere", Some("long-enough-1")), false), Err(Refusal::NotVisible));
        let mut s = laptop();
        s.access_points.push(AccessPoint { enterprise: true, ..ap("Corp", 60, true, false, false) });
        assert_eq!(plan(&s, &request("Corp", Some("long-enough-1")), false), Err(Refusal::Enterprise));
    }

    #[test]
    fn a_raw_hex_key_of_64_digits_is_a_valid_secret() {
        assert!(secret_length_ok(&"a".repeat(64)));
        assert!(!secret_length_ok(&"z".repeat(64)));
        assert!(!secret_length_ok(&"a".repeat(7)));
        assert!(!secret_length_ok(&"a".repeat(65)));
    }
}
