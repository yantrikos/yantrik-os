//! One headless mpv, driven over its JSON IPC socket.
//!
//! The app runs one `mpv --idle` at a time, with no window, no terminal, no config and no
//! scripts, and talks to it over a Unix socket in `$XDG_RUNTIME_DIR` that only this account can
//! open (mode 0600). Every command is a JSON array whose elements are mpv's own arguments, so a
//! file path travels as one string to `loadfile` and never through a shell.
//!
//! Two halves:
//!
//! - `client::Client` is the socket: a reader thread that sorts mpv's lines into replies
//!   (matched to the waiting command by `request_id`) and events, and a writer that sends one
//!   command at a time. It knows nothing about processes, which is what lets the tests drive it
//!   against a fake socket.
//! - [`Engine`] is the process: it spawns mpv, waits for the socket, observes the properties the
//!   window draws, keeps the last value mpv reported for each in [`Observed`], and stops and
//!   reaps mpv when it is stopped or dropped. `PR_SET_PDEATHSIG` covers the case where the app
//!   dies without either.

mod client;
mod engine;
mod observed;
#[cfg(test)]
mod tests;

pub use client::Event;
pub use engine::{socket_path, Engine, REDIRECTED};
pub use observed::{Moment, Observed};
