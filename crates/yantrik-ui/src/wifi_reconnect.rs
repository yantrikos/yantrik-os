//! What the Quick Settings Wi-Fi tile says, what pressing it does, and the way back after a drop.
//!
//! # What went wrong
//!
//! On 14 October 2026 the Wi-Fi on the 2012 Mac mini (a BCM4331 on the `wl` driver) dropped, and
//! the person found no way to reconnect and restarted the machine. The tile read "Not connected"
//! and pressing it switched the radio: with the radio on, the press turned it OFF. A second press
//! turned it on again and hoped NetworkManager would rejoin by itself — which, after four failed
//! attempts on a slow driver, it no longer does (see network-service's `autoconnect.rs`). Nothing
//! on the tile ever offered a network.
//!
//! # What it does now
//!
//! The tile has three states, and pressing it does what it says in each:
//!
//! | radio | joined | the tile says                   | a press                                   |
//! |-------|--------|---------------------------------|-------------------------------------------|
//! | off   | —      | "Wi-Fi is off" / "Tap to turn on" | turns the radio on                      |
//! | on    | no     | "Not connected" / "Tap to reconnect" | rejoins the strongest saved network in range, else opens Network Manager |
//! | on    | yes    | the network's name / "Tap to manage" | opens Network Manager                  |
//!
//! While NetworkManager is joining, the tile says "Connecting" and a press opens Network Manager:
//! a second join on top of one in progress would only fight it.
//!
//! No press turns the radio off. That is the Wi-Fi switch in the network list the tile's chevron
//! opens: a control of its own that says what it is, never the meaning of "connect".
//!
//! Everything here is a pure function of a [`NetworkSnapshot`] and a [`WifiClient`], so the
//! tests below drive it with a client that records what it was asked for.

use yantrik_os::NetworkSnapshot;

/// The tile's state, from one reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tile {
    /// The radio is switched off.
    Off,
    /// The radio is on and NetworkManager is joining a network.
    Connecting,
    /// The radio is on and no Wi-Fi network is joined.
    NotConnected,
    /// Joined to this network.
    Connected(String),
}

/// What a press on the tile asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tap {
    TurnRadioOn,
    Reconnect,
    Manage,
}

impl Tile {
    pub fn from(snapshot: &NetworkSnapshot) -> Tile {
        if !snapshot.radio_on {
            return Tile::Off;
        }
        if snapshot.wifi_joined() {
            // A hidden network has no name to show; it is still joined.
            let name = snapshot.ssid.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| "Connected".to_string());
            return Tile::Connected(name);
        }
        if snapshot.kind == yantrik_os::NetKind::Wifi && snapshot.state == yantrik_os::NetState::Connecting {
            return Tile::Connecting;
        }
        Tile::NotConnected
    }

    /// The tile's first line: the state, in words.
    pub fn title(&self) -> String {
        match self {
            Tile::Off => "Wi-Fi is off".to_string(),
            Tile::Connecting => "Connecting".to_string(),
            Tile::NotConnected => "Not connected".to_string(),
            Tile::Connected(name) => name.clone(),
        }
    }

    /// The tile's second line: what a press does.
    pub fn hint(&self) -> &'static str {
        match self {
            Tile::Off => "Tap to turn on",
            Tile::Connecting | Tile::Connected(_) => "Tap to manage",
            Tile::NotConnected => "Tap to reconnect",
        }
    }

    /// Lit when the radio is on, as every switch-like tile is lit when its thing is on.
    pub fn lit(&self) -> bool {
        !matches!(self, Tile::Off)
    }

    pub fn tap(&self) -> Tap {
        match self {
            Tile::Off => Tap::TurnRadioOn,
            Tile::NotConnected => Tap::Reconnect,
            Tile::Connecting | Tile::Connected(_) => Tap::Manage,
        }
    }
}

/// What the tile, the drop notification and the self-heal ask of the network. The shell's own
/// client goes to NetworkManager through `yantrik_os::network`; the tests' records the calls.
pub trait WifiClient {
    /// Switch the radio on or off; answers with the radio's state afterwards.
    fn set_radio(&self, on: bool) -> Result<bool, String>;
    /// Join a network this machine has a saved profile for. No password: there is no way to pass
    /// one. Blocks until the join has worked or failed.
    fn join_saved(&self, ssid: &str) -> Result<(), String>;
    /// Open Network Manager, which opens on its Wi-Fi list when no cable carries the traffic.
    fn open_manager(&self);
}

