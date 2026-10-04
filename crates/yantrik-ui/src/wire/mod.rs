//! Wire modules — callback wiring registry.
//!
//! Each sub-module has a `pub fn wire(ui: &App, ctx: &AppContext)` function
//! that registers Slint callbacks for one concern.
//!
//! To add a new feature: create a new file, add `mod` + one `wire()` call here.
//! main.rs stays untouched.

mod about;
/// The volume slider and the machine's real volume (PipeWire).
pub mod audio;
/// The brightness slider and the machine's real backlight.
pub mod backlight;
/// The volume / brightness / mic / Caps Lock display: what it says (the pill is the kit's YOsd).
pub mod osd;
// The Agents screen and every agent popped out into its own window.
pub mod agents;
#[cfg(test)]
mod lens_card_layout;
pub mod lens_work;
pub mod agents_workroom;
pub mod runs_on_card;
// The Recipes screen: every recipe as its stages, live.
pub mod recipes;
mod app_grid;
/// The launcher's search and its Running section.
pub mod launcher;
pub mod dep_check;
mod callbacks;
pub(crate) mod files;
/// Launching the app the one MIME rule picked: double-click, "Open with" and `files_open`
/// all run their decision through here (#233).
pub mod open_with;
pub(crate) mod chat;
mod clipboard;
mod shell_overlays;
mod cheat_sheet;
pub mod network;
pub mod dock;
/// A launch by name for an app already open on the desktop raises its window instead.
pub(crate) mod already_open;
/// The grounded dock's buttons and window list, published from the window list.
pub mod dock_bar;
pub mod harness;
pub mod i18n;
mod lens;
mod navigate;
pub mod notifications;
mod power;
pub mod screenshot;
pub mod settings;
pub mod theme;
pub mod theme_files;
mod system_poll;
mod timers;
mod today;
pub mod toast;
mod voice_mode;
pub mod apt;
pub mod flatpak;
pub mod package_manager;
pub mod skill_store;
pub mod device_dashboard;
pub mod permission_dashboard;
pub mod problem_report;
pub mod version;
pub mod ai_assist;
pub mod ai_onboarding;
pub mod ai_provider;
pub mod ai_status;
pub mod harness_provider;
pub mod provider_catalogue;
pub mod provider_models;
pub mod provider_panel;
pub mod decision_model;
pub mod boot;
pub mod location;
pub mod pins;
mod morning_brief;
mod window_switcher;
/// The taskbar's corner button, Super+D and `show_desktop`: every window away, then back (#241).
pub mod show_desktop;
pub mod entity_bridge;
pub mod app_framework;
pub mod cross_app;
pub mod universal_actions;
pub mod command_palette;
pub mod installer;
pub mod installer_disk;
pub mod installer_locale;
pub mod login;
pub mod minds_panel;
pub mod services;
pub mod vault;
pub mod free_ai;

use crate::app_context::AppContext;
use crate::App;

/// Wire all Slint callbacks. Called once from main().
pub fn wire_all(ui: &App, ctx: &AppContext) {
    dep_check::log_dep_summary();
    i18n::wire(ui, ctx);
    timers::wire(ui, ctx);
    chat::wire(ui, ctx);
    harness::wire(ui, ctx);
    clipboard::wire(ui, ctx);
    shell_overlays::wire(ui, ctx);
    cheat_sheet::wire(ui);
    crate::power_status::wire(ui);
    // The network mark and its popover, and the one reading every surface shows of the network.
    network::wire(ui);
    lens::wire(ui, ctx);
    navigate::wire(ui, ctx);
    dock::wire(ui, ctx);
    power::wire(ui, ctx);
    audio::wire(ui, ctx);
    backlight::wire(ui, ctx);
    app_grid::wire(ui, ctx);
    window_switcher::wire(ui, ctx);
    crate::control_switcher::wire(ui);
    show_desktop::wire(ui);
    voice_mode::wire(ui, ctx);
    settings::wire(ui, ctx);
    system_poll::wire(ui, ctx);
    package_manager::wire(ui, ctx);
    screenshot::wire(ui, ctx);
    // Owns the notification centre, the toasts and the poll of the one store. `toast` is the
    // drawing half and has no `wire` of its own any more.
    notifications::wire(ui, ctx);
    skill_store::wire(ui, ctx);
    device_dashboard::wire(ui, ctx);
    permission_dashboard::wire(ui, ctx);
    problem_report::wire(ui, ctx);
    agents::wire(ui, ctx);
    // After `harness` and `agents`: the panel reads the host and the Agents store they set up.
    crate::mind_panel::wire(ui);
    // The Minds panel under the mind chip: the person's accounts and what each has left.
    minds_panel::wire(ui, ctx);
    recipes::wire(ui, ctx);
    about::wire(ui, ctx);
    version::wire(ui, ctx);
    morning_brief::wire(ui, ctx);
    command_palette::wire(ui, ctx);
    cross_app::wire(ui, ctx);
    ai_onboarding::wire(ui, ctx);
    ai_provider::wire(ui, ctx);
    decision_model::wire(ui, ctx);
    free_ai::wire(ui, ctx);
    installer::wire(ui, ctx);
    login::wire(ui, ctx);
    // After `login`, which is the other place a secret reaches the vault, and before `callbacks`,
    // which owns the lock screen that closes it.
    vault::wire(ui, ctx);
    callbacks::wire(ui, ctx);

    // Last, and on a thread of its own. It must come after `settings::wire`, which is what
    // publishes the live settings this reads and writes; and it touches the network, so nothing
    // above it should have to wait for it.
    location::detect_in_background();
}
