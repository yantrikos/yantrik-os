//! Whether the person is at the machine (#412).
//!
//! `SystemEvent::UserIdle` had four readers and no writer, so the auto-lock the person set in
//! Settings never fired: a machine left alone stayed open. Idle now comes from the compositor,
//! over `ext-idle-notify-v1`, which counts input on the seat (keyboard, pointer, touch) and
//! nothing else. A mind's turn at a socket or an app redrawing is not the person, so an agent
//! working through the night does not hold the screen open.
//!
//! One notification per threshold instead of a timer: each `idled` says "nobody has touched the
//! seat for this long", which is exactly `UserIdle { idle_seconds }`. Every auto-lock choice
//! Settings offers is a threshold, so the lock fires when the setting says it will. The first
//! `resumed` after any `idled` is one `UserResumed`.
//!
//! The yos control surface offers no way to hold this off. That is not the same as nothing
//! being able to: on labwc 0.8 any client on the person's Wayland socket can take an idle
//! inhibitor (the compositor withholds `idled` while one exists, visible or not) or drive the
//! virtual keyboard and pointer, and both keep the seat "occupied". Closing that means keeping
//! minds off this socket (#411, #414); `get_input_idle_notification` (notifier v2, labwc 0.9)
//! will ignore inhibitors.
//!
//! Whether the watch is running is public ([`watching`]): a Settings row promising a lock that
//! cannot come is the bug this module fixes, so when the compositor gives no idle signal the
//! desktop says so instead of promising.

use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam_channel::Sender;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::{
    self, ExtIdleNotificationV1,
};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1::ExtIdleNotifierV1;

use crate::events::SystemEvent;

/// Seconds without input after which the person counts as away, each longer than the last.
/// Every auto-lock choice in Settings (30 s, 1, 2, 5 and 10 min) is one of these.
pub const THRESHOLDS: &[u32] = &[30, 60, 120, 300, 600, 900, 1800, 3600];

/// The compositor's per-notification events, as the shell's two presence events.
#[derive(Debug, Default)]
pub struct Presence {
    away_for: Option<u32>,
}

impl Presence {
    /// Nobody has touched the seat for `secs`. Only a longer absence than already reported is
    /// news.
    pub fn idled(&mut self, secs: u32) -> Option<SystemEvent> {
        if self.away_for.is_some_and(|away| away >= secs) {
            return None;
        }
        self.away_for = Some(secs);
        Some(SystemEvent::UserIdle { idle_seconds: u64::from(secs) })
    }

    /// Input again. Every notification that idled says so; the person came back once.
    pub fn resumed(&mut self) -> Option<SystemEvent> {
        self.away_for.take().map(|_| SystemEvent::UserResumed)
    }
}

static WATCHING: AtomicBool = AtomicBool::new(false);

/// Whether the compositor is telling this process when the seat is left alone. False before the
/// watch starts, when the compositor has no idle notifier, and after the connection ends.
pub fn watching() -> bool {
    WATCHING.load(Ordering::SeqCst)
}

struct State {
    tx: Sender<SystemEvent>,
    presence: Presence,
}

impl State {
    fn send(&self, event: Option<SystemEvent>) {
        if let Some(event) = event {
            let _ = self.tx.send(event);
        }
    }
}

/// Watch the seat until the Wayland connection ends. Without a compositor that offers idle
/// notifications, say so once: auto-lock cannot fire on this session.
pub fn run_idle_monitor(tx: Sender<SystemEvent>) {
    let why = match watch(tx.clone()) {
        Ok(()) => "the watch ended".to_string(),
        Err(why) => why,
    };
    WATCHING.store(false, Ordering::SeqCst);
    // Not left "away" forever by a watch that stopped mid-absence.
    let _ = tx.send(SystemEvent::UserResumed);
    tracing::warn!(%why, "No idle signal from the compositor; auto-lock cannot fire (#412)");
}

fn watch(tx: Sender<SystemEvent>) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("no Wayland display: {e}"))?;
    let (globals, mut queue) =
        registry_queue_init::<State>(&conn).map_err(|e| format!("no Wayland registry: {e}"))?;
    let qh = queue.handle();
    let notifier: ExtIdleNotifierV1 = globals
        .bind(&qh, 1..=1, ())
        .map_err(|e| format!("the compositor has no ext-idle-notify-v1: {e}"))?;
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ()).map_err(|e| format!("no seat: {e}"))?;
    // Held for the life of the watch: a dropped proxy is not destroyed, but these are ours.
    let _notifications: Vec<ExtIdleNotificationV1> = THRESHOLDS
        .iter()
        .map(|&secs| notifier.get_idle_notification(secs * 1000, &seat, &qh, secs))
        .collect();
    let mut state = State { tx, presence: Presence::default() };
    // The requests go out on the next flush; a roundtrip makes sure the compositor has them (and
    // has not refused them) before this says it is watching.
    queue
        .roundtrip(&mut state)
        .map_err(|e| format!("the compositor refused the idle notifications: {e}"))?;
    WATCHING.store(true, Ordering::SeqCst);
    tracing::info!(thresholds = ?THRESHOLDS, "Watching the seat for the person (ext-idle-notify-v1)");
    loop {
        queue
            .blocking_dispatch(&mut state)
            .map_err(|e| format!("the Wayland connection ended: {e}"))?;
    }
}

impl Dispatch<ExtIdleNotificationV1, u32> for State {
    fn event(
        state: &mut Self,
        _: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        secs: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let presence = match event {
            ext_idle_notification_v1::Event::Idled => state.presence.idled(*secs),
            ext_idle_notification_v1::Event::Resumed => state.presence.resumed(),
            _ => None,
        };
        state.send(presence);
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ExtIdleNotifierV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ExtIdleNotifierV1,
        _: <ExtIdleNotifierV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_longer_absence_is_reported_once_and_the_return_once() {
        let mut p = Presence::default();
        assert!(p.resumed().is_none(), "no return without an absence");
        assert!(matches!(p.idled(30), Some(SystemEvent::UserIdle { idle_seconds: 30 })));
        assert!(matches!(p.idled(300), Some(SystemEvent::UserIdle { idle_seconds: 300 })));
        assert!(p.idled(60).is_none(), "a shorter threshold arriving late is not news");
        assert!(matches!(p.resumed(), Some(SystemEvent::UserResumed)));
        assert!(p.resumed().is_none(), "every notification resumes; the person came back once");
        assert!(p.idled(30).is_some(), "away again after coming back");
    }

    #[test]
    fn every_auto_lock_choice_in_settings_is_a_threshold() {
        // wire/settings.rs cycles auto_lock_secs through these; a choice between thresholds
        // would lock late, at the next one.
        for choice in [30, 60, 120, 300, 600] {
            assert!(THRESHOLDS.contains(&choice), "{choice}s is not a threshold");
        }
        assert!(THRESHOLDS.windows(2).all(|w| w[0] < w[1]));
    }
}
