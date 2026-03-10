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
pub mod dbus_notif;

mod battery;
mod files;
pub mod keybinds;
mod mock;
mod network;
mod notifications;
mod processes;

pub use events::{FileChangeKind, ProcessInfo, SystemEvent, SystemSnapshot};
pub use event_bus::{
    CardAction, CommitmentAlertType, EventBus, EventKind, EventLog, EventLogEntry,
    EventSource, EventStats, ToolOutcome, TraceId, YantrikEvent,
};
pub use entity_graph::{EntityGraph, ObjectKind, RelationKind, Relation, UniversalObject};
pub use observer::{SystemObserver, SystemObserverConfig};
