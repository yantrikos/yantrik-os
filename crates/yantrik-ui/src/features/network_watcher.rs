//! Network Watcher — narrates connectivity changes, and offers the way back when Wi-Fi drops.
//!
//! Emits urges when a wired link drops, when the machine is back online, and when the signal is
//! weak. A Wi-Fi drop is not narrated as an urge any more: [`DropWatch`] turns it into a
//! notification with two buttons, "Reconnect" and "Choose network", because a whisper card that
//! said "Lost connection, ask me to run network_diagnose" was the only thing the person was shown
//! on 14 October 2026, and it offered no way to reconnect.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use yantrik_os::{NetworkSnapshot, SystemEvent};

use super::{FeatureContext, Outcome, ProactiveFeature, Urge, UrgeCategory};

/// How long after one drop notice, or one urge of a kind, before another of the same.
pub const COOLDOWN: Duration = Duration::from_secs(300);

pub struct NetworkWatcher {
    /// Was connected last time we checked?
    was_connected: Option<bool>,
    /// Was the link that was up a Wi-Fi one? A Wi-Fi drop is [`DropWatch`]'s to tell.
    was_wifi: bool,
    /// Last known SSID.
    last_ssid: Option<String>,
    /// Cooldown tracker.
    cooldowns: HashMap<String, Instant>,
    cooldown_secs: u64,
    /// Tick counter for periodic signal checks.
    tick_count: u64,
}

impl NetworkWatcher {
    pub fn new() -> Self {
        Self {
            was_connected: None,
            was_wifi: false,
            last_ssid: None,
            cooldowns: HashMap::new(),
            cooldown_secs: COOLDOWN.as_secs(),
            tick_count: 0,
        }
    }

    fn should_fire(&mut self, key: &str) -> bool {
        let now = Instant::now();
        if let Some(last) = self.cooldowns.get(key) {
            if now.duration_since(*last).as_secs() < self.cooldown_secs {
                return false;
            }
        }
        self.cooldowns.insert(key.to_string(), now);
        true
    }
}

impl ProactiveFeature for NetworkWatcher {
    fn name(&self) -> &str {
        "network_watcher"
    }

    fn on_event(&mut self, event: &SystemEvent, _ctx: &FeatureContext) -> Vec<Urge> {
        let SystemEvent::NetworkChanged { connected, ssid, signal } = event else {
            return Vec::new();
        };

        let mut urges = Vec::new();

        // Detect disconnect. Not a Wi-Fi one: the drop notice carries that, with its buttons.
        if !connected {
            if self.was_connected == Some(true) && !self.was_wifi && self.should_fire("net_disconnect") {
                let ssid_name = self.last_ssid.as_deref().unwrap_or("network");
                urges.push(Urge {
                    id: "nw:disconnect".into(),
                    source: "network_watcher".into(),
                    title: "Connection lost".into(),
                    body: format!(
                        "Lost connection to '{}'. Ask me to run network_diagnose.",
                        ssid_name
                    ),
                    urgency: 0.7,
                    confidence: 1.0,
                    category: UrgeCategory::Resource,
                });
            }
        }

        // Detect reconnect
        if *connected && self.was_connected == Some(false) && self.should_fire("net_reconnect") {
            let ssid_name = ssid.as_deref().unwrap_or("network");
            urges.push(Urge {
                id: "nw:reconnect".into(),
                source: "network_watcher".into(),
                title: "Back online".into(),
                body: format!("Reconnected to '{}'.", ssid_name),
                urgency: 0.35,
                confidence: 1.0,
                category: UrgeCategory::Celebration,
            });
        }

        // Weak signal warning
        if *connected {
            if let Some(sig) = signal {
                if *sig < 30 && self.should_fire("net_weak_signal") {
                    let ssid_name = ssid.as_deref().unwrap_or("WiFi");
                    urges.push(Urge {
                        id: format!("nw:weak_signal:{}", sig),
                        source: "network_watcher".into(),
                        title: "Weak signal".into(),
                        body: format!(
                            "'{}' signal at {}%. Connection may be unstable.",
                            ssid_name, sig
                        ),
                        urgency: 0.45,
                        confidence: 0.8,
                        category: UrgeCategory::Resource,
                    });
                }
            }
        }

        self.was_connected = Some(*connected);
        // The monitor names an SSID only for a Wi-Fi link.
        self.was_wifi = *connected && ssid.is_some();
        if let Some(s) = ssid {
            self.last_ssid = Some(s.clone());
        }

        urges
    }

