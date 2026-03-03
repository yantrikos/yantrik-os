//! AppContext — bundles all shared state into one struct.
//!
//! Created once in main(), passed by reference to wire modules.
//! New features add fields here instead of touching main.rs.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use slint::{ComponentHandle, ModelRc, Timer, VecModel};
use yantrikdb_companion::CompanionConfig;
use yantrikdb_companion::config::VoiceConfig;

use crate::activity_feed::ActivityAccumulator;
use crate::bridge::CompanionBridge;
use crate::cards::CardManager;
use crate::clipboard;
use crate::features;
use crate::frecency::FrecencyStore;
use crate::notifications;
use crate::system_context;
use crate::wire::image_viewer::ImageViewerState;
use crate::terminal::TerminalHandle;
use crate::wire::media_player::MpvHandle;
use crate::{App, ThemeMode, ThemeOverrides, MessageData, UrgeCardData, WhisperCardItem};

/// Clipboard operation for file browser copy/cut.
#[derive(Clone, Debug)]
pub enum FileClipOp {
    Copy { src_dir: String, name: String },
    Cut { src_dir: String, name: String },
}

/// All shared state needed by wire modules.
pub struct AppContext {
    pub bridge: Arc<CompanionBridge>,
    pub installed_apps: Arc<Vec<crate::apps::DesktopEntry>>,
    pub clip_history: clipboard::SharedHistory,
    pub browser_path: Rc<RefCell<String>>,
    pub browser_show_hidden: Rc<RefCell<bool>>,
    pub file_clipboard: Rc<RefCell<Option<FileClipOp>>>,
    pub card_manager: Rc<RefCell<CardManager>>,
    pub observer: Rc<yantrik_os::SystemObserver>,
    pub feature_registry: Rc<RefCell<features::FeatureRegistry>>,
    pub scorer: Rc<RefCell<features::UrgencyScorer>>,
    pub system_snapshot: Rc<RefCell<yantrik_os::SystemSnapshot>>,
    pub accumulator: Rc<RefCell<ActivityAccumulator>>,
    pub notification_store: notifications::SharedStore,
    pub voice_config: VoiceConfig,
    pub image_viewer_state: Rc<RefCell<ImageViewerState>>,
    pub editor_file_path: Rc<RefCell<String>>,
    pub media_player: Rc<RefCell<Option<MpvHandle>>>,
    pub frecency: Rc<RefCell<FrecencyStore>>,
    pub browser_history_back: Rc<RefCell<Vec<String>>>,
    pub browser_history_forward: Rc<RefCell<Vec<String>>>,
    pub browser_sort_field: Rc<RefCell<String>>,
    pub browser_sort_ascending: Rc<RefCell<bool>>,
    pub browser_filter: Rc<RefCell<String>>,
    pub summary_timer: Rc<RefCell<Option<Timer>>>,
    pub telegram: Option<Arc<crate::telegram::TelegramHandle>>,
    pub terminal: Rc<RefCell<Option<TerminalHandle>>>,
    pub user_name: String,
}

impl AppContext {
    /// Initialize all shared state. Moves setup logic that used to live in main().
    pub fn init(config: CompanionConfig, ui: &App) -> Self {
        // Theme mode (load persisted preference, default dark)
        let dark = crate::wire::settings::load_theme_preference();
        ui.global::<ThemeMode>().set_dark(dark);
        ui.set_settings_dark_mode(dark);

        // Community theme overrides
        load_theme_overrides(ui);

        // Boot status + greeting (personalized with user name)
        ui.set_boot_status("remembering...".into());
        ui.set_greeting_text(
            format!("{}, {}", time_of_day_greeting(), config.user_name).into(),
        );

        // First-boot onboarding check
        if !crate::onboarding::marker_path().exists() {
            ui.set_onboarding_step(1);
            tracing::info!("First boot detected — onboarding enabled");
        }

        // Lock screen PIN file
        crate::lock::ensure_pin_file();

        // Populate settings panel
        ui.set_settings_user_name(config.user_name.clone().into());
        ui.set_settings_companion_name(config.personality.name.clone().into());
        ui.set_settings_model_name(config.llm.hub_repo.clone().into());
        ui.set_settings_max_context(config.llm.max_context_tokens as i32);
        ui.set_settings_max_tokens(config.llm.max_tokens as i32);
        ui.set_settings_tool_permission(config.tools.max_permission.clone().into());
        ui.set_settings_auto_lock_secs(crate::lock::DEFAULT_IDLE_LOCK_SECS as i32);

        // LLM backend settings
        ui.set_settings_llm_backend(config.llm.backend.clone().into());
        if let Some(ref url) = config.llm.resolve_api_base_url() {
            ui.set_settings_llm_api_url(url.clone().into());
        }
        if let Some(ref model) = config.llm.api_model {
            ui.set_settings_llm_api_model(model.clone().into());
        }

        // Display resolution (best effort via wlr-randr)
        if let Ok(output) = std::process::Command::new("wlr-randr").output() {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                for line in text.lines() {
                    let trimmed = line.trim();
                    if trimmed.contains('x')
                        && trimmed.chars().next().map_or(false, |c| c.is_ascii_digit())
                    {
                        if let Some(res) = trimmed.split_whitespace().next() {
                            ui.set_settings_display_resolution(res.into());
                            break;
                        }
                    }
                }
            }
        }

