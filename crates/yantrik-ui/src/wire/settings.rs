//! Settings wiring — persistent user preferences via ~/.config/yantrik/settings.yaml.
//! AI provider management via ~/.config/yantrik/providers.yaml.

use serde::{Deserialize, Serialize};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::sync::{Arc, Mutex};

use crate::app_context::AppContext;
use crate::{
    AIModelData, AIProviderData, AIStatusData, AccentPreset, App, SettingsCategoryItem, ThemeMode,
};

/// Accent color preset names in cycle order (matches AccentPreset.index).
const ACCENT_PRESETS: &[&str] = &["cyan", "amber", "purple", "green", "pink"];

/// Known wallpaper preset names.
/// The wallpapers this OS ships, scenes first.
///
/// The scenes are rendered by scripts/render-scene-wallpapers.py and the gradients by
/// scripts/render-wallpapers.py; both write into crates/yantrik-ui-slint/ui/wallpapers and both
/// are committed beside their output, so the desktop stays editable and reproducible rather
/// than being four PNGs somebody exported once.
const WALLPAPER_PRESETS: &[&str] = &[
    "serenity",
    "first-light",
    "nightfall",
    "aurora",
    "sunset",
    "ocean",
    "nebula",
];

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
    /// Show the operator console (working-set rail, machine rail, ask bar) instead
    /// of a desktop. Off by default: the machine is for a person until told
    /// otherwise, and a console is unusable to someone who did not build it.
    #[serde(default)]
    pub agent_mode: bool,
    /// Where this machine is. Empty until something has worked it out or somebody has said.
    #[serde(default)]
    pub place: Place,
    /// The apps pinned to START, in the order they are shown.
    ///
    /// The shell's ids for its own apps (`notes`, `files`) and the catalogue's ids for everything
    /// else (`chromium`). An empty list is a choice and is kept as one — only a settings file
    /// that has never heard of pins gets the defaults.
    pub pinned_apps: Vec<String>,
    /// Which mind the person last chose to answer for them.
    ///
    /// Empty means "whatever the shell starts with", which is the built-in companion. Kept here
    /// rather than in the harness host because the host's list is LIVE — a harness exists only
    /// while it is attached — and this is the opposite kind of fact: a preference that outlives
    /// every process involved, including the mind it names.
    #[serde(default)]
    pub preferred_mind: String,
    /// What a mind on the socket may do without being asked: `plan`, `ask` or `auto`.
    ///
    /// Deliberately not `bypass`. Bypass is the fourth mode and it is never written here — a
    /// machine that booted into "do not ask me about anything" would be in a mode nobody had
    /// chosen in that sitting, and the only thing that makes bypass acceptable is that somebody
    /// picked it, just now, off a confirmation that said what it meant. A bypass persists the
    /// mode it will fall back to; see `mind_mode::persist`.
    #[serde(default)]
    pub mind_mode: String,
    /// Where an app a mind opens goes: into Mind View, a desktop of the mind's own inside one
    /// window (#239), or onto the person's desktop over whatever they were doing.
    ///
    /// On by default, as the issue asked: a mind acting on its own should not be putting windows
    /// over the person's work. It is a UI-only choice, like the mode — nothing on the socket sets
    /// it — because "put your windows on my desktop" is the person's to say.
    pub minds_open_in_mind_view: bool,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            dark_mode: true,
            accent_color: "cyan".into(),
            tool_permission: "sensitive".into(),
            auto_lock_secs: 300,
            dnd_mode: false,
            // Named rather than empty: an empty wallpaper draws the flat fallback gradient,
            // which is the one backdrop that makes the translucent surfaces above it pointless.
            wallpaper: "serenity".to_string(),
            user_name: String::new(),
            companion_name: String::new(),
            agent_mode: false,
            place: Place::default(),
            pinned_apps: super::pins::DEFAULT_PINS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            preferred_mind: String::new(),
            // The behaviour that shipped before modes existed, so an upgrade changes nothing
            // about a machine somebody already trusts.
            mind_mode: "ask".into(),
            minds_open_in_mind_view: true,
        }
    }
}

/// Where the machine is.
///
/// Written here rather than held in memory so a person can read it, correct it, and have the
/// correction stick — which is the difference between a machine that detected something and a
/// machine that decided something on your behalf.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Place {
    pub city: String,
    pub region: String,
    pub country: String,
    pub lat: f64,
    pub lon: f64,
    /// IANA name, e.g. `America/Chicago`.
    pub timezone: String,
    /// `detected` or `chosen`. A detected place may be re-detected; a chosen one never is.
    pub source: String,
}

/// The live settings, so other wiring can persist one preference without keeping a second copy
/// of the file. Set by [`wire`]; everything below tolerates it being unset.
static LIVE: std::sync::OnceLock<SharedSettings> = std::sync::OnceLock::new();

