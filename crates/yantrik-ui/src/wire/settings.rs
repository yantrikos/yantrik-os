//! Settings wiring — persistent user preferences via ~/.config/yantrik/settings.yaml.
//! AI provider management via ~/.config/yantrik/providers.yaml.

use serde::{Deserialize, Serialize};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::sync::{Arc, Mutex};

use crate::app_context::AppContext;
use crate::{App, AccentPreset, ThemeMode, AIStatusData, AIProviderData, AIModelData, SettingsCategoryItem};

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

    // ── AI Provider Management ──

    let providers = Arc::new(Mutex::new(ProviderStore::load()));

    // Push initial AI status + providers to UI
    {
        let ps = providers.lock().unwrap();
        push_providers_to_ui(ui, &ps);
        push_ai_status_to_ui(ui, &ps, ctx.bridge.is_online());
    }

    // Settings search (sidebar category filtering)
    let all_cats: Vec<SettingsCategoryItem> = vec![
        SettingsCategoryItem { icon: "\u{25D0}".into(), label: "Appearance".into(), id: 0 },
        SettingsCategoryItem { icon: "\u{25C9}".into(), label: "AI & Intelligence".into(), id: 1 },
        SettingsCategoryItem { icon: "\u{25A3}".into(), label: "Desktop".into(), id: 2 },
        SettingsCategoryItem { icon: "\u{25CE}".into(), label: "Network".into(), id: 3 },
        SettingsCategoryItem { icon: "\u{1F517}".into(), label: "Accounts".into(), id: 4 },
        SettingsCategoryItem { icon: "\u{2298}".into(), label: "Privacy & Security".into(), id: 5 },
        SettingsCategoryItem { icon: "\u{2299}".into(), label: "System".into(), id: 6 },
        SettingsCategoryItem { icon: "\u{26A1}".into(), label: "Skills".into(), id: 7 },
    ];
    // Push initial categories
    ui.set_settings_categories(ModelRc::new(VecModel::from(all_cats.clone())));
    let ui_weak = ui.as_weak();
    ui.on_settings_search(move |query| {
        let Some(ui) = ui_weak.upgrade() else { return };
        let q = query.to_string().to_lowercase();
        if q.is_empty() {
            ui.set_settings_categories(ModelRc::new(VecModel::from(all_cats.clone())));
            return;
        }
        let filtered: Vec<SettingsCategoryItem> = all_cats.iter()
            .filter(|cat| cat.label.to_string().to_lowercase().contains(&q))
            .cloned()
            .collect();
        ui.set_settings_categories(ModelRc::new(VecModel::from(filtered)));
    });

    // Add provider (opens panel — UI-side only, but we log it)
    ui.on_add_provider(move || {
        tracing::info!("Add provider panel opened");
    });

    // Provider preset selected — fill form fields with known defaults
    let ui_weak = ui.as_weak();
    ui.on_provider_preset_selected(move |preset| {
        let Some(ui) = ui_weak.upgrade() else { return };
        let preset_str = preset.to_string();
        let (name, url) = match preset_str.as_str() {
            "openai"       => ("OpenAI",       "https://api.openai.com/v1"),
            "anthropic"    => ("Anthropic",     "https://api.anthropic.com/v1"),
            "gemini"       => ("Google Gemini", "https://generativelanguage.googleapis.com/v1beta/openai"),
            "deepseek"     => ("DeepSeek",      "https://api.deepseek.com/v1"),
            "groq"         => ("Groq",          "https://api.groq.com/openai/v1"),
            "mistral"      => ("Mistral",       "https://api.mistral.ai/v1"),
            "xai"          => ("xAI Grok",      "https://api.x.ai/v1"),
            "perplexity"   => ("Perplexity",    "https://api.perplexity.ai"),
            "cerebras"     => ("Cerebras",      "https://api.cerebras.ai/v1"),
            "sambanova"    => ("SambaNova",     "https://api.sambanova.ai/v1"),
            "qwen"         => ("Qwen",          "https://dashscope.aliyuncs.com/compatible-mode/v1"),
            "minimax"      => ("MiniMax",       "https://api.minimax.chat/v1"),
            "kimi"         => ("Kimi",          "https://api.moonshot.cn/v1"),
            "baidu"        => ("Baidu",         "https://qianfan.baidubce.com/v2"),
            "zhipu"        => ("Zhipu GLM",     "https://open.bigmodel.cn/api/paas/v4"),
            "openrouter"   => ("OpenRouter",    "https://openrouter.ai/api/v1"),
            "together"     => ("Together",      "https://api.together.xyz/v1"),
            "fireworks"    => ("Fireworks",     "https://api.fireworks.ai/inference/v1"),
            "huggingface"  => ("HuggingFace",   "https://api-inference.huggingface.co/v1"),
            "nanogpt"      => ("NanoGPT",       "https://api.nano-gpt.com/v1"),
            "ollama"       => ("Ollama",        "http://localhost:11434/v1"),
            "ollama-cloud" => ("Ollama Cloud",  ""),
            "llamacpp"     => ("llama.cpp",     "http://localhost:8080/v1"),
            "lmstudio"     => ("LM Studio",     "http://localhost:1234/v1"),
            "vllm"         => ("vLLM",          "http://localhost:8000/v1"),
            _              => ("Custom",        ""),
        };
        tracing::info!(preset = %preset_str, name, url, "Provider preset selected");
        ui.set_settings_provider_form_name(name.into());
        ui.set_settings_provider_form_url(url.into());
        ui.set_settings_provider_test_result("".into());
    });

    // Save provider
    let ui_weak = ui.as_weak();
    let ps = providers.clone();
    let bridge = ctx.bridge.clone();
    ui.on_save_provider(move |name, ptype, url, key, auth| {
        let entry = ProviderStoreEntry {
            id: format!("{}-{}", ptype.to_string().to_lowercase(), uuid_short()),
            name: name.to_string(),
            provider_type: ptype.to_string(),
            base_url: url.to_string(),
            api_key: if key.is_empty() { None } else { Some(key.to_string()) },
            auth_type: auth.to_string(),
            is_primary: false,
            is_fallback: false,
        };
        tracing::info!(name = %entry.name, provider_type = %entry.provider_type, "Saving provider");
        if let Ok(mut store) = ps.lock() {
            // If this is the first provider, make it primary
            let make_primary = store.entries.is_empty();
            store.entries.push(entry);
            if make_primary {
                if let Some(e) = store.entries.last_mut() {
                    e.is_primary = true;
                }
            }
            store.save();
            if let Some(ui) = ui_weak.upgrade() {
                push_providers_to_ui(&ui, &store);
                push_ai_status_to_ui(&ui, &store, bridge.is_online());
            }
        }
    });

    // Delete provider
    let ui_weak = ui.as_weak();
    let ps = providers.clone();
    let bridge = ctx.bridge.clone();
    ui.on_delete_provider(move |id| {
        let id = id.to_string();
        tracing::info!(id = %id, "Deleting provider");
        if let Ok(mut store) = ps.lock() {
            store.entries.retain(|e| e.id != id);
            store.save();
            if let Some(ui) = ui_weak.upgrade() {
                push_providers_to_ui(&ui, &store);
                push_ai_status_to_ui(&ui, &store, bridge.is_online());
            }
        }
    });

    // Test existing provider
    let ui_weak = ui.as_weak();
    let ps = providers.clone();
    ui.on_test_provider(move |id| {
        let id_str = id.to_string();
        tracing::info!(id = %id_str, "Testing provider");

        let entry = {
            let store = ps.lock().unwrap();
            store.entries.iter().find(|e| e.id == id_str).cloned()
        };

        if let Some(entry) = entry {
            let weak = ui_weak.clone();
            let ps2 = ps.clone();
            std::thread::spawn(move || {
                let result = test_provider_connection(&entry.base_url, entry.api_key.as_deref(), &entry.auth_type);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = weak.upgrade() {
                        // Update the provider status in the store
                        if let Ok(mut store) = ps2.lock() {
                            if let Some(e) = store.entries.iter_mut().find(|e| e.id == id_str) {
                                // We don't store status in the YAML, just push to UI
                            }
                            push_providers_to_ui_with_test(&ui, &store, &result);
                        }
                        ui.set_settings_provider_test_result(
                            if result.success { "success".into() } else { format!("error: {}", result.message).into() }
                        );
                    }
                });
            });
        }
    });

    // Test new provider (from add panel)
    let ui_weak = ui.as_weak();
    ui.on_test_new_provider(move |url, key, auth| {
        let url_str = url.to_string();
        let key_str = if key.is_empty() { None } else { Some(key.to_string()) };
        let auth_str = auth.to_string();

        tracing::info!(url = %url_str, "Testing new provider connection");

        let weak = ui_weak.clone();
        // Set testing state
        if let Some(ui) = weak.upgrade() {
            ui.set_settings_provider_test_result("testing".into());
        }

        let weak2 = weak.clone();
        std::thread::spawn(move || {
            let result = test_provider_connection(&url_str, key_str.as_deref(), &auth_str);
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak2.upgrade() {
                    ui.set_settings_provider_test_result(
                        if result.success { "success".into() } else { format!("error: {}", result.message).into() }
                    );
                }
            });
        });
    });

    // Set primary provider
    let ui_weak = ui.as_weak();
    let ps = providers.clone();
    let bridge = ctx.bridge.clone();
    ui.on_set_primary_provider(move |id| {
        let id = id.to_string();
        tracing::info!(id = %id, "Setting primary provider");
        if let Ok(mut store) = ps.lock() {
            for e in &mut store.entries {
                e.is_primary = e.id == id;
            }
            store.save();
            if let Some(ui) = ui_weak.upgrade() {
                push_providers_to_ui(&ui, &store);
                push_ai_status_to_ui(&ui, &store, bridge.is_online());
            }
        }
    });

    // Set fallback provider
    let ui_weak = ui.as_weak();
    let ps = providers.clone();
    let bridge = ctx.bridge.clone();
    ui.on_set_fallback_provider(move |id| {
        let id = id.to_string();
        tracing::info!(id = %id, "Setting fallback provider");
        if let Ok(mut store) = ps.lock() {
            for e in &mut store.entries {
                e.is_fallback = e.id == id;
            }
            store.save();
            if let Some(ui) = ui_weak.upgrade() {
                push_providers_to_ui(&ui, &store);
                push_ai_status_to_ui(&ui, &store, bridge.is_online());
            }
        }
    });

    // Select model
    let ui_weak = ui.as_weak();
    ui.on_select_model(move |model_id| {
        let id = model_id.to_string();
        tracing::info!(model = %id, "Model selected");
        // Update the UI to show the selected model as active
        if let Some(ui) = ui_weak.upgrade() {
            let models = ui.get_settings_available_models();
            let updated: Vec<AIModelData> = (0..models.row_count())
                .filter_map(|i| {
                    let mut m = models.row_data(i)?;
                    m.is_active = m.id.to_string() == id;
                    Some(m)
                })
                .collect();
            ui.set_settings_available_models(ModelRc::new(VecModel::from(updated)));
        }
    });

    // Refresh models — fetches from primary provider
    let ui_weak = ui.as_weak();
    let ps = providers.clone();
    ui.on_refresh_models(move || {
        tracing::info!("Refreshing model list");
        let primary = {
            let store = ps.lock().unwrap();
            store.entries.iter().find(|e| e.is_primary).cloned()
        };

        if let Some(provider) = primary {
            let weak = ui_weak.clone();
            std::thread::spawn(move || {
                let models = fetch_models(&provider.base_url, provider.api_key.as_deref(), &provider.auth_type, &provider.provider_type);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = weak.upgrade() {
                        let model_data: Vec<AIModelData> = models
                            .into_iter()
                            .map(|m| AIModelData {
                                id: m.id.into(),
                                name: m.name.into(),
                                tier: m.tier.into(),
                                param_count: m.param_count.into(),
                                context_length: m.context_length.into(),
                                is_active: m.is_active,
                                is_local: m.is_local,
                            })
                            .collect();
                        ui.set_settings_available_models(ModelRc::new(VecModel::from(model_data)));
                        tracing::info!(count = ui.get_settings_available_models().row_count(), "Models refreshed");
                    }
                });
            });
        }
    });

    // Toggle auto-fallback
    ui.on_toggle_auto_fallback(move || {
        tracing::info!("Auto-fallback toggled");
    });
}

