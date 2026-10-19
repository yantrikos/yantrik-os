//! The shell rejoining Wi-Fi by itself, a bounded number of times, when NetworkManager has stopped.
//!
//! network-service now sets saved profiles to retry forever, so NetworkManager should rarely give
//! up. This is the net under that, for a profile it could not change or a NetworkManager that
//! blocked autoconnect for some other reason: when Wi-Fi is down, the radio is on, a saved network
//! was last seen in range, and NetworkManager has had the device `disconnected` — not
//! `connecting`, which is NetworkManager still at it — for a whole minute, the shell asks for the
//! same rejoin the tile does, once. Then it waits 1, 2, 5 and 10 minutes between attempts, and an
//! hour after that, so a network that is really gone costs one log line an hour and not a radio
//! kept busy. It stops the moment Wi-Fi is joined or the person turns the radio off, and does not
//! start while the person has left the network on purpose (the popover's Disconnect).
//!
//! Pure: [`Heal::observe`] takes the time and the reading, so the schedule is a test.

use std::time::{Duration, Instant};

use yantrik_os::NetworkSnapshot;

/// How long the device must sit `disconnected` before the shell steps in.
pub const IDLE_FOR: Duration = Duration::from_secs(60);

/// The waits after each attempt: 1, 2, 5, 10 minutes, then [`HOURLY`] for as long as it lasts.
pub const BACKOFF: [Duration; 4] = [
    Duration::from_secs(60),
    Duration::from_secs(120),
    Duration::from_secs(300),
    Duration::from_secs(600),
];
pub const HOURLY: Duration = Duration::from_secs(3600);

/// The wait after the `attempts`-th attempt.
pub fn wait_after(attempts: u32) -> Duration {
    BACKOFF.get(attempts.saturating_sub(1) as usize).copied().unwrap_or(HOURLY)
}

/// One attempt the shell should make now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    /// 1 for the first since Wi-Fi went down.
    pub number: u32,
    /// The saved network last seen in range, for the log line.
    pub ssid: String,
}

#[derive(Debug, Default)]
pub struct Heal {
    /// When the device was first seen `disconnected` in the current stretch.
    idle_since: Option<Instant>,
    /// Attempts since Wi-Fi was last joined.
    attempts: u32,
    /// No attempt before this.
    not_before: Option<Instant>,
    /// The saved network most recently seen in range. Kept across readings, because
    /// NetworkManager may empty its access-point list between scans while the network is there.
    last_saved_in_range: Option<String>,
}

