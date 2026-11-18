//! The Bench: an ordinary desktop that remembers (issue #1030, phase 1).
//!
//! Everything here is what the desktop's icons are made of, with no Slint and no shell in it, so
//! each rule can be tested without a window:
//!
//! - [`items`]: what `~/Desktop` holds and what kind each thing is, with names made safe to draw;
//! - [`shortcut`]: a `.desktop` file read as an app shortcut;
//! - [`layout`]: where each icon sits, per screen size, and the file that remembers it;
//! - [`thumbs`]: the picture on an icon, cut once per (path, mtime) and kept in the cache;
//! - [`archive`]: putting something away without deleting it, and bringing it back;
//! - [`rename`]: the one rename that never replaces what is at the destination;
//! - [`recent`]: the one thing the Continue row offers, kept by itself.
//!
//! Nothing in this crate deletes a file the person made. The one thing it removes is its own
//! thumbnail cache, and only to keep that under a size.

pub mod archive;
mod atomic;
pub mod items;
pub mod layout;
pub mod recent;
pub mod rename;
pub mod shortcut;
pub mod thumbs;
