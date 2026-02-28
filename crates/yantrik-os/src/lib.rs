//! Yantrik OS — system integration layer.
//!
//! Observes the machine via D-Bus, inotify, and sysinfo.
//! Emits `SystemEvent` variants over a crossbeam channel.
//!
//! Zero AI dependencies. This crate knows nothing about LLMs,
//! memory databases, or companions. It just watches the machine.

pub mod events;
pub mod observer;

mod battery;
mod files;
mod mock;
mod network;
mod processes;

pub use events::{FileChangeKind, ProcessInfo, SystemEvent, SystemSnapshot};
pub use observer::{SystemObserver, SystemObserverConfig};