/// Where this machine is, as far as it knows.
pub fn place() -> Place {
    match LIVE.get().and_then(|s| s.lock().ok()) {
        Some(settings) => settings.place.clone(),
        None => load().place,
    }
}

/// Record where this machine is.
pub fn set_place(place: Place) {
    if let Some(shared) = LIVE.get() {
        if let Ok(mut settings) = shared.lock() {
            settings.place = place;
        }
        persist(shared);
        return;
    }
    let mut settings = load();
    settings.place = place;
    save(&settings);
}

/// The apps pinned to START, in order.
pub fn pinned_apps() -> Vec<String> {
    match LIVE.get().and_then(|s| s.lock().ok()) {
        Some(settings) => settings.pinned_apps.clone(),
        None => load().pinned_apps,
    }
}

/// Replace the pinned list.
pub fn set_pinned_apps(pins: Vec<String>) {
    if let Some(shared) = LIVE.get() {
        if let Ok(mut settings) = shared.lock() {
            settings.pinned_apps = pins;
        }
        persist(shared);
        return;
    }
    let mut settings = load();
    settings.pinned_apps = pins;
    save(&settings);
}

/// Whether notifications are being held.
pub fn dnd_mode() -> bool {
    match LIVE.get().and_then(|s| s.lock().ok()) {
        Some(settings) => settings.dnd_mode,
        None => load().dnd_mode,
    }
}

/// Hold or release notifications, and say whether the file was written.
///
/// The one writer for this preference, and the only setting whose writer hands the failure
/// back. The settings screen used to be the whole of it: its toggle flipped the property on
/// screen, called `persist` and dropped the `Result`. That is fine for a row with a "Not saved"
/// line under it and wrong for `set_do_not_disturb`, whose caller gets `settled` and nothing
/// else to read — a mind that turns notifications off for the night needs to hear about a
/// read-only settings file now, not discover it at the next restart.
///
/// Written even when the shell is already in the asked-for state. A copy in memory that says
/// `true` over a file that says `false` is what a dropped write leaves behind, and asking for
/// `true` a second time is how somebody repairs it.
pub fn set_dnd_mode(on: bool) -> Result<(), String> {
    if let Some(shared) = LIVE.get() {
        // A lock we cannot take is a write we cannot make: `persist` locks the same mutex and
        // returns that as the error, rather than this reporting success over an unchanged value.
        if let Ok(mut settings) = shared.lock() {
            settings.dnd_mode = on;
        }
        return persist(shared);
    }
    let mut settings = load();
    settings.dnd_mode = on;
    save(&settings)
}

/// Which mind the person last chose. Empty if they never have.
pub fn preferred_mind() -> String {
    match LIVE.get().and_then(|s| s.lock().ok()) {
        Some(settings) => settings.preferred_mind.clone(),
        None => load().preferred_mind,
    }
}

/// Remember which mind they chose.
///
/// Through the shared handle when there is one: a direct load-modify-save would be silently
/// undone by the next `persist`, which writes the whole struct from memory and would know
/// nothing about this field having changed on disk.
pub fn set_preferred_mind(id: &str) {
    if let Some(shared) = LIVE.get() {
        if let Ok(mut settings) = shared.lock() {
            if settings.preferred_mind == id {
                return;
            }
            settings.preferred_mind = id.to_string();
        }
        persist(shared);
        return;
    }
    let mut settings = load();
    if settings.preferred_mind == id {
        return;
    }
    settings.preferred_mind = id.to_string();
    save(&settings);
}

/// What a mind on the socket may do without being asked. Empty means the file predates modes.
///
/// Read through the same shared handle as `place()` and `preferred_mind()`, for the same reason:
/// a direct load here and a `persist` there would each write the whole struct from their own copy
/// and quietly undo each other.
pub fn mind_mode() -> String {
    match LIVE.get().and_then(|s| s.lock().ok()) {
        Some(settings) => settings.mind_mode.clone(),
        None => load().mind_mode,
    }
}

/// Record the mode. Never called with `bypass` — see the field's comment and `mind_mode::persist`.
pub fn set_mind_mode(mode: &str) {
    if let Some(shared) = LIVE.get() {
        if let Ok(mut settings) = shared.lock() {
            if settings.mind_mode == mode {
                return;
            }
            settings.mind_mode = mode.to_string();
        }
        persist(shared);
        return;
    }
    let mut settings = load();
    if settings.mind_mode == mode {
        return;
    }
    settings.mind_mode = mode.to_string();
    save(&settings);
}

