//! Pure tool implementations for the Yantrik companion agent.
//!
//! These tools depend only on companion-core (Tool trait, ToolContext, helpers)
//! and have no dependency on the full companion crate. This enables faster
//! incremental builds — changing a tool file only recompiles this crate (~15K LOC)
//! instead of the entire companion (~81K LOC).

// Re-export core types at crate root so `use super::{Tool, ...}` works in tool files.
pub use yantrik_companion_core::tools::*;
pub use yantrik_companion_core::permission::{PermissionLevel, parse_permission};
pub use yantrik_companion_core::sanitize;

pub mod antivirus;
pub mod archive;
pub mod artifacts;
pub mod bluetooth;
pub mod browser;
pub mod browser_lifecycle;
pub mod calculator;
pub mod canvas;
pub mod claude;
pub mod clipboard;
pub mod coder;
pub mod desktop;
pub mod discovery;
pub mod disk;
pub mod display;
pub mod docker;
pub mod edit;
pub mod encoding;
pub mod files;
pub mod firewall;
pub mod git;
pub mod github;
pub mod glob;
pub mod grep;
pub mod home_assistant;
pub mod knowledge;
pub mod media;
pub mod memory_hygiene;
pub mod network;
pub mod networking;
pub mod package;
pub mod plugin;
pub mod process;
pub mod project;
pub mod service;
pub mod ssh;
pub mod system;
pub mod terminal;
pub mod terminal_analysis;
pub mod text;
pub mod time;
pub mod vault;
pub mod vision;
pub mod wallpaper;
pub mod weather;
pub mod wifi;
pub mod window;
pub mod workspace;

use yantrik_companion_core::tools::ToolRegistry;

/// Register all pure tools into the given registry.
pub fn register_all(reg: &mut ToolRegistry) {
    antivirus::register(reg);
    archive::register(reg);
    artifacts::register(reg);
    bluetooth::register(reg);
    browser::register(reg);
    browser_lifecycle::register(reg);
    calculator::register(reg);
    // canvas and vision require config params — registered by companion
    claude::register(reg);
    clipboard::register(reg);
    coder::register(reg);
    desktop::register(reg);
    discovery::register(reg);
    disk::register(reg);
    display::register(reg);
    docker::register(reg);
    edit::register(reg);
    encoding::register(reg);
    files::register(reg);
    firewall::register(reg);
    git::register(reg);
    // github requires config param — registered by companion
    glob::register(reg);
    grep::register(reg);
    // home_assistant requires config params — registered by companion
    knowledge::register(reg);
    media::register(reg);
    memory_hygiene::register(reg);
    // network requires config params — registered by companion
    networking::register(reg);
    package::register(reg);
    plugin::load_plugins(reg);
    process::register(reg);
    project::register(reg);
    service::register(reg);
    ssh::register(reg);
    system::register(reg);
    terminal::register(reg);
    terminal_analysis::register(reg);
    text::register(reg);
    time::register(reg);
    vault::register(reg);
    wallpaper::register(reg);
    weather::register(reg);
    wifi::register(reg);
    window::register(reg);
    workspace::register(reg);
}
