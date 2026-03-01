//! Wire modules — callback wiring registry.
//!
//! Each sub-module has a `pub fn wire(ui: &App, ctx: &AppContext)` function
//! that registers Slint callbacks for one concern.
//!
//! To add a new feature: create a new file, add `mod` + one `wire()` call here.
//! main.rs stays untouched.

mod app_grid;
mod callbacks;
mod chat;
mod clipboard;
mod dock;
mod lens;
mod navigate;
mod power;
mod system_poll;
mod timers;
mod voice_mode;
mod window_switcher;

use crate::app_context::AppContext;
use crate::App;

/// Wire all Slint callbacks. Called once from main().
pub fn wire_all(ui: &App, ctx: &AppContext) {
    timers::wire(ui, ctx);
    chat::wire(ui, ctx);
    clipboard::wire(ui, ctx);
    lens::wire(ui, ctx);
    navigate::wire(ui, ctx);
    dock::wire(ui, ctx);
    power::wire(ui, ctx);
    app_grid::wire(ui, ctx);
    window_switcher::wire(ui, ctx);
    voice_mode::wire(ui, ctx);
    system_poll::wire(ui, ctx);
    callbacks::wire(ui, ctx);
}