/// Whether apps a mind opens go into Mind View rather than onto the person's desktop.
pub fn minds_open_in_mind_view() -> bool {
    match LIVE.get().and_then(|s| s.lock().ok()) {
        Some(settings) => settings.minds_open_in_mind_view,
        None => load().minds_open_in_mind_view,
    }
}

/// Record where apps a mind opens should go. Reached from the mode menu's row and nothing else.
pub fn set_minds_open_in_mind_view(on: bool) -> Result<(), String> {
    if let Some(shared) = LIVE.get() {
        if let Ok(mut settings) = shared.lock() {
            settings.minds_open_in_mind_view = on;
        }
        return persist(shared);
    }
    let mut settings = load();
    settings.minds_open_in_mind_view = on;
    save(&settings)
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
    match crate::config_store::load(&path).and_then(|v| v.ok_or_else(|| "No settings file".into()))
    {
        Ok(content) => serde_yaml::from_str(&content).unwrap_or_else(|e| {
            tracing::warn!("Corrupt settings.yaml, using defaults; original file preserved");
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
                        let val = trimmed
                            .trim_start_matches("accent_color:")
                            .trim()
                            .trim_matches('"');
                        if ACCENT_PRESETS.contains(&val) {
                            settings.accent_color = val.to_string();
                        }
                    }
                }
                if save(&settings).is_ok() {
                    let _ = std::fs::remove_file(&old_theme);
                }
                tracing::info!("Migrated theme.yaml → settings.yaml");
            }
            settings
        }
    }
}

thread_local! { static STATUS_UI: std::cell::RefCell<Option<slint::Weak<App>>> = const { std::cell::RefCell::new(None) }; }
fn report(result: &Result<(), String>) {
    report_for(result, true);
}
fn report_for(result: &Result<(), String>, can_retry: bool) {
    STATUS_UI.with(|slot| {
        if let Some(ui) = slot.borrow().as_ref().and_then(|w| w.upgrade()) {
            ui.set_settings_save_error(result.is_err());
            ui.set_settings_save_can_retry(can_retry);
            ui.set_settings_save_status(match result {
                Ok(()) => "Changes saved on this device".into(),
                Err(e) => format!("Not saved: {e}").into(),
            });
        }
    });
    if let Err(e) = result {
        tracing::warn!(error=%e,"Preferences were not saved");
    }
}
fn validate_file<T: serde::de::DeserializeOwned>(path: &str) -> Result<(), String> {
    if let Some(raw) = crate::config_store::load(path)? {
        serde_yaml::from_str::<T>(&raw).map_err(|_|"Existing preferences have invalid values and were preserved. Repair the file and restart the shell.".to_string())?;
    }
    Ok(())
}
/// The whole of the durable write, against a named file and with nothing global in it.
///
/// Split out from `save` so the write can be tested for what it actually leaves on the disk.
/// Everything above it took the file path from `HOME` and reported the outcome to a status line,
/// which made "does this preference survive a restart" a question only a running shell could
/// answer — and the answer nobody had checked was do-not-disturb's.
fn save_to(path: &str, settings: &UserSettings) -> Result<(), String> {
    validate_file::<UserSettings>(path).and_then(|()| {
        serde_yaml::to_string(settings)
            .map_err(|_| "Cannot serialize preferences.".to_string())
            .and_then(|yaml| crate::config_store::save(path, &yaml))
    })
}
fn save(settings: &UserSettings) -> Result<(), String> {
    let result = save_to(&settings_path(), settings);
    report(&result);
    result
}
fn persist(shared: &SharedSettings) -> Result<(), String> {
    match shared.lock() {
        Ok(s) => save(&s),
        Err(_) => {
            let result = Err("Preference store unavailable".into());
            report(&result);
            result
        }
    }
}

