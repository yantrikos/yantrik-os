//! The way back after a Wi-Fi drop, wired to the shell: the Quick Settings tile's press, the
//! notification a drop raises, and the shell rejoining by itself when NetworkManager has stopped.
//!
//! The decisions are pure and tested where they live — the tile in `crate::wifi_reconnect`, the
//! notice in `features::network_watcher::DropWatch`, the schedule in `crate::wifi_heal`. This file
//! only gives them the machine: NetworkManager through `yantrik_os::network`, which is the shell's
//! one reader and writer of the network (`wire::network` says why there is only one), and the
//! notification store through `notify`.
//!
//! Everything that waits on NetworkManager runs on a worker thread: a join waits for the handshake
//! and DHCP, and the thread that draws must not.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use slint::ComponentHandle;
use yantrik_app_runtime::notify;
use yantrik_ipc_contracts::notifications::Urgency;
use yantrik_os::{ConnectRequest, NetworkSnapshot};

use crate::features::network_watcher::DropWatch;
use crate::wifi_heal::Heal;
use crate::wifi_reconnect::{self, Reconnected, WifiClient};
use crate::{App, NetworkState};

/// How often the self-heal looks at the reading. The rule is in minutes; ten seconds is close
/// enough and costs nothing, since it reads the held snapshot and asks NetworkManager nothing.
const HEAL_EVERY: Duration = Duration::from_secs(10);

/// The person pressed Disconnect, and Wi-Fi has not been joined since. While it holds, a drop is
/// theirs: no notice, no rejoin by itself.
static LEFT_ON_PURPOSE: AtomicBool = AtomicBool::new(false);

thread_local! {
    static DROPS: RefCell<DropWatch> = RefCell::new(DropWatch::default());
    static HEAL: RefCell<Heal> = RefCell::new(Heal::default());
    static HEAL_TIMER: slint::Timer = slint::Timer::default();
}

/// The person left the network on purpose, from the popover's Disconnect.
pub fn person_left() {
    LEFT_ON_PURPOSE.store(true, Ordering::Relaxed);
}

/// The shell's client: NetworkManager over D-Bus, and Network Manager opened on the shell's
/// thread. `by_person` is whose join it is, which `yantrik_os::network` uses to space a caller's
/// joins and to refuse anything but a saved network to one that is not a person's.
struct ShellClient {
    weak: slint::Weak<App>,
    by_person: bool,
}

impl WifiClient for ShellClient {
    fn set_radio(&self, on: bool) -> Result<bool, String> {
        let now = yantrik_os::network::set_wifi_enabled(on)?;
        tracing::info!(asked_for = on, now, "Wi-Fi radio set from the Quick Settings tile");
        Ok(now)
    }

    fn join_saved(&self, ssid: &str) -> Result<(), String> {
        let slot = yantrik_os::network::begin_join(self.by_person)?;
        let request = ConnectRequest { ssid: ssid.to_string(), secret: None, by_person: self.by_person };
        yantrik_os::network::connect(request, true, slot).map(|_| ())
    }

    fn open_manager(&self) {
        let weak = self.weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_quick_settings_open(false);
                ui.set_network_open(false);
                ui.invoke_launch_app("network-manager".into());
            }
        });
    }
}

/// The reading to act on: one taken now, or the last one if NetworkManager did not answer.
fn reading() -> Option<NetworkSnapshot> {
    yantrik_os::network::read_fresh().or_else(yantrik_os::network::latest)
}

fn log_outcome(outcome: &Reconnected, why: &str) {
    match outcome {
        Reconnected::Joined(ssid) => tracing::info!(%ssid, why, "Wi-Fi rejoined"),
        Reconnected::NothingInRange => tracing::info!(why, "Wi-Fi not rejoined: no saved network in range"),
        Reconnected::Failed(ssid, reason) => tracing::info!(%ssid, %reason, why, "Wi-Fi rejoin did not complete"),
        Reconnected::NotNeeded => tracing::debug!(why, "Wi-Fi rejoin not needed"),
    }
}

/// The person asked to reconnect — the tile, the notification's button. Off this thread.
pub fn reconnect_for_person(weak: slint::Weak<App>) {
    std::thread::spawn(move || {
        let Some(snapshot) = reading() else { return };
        let client = ShellClient { weak, by_person: true };
        let outcome = wifi_reconnect::reconnect_for_person(&snapshot, &client);
        log_outcome(&outcome, "asked for by the person");
    });
}

/// Open Network Manager on its network list: the drop notification's "Choose network".
pub fn choose_network(weak: slint::Weak<App>) {
    ShellClient { weak, by_person: true }.open_manager();
}

/// Wire the tile's press and start the self-heal's clock.
pub fn wire(ui: &App) {
    let weak = ui.as_weak();
    ui.global::<NetworkState>().on_tile_pressed(move || {
        let weak = weak.clone();
        std::thread::spawn(move || {
            let Some(snapshot) = reading() else { return };
            let client = ShellClient { weak, by_person: true };
            if let Some(outcome) = wifi_reconnect::press(&snapshot, &client) {
                log_outcome(&outcome, "the Quick Settings tile");
            }
        });
    });

    let weak = ui.as_weak();
    HEAL_TIMER.with(|timer| {
        timer.start(slint::TimerMode::Repeated, HEAL_EVERY, move || {
            let Some(snapshot) = yantrik_os::network::latest() else { return };
            let left = LEFT_ON_PURPOSE.load(Ordering::Relaxed);
            let Some(attempt) = HEAL.with(|heal| heal.borrow_mut().observe(Instant::now(), &snapshot, left)) else {
                return;
            };
            tracing::info!(
                attempt = attempt.number,
                ssid = %attempt.ssid,
                "Wi-Fi has been down for a minute with the radio on and NetworkManager not trying; \
                 asking it to rejoin"
            );
            let weak = weak.clone();
            std::thread::spawn(move || {
                let Some(snapshot) = reading() else { return };
                // Not a person's: nothing opens by itself, and only a saved network is joined.
                let client = ShellClient { weak, by_person: false };
                log_outcome(&wifi_reconnect::reconnect(&snapshot, &client), "the shell, by itself");
            });
        });
    });
}

/// One new reading, from `wire::network`: the tile's words, and a drop's notice.
pub fn observe(ui: &App, snapshot: &NetworkSnapshot) {
    let tile = wifi_reconnect::Tile::from(snapshot);
    let state = ui.global::<NetworkState>();
    let title = tile.title();
    if state.get_tile_title().as_str() != title {
        state.set_tile_title(title.as_str().into());
    }
    if state.get_tile_hint().as_str() != tile.hint() {
        state.set_tile_hint(tile.hint().into());
    }

    if snapshot.wifi_joined() {
        LEFT_ON_PURPOSE.store(false, Ordering::Relaxed);
    }
    let left = LEFT_ON_PURPOSE.load(Ordering::Relaxed);
    let Some(notice) = DROPS.with(|drops| drops.borrow_mut().observe(Instant::now(), snapshot, left)) else {
        return;
    };
    tracing::info!("Wi-Fi dropped with the radio on; offering Reconnect and Choose network");
    let mut notification = notify::Notification::new("Yantrik", notice.title)
        .body(notice.body)
        .urgency(Urgency::Normal);
    for (id, label) in notice.actions {
        notification = notification.action(id, label);
    }
    notify::send(notification);
}
