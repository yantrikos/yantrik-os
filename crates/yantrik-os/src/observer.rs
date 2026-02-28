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
            let h = spawn_mock(event_tx);
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

            // Network monitor (D-Bus NetworkManager)
            let tx = event_tx.clone();
            let h = std::thread::Builder::new()
                .name("yos-network".into())
                .spawn(move || {
                    crate::network::run_network_monitor(tx);
                })
                .expect("failed to spawn network monitor");
            handles.push(h);

            // Notification daemon (session D-Bus — org.freedesktop.Notifications)
            let tx = event_tx.clone();
            let h = std::thread::Builder::new()
                .name("yos-notifications".into())
                .spawn(move || {
                    crate::notifications::run_notification_daemon(tx);
                })
                .expect("failed to spawn notification daemon");
            handles.push(h);

            // Keybind daemon (session D-Bus — org.yantrik.Keybinds)
            let tx = event_tx;
            let h = std::thread::Builder::new()
                .name("yos-keybinds".into())
                .spawn(move || {
                    crate::keybinds::run_keybind_daemon(tx);
                })
                .expect("failed to spawn keybind daemon");
            handles.push(h);
        }

        Self {
            event_rx,
            _handles: handles,
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