fn unavailable_service(ui: &App, service: &str) {
    let status = "unavailable".into();
    let detail =
        "Connection adapter is not installed in this build. No account data has been synced."
            .into();
    match service {
        "google" => {
            ui.set_conn_google_status(status);
            ui.set_conn_google_detail(detail)
        }
        "spotify" => {
            ui.set_conn_spotify_status(status);
            ui.set_conn_spotify_detail(detail)
        }
        "facebook" => {
            ui.set_conn_facebook_status(status);
            ui.set_conn_facebook_detail(detail)
        }
        "instagram" => {
            ui.set_conn_instagram_status(status);
            ui.set_conn_instagram_detail(detail)
        }
        _ => {}
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
    STATUS_UI.with(|slot| *slot.borrow_mut() = Some(ui.as_weak()));
    let settings = Arc::new(Mutex::new(load()));
    let initial = validate_file::<UserSettings>(&settings_path())
        .and_then(|()| validate_file::<ProviderStore>(&providers_path()));
    if initial.is_err() {
        report(&initial);
    }
    // Published before anything reads a preference out of it.
    let _ = LIVE.set(settings.clone());

    // The version on Settings → System. Nothing ever set this, so the row showed the Slint
    // property's own default, `0.1.0`, on every machine this OS has ever run on — a third
    // answer beside About's 0.3.0 and the BUILD marker's git describe.
    ui.set_settings_version(yantrik_version::version().into());

    // YANTRIK_AGENT_MODE wins over the file so an agent harness or a kiosk image can
    // force either face without rewriting a user's settings.
    let agent_mode = match std::env::var("YANTRIK_AGENT_MODE") {
        Ok(v) => matches!(v.trim(), "1" | "true" | "yes" | "on"),
        Err(_) => settings.lock().map(|s| s.agent_mode).unwrap_or(false),
    };
    ui.set_agent_mode(agent_mode);
    tracing::info!(agent_mode, "Shell face selected");

    let ui_weak = ui.as_weak();
    let mode_settings = settings.clone();
    ui.on_set_agent_mode(move |value| {
        let Some(ui) = ui_weak.upgrade() else { return };
        ui.set_agent_mode(value);
        if let Ok(mut saved) = mode_settings.lock() {
            saved.agent_mode = value;
        }
        persist(&mode_settings);
    });

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
    ui.on_toggle_dnd_mode(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let new_val = !ui.get_dnd_mode();
        ui.set_dnd_mode(new_val);
        // Through `set_dnd_mode`, which is also what the control surface writes with, so there
        // is one answer to what a persisted do-not-disturb is. A failure is dropped here on
        // purpose: this row has the save-status line under it, like every other row on the
        // screen, and a click has nowhere else to put an error.
        let _ = set_dnd_mode(new_val);
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

    // Account adapters are not implemented. Never fabricate connection or sync results.
    for service in ["google", "spotify", "facebook", "instagram"] {
        unavailable_service(ui, service);
    }
    let weak = ui.as_weak();
    ui.on_connect_service(move |service| {
        if let Some(ui) = weak.upgrade() {
            unavailable_service(&ui, service.as_str());
        }
    });
    let weak = ui.as_weak();
    ui.on_sync_service(move |service| {
        if let Some(ui) = weak.upgrade() {
            unavailable_service(&ui, service.as_str());
        }
    });
    let weak = ui.as_weak();
    ui.on_disconnect_service(move |service| {
        if let Some(ui) = weak.upgrade() {
            unavailable_service(&ui, service.as_str());
        }
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
                    if let Ok(saved) = s.lock() {
                        ui.set_wallpaper_path(saved.wallpaper.clone().into());
                    }
                    report(&Err("This file could not be loaded as wallpaper.".into()));
                }
            }
        } else {
            tracing::warn!(path = %wp, "Wallpaper file not found");
            if let Ok(saved) = s.lock() {
                ui.set_wallpaper_path(saved.wallpaper.clone().into());
            }
            report(&Err(
                "Wallpaper file not found. The saved wallpaper is unchanged.".into(),
            ));
        }
    });

    // Rename user
    let s = settings.clone();
    let bridge = ctx.bridge.clone();
    ui.on_rename_user(move |name| {
        let name = name.to_string().trim().to_string();
        if name.is_empty() {
            return;
        }
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
        if name.is_empty() {
            return;
        }
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
    let s = settings.clone();
    ui.on_retry_settings_save(move || {
        let _ = persist(&s);
    });
    let weak = ui.as_weak();
    let accent_settings = settings.clone();
    ui.on_choose_accent(move |name| {
        let Some(ui) = weak.upgrade() else { return };
        if !ACCENT_PRESETS.contains(&name.as_str()) {
            return;
        }
        ui.set_settings_accent_color(name.clone());
        ui.global::<AccentPreset>()
            .set_index(accent_name_to_index(name.as_str()));
        if let Ok(mut saved) = accent_settings.lock() {
            saved.accent_color = name.to_string();
        }
        let _ = persist(&accent_settings);
    });

    // Push initial AI status + providers to UI
    {
        let ps = providers.lock().unwrap();
        push_providers_to_ui(ui, &ps);
        push_ai_status_to_ui(ui, &ps, ctx.bridge.is_online());
    }

    // Settings search (sidebar category filtering)
    let all_cats: Vec<SettingsCategoryItem> = vec![
        SettingsCategoryItem {
            icon: "".into(),
            label: "Appearance".into(),
            id: 0,
        },
        SettingsCategoryItem {
            icon: "".into(),
            label: "AI & Intelligence".into(),
            id: 1,
        },
        SettingsCategoryItem {
            icon: "".into(),
            label: "Desktop".into(),
            id: 2,
        },
        SettingsCategoryItem {
            icon: "".into(),
            label: "Network".into(),
            id: 3,
        },
        SettingsCategoryItem {
            icon: "".into(),
            label: "Accounts".into(),
            id: 4,
        },
        SettingsCategoryItem {
            icon: "".into(),
            label: "Privacy & Security".into(),
            id: 5,
        },
        SettingsCategoryItem {
            icon: "".into(),
            label: "System".into(),
            id: 6,
        },
        SettingsCategoryItem {
            icon: "".into(),
            label: "Skills".into(),
            id: 7,
        },
        SettingsCategoryItem {
            icon: "".into(),
            label: "Harnesses".into(),
            id: 8,
        },
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
        let matches = crate::config_store::search_categories(&q);
        let filtered: Vec<SettingsCategoryItem> = all_cats
            .iter()
            .filter(|cat| matches.contains(&cat.id))
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
        let (name, url) = provider_preset(&preset_str);
        tracing::info!(preset = %preset_str, name, url, "Provider preset selected");
        ui.set_settings_provider_form_name(name.into());
        ui.set_settings_provider_form_url(url.into());
        ui.set_settings_provider_test_result("".into());
    });

    wire_rest(ui, ctx, providers);
}

