//! Yantrik OS — system integration layer.
//!
//! Observes the machine via D-Bus, inotify, and sysinfo.
//! Emits `SystemEvent` variants over a crossbeam channel.
//!
//! Zero AI dependencies. This crate knows nothing about LLMs,
//! memory databases, or companions. It just watches the machine.

pub mod events;
pub mod event_bus;
pub mod entity_graph;
pub mod observer;
pub mod screenshot;

mod battery;
mod files;
#[cfg(target_os = "linux")]
mod idle;
pub mod keybinds;
mod mock;
mod network;
mod processes;

pub use events::{FileChangeKind, ProcessInfo, SystemEvent, SystemSnapshot};
pub use event_bus::{
    CardAction, CommitmentAlertType, EventBus, EventKind, EventLog, EventLogEntry,
    EventSource, EventStats, ToolOutcome, TraceId, YantrikEvent,
};
pub use entity_graph::{EntityGraph, ObjectKind, RelationKind, Relation, UniversalObject};
pub use observer::{SystemObserver, SystemObserverConfig};

/// Whether the compositor is telling the desktop when the person leaves the seat (#412). Without
/// it the auto-lock cannot fire, and the desktop must say so rather than promise it.
pub fn idle_watch_active() -> bool {
    #[cfg(target_os = "linux")]
    {
        idle::watching()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}