    fn on_tick(&mut self, ctx: &FeatureContext) -> Vec<Urge> {
        self.tick_count += 1;

        // Every ~30 ticks (5 min at 10s intervals), check signal from snapshot
        if self.tick_count % 30 != 0 {
            return Vec::new();
        }

        if !ctx.system.network_connected {
            return Vec::new();
        }

        if let Some(sig) = ctx.system.network_signal {
            if sig < 30 && self.should_fire("net_weak_signal_tick") {
                let ssid = ctx.system.network_ssid.as_deref().unwrap_or("WiFi");
                return vec![Urge {
                    id: format!("nw:weak_signal_tick:{}", sig),
                    source: "network_watcher".into(),
                    title: "Weak WiFi".into(),
                    body: format!("'{}' signal at {}%. Move closer to router?", ssid, sig),
                    urgency: 0.4,
                    confidence: 0.7,
                    category: UrgeCategory::Resource,
                }];
            }
        }

        Vec::new()
    }

    fn on_feedback(&mut self, _urge_id: &str, _outcome: Outcome) {
        // Could adjust signal threshold if user keeps dismissing weak signal warnings
    }
}

/// The notification a Wi-Fi drop raises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropNotice {
    pub title: String,
    pub body: String,
    /// `(action id, label)`, in the order they are drawn. The ids are actions on the shell's own
    /// surface (`control_wifi_reconnect`), which is where a notification button sent as
    /// `Yantrik` is carried out.
    pub actions: [(&'static str, &'static str); 2],
}

pub const RECONNECT_ACTION: &str = "reconnect_wifi";
pub const CHOOSE_ACTION: &str = "choose_wifi_network";

/// Watches the network readings for Wi-Fi going from joined to not joined with the radio still
/// on, and says so once per drop, at most once per [`COOLDOWN`], so a link that flaps does not
/// raise a notification a minute.
///
/// Fed the shell's network readings rather than `NetworkChanged`, because the event does not say
/// whether the radio is on, and a drop the person caused by turning it off is not news to them.
/// Nothing is said while a cable keeps the machine online (it is not off the network), nor when
/// the person left the network on purpose.
#[derive(Debug, Default)]
pub struct DropWatch {
    /// The network Wi-Fi was last joined to, while it was.
    joined: Option<String>,
    last_notice: Option<Instant>,
}