/// Known provider presets: id -> (display name, OpenAI-compatible base URL).
///
/// Shared with onboarding so first boot and Settings cannot drift apart.
pub(crate) fn provider_preset(id: &str) -> (&'static str, &'static str) {
    match id {
        "openai" => ("OpenAI", "https://api.openai.com/v1"),
        "anthropic" => ("Anthropic", "https://api.anthropic.com/v1"),
        "gemini" => (
            "Google Gemini",
            "https://generativelanguage.googleapis.com/v1beta/openai",
        ),
        "deepseek" => ("DeepSeek", "https://api.deepseek.com/v1"),
        "groq" => ("Groq", "https://api.groq.com/openai/v1"),
        "mistral" => ("Mistral", "https://api.mistral.ai/v1"),
        "xai" => ("xAI Grok", "https://api.x.ai/v1"),
        "perplexity" => ("Perplexity", "https://api.perplexity.ai"),
        "cerebras" => ("Cerebras", "https://api.cerebras.ai/v1"),
        "sambanova" => ("SambaNova", "https://api.sambanova.ai/v1"),
        "qwen" => ("Qwen", "https://dashscope.aliyuncs.com/compatible-mode/v1"),
        "minimax" => ("MiniMax", "https://api.minimax.chat/v1"),
        "kimi" => ("Kimi", "https://api.moonshot.cn/v1"),
        "baidu" => ("Baidu", "https://qianfan.baidubce.com/v2"),
        "zhipu" => ("Zhipu GLM", "https://open.bigmodel.cn/api/paas/v4"),
        "openrouter" => ("OpenRouter", "https://openrouter.ai/api/v1"),
        "together" => ("Together", "https://api.together.xyz/v1"),
        "fireworks" => ("Fireworks", "https://api.fireworks.ai/inference/v1"),
        "huggingface" => ("HuggingFace", "https://api-inference.huggingface.co/v1"),
        "nanogpt" => ("NanoGPT", "https://api.nano-gpt.com/v1"),
        "ollama" => ("Ollama", "http://localhost:11434/v1"),
        "ollama-cloud" => ("Ollama Cloud", ""),
        "llamacpp" => ("llama.cpp", "http://localhost:8080/v1"),
        "lmstudio" => ("LM Studio", "http://localhost:1234/v1"),
        "vllm" => ("vLLM", "http://localhost:8000/v1"),
        _ => ("Custom", ""),
    }
}