/// Back-compat: load theme preference (delegates to full settings).
pub fn load_theme_preference() -> bool {
    load().dark_mode
}

// ──────────────────────────────────────────────────────────────
// Provider Store — YAML-persisted provider list
// ──────────────────────────────────────────────────────────────

fn providers_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    format!("{}/.config/yantrik/providers.yaml", home)
}

/// A single AI provider entry persisted to YAML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderStoreEntry {
    pub id: String,
    pub name: String,
    #[serde(default = "default_provider_type")]
    pub provider_type: String,
    pub base_url: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "default_auth_type")]
    pub auth_type: String,
    #[serde(default)]
    pub is_primary: bool,
    #[serde(default)]
    pub is_fallback: bool,
}

fn default_provider_type() -> String { "custom".into() }
fn default_auth_type() -> String { "bearer".into() }

/// Manages the list of AI providers.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderStore {
    #[serde(default)]
    pub entries: Vec<ProviderStoreEntry>,
}

impl ProviderStore {
    pub fn load() -> Self {
        let path = providers_path();
        match std::fs::read_to_string(&path) {
            Ok(content) => serde_yaml::from_str(&content).unwrap_or_else(|e| {
                tracing::warn!("Corrupt providers.yaml, using empty: {e}");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let path = providers_path();
        if let Some(parent) = std::path::Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_yaml::to_string(self) {
            Ok(yaml) => {
                if let Err(e) = std::fs::write(&path, yaml) {
                    tracing::warn!("Failed to write providers.yaml: {e}");
                }
            }
            Err(e) => tracing::warn!("Failed to serialize providers: {e}"),
        }
    }

    pub fn primary(&self) -> Option<&ProviderStoreEntry> {
        self.entries.iter().find(|e| e.is_primary)
    }

    pub fn fallback(&self) -> Option<&ProviderStoreEntry> {
        self.entries.iter().find(|e| e.is_fallback)
    }
}

/// Generate a short unique ID.
fn uuid_short() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
    format!("{:x}", ts & 0xFFFFFFFF)
}

/// Push provider list to UI.
fn push_providers_to_ui(ui: &App, store: &ProviderStore) {
    let items: Vec<AIProviderData> = store.entries.iter().map(|e| AIProviderData {
        id: e.id.clone().into(),
        name: e.name.clone().into(),
        provider_type: e.provider_type.clone().into(),
        base_url: e.base_url.clone().into(),
        status: "connected".into(), // default — will be updated by test
        is_primary: e.is_primary,
        is_fallback: e.is_fallback,
        latency_ms: -1,
        error_message: SharedString::default(),
    }).collect();
    ui.set_settings_ai_providers(ModelRc::new(VecModel::from(items)));
}

/// Push provider list with test result applied.
fn push_providers_to_ui_with_test(ui: &App, store: &ProviderStore, result: &TestResult) {
    let items: Vec<AIProviderData> = store.entries.iter().map(|e| AIProviderData {
        id: e.id.clone().into(),
        name: e.name.clone().into(),
        provider_type: e.provider_type.clone().into(),
        base_url: e.base_url.clone().into(),
        status: if result.success { "connected".into() } else { "error".into() },
        is_primary: e.is_primary,
        is_fallback: e.is_fallback,
        latency_ms: result.latency_ms,
        error_message: if result.success { SharedString::default() } else { result.message.clone().into() },
    }).collect();
    ui.set_settings_ai_providers(ModelRc::new(VecModel::from(items)));
}

/// Push AI status derived from provider store + bridge state.
fn push_ai_status_to_ui(ui: &App, store: &ProviderStore, online: bool) {
    let primary = store.primary();
    let fallback = store.fallback();

    let status = AIStatusData {
        provider_name: primary.map_or(SharedString::default(), |p| p.name.clone().into()),
        provider_type: primary.map_or(SharedString::default(), |p| p.provider_type.clone().into()),
        model_name: ui.get_settings_llm_api_model(),
        model_tier: SharedString::default(), // filled by capability profile later
        status: if online { "connected".into() } else { "disconnected".into() },
        latency_ms: -1,
        tokens_per_sec: 0.0,
        tokens_today: 0,
        using_fallback: !online && fallback.is_some(),
        fallback_provider: fallback.map_or(SharedString::default(), |f| f.name.clone().into()),
        fallback_model: SharedString::default(),
    };
    ui.set_settings_ai_status(status);
}

// ──────────────────────────────────────────────────────────────
// Provider testing + model fetching
// ──────────────────────────────────────────────────────────────

struct TestResult {
    success: bool,
    message: String,
    latency_ms: i32,
}

/// Test a provider connection by hitting its models endpoint.
fn test_provider_connection(base_url: &str, api_key: Option<&str>, auth_type: &str) -> TestResult {
    let url = if base_url.contains("/v1") {
        format!("{}/models", base_url.trim_end_matches('/'))
    } else {
        // Ollama-style: /api/tags
        format!("{}/api/tags", base_url.trim_end_matches('/'))
    };

    let start = std::time::Instant::now();

    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(10))
        .build();

    let mut request = agent.get(&url);
    if let Some(key) = api_key {
        match auth_type {
            "x-api-key" => { request = request.set("x-api-key", key); }
            "none" => {}
            _ => { request = request.set("Authorization", &format!("Bearer {}", key)); }
        }
    }

    match request.call() {
        Ok(response) => {
            let latency = start.elapsed().as_millis() as i32;
            TestResult { success: true, message: "OK".into(), latency_ms: latency }
        }
        Err(ureq::Error::Status(code, _response)) => {
            let latency = start.elapsed().as_millis() as i32;
            TestResult {
                success: false,
                message: format!("HTTP {}", code),
                latency_ms: latency,
            }
        }
        Err(e) => TestResult {
            success: false,
            message: format!("{}", e),
            latency_ms: -1,
        },
    }
}

/// A model entry from the provider API.
struct FetchedModel {
    id: String,
    name: String,
    tier: String,
    param_count: String,
    context_length: String,
    is_active: bool,
    is_local: bool,
}

/// Fetch available models from a provider.
fn fetch_models(base_url: &str, api_key: Option<&str>, auth_type: &str, provider_type: &str) -> Vec<FetchedModel> {
    let url = if provider_type == "ollama" || (!base_url.contains("/v1") && !base_url.contains("api.")) {
        format!("{}/api/tags", base_url.trim_end_matches('/'))
    } else {
        format!("{}/models", base_url.trim_end_matches('/'))
    };

    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(15))
        .build();