        // IP address (best effort)
        if let Ok(output) = std::process::Command::new("hostname").arg("-I").output() {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                if let Some(ip) = text.split_whitespace().next() {
                    ui.set_settings_ip_address(ip.into());
                }
            }
        }

        // Generate labwc keybind config
        yantrik_os::keybinds::ensure_labwc_config();

        // Scan installed apps
        let installed_apps = Arc::new(crate::apps::scan());

        // Start clipboard watcher
        let clip_history = clipboard::start_watcher();

        // Save fields before moving config into bridge
        let user_name = config.user_name.clone();
        let voice_config = config.voice.clone();
        let tg_config = config.telegram.clone();

        // Start companion bridge (spawns worker thread)
        let bridge = Arc::new(CompanionBridge::start(config, ui.as_weak()));

        // Start Telegram poller if configured
        tracing::info!(
            enabled = tg_config.enabled,
            has_token = tg_config.bot_token.is_some(),
            has_chat_id = tg_config.chat_id.is_some(),
            "Telegram config check"
        );
        let telegram = if tg_config.enabled
            && tg_config.bot_token.is_some()
            && tg_config.chat_id.is_some()
        {
            tracing::info!("Starting Telegram bot poller");
            Some(Arc::new(crate::telegram::start_poller(
                tg_config,
                bridge.clone(),
                ui.as_weak(),
            )))
        } else {
            None
        };

        // Set up UI models
        ui.set_messages(ModelRc::new(VecModel::<MessageData>::default()));
        ui.set_urges(ModelRc::new(VecModel::<UrgeCardData>::default()));
        ui.set_whisper_cards(ModelRc::new(VecModel::<WhisperCardItem>::default()));

        // Set initial clock
        ui.set_clock_text(current_time_hhmm().into());

        // Ensure ~/.yantrik directory and cmd_log exist for ErrorCompanion
        if let Ok(home) = std::env::var("HOME") {
            let yantrik_dir = PathBuf::from(&home).join(".yantrik");
            let _ = std::fs::create_dir_all(&yantrik_dir);
            let cmd_log = yantrik_dir.join("cmd_log");
            if !cmd_log.exists() {
                let _ = std::fs::File::create(&cmd_log);
            }
        }

        // Load system observer config
        let mut sys_config =
            system_context::load_system_config(std::env::args().nth(1).map(PathBuf::from));
        sys_config.watch_dirs.push("~/.yantrik".to_string());

        // Start system observer
        let observer = Rc::new(yantrik_os::SystemObserver::start(&sys_config));

        // Create feature registry and register all v1 features
        let mut registry = features::FeatureRegistry::new();
        registry.register(Box::new(features::resource_guardian::ResourceGuardian::new()));
        registry.register(Box::new(features::process_sentinel::ProcessSentinel::new()));
        registry.register(Box::new(features::focus_flow::FocusFlow::new()));
        registry.register(Box::new(features::error_companion::ErrorCompanion::new()));
        registry.register(Box::new(features::notification_relay::NotificationRelay::new()));
        registry.register(Box::new(features::tool_suggester::ToolSuggester::new()));

        Self {
            bridge,
            installed_apps,
            clip_history,
            browser_path: Rc::new(RefCell::new("~".to_string())),
            browser_show_hidden: Rc::new(RefCell::new(false)),
            file_clipboard: Rc::new(RefCell::new(None)),
            card_manager: Rc::new(RefCell::new(CardManager::new())),
            observer,
            feature_registry: Rc::new(RefCell::new(registry)),
            scorer: Rc::new(RefCell::new(features::UrgencyScorer::new())),
            system_snapshot: Rc::new(RefCell::new(yantrik_os::SystemSnapshot::default())),
            accumulator: Rc::new(RefCell::new(ActivityAccumulator::new())),
            notification_store: Rc::new(RefCell::new(notifications::NotificationStore::new())),
            voice_config,
            image_viewer_state: Rc::new(RefCell::new(ImageViewerState::default())),
            editor_file_path: Rc::new(RefCell::new(String::new())),
            media_player: Rc::new(RefCell::new(None)),
            frecency: Rc::new(RefCell::new(FrecencyStore::load())),
            browser_history_back: Rc::new(RefCell::new(Vec::new())),
            browser_history_forward: Rc::new(RefCell::new(Vec::new())),
            browser_sort_field: Rc::new(RefCell::new("name".to_string())),
            browser_sort_ascending: Rc::new(RefCell::new(true)),
            browser_filter: Rc::new(RefCell::new(String::new())),
            summary_timer: Rc::new(RefCell::new(None)),
            telegram,
            terminal: Rc::new(RefCell::new(None)),
            user_name,
        }
    }
}

