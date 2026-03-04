//! Settings wiring — persistent user preferences via ~/.config/yantrik/settings.yaml.

use serde::{Deserialize, Serialize};
use slint::ComponentHandle;
use std::sync::{Arc, Mutex};

use crate::app_context::AppContext;
use crate::{App, AccentPreset, ThemeMode};

/// Accent color preset names in cycle order (matches AccentPreset.index).
const ACCENT_PRESETS: &[&str] = &["cyan", "amber", "purple", "green", "pink"];

/// All user-facing settings that persist across reboots.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UserSettings {
    pub dark_mode: bool,
    pub accent_color: String,
    pub tool_permission: String,
    pub auto_lock_secs: i32,
    pub dnd_mode: bool,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            dark_mode: true,
            accent_color: "cyan".into(),
            tool_permission: "sensitive".into(),
            auto_lock_secs: 300,
            dnd_mode: false,
        }
    }
}

/// Shared handle for persisting settings from callbacks.
type SharedSettings = Arc<Mutex<UserSettings>>;

fn settings_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    format!("{}/.config/yantrik/settings.yaml", home)
}

/// Load persisted settings (or defaults if missing/corrupt).
pub fn load() -> UserSettings {
    let path = settings_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_yaml::from_str(&content).unwrap_or_else(|e| {
            tracing::warn!("Corrupt settings.yaml, using defaults: {e}");
            UserSettings::default()
        }),
        Err(_) => {
            // Migrate from old theme.yaml if it exists
            let mut settings = UserSettings::default();
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            let old_theme = format!("{}/.config/yantrik/theme.yaml", home);
            if let Ok(content) = std::fs::read_to_string(&old_theme) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("dark:") {
                        let val = trimmed.trim_start_matches("dark:").trim();
                        settings.dark_mode = val != "false";
                    } else if trimmed.starts_with("accent_color:") {
                        let val = trimmed.trim_start_matches("accent_color:").trim().trim_matches('"');
                        if ACCENT_PRESETS.contains(&val) {
                            settings.accent_color = val.to_string();
                        }
                    }
                }
                save(&settings);
                let _ = std::fs::remove_file(&old_theme);
                tracing::info!("Migrated theme.yaml → settings.yaml");
            }
            settings
        }
    }
}

/// Persist settings to YAML.
fn save(settings: &UserSettings) {
    let path = settings_path();
    if let Some(parent) = std::path::Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_yaml::to_string(settings) {
        Ok(yaml) => {
            if let Err(e) = std::fs::write(&path, yaml) {
                tracing::warn!("Failed to write settings: {e}");
            }
        }
        Err(e) => tracing::warn!("Failed to serialize settings: {e}"),
    }
}

/// Save via shared handle (used from callbacks).
fn persist(shared: &SharedSettings) {
    if let Ok(s) = shared.lock() {
        save(&s);
    }
}

/// Convert accent color name to AccentPreset index.
pub fn accent_name_to_index(name: &str) -> i32 {
    match name {
        "cyan" => 0,
        "amber" => 1,
        "purple" => 2,
        "green" => 3,
        "pink" => 4,
        _ => 0,
    }
}

/// Wire settings callbacks with persistence.
pub fn wire(ui: &App, ctx: &AppContext) {
    let settings = Arc::new(Mutex::new(load()));

    // Dark mode toggle
    let ui_weak = ui.as_weak();
    let s = settings.clone();
    ui.on_toggle_dark_mode(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let new_val = !ui.get_settings_dark_mode();
        ui.set_settings_dark_mode(new_val);
        ui.global::<ThemeMode>().set_dark(new_val);
        if let Ok(mut st) = s.lock() {
            st.dark_mode = new_val;
        }
        persist(&s);
    });

    // Cycle accent color: cyan → amber → purple → green → pink → cyan
    let ui_weak = ui.as_weak();
    let s = settings.clone();
    ui.on_cycle_accent_color(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let current = ui.get_settings_accent_color().to_string();
        let current_idx = accent_name_to_index(&current);
        let next_idx = (current_idx + 1) % ACCENT_PRESETS.len() as i32;
        let next_name = ACCENT_PRESETS[next_idx as usize];
        ui.set_settings_accent_color(next_name.into());
        ui.global::<AccentPreset>().set_index(next_idx);
        if let Ok(mut st) = s.lock() {
            st.accent_color = next_name.to_string();
        }
        persist(&s);
        tracing::info!(from = %current, to = next_name, "Accent color changed");
    });

    // Cycle tool permission: safe → standard → sensitive → safe
    let ui_weak = ui.as_weak();
    let s = settings.clone();
    ui.on_cycle_tool_permission(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let current = ui.get_settings_tool_permission().to_string();
        let next = match current.as_str() {
            "safe" => "standard",
            "standard" => "sensitive",
            _ => "safe",
        };
        ui.set_settings_tool_permission(next.into());
        if let Ok(mut st) = s.lock() {
            st.tool_permission = next.to_string();
        }
        persist(&s);
        tracing::info!(from = %current, to = next, "Tool permission level changed");
    });

    // Incognito mode toggle — intentionally NO persistence (resets on boot)
    let ui_weak = ui.as_weak();
    let bridge = ctx.bridge.clone();
    ui.on_toggle_incognito_mode(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let new_val = !ui.get_settings_incognito_mode();
        ui.set_settings_incognito_mode(new_val);
        bridge.set_incognito(new_val);
        tracing::info!(incognito = new_val, "Incognito mode toggled");
    });

    // Do Not Disturb toggle — persists across reboots
    let ui_weak = ui.as_weak();
    let s = settings.clone();
    ui.on_toggle_dnd_mode(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let new_val = !ui.get_dnd_mode();
        ui.set_dnd_mode(new_val);
        if let Ok(mut st) = s.lock() {
            st.dnd_mode = new_val;
        }
        persist(&s);
        tracing::info!(dnd = new_val, "Do Not Disturb toggled");
    });

    // Cycle auto-lock timeout: 30s → 1m → 2m → 5m → 10m → never → 30s
    let ui_weak = ui.as_weak();
    let s = settings.clone();
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
        if let Ok(mut st) = s.lock() {
            st.auto_lock_secs = next;
        }
        persist(&s);
        tracing::info!(from = current, to = next, "Auto-lock timeout changed");
    });
}

/// Back-compat: load theme preference (delegates to full settings).
pub fn load_theme_preference() -> bool {
    load().dark_mode
}
