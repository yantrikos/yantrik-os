//! SystemObserver — spawns monitor threads, fans events into one channel.
//!
//! Start it, then poll `try_recv()` from the main thread's Timer callback.
//! All monitors send their events to a single crossbeam sender.

use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};

use crate::events::SystemEvent;

/// Configuration for the system observer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemObserverConfig {
    /// Use mock mode (fake events on timers, for QEMU dev).
    #[serde(default)]
    pub mock: bool,

    /// Directories to watch for file changes.
    #[serde(default = "default_watch_dirs")]
    pub watch_dirs: Vec<String>,

    /// Process poll interval in seconds.
    #[serde(default = "default_process_poll_secs")]
    pub process_poll_secs: u64,

    /// Resource poll interval in seconds (CPU, memory, disk).
    #[serde(default = "default_resource_poll_secs")]
    pub resource_poll_secs: u64,
}

fn default_watch_dirs() -> Vec<String> {
    vec![
        "~/Downloads".to_string(),
        "~/Documents".to_string(),
        "~/Desktop".to_string(),
    ]
}

fn default_process_poll_secs() -> u64 {
    5
}

fn default_resource_poll_secs() -> u64 {
    10
}

impl Default for SystemObserverConfig {
    fn default() -> Self {
        Self {
            mock: false,
            watch_dirs: default_watch_dirs(),
            process_poll_secs: default_process_poll_secs(),
            resource_poll_secs: default_resource_poll_secs(),
        }
    }
}

/// The system observer. Spawns background threads that monitor the machine
/// and fan all events into a single crossbeam channel.
pub struct SystemObserver {
    event_rx: Receiver<SystemEvent>,
    /// Kept so something outside the observer can put an event on the same channel — see
    /// [`SystemObserver::inject`].
    event_tx: Sender<SystemEvent>,
    _handles: Vec<std::thread::JoinHandle<()>>,
}

impl SystemObserver {
    /// Start the system observer. Spawns monitor threads immediately.
    pub fn start(config: &SystemObserverConfig) -> Self {
        let (event_tx, event_rx) = crossbeam_channel::bounded(256);

        let mut handles = Vec::new();

        if config.mock {
            // Mock mode — emit fake events on timers (for QEMU dev)
            tracing::info!("SystemObserver starting in MOCK mode");
            let h = spawn_mock(event_tx.clone());
            handles.push(h);
        } else {
            // Real monitors
            tracing::info!("SystemObserver starting real monitors");

            // Process + resource monitor (sysinfo, no async)
            let tx = event_tx.clone();
            let process_secs = config.process_poll_secs;
            let resource_secs = config.resource_poll_secs;
            let h = std::thread::Builder::new()
                .name("yos-processes".into())
                .spawn(move || {
                    crate::processes::run_process_monitor(tx, process_secs, resource_secs);
                })
                .expect("failed to spawn process monitor");
            handles.push(h);

            // File watcher (notify/inotify)
            let tx = event_tx.clone();
            let dirs = config.watch_dirs.clone();
            let h = std::thread::Builder::new()
                .name("yos-files".into())
                .spawn(move || {
                    crate::files::run_file_watcher(tx, &dirs);
                })
                .expect("failed to spawn file watcher");
            handles.push(h);

            // Battery monitor (D-Bus UPower)
            let tx = event_tx.clone();
            let h = std::thread::Builder::new()
                .name("yos-battery".into())
                .spawn(move || {
                    crate::battery::run_battery_monitor(tx);
                })
                .expect("failed to spawn battery monitor");
            handles.push(h);

            // Power profiles (D-Bus power-profiles-daemon). Ends at once without the daemon.
            let tx = event_tx.clone();
            let h = std::thread::Builder::new()
                .name("yos-power-profile".into())
                .spawn(move || {
                    crate::power_profile::run_power_profile_monitor(tx);
                })
                .expect("failed to spawn power profile monitor");
            handles.push(h);

            // Network monitor (D-Bus NetworkManager)
            let tx = event_tx.clone();
            let h = std::thread::Builder::new()
                .name("yos-network".into())
                .spawn(move || {
                    crate::network::run_network_monitor(tx);
                })
                .expect("failed to spawn network monitor");
            handles.push(h);

            // The volume, wherever it was changed (PipeWire, through pactl subscribe).
            let tx = event_tx.clone();
            let h = std::thread::Builder::new()
                .name("yos-audio".into())
                .spawn(move || {
                    crate::audio::run_audio_watcher(tx);
                })
                .expect("failed to spawn audio watcher");
            handles.push(h);

            // No notification daemon here.
            //
            // This used to hold `org.freedesktop.Notifications` from a thread of its own — and
            // so did mako, started from the same session's labwc autostart. Only one process
            // can own a well-known name, so which of the two a `notify-send` reached depended
            // on start order; the audit of 17 September caught mako winning by a few hundred
            // milliseconds, which left the shell's notification centre empty on a machine that
            // was showing popups all day. The name belongs to the notifications service now,
            // which is also the one store, and the shell reads that store over its socket.
            // See services/notifications-service/src/freedesktop.rs.

            // The person at the seat (#412): without this, nothing said the machine was left
            // alone and the auto-lock never fired.
            #[cfg(target_os = "linux")]
            {
                let tx = event_tx.clone();
                let h = std::thread::Builder::new()
                    .name("yos-idle".into())
                    .spawn(move || {
                        crate::idle::run_idle_monitor(tx);
                    })
                    .expect("failed to spawn idle monitor");
                handles.push(h);
            }
        }

        Self {
            event_rx,
            event_tx,
            _handles: handles,
        }
    }

    /// Put an event on the same channel the monitor threads use.
    ///
    /// For a fact about the machine that is observed somewhere other than in this crate. The
    /// shell learns about notifications by polling the notifications service — which owns
    /// `org.freedesktop.Notifications`, because only one process can — and puts each one back
    /// here so the feature registry, the activity feed and the system context see them exactly
    /// as they did when this crate ran the daemon itself.
    ///
    /// Drops the event rather than blocking if the channel is full: the channel is bounded at
    /// 256 and the only reason it would fill is that nothing is draining it, in which case
    /// waiting would hold up whoever called this.
    pub fn inject(&self, event: SystemEvent) {
        if self.event_tx.try_send(event).is_err() {
            tracing::debug!("system event channel is full; an injected event was dropped");
        }
    }

    /// Non-blocking: try to receive the next event.
    pub fn try_recv(&self) -> Option<SystemEvent> {
        self.event_rx.try_recv().ok()
    }

    /// Drain all pending events (non-blocking).
    pub fn drain(&self) -> Vec<SystemEvent> {
        let mut events = Vec::new();
        while let Some(event) = self.try_recv() {
            events.push(event);
        }
        events
    }
}

fn spawn_mock(tx: Sender<SystemEvent>) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("yos-mock".into())
        .spawn(move || {
            crate::mock::run_mock_observer(tx);
        })
        .expect("failed to spawn mock observer")
}
