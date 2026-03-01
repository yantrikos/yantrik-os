//! AppContext — bundles all shared state into one struct.
//!
//! Created once in main(), passed by reference to wire modules.
//! New features add fields here instead of touching main.rs.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use slint::{ComponentHandle, ModelRc, VecModel};
use yantrikdb_companion::CompanionConfig;
use yantrikdb_companion::config::VoiceConfig;

use crate::bridge::CompanionBridge;
use crate::cards::CardManager;
use crate::clipboard;
use crate::features;
use crate::notifications;
use crate::system_context;
use crate::{App, ThemeMode, MessageData, UrgeCardData, WhisperCardItem};

/// All shared state needed by wire modules.
pub struct AppContext {
    pub bridge: Arc<CompanionBridge>,
    pub installed_apps: Arc<Vec<crate::apps::DesktopEntry>>,
    pub clip_history: clipboard::SharedHistory,
    pub browser_path: Rc<RefCell<String>>,
    pub card_manager: Rc<RefCell<CardManager>>,
    pub observer: Rc<yantrik_os::SystemObserver>,
    pub feature_registry: Rc<RefCell<features::FeatureRegistry>>,
    pub scorer: Rc<RefCell<features::UrgencyScorer>>,
    pub system_snapshot: Rc<RefCell<yantrik_os::SystemSnapshot>>,
    pub notification_store: notifications::SharedStore,
    pub voice_config: VoiceConfig,
}

impl AppContext {
    /// Initialize all shared state. Moves setup logic that used to live in main().
    pub fn init(config: CompanionConfig, ui: &App) -> Self {
        // Theme mode (dark by default)
        ui.global::<ThemeMode>().set_dark(true);

        // Boot status + greeting
        ui.set_boot_status("remembering...".into());
        ui.set_greeting_text(time_of_day_greeting().into());

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

        // Save voice config before moving config into bridge
        let voice_config = config.voice.clone();

        // Start companion bridge (spawns worker thread)
        let bridge = Arc::new(CompanionBridge::start(config, ui.as_weak()));

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
            card_manager: Rc::new(RefCell::new(CardManager::new())),
            observer,
            feature_registry: Rc::new(RefCell::new(registry)),
            scorer: Rc::new(RefCell::new(features::UrgencyScorer::new())),
            system_snapshot: Rc::new(RefCell::new(yantrik_os::SystemSnapshot::default())),
            notification_store: Rc::new(RefCell::new(notifications::NotificationStore::new())),
            voice_config,
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
