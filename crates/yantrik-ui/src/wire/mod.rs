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
pub mod image_viewer;
mod lens;
pub mod media_player;
mod navigate;
mod power;
pub mod screenshot;
pub mod settings;
mod system_poll;
pub mod text_editor;
mod timers;
mod voice_mode;
pub mod terminal;
pub mod notes;
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
    settings::wire(ui, ctx);
    system_poll::wire(ui, ctx);
    image_viewer::wire(ui, ctx);
    text_editor::wire(ui, ctx);
    media_player::wire(ui, ctx);
    terminal::wire(ui, ctx);
    notes::wire(ui, ctx);
    screenshot::wire(ui, ctx);
    callbacks::wire(ui, ctx);
}