    let mut request = agent.get(&url);
    if let Some(key) = api_key {
        match auth_type {
            "x-api-key" => { request = request.set("x-api-key", key); }
            "none" => {}
            _ => { request = request.set("Authorization", &format!("Bearer {}", key)); }
        }
    }

    let response = match request.call() {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to fetch models");
            return vec![];
        }
    };

    let body: String = match response.into_string() {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to read model response body");
            return vec![];
        }
    };

    // Parse JSON — handle both Ollama and OpenAI formats
    let json: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to parse model JSON");
            return vec![];
        }
    };

    let mut models = Vec::new();

    // Ollama format: { "models": [ { "name": "...", "size": ... } ] }
    if let Some(model_list) = json.get("models").and_then(|v| v.as_array()) {
        for m in model_list {
            let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
            let size = m.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
            let param_count = format_param_count(size);
            let tier = detect_tier(name);
            models.push(FetchedModel {
                id: name.to_string(),
                name: name.to_string(),
                tier,
                param_count,
                context_length: String::new(),
                is_active: false,
                is_local: true,
            });
        }
    }
    // OpenAI format: { "data": [ { "id": "..." } ] }
    else if let Some(data_list) = json.get("data").and_then(|v| v.as_array()) {
        for m in data_list {
            let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
            let tier = detect_tier(id);
            models.push(FetchedModel {
                id: id.to_string(),
                name: id.to_string(),
                tier,
                param_count: String::new(),
                context_length: String::new(),
                is_active: false,
                is_local: false,
            });
        }
    }

    models
}

