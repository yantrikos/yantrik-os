//! Settings wiring — theme toggle with persistence.

use slint::ComponentHandle;

use crate::app_context::AppContext;
use crate::{App, ThemeMode};

/// Wire settings callbacks.
pub fn wire(ui: &App, _ctx: &AppContext) {
    let ui_weak = ui.as_weak();
    ui.on_toggle_dark_mode(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let new_val = !ui.get_settings_dark_mode();
        ui.set_settings_dark_mode(new_val);
        ui.global::<ThemeMode>().set_dark(new_val);
        persist_theme(new_val);
    });

    // Cycle tool permission: safe → standard → sensitive → safe
    let ui_weak = ui.as_weak();
    ui.on_cycle_tool_permission(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let current = ui.get_settings_tool_permission().to_string();
        let next = match current.as_str() {
            "safe" => "standard",
            "standard" => "sensitive",
            _ => "safe",
        };
        ui.set_settings_tool_permission(next.into());
        tracing::info!(from = %current, to = next, "Tool permission level changed");
    });

    // Cycle auto-lock timeout: 30s → 1m → 2m → 5m → 10m → never → 30s
    let ui_weak = ui.as_weak();
    ui.on_cycle_auto_lock(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let current = ui.get_settings_auto_lock_secs();
        let next = match current {
            30 => 60,
            60 => 120,
            120 => 300,
            300 => 600,
            600 => 0,
            _ => 30,
        };
        ui.set_settings_auto_lock_secs(next);
        tracing::info!(from = current, to = next, "Auto-lock timeout changed");
    });
}

/// Load theme preference from ~/.config/yantrik/theme.yaml.
/// Returns true (dark) if file doesn't exist or can't be parsed.
pub fn load_theme_preference() -> bool {
    let path = theme_config_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            // Simple YAML: look for "dark: false" to enable light mode
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("dark:") {
                    let val = trimmed.trim_start_matches("dark:").trim();
                    return val != "false";
                }
            }
            true // default dark
        }
        Err(_) => true, // default dark if no file
    }
}

/// Persist theme preference to ~/.config/yantrik/theme.yaml.
fn persist_theme(dark: bool) {
    let path = theme_config_path();
    if let Some(parent) = std::path::Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let content = format!("dark: {}\n", dark);
    if let Err(e) = std::fs::write(&path, content) {
        tracing::warn!("Failed to persist theme: {e}");
    }
}

fn theme_config_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    format!("{}/.config/yantrik/theme.yaml", home)
}
