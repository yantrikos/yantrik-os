//! Wire modules — callback wiring registry.
//!
//! Each sub-module has a `pub fn wire(ui: &App, ctx: &AppContext)` function
//! that registers Slint callbacks for one concern.
//!
//! To add a new feature: create a new file, add `mod` + one `wire()` call here.
//! main.rs stays untouched.

mod about;
mod app_grid;
pub mod dep_check;
mod callbacks;
mod chat;
mod clipboard;
mod dock;
pub mod i18n;
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
pub mod toast;
mod voice_mode;
pub mod terminal;
pub mod notes;
pub mod email;
pub mod calendar;
pub mod network_manager;
pub mod package_manager;
pub mod skill_store;
pub mod system_monitor;
pub mod weather;
pub mod music_player;
pub mod download_manager;
pub mod snippet_manager;
pub mod container_manager;
pub mod device_dashboard;
pub mod permission_dashboard;
pub mod spreadsheet;
pub mod document_editor;
pub mod presentation;
pub mod version;
pub mod ai_assist;
pub mod ai_onboarding;
pub mod ai_provider;
mod morning_brief;
mod window_switcher;
pub mod entity_bridge;
pub mod app_framework;
pub mod cross_app;
pub mod universal_actions;
pub mod command_palette;
pub mod installer;
pub mod login;

use crate::app_context::AppContext;
use crate::App;

/// Wire all Slint callbacks. Called once from main().
pub fn wire_all(ui: &App, ctx: &AppContext) {
    dep_check::log_dep_summary();
    i18n::wire(ui, ctx);
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
    email::wire(ui, ctx);
    calendar::wire(ui, ctx);
    network_manager::wire(ui, ctx);
    package_manager::wire(ui, ctx);
    system_monitor::wire(ui, ctx);
    weather::wire(ui, ctx);
    music_player::wire(ui, ctx);
    screenshot::wire(ui, ctx);
    toast::wire(ui, ctx);
    skill_store::wire(ui, ctx);
    download_manager::wire(ui, ctx);
    snippet_manager::wire(ui, ctx);
    container_manager::wire(ui, ctx);
    device_dashboard::wire(ui, ctx);
    permission_dashboard::wire(ui, ctx);
    spreadsheet::wire(ui, ctx);
    document_editor::wire(ui, ctx);
    presentation::wire(ui, ctx);
    about::wire(ui, ctx);
    version::wire(ui, ctx);
    morning_brief::wire(ui, ctx);
    command_palette::wire(ui, ctx);
    cross_app::wire(ui, ctx);
    ai_onboarding::wire(ui, ctx);
    ai_provider::wire(ui, ctx);
    installer::wire(ui, ctx);
    login::wire(ui, ctx);
    callbacks::wire(ui, ctx);
}
