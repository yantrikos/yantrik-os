//! YantrikFoundation: the rules every app, service and the shell share, with no UI and no service
//! behind them (docs/architecture.md, layer L0). One home per concept: a rule that two crates need
//! lives here, never as a copy in each.

#![warn(unreachable_pub)]

pub mod home;
pub mod json_store;
pub mod place;
pub mod problems;
pub mod time;
pub mod vault;
pub mod xdg;
