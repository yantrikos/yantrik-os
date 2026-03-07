//! Settings wiring — persistent user preferences via ~/.config/yantrik/settings.yaml.

use serde::{Deserialize, Serialize};
use slint::ComponentHandle;
use std::sync::{Arc, Mutex};

use crate::app_context::AppContext;
use crate::{App, AccentPreset, ThemeMode};

/// Accent color preset names in cycle order (matches AccentPreset.index).
const ACCENT_PRESETS: &[&str] = &["cyan", "amber", "purple", "green", "pink"];

/// Known wallpaper preset names.
const WALLPAPER_PRESETS: &[&str] = &["aurora", "sunset", "ocean", "nebula"];

/// All user-facing settings that persist across reboots.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UserSettings {
    pub dark_mode: bool,
    pub accent_color: String,
    pub tool_permission: String,
    pub auto_lock_secs: i32,
    pub dnd_mode: bool,
    pub wallpaper: String,
    #[serde(default)]
    pub user_name: String,
    #[serde(default)]
    pub companion_name: String,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            dark_mode: true,
            accent_color: "cyan".into(),
            tool_permission: "sensitive".into(),
            auto_lock_secs: 300,
            dnd_mode: false,
            wallpaper: String::new(),
            user_name: String::new(),
            companion_name: String::new(),
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

    // ── Connected Services ──

    // Connect service callback
    let ui_weak = ui.as_weak();
    ui.on_connect_service(move |service| {
        let svc = service.to_string();
        tracing::info!(service = %svc, "Connect service requested");

        let Some(ui) = ui_weak.upgrade() else { return };

        // Set "connecting" state immediately
        match svc.as_str() {
            "google" => ui.set_conn_google_status("connecting".into()),
            "spotify" => ui.set_conn_spotify_status("connecting".into()),
            "facebook" => ui.set_conn_facebook_status("connecting".into()),
            "instagram" => ui.set_conn_instagram_status("connecting".into()),
            _ => {}
        }

        // Simulate connection completing after 2 seconds (dummy for UI testing)
        let ui_weak2 = ui.as_weak();
        let svc2 = svc.clone();
        slint::Timer::single_shot(std::time::Duration::from_secs(2), move || {
            let Some(ui) = ui_weak2.upgrade() else { return };
            match svc2.as_str() {
                "google" => {
                    ui.set_conn_google_status("connected".into());
                    ui.set_conn_google_detail("Last sync: just now — 42 contacts, 8 events".into());
                }
                "spotify" => {
                    ui.set_conn_spotify_status("connected".into());
                    ui.set_conn_spotify_detail("Last sync: just now — 15 artists, 6 genres".into());
                }
                "facebook" => {
                    ui.set_conn_facebook_status("connected".into());
                    ui.set_conn_facebook_detail("Last sync: just now — 128 friends, 3 events".into());
                }
                "instagram" => {
                    ui.set_conn_instagram_status("connected".into());
                    ui.set_conn_instagram_detail("Last sync: just now — 5 interests, 12 hashtags".into());
                }
                _ => {}
            }
            tracing::info!(service = %svc2, "Service connected (dummy)");
        });
    });

    // Disconnect service callback
    let ui_weak = ui.as_weak();
    ui.on_disconnect_service(move |service| {
        let svc = service.to_string();
        tracing::info!(service = %svc, "Disconnect service requested");

        let Some(ui) = ui_weak.upgrade() else { return };
        match svc.as_str() {
            "google" => {
                ui.set_conn_google_status("disconnected".into());
                ui.set_conn_google_detail("".into());
            }
            "spotify" => {
                ui.set_conn_spotify_status("disconnected".into());
                ui.set_conn_spotify_detail("".into());
            }
            "facebook" => {
                ui.set_conn_facebook_status("disconnected".into());
                ui.set_conn_facebook_detail("".into());
            }
            "instagram" => {
                ui.set_conn_instagram_status("disconnected".into());
                ui.set_conn_instagram_detail("".into());
            }
            _ => {}
        }
    });

    // Sync service callback
    let ui_weak = ui.as_weak();
    ui.on_sync_service(move |service| {
        let svc = service.to_string();
        tracing::info!(service = %svc, "Sync service requested");

        let Some(ui) = ui_weak.upgrade() else { return };

        // Update detail text to show syncing
        let syncing_text = "Syncing...";
        match svc.as_str() {
            "google" => ui.set_conn_google_detail(syncing_text.into()),
            "spotify" => ui.set_conn_spotify_detail(syncing_text.into()),
            "facebook" => ui.set_conn_facebook_detail(syncing_text.into()),
            "instagram" => ui.set_conn_instagram_detail(syncing_text.into()),
            _ => {}
        }

        // Simulate sync completing
        let ui_weak2 = ui.as_weak();
        let svc2 = svc.clone();
        slint::Timer::single_shot(std::time::Duration::from_secs(1), move || {
            let Some(ui) = ui_weak2.upgrade() else { return };
            let detail = format!("Last sync: just now — synced successfully");
            match svc2.as_str() {
                "google" => ui.set_conn_google_detail(detail.into()),
                "spotify" => ui.set_conn_spotify_detail(detail.into()),
                "facebook" => ui.set_conn_facebook_detail(detail.into()),
                "instagram" => ui.set_conn_instagram_detail(detail.into()),
                _ => {}
            }
        });
    });

    // Wallpaper changed: preset name or file path
    let ui_weak = ui.as_weak();
    let s = settings.clone();
    ui.on_wallpaper_changed(move |value| {
        let Some(ui) = ui_weak.upgrade() else { return };
        let wp = value.to_string();

        // For preset names, just store them
        if wp.is_empty() || WALLPAPER_PRESETS.contains(&wp.as_str()) {
            ui.set_wallpaper_path(value.clone());
            if let Ok(mut st) = s.lock() {
                st.wallpaper = wp.clone();
            }
            persist(&s);
            tracing::info!(wallpaper = %wp, "Wallpaper changed (preset)");
            return;
        }

        // For file paths, validate and load the image
        let path = std::path::Path::new(&wp);
        if path.exists() && path.is_file() {
            match slint::Image::load_from_path(path) {
                Ok(img) => {
                    ui.set_wallpaper_image(img);
                    ui.set_wallpaper_path(value.clone());
                    if let Ok(mut st) = s.lock() {
                        st.wallpaper = wp.clone();
                    }
                    persist(&s);
                    tracing::info!(wallpaper = %wp, "Wallpaper changed (custom image)");
                }
                Err(e) => {
                    tracing::warn!(path = %wp, error = %e, "Failed to load wallpaper image");
                }
            }
        } else {
            tracing::warn!(path = %wp, "Wallpaper file not found");
        }
    });

    // Rename user
    let s = settings.clone();
    let bridge = ctx.bridge.clone();
    ui.on_rename_user(move |name| {
        let name = name.to_string().trim().to_string();
        if name.is_empty() { return; }
        if let Ok(mut st) = s.lock() {
            st.user_name = name.clone();
        }
        persist(&s);
        bridge.rename_user(name.clone());
        tracing::info!(user_name = %name, "User renamed");
    });

    // Rename companion
    let s = settings.clone();
    let bridge = ctx.bridge.clone();
    ui.on_rename_companion(move |name| {
        let name = name.to_string().trim().to_string();
        if name.is_empty() { return; }
        if let Ok(mut st) = s.lock() {
            st.companion_name = name.clone();
        }
        persist(&s);
        bridge.rename_companion(name.clone());
        tracing::info!(companion_name = %name, "Companion renamed");
    });

    // Skill toggles are now handled by wire/skill_store.rs
}

/// Back-compat: load theme preference (delegates to full settings).
pub fn load_theme_preference() -> bool {
    load().dark_mode
}
