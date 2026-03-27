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
use yantrik_companion::CompanionConfig;

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

// Slint-generated types live in yantrik-ui-slint (separate crate so that
// Rust-only changes here don't trigger Slint recompilation).
pub use yantrik_ui_slint::*;

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

    // Propagate OAuth credentials from config to env vars (if not already set)
    if let Some(ref id) = config.connectors.google_client_id {
        if std::env::var("GOOGLE_CLIENT_ID").is_err() {
            std::env::set_var("GOOGLE_CLIENT_ID", id);
        }
    }
    if let Some(ref secret) = config.connectors.google_client_secret {
        if std::env::var("GOOGLE_CLIENT_SECRET").is_err() {
            std::env::set_var("GOOGLE_CLIENT_SECRET", secret);
        }
    }

    // Create Slint UI
    let ui = App::new().unwrap();

    // Initialize all shared state
    let ctx = app_context::AppContext::init(config, &ui, config_path);

    // Wire all callbacks
    wire::wire_all(&ui, &ctx);

    // Start background services
    let service_manager = start_services();

    // Debug: navigate to specific screen on startup via env var
    if let Ok(screen_str) = std::env::var("YANTRIK_START_SCREEN") {
        if let Ok(screen) = screen_str.parse::<i32>() {
            tracing::info!(screen, "Debug: navigating to startup screen");
            ui.set_current_screen(screen);
            ui.invoke_navigate(screen);
        }
    }

    // Run
    tracing::info!("Starting Yantrik OS desktop shell");
    ui.run().unwrap();

    // Clean shutdown
    tracing::info!("Yantrik OS shutting down");
    service_manager.stop_all();
}

/// Start background services via the ServiceManager.
/// Discovers services from manifests in the services directory, falling back
/// to hardcoded registrations for built-in services.
fn start_services() -> yantrik_shell_core::service_manager::ServiceManager {
    use yantrik_shell_core::service_manager::ServiceManager;

    // Services binary dir: same directory as the main yantrik-ui binary
    let bin_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    let mgr = ServiceManager::new(bin_dir.clone());

    // Try manifest-based discovery first (installed services have yantrik.toml)
    let services_dir = bin_dir.join("services");
    if services_dir.is_dir() {
        mgr.scan_and_register(&services_dir);
        tracing::info!(path = %services_dir.display(), "Scanned service manifests");
    }

    // Register built-in services as fallback (for dev builds without manifest dirs)
    mgr.register("weather", "weather-service", true);
    mgr.register("system-monitor", "system-monitor-service", true);
    mgr.register("notes", "notes-service", false);
    mgr.register("notifications", "notifications-service", true);
    mgr.register("calendar", "calendar-service", false);
    mgr.register("network", "network-service", true);
    mgr.register("email", "email-service", false);

    // Start autostart services (best-effort — binary may not exist in dev)
    mgr.start_autostart();

    mgr
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