/// How one reconnect ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reconnected {
    /// Back on this network.
    Joined(String),
    /// No saved network is in range, so there was nothing to rejoin.
    NothingInRange,
    /// The join to this network did not work, and why.
    Failed(String, String),
    /// Nothing was tried: the radio is off or a network is already joined.
    NotNeeded,
}

/// Rejoin the strongest saved network in range. Never touches the radio.
pub fn reconnect(snapshot: &NetworkSnapshot, client: &dyn WifiClient) -> Reconnected {
    if !snapshot.radio_on || snapshot.wifi_joined() {
        return Reconnected::NotNeeded;
    }
    let Some(best) = snapshot.best_saved_in_range() else {
        return Reconnected::NothingInRange;
    };
    let ssid = best.ssid.clone();
    match client.join_saved(&ssid) {
        Ok(()) => Reconnected::Joined(ssid),
        Err(why) => Reconnected::Failed(ssid, why),
    }
}

/// A person's reconnect — the tile's press, the notification's button. Where nothing could be
/// rejoined, Network Manager is opened, so the person always has a list of networks to choose
/// from and a field to type a password into.
pub fn reconnect_for_person(snapshot: &NetworkSnapshot, client: &dyn WifiClient) -> Reconnected {
    let outcome = reconnect(snapshot, client);
    if matches!(outcome, Reconnected::NothingInRange | Reconnected::Failed(..)) {
        client.open_manager();
    }
    outcome
}