/// Get current time as HH:MM string.
pub fn current_time_hhmm() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let hours = (now / 3600) % 24;
    let minutes = (now / 60) % 60;
    format!("{:02}:{:02}", hours, minutes)
}

/// Generate a time-of-day greeting.
pub fn time_of_day_greeting() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let hour = (now / 3600) % 24;
    match hour {
        5..=11 => "Good morning".to_string(),
        12..=17 => "Good afternoon".to_string(),
        18..=21 => "Good evening".to_string(),
        _ => "Good night".to_string(),
    }
}

/// Load community theme overrides from ~/.config/yantrik/theme-override.yaml.
///
/// YAML format:
/// ```yaml
/// name: "Nord"
/// enabled: true
/// bg_deep: "#2e3440"
/// bg_surface: "#3b4252"
/// bg_card: "#434c5e"
/// bg_elevated: "#4c566a"
/// amber: "#ebcb8b"
/// cyan: "#88c0d0"
/// text_primary: "#eceff4"
/// text_secondary: "#d8dee9"
/// text_dim: "#4c566a"
/// accent: "#81a1c1"
/// ```
fn load_theme_overrides(ui: &App) {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let path = format!("{}/.config/yantrik/theme-override.yaml", home);

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return, // No override file — use defaults
    };

    // Parse YAML manually (simple key: value pairs)
    let mut enabled = false;
    let mut colors: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, val)) = trimmed.split_once(':') {
            let key = key.trim();
            let val = val.trim().trim_matches('"');
            if key == "enabled" {
                enabled = val == "true";
            } else if val.starts_with('#') && key != "name" {
                colors.insert(key.to_string(), val.to_string());
            }
        }
    }

    if !enabled {
        return;
    }

    let overrides = ui.global::<ThemeOverrides>();
    overrides.set_enabled(true);

    let set_color = |key: &str, setter: &dyn Fn(slint::Color)| {
        if let Some(hex) = colors.get(key) {
            if let Some(color) = parse_hex_color(hex) {
                setter(color);
            }
        }
    };

    set_color("bg_deep", &|c| overrides.set_bg_deep_override(c));
    set_color("bg_surface", &|c| overrides.set_bg_surface_override(c));
    set_color("bg_card", &|c| overrides.set_bg_card_override(c));
    set_color("bg_elevated", &|c| overrides.set_bg_elevated_override(c));
    set_color("amber", &|c| overrides.set_amber_override(c));
    set_color("cyan", &|c| overrides.set_cyan_override(c));
    set_color("text_primary", &|c| overrides.set_text_primary_override(c));
    set_color("text_secondary", &|c| overrides.set_text_secondary_override(c));
    set_color("text_dim", &|c| overrides.set_text_dim_override(c));
    set_color("accent", &|c| overrides.set_accent_override(c));

    tracing::info!(
        tokens = colors.len(),
        "Community theme override loaded"
    );
}

/// Parse a hex color string (#RRGGBB or #RRGGBBAA) into a slint::Color.
fn parse_hex_color(hex: &str) -> Option<slint::Color> {
    let hex = hex.trim_start_matches('#');
    if hex.len() == 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        Some(slint::Color::from_rgb_u8(r, g, b))
    } else if hex.len() == 8 {
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
        Some(slint::Color::from_argb_u8(a, r, g, b))
    } else {
        None
    }
}