/// Public wrapper for tier detection (used by system_monitor.rs).
pub fn detect_tier_from_name(name: &str) -> String {
    detect_tier(name)
}

/// Detect model tier from name (matches capability.rs logic).
fn detect_tier(name: &str) -> String {
    let lower = name.to_lowercase();

    // Try to extract parameter count like "0.8b", "3b", "27b", "70b"
    if let Some(b) = extract_param_billions(&lower) {
        if b <= 1.5 { return "Tiny".into(); }
        if b <= 4.0 { return "Small".into(); }
        if b <= 14.0 { return "Medium".into(); }
        return "Large".into();
    }

    // Name-based heuristics
    if lower.contains("gpt-4") || lower.contains("claude") || lower.contains("opus") {
        "Large".into()
    } else if lower.contains("gpt-3.5") || lower.contains("sonnet") || lower.contains("haiku") {
        "Medium".into()
    } else {
        String::new()
    }
}

/// Extract parameter count in billions from model name (e.g. "qwen3.5:27b" → 27.0).
fn extract_param_billions(name: &str) -> Option<f64> {
    // Look for patterns like "0.8b", "3b", "27b-", "70b:"
    let bytes = name.as_bytes();
    let len = bytes.len();
    for i in 0..len {
        if bytes[i] == b'b' && (i + 1 >= len || !bytes[i + 1].is_ascii_alphanumeric()) {
            // Walk backwards to find the number
            let mut end = i;
            let mut start = end;
            while start > 0 && (bytes[start - 1].is_ascii_digit() || bytes[start - 1] == b'.') {
                start -= 1;
            }
            if start < end {
                if let Ok(val) = name[start..end].parse::<f64>() {
                    if val > 0.0 && val < 10000.0 {
                        return Some(val);
                    }
                }
            }
        }
    }
    None
}

/// Format byte size to parameter count string.
fn format_param_count(size_bytes: u64) -> String {
    if size_bytes == 0 { return String::new(); }
    let gb = size_bytes as f64 / 1_073_741_824.0;
    if gb >= 1.0 {
        format!("{:.1}GB", gb)
    } else {
        let mb = size_bytes as f64 / 1_048_576.0;
        format!("{:.0}MB", mb)
    }
}