/// A press on the tile, carried out. Call it off the thread that draws: a reconnect waits for the
/// join.
pub fn press(snapshot: &NetworkSnapshot, client: &dyn WifiClient) -> Option<Reconnected> {
    match Tile::from(snapshot).tap() {
        Tap::TurnRadioOn => {
            if let Err(why) = client.set_radio(true) {
                tracing::warn!(%why, "Wi-Fi radio could not be turned on");
            }
            None
        }
        Tap::Reconnect => Some(reconnect_for_person(snapshot, client)),
        Tap::Manage => {
            client.open_manager();
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use yantrik_os::{AccessPoint, Connectivity, NetKind, NetState};

    /// Records every call, and answers a join the way it was told to.
    #[derive(Default)]
    struct FakeClient {
        calls: RefCell<Vec<String>>,
        join_fails: bool,
    }

    impl WifiClient for FakeClient {
        fn set_radio(&self, on: bool) -> Result<bool, String> {
            self.calls.borrow_mut().push(format!("radio {}", if on { "on" } else { "off" }));
            Ok(on)
        }
        fn join_saved(&self, ssid: &str) -> Result<(), String> {
            self.calls.borrow_mut().push(format!("join {ssid}"));
            if self.join_fails { Err("Could not join".into()) } else { Ok(()) }
        }
        fn open_manager(&self) {
            self.calls.borrow_mut().push("open manager".into());
        }
    }

    impl FakeClient {
        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    fn ap(ssid: &str, strength: u8, known: bool, connected: bool) -> AccessPoint {
        AccessPoint { ssid: ssid.into(), strength, secured: true, known, connected, enterprise: false }
    }

    fn joined() -> NetworkSnapshot {
        NetworkSnapshot {
            kind: NetKind::Wifi,
            state: NetState::Connected,
            ssid: Some("Home".into()),
            strength: Some(70),
            connectivity: Connectivity::Full,
            wifi_present: true,
            radio_on: true,
            access_points: vec![ap("Home", 70, true, true), ap("Cafe", 50, false, false)],
            links: 1,
            ..NetworkSnapshot::default()
        }
    }

    /// The Mac mini after the drop: radio on, nothing joined, its network still in range.
    fn dropped() -> NetworkSnapshot {
        NetworkSnapshot {
            kind: NetKind::None,
            state: NetState::Disconnected,
            ssid: None,
            strength: None,
            links: 0,
            wifi_idle: true,
            access_points: vec![ap("Cafe", 90, false, false), ap("Home", 70, true, false), ap("Office", 40, true, false)],
            ..joined()
        }
    }

    fn radio_off() -> NetworkSnapshot {
        NetworkSnapshot { radio_on: false, access_points: Vec::new(), ..dropped() }
    }

    #[test]
    fn the_tile_says_the_radio_is_off_and_a_press_turns_it_on() {
        let tile = Tile::from(&radio_off());
        assert_eq!((tile.title().as_str(), tile.hint()), ("Wi-Fi is off", "Tap to turn on"));
        assert!(!tile.lit());
        let client = FakeClient::default();
        press(&radio_off(), &client);
        assert_eq!(client.calls(), ["radio on"]);
    }

    #[test]
    fn the_tile_says_not_connected_and_a_press_rejoins_the_strongest_saved_network() {
        let tile = Tile::from(&dropped());
        assert_eq!((tile.title().as_str(), tile.hint()), ("Not connected", "Tap to reconnect"));
        assert!(tile.lit(), "the radio is on");
        let client = FakeClient::default();
        assert_eq!(press(&dropped(), &client), Some(Reconnected::Joined("Home".into())));
        // Home, not Cafe: Cafe is stronger and has no saved profile, so it would need a password.
        assert_eq!(client.calls(), ["join Home"]);
    }

    #[test]
    fn the_tile_names_the_network_and_a_press_opens_network_manager() {
        let tile = Tile::from(&joined());
        assert_eq!((tile.title().as_str(), tile.hint()), ("Home", "Tap to manage"));
        let client = FakeClient::default();
        press(&joined(), &client);
        assert_eq!(client.calls(), ["open manager"]);
    }

    /// The bug itself: with the radio on and no network joined, the press turned the radio off.
    #[test]
    fn with_the_radio_on_and_nothing_joined_a_press_never_turns_the_radio_off() {
        for client in [FakeClient::default(), FakeClient { join_fails: true, ..FakeClient::default() }] {
            for snapshot in [dropped(), NetworkSnapshot { access_points: Vec::new(), ..dropped() }] {
                press(&snapshot, &client);
            }
            assert!(!client.calls().iter().any(|c| c.starts_with("radio")), "a connect press switched the radio: {:?}", client.calls());
        }
        // And from every state, nothing a press does is "radio off".
        for snapshot in [radio_off(), dropped(), joined()] {
            let client = FakeClient::default();
            press(&snapshot, &client);
            assert!(!client.calls().contains(&"radio off".to_string()));
        }
    }

    #[test]
    fn with_no_saved_network_in_range_the_press_opens_network_manager() {
        let nothing_saved = NetworkSnapshot { access_points: vec![ap("Cafe", 90, false, false)], ..dropped() };
        let client = FakeClient::default();
        assert_eq!(press(&nothing_saved, &client), Some(Reconnected::NothingInRange));
        assert_eq!(client.calls(), ["open manager"], "the person always gets a list to choose from");
    }

    #[test]
    fn a_rejoin_that_fails_opens_network_manager() {
        let client = FakeClient { join_fails: true, ..FakeClient::default() };
        assert!(matches!(press(&dropped(), &client), Some(Reconnected::Failed(ssid, _)) if ssid == "Home"));
        assert_eq!(client.calls(), ["join Home", "open manager"]);
    }

    /// The self-heal's reconnect is the same rejoin, without a window opening by itself.
    #[test]
    fn a_reconnect_nobody_pressed_for_opens_nothing() {
        let client = FakeClient { join_fails: true, ..FakeClient::default() };
        assert!(matches!(reconnect(&dropped(), &client), Reconnected::Failed(..)));
        assert_eq!(client.calls(), ["join Home"]);
        let client = FakeClient::default();
        assert_eq!(reconnect(&joined(), &client), Reconnected::NotNeeded);
        assert_eq!(reconnect(&radio_off(), &client), Reconnected::NotNeeded);
        assert!(client.calls().is_empty(), "nothing is joined over a network that is up, or with the radio off");
    }

    #[test]
    fn while_networkmanager_is_joining_a_press_opens_network_manager_rather_than_join_again() {
        let joining = NetworkSnapshot { kind: NetKind::Wifi, state: NetState::Connecting, ..dropped() };
        let tile = Tile::from(&joining);
        assert_eq!((tile.title().as_str(), tile.hint()), ("Connecting", "Tap to manage"));
        let client = FakeClient::default();
        press(&joining, &client);
        assert_eq!(client.calls(), ["open manager"]);
    }

    /// On a cable with the Wi-Fi radio on and no Wi-Fi joined, the tile is about Wi-Fi: it says
    /// not connected, and offers the rejoin.
    #[test]
    fn a_wired_machine_with_wifi_not_joined_is_not_connected_on_wifi() {
        let wired = NetworkSnapshot { kind: NetKind::Wired, state: NetState::Connected, ..dropped() };
        assert_eq!(Tile::from(&wired), Tile::NotConnected);
    }
}