impl DropWatch {
    pub fn observe(&mut self, now: Instant, snapshot: &NetworkSnapshot, left_on_purpose: bool) -> Option<DropNotice> {
        if snapshot.wifi_joined() {
            self.joined = Some(snapshot.ssid.clone().unwrap_or_default());
            return None;
        }
        // Connecting is not dropped yet: NetworkManager moving between access points, or
        // re-associating after a blip, passes through it.
        if snapshot.state == yantrik_os::NetState::Connecting {
            return None;
        }
        let ssid = self.joined.take()?;
        if !snapshot.radio_on || left_on_purpose || snapshot.online() {
            return None;
        }
        if self.last_notice.is_some_and(|t| now.saturating_duration_since(t) < COOLDOWN) {
            return None;
        }
        self.last_notice = Some(now);
        let name = if ssid.is_empty() { "the Wi-Fi network".to_string() } else { format!("\"{ssid}\"") };
        Some(DropNotice {
            title: "Wi-Fi disconnected".to_string(),
            body: format!(
                "This computer lost {name}. The radio is still on. Reconnect rejoins the strongest \
                 saved network in range; Choose network opens the list."
            ),
            actions: [(RECONNECT_ACTION, "Reconnect"), (CHOOSE_ACTION, "Choose network")],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_os::{NetKind, NetState};

    fn joined() -> NetworkSnapshot {
        NetworkSnapshot {
            kind: NetKind::Wifi,
            state: NetState::Connected,
            ssid: Some("Home".into()),
            wifi_present: true,
            radio_on: true,
            ..NetworkSnapshot::default()
        }
    }

    fn dropped() -> NetworkSnapshot {
        NetworkSnapshot { kind: NetKind::None, state: NetState::Disconnected, ssid: None, ..joined() }
    }

    #[test]
    fn a_wifi_drop_with_the_radio_on_raises_one_notice_with_both_buttons() {
        let t0 = Instant::now();
        let mut watch = DropWatch::default();
        assert_eq!(watch.observe(t0, &joined(), false), None);
        let notice = watch.observe(t0 + Duration::from_secs(1), &dropped(), false).expect("the drop is said");
        assert_eq!(notice.title, "Wi-Fi disconnected");
        assert!(notice.body.contains("\"Home\""), "{}", notice.body);
        assert_eq!(notice.actions, [("reconnect_wifi", "Reconnect"), ("choose_wifi_network", "Choose network")]);
        // Once per drop: the readings that follow while it stays down say nothing.
        for later in 2..100 {
            assert_eq!(watch.observe(t0 + Duration::from_secs(later), &dropped(), false), None);
        }
    }

    #[test]
    fn a_flapping_link_raises_one_notice_per_cooldown() {
        let t0 = Instant::now();
        let mut watch = DropWatch::default();
        let mut notices = 0;
        // Up and down every 20 seconds for ten minutes.
        for step in 0..30u64 {
            let at = t0 + Duration::from_secs(step * 20);
            let reading = if step % 2 == 0 { joined() } else { dropped() };
            notices += watch.observe(at, &reading, false).is_some() as u32;
        }
        assert_eq!(notices, 2, "ten minutes of flapping is two notices with a five-minute cooldown");
    }

    #[test]
    fn a_drop_the_person_caused_is_not_news() {
        let t0 = Instant::now();
        let mut watch = DropWatch::default();
        watch.observe(t0, &joined(), false);
        let off = NetworkSnapshot { radio_on: false, ..dropped() };
        assert_eq!(watch.observe(t0, &off, false), None, "they turned the radio off");
        watch.observe(t0, &joined(), false);
        assert_eq!(watch.observe(t0, &dropped(), true), None, "they pressed Disconnect");
    }

    #[test]
    fn nothing_is_said_while_a_cable_keeps_the_machine_online_or_while_networkmanager_is_joining() {
        let t0 = Instant::now();
        let mut watch = DropWatch::default();
        watch.observe(t0, &joined(), false);
        let joining = NetworkSnapshot { kind: NetKind::Wifi, state: NetState::Connecting, ..dropped() };
        assert_eq!(watch.observe(t0, &joining, false), None);
        let wired = NetworkSnapshot { kind: NetKind::Wired, state: NetState::Connected, ..dropped() };
        assert_eq!(watch.observe(t0, &wired, false), None);
    }

    #[test]
    fn a_wifi_drop_is_not_also_narrated_as_an_urge() {
        let mut watcher = NetworkWatcher::new();
        let system = yantrik_os::SystemSnapshot::default();
        let ctx = FeatureContext { system: &system, clock: std::time::SystemTime::now(), bond_level: 0 };
        let up = SystemEvent::NetworkChanged { connected: true, ssid: Some("Home".into()), signal: Some(70) };
        let down = SystemEvent::NetworkChanged { connected: false, ssid: None, signal: None };
        watcher.on_event(&up, &ctx);
        assert!(watcher.on_event(&down, &ctx).is_empty(), "the drop notice says it, with its buttons");
        // A wired link that drops is still narrated.
        let mut watcher = NetworkWatcher::new();
        watcher.on_event(&SystemEvent::NetworkChanged { connected: true, ssid: None, signal: None }, &ctx);
        assert_eq!(watcher.on_event(&down, &ctx).len(), 1);
    }
}