impl Heal {
    /// Look at one reading. `left_on_purpose` is the person having pressed Disconnect since Wi-Fi
    /// was last joined. Answers with an attempt to make now, or `None`.
    pub fn observe(&mut self, now: Instant, snapshot: &NetworkSnapshot, left_on_purpose: bool) -> Option<Attempt> {
        // Joined, or the radio off: whatever was going on is over. A drop after this starts the
        // schedule afresh.
        if snapshot.wifi_joined() || !snapshot.wifi_present || !snapshot.radio_on {
            *self = Heal::default();
            return None;
        }
        if let Some(best) = snapshot.best_saved_in_range() {
            self.last_saved_in_range = Some(best.ssid.clone());
        }
        if left_on_purpose || !snapshot.wifi_idle {
            // Connecting, unavailable, or the person's choice: not the shell's to help with. The
            // minute starts again from the next time the device is idle.
            self.idle_since = None;
            return None;
        }
        let idle_since = *self.idle_since.get_or_insert(now);
        if now.saturating_duration_since(idle_since) < IDLE_FOR {
            return None;
        }
        if self.not_before.is_some_and(|t| now < t) {
            return None;
        }
        let ssid = self.last_saved_in_range.clone()?;
        self.attempts += 1;
        self.not_before = Some(now + wait_after(self.attempts));
        Some(Attempt { number: self.attempts, ssid })
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_os::{AccessPoint, NetKind, NetState};

    fn ap(ssid: &str, known: bool) -> AccessPoint {
        AccessPoint { ssid: ssid.into(), strength: 60, secured: true, known, connected: false, enterprise: false }
    }

    fn idle() -> NetworkSnapshot {
        NetworkSnapshot {
            kind: NetKind::None,
            state: NetState::Disconnected,
            wifi_present: true,
            radio_on: true,
            wifi_idle: true,
            access_points: vec![ap("Home", true)],
            ..NetworkSnapshot::default()
        }
    }

    fn joined() -> NetworkSnapshot {
        NetworkSnapshot { kind: NetKind::Wifi, state: NetState::Connected, ssid: Some("Home".into()), wifi_idle: false, ..idle() }
    }

    const fn s(secs: u64) -> Duration {
        Duration::from_secs(secs)
    }

    #[test]
    fn it_asks_once_after_a_minute_of_the_device_sitting_disconnected() {
        let t0 = Instant::now();
        let mut heal = Heal::default();
        assert_eq!(heal.observe(t0, &idle(), false), None);
        assert_eq!(heal.observe(t0 + s(59), &idle(), false), None, "not before the minute is up");
        assert_eq!(heal.observe(t0 + s(60), &idle(), false), Some(Attempt { number: 1, ssid: "Home".into() }));
        assert_eq!(heal.observe(t0 + s(61), &idle(), false), None, "once, not on every reading");
    }

    #[test]
    fn it_backs_off_one_two_five_ten_minutes_then_hourly() {
        let t0 = Instant::now();
        let mut heal = Heal::default();
        heal.observe(t0, &idle(), false);
        let mut at = t0 + IDLE_FOR;
        let mut gaps = Vec::new();
        let mut last: Option<Instant> = None;
        // Walk forward a minute at a time for a day and note when each attempt came.
        while at < t0 + s(24 * 3600) {
            if heal.observe(at, &idle(), false).is_some() {
                if let Some(prev) = last {
                    gaps.push((at - prev).as_secs() / 60);
                }
                last = Some(at);
            }
            at += s(60);
        }
        assert_eq!(&gaps[..5], [1, 2, 5, 10, 60], "the schedule, in minutes");
        assert!(gaps[5..].iter().all(|g| *g == 60), "hourly after that: {gaps:?}");
    }

    #[test]
    fn a_device_networkmanager_is_still_connecting_is_left_to_it() {
        let t0 = Instant::now();
        let mut heal = Heal::default();
        let connecting = NetworkSnapshot { wifi_idle: false, kind: NetKind::Wifi, state: NetState::Connecting, ..idle() };
        for minute in 0..10 {
            assert_eq!(heal.observe(t0 + s(minute * 60), &connecting, false), None);
        }
        // And the minute counts from when it became idle, not from when it went down.
        assert_eq!(heal.observe(t0 + s(600), &idle(), false), None);
        assert!(heal.observe(t0 + s(660), &idle(), false).is_some());
    }

    #[test]
    fn it_stops_when_wifi_is_joined_and_starts_afresh_on_the_next_drop() {
        let t0 = Instant::now();
        let mut heal = Heal::default();
        heal.observe(t0, &idle(), false);
        assert!(heal.observe(t0 + s(60), &idle(), false).is_some());
        assert_eq!(heal.observe(t0 + s(90), &joined(), false), None);
        assert_eq!(heal.attempts(), 0);
        heal.observe(t0 + s(100), &idle(), false);
        assert_eq!(heal.observe(t0 + s(160), &idle(), false).map(|a| a.number), Some(1), "a new drop, a new schedule");
    }

    #[test]
    fn it_stops_when_the_person_turns_the_radio_off() {
        let t0 = Instant::now();
        let mut heal = Heal::default();
        heal.observe(t0, &idle(), false);
        let off = NetworkSnapshot { radio_on: false, wifi_idle: false, access_points: Vec::new(), ..idle() };
        for minute in 1..120 {
            assert_eq!(heal.observe(t0 + s(minute * 60), &off, false), None, "the radio is the person's");
        }
    }

    #[test]
    fn it_does_not_undo_the_persons_disconnect() {
        let t0 = Instant::now();
        let mut heal = Heal::default();
        for minute in 0..30 {
            assert_eq!(heal.observe(t0 + s(minute * 60), &idle(), true), None);
        }
    }

    #[test]
    fn with_no_saved_network_ever_seen_in_range_it_asks_for_nothing() {
        let t0 = Instant::now();
        let mut heal = Heal::default();
        let strangers = NetworkSnapshot { access_points: vec![ap("Cafe", false)], ..idle() };
        for minute in 0..30 {
            assert_eq!(heal.observe(t0 + s(minute * 60), &strangers, false), None);
        }
    }

    /// NetworkManager can empty its list between scans: the saved network last seen is still the
    /// one tried.
    #[test]
    fn the_saved_network_last_seen_in_range_is_remembered_across_an_empty_list() {
        let t0 = Instant::now();
        let mut heal = Heal::default();
        heal.observe(t0, &idle(), false);
        let empty = NetworkSnapshot { access_points: Vec::new(), ..idle() };
        assert_eq!(heal.observe(t0 + s(60), &empty, false).map(|a| a.ssid), Some("Home".into()));
    }
}