/// Remainder of the settings wiring, split out when `provider_preset` was
/// lifted to a shared function.
fn wire_rest(ui: &App, ctx: &AppContext, providers: Arc<Mutex<ProviderStore>>) {
    // Save provider
    let ui_weak = ui.as_weak();
    let ps = providers.clone();
    let bridge = ctx.bridge.clone();
    let bridge_for_save = ctx.bridge.clone();
    ui.on_save_provider(move |name, ptype, url, key, auth| {
        let entry = ProviderStoreEntry {
            id: format!("{}-{}", ptype.to_string().to_lowercase(), uuid_short()),
            name: name.to_string(),
            provider_type: ptype.to_string(),
            base_url: url.to_string(),
            api_key: if key.is_empty() {
                None
            } else {
                Some(key.to_string())
            },
            auth_type: auth.to_string(),
            is_primary: false,
            is_fallback: false,
        };
        tracing::info!(name = %entry.name, provider_type = %entry.provider_type, "Saving provider");
        let made_primary = if let Ok(mut store) = ps.lock() {
            let before = store.clone();
            // If this is the first provider, make it primary
            let make_primary = store.entries.is_empty();
            store.entries.push(entry.clone());
            if make_primary {
                if let Some(e) = store.entries.last_mut() {
                    e.is_primary = true;
                }
            }
            if store.save().is_err() {
                *store = before;
                return;
            }
            if let Some(ui) = ui_weak.upgrade() {
                push_providers_to_ui(&ui, &store);
                push_ai_status_to_ui(&ui, &store, bridge.is_online());
            }
            make_primary
        } else {
            false
        };

        // Hot-reload the LLM backend if this is the primary provider
        if made_primary {
            // Default model per provider type
            let default_model = match entry.provider_type.as_str() {
                "ollama" => "llama3.2:latest",
                "openai" => "gpt-4o-mini",
                "anthropic" => "claude-3-5-sonnet-latest",
                "google" => "gemini-2.0-flash",
                "deepseek" => "deepseek-chat",
                _ => "default",
            };
            // Build base URL with /v1 suffix for OpenAI-compatible APIs
            let base_url = if entry.provider_type == "ollama" && !entry.base_url.contains("/v1") {
                format!("{}/v1", entry.base_url.trim_end_matches('/'))
            } else {
                entry.base_url.clone()
            };
            tracing::info!(
                model = default_model,
                "Hot-reloading LLM with new primary provider"
            );
            bridge_for_save.reload_llm(
                entry.provider_type.clone(),
                base_url,
                entry.api_key.clone(),
                default_model.to_string(),
            );
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
            let before = store.clone();
            store.entries.retain(|e| e.id != id);
            if store.save().is_err() {
                *store = before;
                return;
            }
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
                let result = test_provider_connection(
                    &entry.base_url,
                    entry.api_key.as_deref(),
                    &entry.auth_type,
                );
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = weak.upgrade() {
                        // Update the provider status in the store
                        if let Ok(mut store) = ps2.lock() {
                            if let Some(e) = store.entries.iter_mut().find(|e| e.id == id_str) {
                                // We don't store status in the YAML, just push to UI
                            }
                            push_providers_to_ui_with_test(&ui, &store, &result);
                        }
                        ui.set_settings_provider_test_result(if result.success {
                            "success".into()
                        } else {
                            format!("error: {}", result.message).into()
                        });
                    }
                });
            });
        }
    });

    // Test new provider (from add panel)
    let ui_weak = ui.as_weak();
    ui.on_test_new_provider(move |url, key, auth| {
        let url_str = url.to_string();
        let key_str = if key.is_empty() {
            None
        } else {
            Some(key.to_string())
        };
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
                    ui.set_settings_provider_test_result(if result.success {
                        "success".into()
                    } else {
                        format!("error: {}", result.message).into()
                    });
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
            let before = store.clone();
            for e in &mut store.entries {
                e.is_primary = e.id == id;
            }
            if store.save().is_err() {
                *store = before;
                return;
            }
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
            let before = store.clone();
            for e in &mut store.entries {
                e.is_fallback = e.id == id;
            }
            if store.save().is_err() {
                *store = before;
                return;
            }
            if let Some(ui) = ui_weak.upgrade() {
                push_providers_to_ui(&ui, &store);
                push_ai_status_to_ui(&ui, &store, bridge.is_online());
            }
        }
    });

    // Select model
    let ui_weak = ui.as_weak();
    let ui_weak_sm = ui.as_weak();
    let ps_sm = providers.clone();
    let bridge_sm = ctx.bridge.clone();
    ui.on_select_model(move |model_id| {
        let id = model_id.to_string();
        tracing::info!(model = %id, "Model selected — reloading LLM");

        // Update the UI to show the selected model as active
        if let Some(ui) = ui_weak_sm.upgrade() {
            let models = ui.get_settings_available_models();
            let updated: Vec<AIModelData> = (0..models.row_count())
                .filter_map(|i| {
                    let mut m = models.row_data(i)?;
                    m.is_active = m.id.to_string() == id;
                    Some(m)
                })
                .collect();
            ui.set_settings_available_models(ModelRc::new(VecModel::from(updated)));
            // Update the active model display
            ui.set_settings_llm_api_model(id.clone().into());
        }

        // Find the primary provider and hot-reload the LLM with the new model
        let primary = if let Ok(store) = ps_sm.lock() {
            store.entries.iter().find(|e| e.is_primary).cloned()
        } else {
            None
        };

        if let Some(provider) = primary {
            // Build base URL with /v1 suffix for OpenAI-compatible APIs
            let base_url = if provider.provider_type == "ollama" && !provider.base_url.contains("/v1") {
                format!("{}/v1", provider.base_url.trim_end_matches('/'))
            } else {
                provider.base_url.clone()
            };
            tracing::info!(model = %id, base_url = %base_url, "Hot-reloading LLM with selected model");
            bridge_sm.reload_llm(
                provider.provider_type.clone(),
                base_url,
                provider.api_key.clone(),
                id,
            );
            // Push fresh AI status with new online state
            if let (Some(ui), Ok(store)) = (ui_weak_sm.upgrade(), ps_sm.lock()) {
                push_ai_status_to_ui(&ui, &store, true);
                push_providers_to_ui(&ui, &store);
            }
        } else {
            tracing::warn!("No primary provider configured — cannot reload LLM");
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
                let models = fetch_models(
                    &provider.base_url,
                    provider.api_key.as_deref(),
                    &provider.auth_type,
                    &provider.provider_type,
                );
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
                        tracing::info!(
                            count = ui.get_settings_available_models().row_count(),
                            "Models refreshed"
                        );
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

fn default_provider_type() -> String {
    "custom".into()
}
fn default_auth_type() -> String {
    "bearer".into()
}

/// Manages the list of AI providers.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderStore {
    #[serde(default)]
    pub entries: Vec<ProviderStoreEntry>,
}

impl ProviderStore {
    pub fn load() -> Self {
        let path = providers_path();
        match crate::config_store::load(&path)
            .and_then(|v| v.ok_or_else(|| "No provider file".into()))
        {
            Ok(content) => serde_yaml::from_str(&content).unwrap_or_else(|e| {
                tracing::warn!("Corrupt providers.yaml, using empty; original file preserved");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let result = validate_file::<Self>(&providers_path()).and_then(|()| {
            serde_yaml::to_string(self)
                .map_err(|_| "Cannot serialize providers.".into())
                .and_then(|yaml| crate::config_store::save(providers_path(), &yaml))
        });
        report_for(&result, false);
        result
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
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    format!("{:x}", ts & 0xFFFFFFFF)
}

/// Push provider list to UI.
fn push_providers_to_ui(ui: &App, store: &ProviderStore) {
    let items: Vec<AIProviderData> = store
        .entries
        .iter()
        .map(|e| AIProviderData {
            id: e.id.clone().into(),
            name: e.name.clone().into(),
            provider_type: e.provider_type.clone().into(),
            base_url: e.base_url.clone().into(),
            status: "connected".into(), // default — will be updated by test
            is_primary: e.is_primary,
            is_fallback: e.is_fallback,
            latency_ms: -1,
            error_message: SharedString::default(),
        })
        .collect();
    ui.set_settings_ai_providers(ModelRc::new(VecModel::from(items)));
}

/// Push provider list with test result applied.
fn push_providers_to_ui_with_test(ui: &App, store: &ProviderStore, result: &TestResult) {
    let items: Vec<AIProviderData> = store
        .entries
        .iter()
        .map(|e| AIProviderData {
            id: e.id.clone().into(),
            name: e.name.clone().into(),
            provider_type: e.provider_type.clone().into(),
            base_url: e.base_url.clone().into(),
            status: if result.success {
                "connected".into()
            } else {
                "error".into()
            },
            is_primary: e.is_primary,
            is_fallback: e.is_fallback,
            latency_ms: result.latency_ms,
            error_message: if result.success {
                SharedString::default()
            } else {
                result.message.clone().into()
            },
        })
        .collect();
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
        status: if online {
            "connected".into()
        } else {
            "disconnected".into()
        },
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

pub(crate) struct TestResult {
    pub success: bool,
    pub message: String,
    pub latency_ms: i32,
}

/// Test a provider connection by hitting its models endpoint.
///
/// Shared with onboarding (`wire::ai_onboarding`) so first boot validates a
/// provider the same way Settings does, instead of simulating a result.
pub(crate) fn test_provider_connection(
    base_url: &str,
    api_key: Option<&str>,
    auth_type: &str,
) -> TestResult {
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
            "x-api-key" => {
                request = request.set("x-api-key", key);
            }
            "none" => {}
            _ => {
                request = request.set("Authorization", &format!("Bearer {}", key));
            }
        }
    }

    match request.call() {
        Ok(response) => {
            let latency = start.elapsed().as_millis() as i32;
            TestResult {
                success: true,
                message: "OK".into(),
                latency_ms: latency,
            }
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
fn fetch_models(
    base_url: &str,
    api_key: Option<&str>,
    auth_type: &str,
    provider_type: &str,
) -> Vec<FetchedModel> {
    let url =
        if provider_type == "ollama" || (!base_url.contains("/v1") && !base_url.contains("api.")) {
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
            "x-api-key" => {
                request = request.set("x-api-key", key);
            }
            "none" => {}
            _ => {
                request = request.set("Authorization", &format!("Bearer {}", key));
            }
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
        if b <= 1.5 {
            return "Tiny".into();
        }
        if b <= 4.0 {
            return "Small".into();
        }
        if b <= 14.0 {
            return "Medium".into();
        }
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
    if size_bytes == 0 {
        return String::new();
    }
    let gb = size_bytes as f64 / 1_073_741_824.0;
    if gb >= 1.0 {
        format!("{:.1}GB", gb)
    } else {
        let mb = size_bytes as f64 / 1_048_576.0;
        format!("{:.0}MB", mb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// A settings file of our own, in a directory of its own, so the preference store's
    /// per-path bookkeeping never sees two tests through one file.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yantrik-settings-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir.join("settings.yaml")
    }

    fn reload(path: &Path) -> UserSettings {
        let raw = std::fs::read_to_string(path).expect("the settings file is on the disk");
        serde_yaml::from_str(&raw).expect("the settings file parses")
    }

    /// Do-not-disturb has to come back after a restart, which means it has to be in the file.
    ///
    /// The audit that found this ran `grep dnd ~/.config/yantrik/settings.yaml` straight after a
    /// toggle that had answered `settled: true`, and read back `dnd_mode: false`. So this is the
    /// same check: write the preferences the way the shell writes them, read the bytes off the
    /// disk, and load them again. Both directions, because releasing the hold has to stick too —
    /// a file that merely lost the key would come up quiet on a machine somebody had unmuted.
    #[test]
    fn do_not_disturb_survives_the_write_and_the_reload() {
        let path = scratch("roundtrip");
        let name = path.to_string_lossy().to_string();

        let mut settings = UserSettings::default();
        assert!(
            !settings.dnd_mode,
            "a machine nobody has told anything comes up able to interrupt"
        );

        settings.dnd_mode = true;
        settings.pinned_apps = vec!["notes".to_string()];
        save_to(&name, &settings).expect("the settings file is written");

        let raw = std::fs::read_to_string(&path).expect("the settings file is on the disk");
        assert!(
            raw.contains("dnd_mode: true"),
            "the durable copy says nothing about do-not-disturb:\n{raw}"
        );
        let back = reload(&path);
        assert!(back.dnd_mode, "do-not-disturb did not survive the reload");
        assert_eq!(
            back.pinned_apps,
            vec!["notes".to_string()],
            "the neighbouring preferences were rewritten by the do-not-disturb write"
        );

        let mut released = back;
        released.dnd_mode = false;
        save_to(&name, &released).expect("the settings file is written again");
        let raw = std::fs::read_to_string(&path).expect("the settings file is on the disk");
        assert!(
            raw.contains("dnd_mode: false"),
            "releasing the hold left the file saying nothing:\n{raw}"
        );
        assert!(!reload(&path).dnd_mode, "the release did not survive the reload");

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// A write that cannot happen comes back as an error, not as a preference that quietly did
    /// not change.
    ///
    /// This is the half that `set_do_not_disturb` needs: it answers `settled`, which is a promise
    /// about the disk, so it has to be able to fail. A read-only settings file is the ordinary way
    /// this happens — the shell preserves it rather than replacing it.
    #[test]
    fn a_settings_file_that_cannot_be_written_is_reported_not_swallowed() {
        use std::os::unix::fs::PermissionsExt;

        let path = scratch("readonly");
        let name = path.to_string_lossy().to_string();
        let mut settings = UserSettings::default();
        save_to(&name, &settings).expect("the settings file is written once");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444))
            .expect("the settings file can be made read-only");

        settings.dnd_mode = true;
        let err = save_to(&name, &settings)
            .expect_err("a read-only settings file must not report a successful write");
        assert!(!err.is_empty(), "the failure has to say something");
        assert!(
            !reload(&path).dnd_mode,
            "the file changed after a write that reported failure"
        );

        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// A settings file written before the preference existed opens with notifications audible.
    #[test]
    fn a_settings_file_with_no_do_not_disturb_reads_as_released() {
        let settings: UserSettings = serde_yaml::from_str("dark_mode: true\naccent_color: cyan\n")
            .expect("an older settings file parses");
        assert!(!settings.dnd_mode);
    }

    /// And something puts it back on the window at boot, which is the other half of persisting it.
    ///
    /// Read from the source because there is no second reader: every toast decision asks the
    /// window for `dnd_mode`, so deleting this one line in `AppContext::init` would take the
    /// preference away again with every test in this file still passing.
    #[test]
    fn do_not_disturb_is_restored_when_the_shell_starts() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app_context.rs");
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let restores = src
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .any(|l| l.contains("set_dnd_mode(") && l.contains("user_settings.dnd_mode"));
        assert!(
            restores,
            "nothing in {} puts the saved do-not-disturb back on the window at start. \
             Persisting it is only half of the fix; without this the file is written and \
             never read.",
            path.display()
        );
    }
}
