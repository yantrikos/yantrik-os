// NOTE: Crate-level allow(unused) works around rustc 1.93.1 ICE in early_lint_checks.
// The ICE is triggered by the lint emission formatter (StyledBuffer::replace panic).
// Remove once rustc is updated past 1.93.1.
#![allow(unused)]

//! Yantrik OS — AI-native desktop shell.
//!
//! The desktop's primary interface. Embeds CompanionService in-process
//! on a worker thread, renders via Slint on the main thread.
//!
//! Layout: boot animation → desktop (particle field, orb, Intent Lens).
//! The Intent Lens is the primary interaction — search, ask, launch, control.
//!
//! Modules:
//! - app_context:    Shared state bundle (AppContext) + initialization
//! - wire:           Callback wiring registry (one sub-module per concern)
//! - streaming:      Shared token streaming helper
//! - lens:           Intent Lens query routing, NL→tool matching, action resolution
//! - cards:          Whisper Card lifecycle (add, dismiss, auto-expire, sync to UI)
//! - focus:          Focus mode countdown timer
//! - notifications:  Notification store + Slint sync helpers
//! - onboarding:     First-boot marker + guided results
//! - system_context: System snapshot formatting, event→memory, config loading
//! - bridge:         Crossbeam companion bridge (send messages, query memory)
//! - features:       Proactive features (ResourceGuardian, ProcessSentinel, etc.)
//! - voice:          Voice input via Whisper
//!
//! Usage:
//!   yantrik-ui [config.yaml]

use std::path::PathBuf;
use yantrikdb_companion::CompanionConfig;

mod activity_feed;
mod ambient;
mod app_context;
// NOTE: #[allow(dead_code)] required to avoid rustc 1.93.1 ICE in check_mod_deathness.
#[allow(dead_code)]
mod apps;
mod bridge;
mod cards;
mod clipboard;
// NOTE: #[allow(dead_code)] required to avoid rustc 1.93.1 ICE in check_mod_deathness.
// Remove once rustc is updated past the fix.
#[allow(dead_code)]
mod features;
mod filebrowser;
mod focus;
mod frecency;
mod i18n;
#[allow(dead_code)]
mod mime_dispatch;
mod lens;
mod lock;
mod markdown;
mod notifications;
mod onboarding;
mod streaming;
mod system_context;
mod telegram;
mod terminal;
mod voice;
// NOTE: #[allow(dead_code)] required to avoid rustc 1.93.1 ICE in check_mod_deathness.
#[allow(dead_code)]
mod windows;
mod wire;

slint::include_modules!();

fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Load config
    let config_path = std::env::args().nth(1).map(PathBuf::from);
    let config = load_config(config_path.clone());

    // Create Slint UI
    let ui = App::new().unwrap();

    // Initialize all shared state
    let ctx = app_context::AppContext::init(config, &ui, config_path);

    // Wire all callbacks
    wire::wire_all(&ui, &ctx);

    // Run
    tracing::info!("Starting Yantrik OS desktop shell");
    ui.run().unwrap();
    tracing::info!("Yantrik OS shutting down");
}

fn load_config(path: Option<PathBuf>) -> CompanionConfig {
    match path {
        Some(p) => {
            tracing::info!(path = %p.display(), "Loading config");
            CompanionConfig::from_yaml(&p).expect("failed to load config")
        }
        None => {
            tracing::info!("Using default config");
            CompanionConfig::default()
        }
    }
}
