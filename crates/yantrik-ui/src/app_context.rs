//! AppContext — bundles all shared state into one struct.
//!
//! Created once in main(), passed by reference to wire modules.
//! New features add fields here instead of touching main.rs.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use slint::{ComponentHandle, ModelRc, Timer, VecModel};
use yantrik_companion::CompanionConfig;
use yantrik_companion::config::VoiceConfig;

use crate::activity_feed::ActivityAccumulator;
use crate::bridge::CompanionBridge;
use crate::cards::CardManager;
use crate::clipboard;
use crate::features;
use crate::frecency::FrecencyStore;
use crate::i18n::I18n;
use crate::notifications;
use crate::system_context;
use crate::wire::app_framework::{AppState, BackgroundJobManager};
use crate::wire::entity_bridge::SharedEntityGraph;
use crate::wire::image_viewer::ImageViewerState;
use crate::terminal::TerminalHandle;
use crate::wire::media_player::MpvHandle;
use crate::{App, AccentPreset, ThemeMode, ThemeOverrides, MessageData, UrgeCardData, WhisperCardItem};
use yantrik_companion::skills::SkillRegistry;

/// Clipboard operation for file browser copy/cut.
#[derive(Clone, Debug)]
pub enum FileClipOp {
    Copy { src_dir: String, name: String },
    Cut { src_dir: String, name: String },
}

/// All shared state needed by wire modules.
pub struct AppContext {
    pub bridge: Arc<CompanionBridge>,
    pub event_bus: yantrik_os::EventBus,
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
    pub editor_tabs: Rc<RefCell<Option<Rc<RefCell<Vec<crate::wire::text_editor::TabState>>>>>>,
    pub editor_active_tab: Rc<RefCell<Option<Rc<RefCell<usize>>>>>,
    pub media_player: Rc<RefCell<Option<MpvHandle>>>,
    pub frecency: Rc<RefCell<FrecencyStore>>,
    pub browser_history_back: Rc<RefCell<Vec<String>>>,
    pub browser_history_forward: Rc<RefCell<Vec<String>>>,
    pub browser_sort_field: Rc<RefCell<String>>,
    pub browser_sort_ascending: Rc<RefCell<bool>>,
    pub browser_filter: Rc<RefCell<String>>,
    pub summary_timer: Rc<RefCell<Option<Timer>>>,
    pub browser_multi_selection: Rc<RefCell<BTreeSet<usize>>>,
    pub telegram: Option<Arc<crate::telegram::TelegramHandle>>,
    pub terminals: Rc<RefCell<Vec<TerminalHandle>>>,
    pub terminal_active: Rc<RefCell<usize>>,
    pub terminal_split_handle: Rc<RefCell<Option<Rc<RefCell<Option<TerminalHandle>>>>>>,
    pub user_name: String,
    pub config_path: Option<PathBuf>,
    pub skill_registry: Rc<RefCell<SkillRegistry>>,
    pub i18n: I18n,
    pub entity_graph: SharedEntityGraph,
    pub app_state: AppState,
    pub job_manager: BackgroundJobManager,
}

impl AppContext {
    /// Initialize all shared state. Moves setup logic that used to live in main().
    pub fn init(mut config: CompanionConfig, ui: &App, config_path: Option<PathBuf>) -> Self {
        // Load persisted user settings (theme, tool perm, auto-lock, etc.)
        let user_settings = crate::wire::settings::load();
        ui.global::<ThemeMode>().set_dark(user_settings.dark_mode);
        ui.set_settings_dark_mode(user_settings.dark_mode);
        ui.set_settings_tool_permission(user_settings.tool_permission.clone().into());
        ui.set_settings_auto_lock_secs(user_settings.auto_lock_secs);
        ui.set_dnd_mode(user_settings.dnd_mode);

        // Accent color (persisted)
        let accent_idx = crate::wire::settings::accent_name_to_index(&user_settings.accent_color);
        ui.global::<AccentPreset>().set_index(accent_idx);
        ui.set_settings_accent_color(user_settings.accent_color.into());

        // Wallpaper (persisted)
        if !user_settings.wallpaper.is_empty() {
            let wp = &user_settings.wallpaper;
            ui.set_wallpaper_path(wp.as_str().into());
            // For custom file paths (not presets), load the image
            let presets = ["aurora", "sunset", "ocean", "nebula"];
            if !presets.contains(&wp.as_str()) {
                let path = std::path::Path::new(wp.as_str());
                if path.exists() && path.is_file() {
                    match slint::Image::load_from_path(path) {
                        Ok(img) => {
                            ui.set_wallpaper_image(img);
                            tracing::info!(wallpaper = %wp, "Restored custom wallpaper");
                        }
                        Err(e) => {
                            tracing::warn!(path = %wp, error = %e, "Failed to restore wallpaper image, clearing");
                            ui.set_wallpaper_path("".into());
                        }
                    }
                } else {
                    tracing::warn!(path = %wp, "Saved wallpaper file not found, clearing");
                    ui.set_wallpaper_path("".into());
                }
            } else {
                tracing::info!(wallpaper = %wp, "Restored preset wallpaper");
            }
        }

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

        // Populate settings panel — saved names override config defaults
        let display_user_name = if !user_settings.user_name.is_empty() {
            user_settings.user_name.clone()
        } else {
            config.user_name.clone()
        };
        let display_companion_name = if !user_settings.companion_name.is_empty() {
            user_settings.companion_name.clone()
        } else {
            config.personality.name.clone()
        };
        ui.set_settings_user_name(display_user_name.into());
        ui.set_settings_companion_name(display_companion_name.into());
        ui.set_settings_model_name(config.llm.hub_repo.clone().into());
        ui.set_settings_max_context(config.llm.max_context_tokens as i32);
        ui.set_settings_max_tokens(config.llm.max_tokens as i32);
        // tool_permission and auto_lock_secs already loaded from persisted settings above

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

        // Override config names with saved user settings (if user renamed)
        if !user_settings.user_name.is_empty() {
            config.user_name = user_settings.user_name.clone();
        }
        if !user_settings.companion_name.is_empty() {
            config.personality.name = user_settings.companion_name.clone();
        }

        // Save fields before moving config into bridge
        let user_name = config.user_name.clone();
        let voice_config = config.voice.clone();
        let chat_config_snapshot = config.clone(); // For multi-provider chat bridge
        let enabled_services = config.enabled_services.clone();

        // Create the cognitive event bus (+ persistent log)
        let event_bus = yantrik_os::EventBus::new();
        {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            let log_dir = format!("{}/.config/yantrik", home);
            let _ = std::fs::create_dir_all(&log_dir);
            let log_path = format!("{}/event_log.db", log_dir);
            match yantrik_os::EventLog::open(&log_path) {
                Ok(log) => {
                    event_bus.attach_log(log);
                    tracing::info!(path = %log_path, "Event log attached to bus");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to open event log — events will not persist");
                }
            }
        }

        // Start companion bridge (spawns worker thread)
        let bridge = Arc::new(CompanionBridge::start(config, ui.as_weak(), event_bus.clone()));

        // Start multi-provider chat system (Discord, Matrix, IRC, Slack, Signal, etc.)
        // This also handles Telegram if configured, replacing the legacy poller.
        let chat_bridge_ref = bridge.clone();
        let _chat_handle = yantrik_companion::chat_bridge::start_chat(
            &chat_config_snapshot,
            // AI callback: sends message through CompanionBridge, collects streaming response
            Box::new(move |text: &str, context: &[String], policy: &yantrik_chat::policy::ConversationPolicy| {
                let prompt = if context.is_empty() {
                    text.to_string()
                } else {
                    let history = context.iter()
                        .rev()
                        .take(6)
                        .rev()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!("[Chat context]\n{history}\n\n[Latest message]\n{text}")
                };

                // Send through bridge and collect all tokens
                let token_rx = chat_bridge_ref.send_message(prompt);
                let mut full_response = String::new();
                let mut replacing = false;
                while let Ok(token) = token_rx.recv() {
                    match token.as_str() {
                        "__DONE__" => break,
                        "__REPLACE__" => {
                            full_response.clear();
                            replacing = true;
                        }
                        _ => {
                            if replacing {
                                full_response = token;
                                replacing = false;
                            } else {
                                full_response.push_str(&token);
                            }
                        }
                    }
                }

                if full_response.is_empty() {
                    return None;
                }

                // Respect max reply length from policy
                if let Some(max_len) = policy.max_reply_length {
                    if full_response.len() > max_len {
                        let boundary = full_response.floor_char_boundary(max_len.saturating_sub(3));
                        full_response = format!("{}...", &full_response[..boundary]);
                    }
                }

                Some(full_response)
            }),
            // Brain callback: record events for cross-platform memory
            Box::new(move |sender_name: &str, _sender_id: &str, provider: &str, content_type: &str| {
                tracing::debug!(
                    sender = sender_name,
                    provider,
                    content_type,
                    "Chat brain: recording event"
                );
                // Brain integration happens via the CompanionBridge's RecordSystemEvent command
                // The companion worker thread will process this and update brain state
            }),
        );

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
        registry.register(Box::new(features::network_watcher::NetworkWatcher::new()));
        registry.register(Box::new(features::clipboard_intelligence::ClipboardIntelligence::new(clip_history.clone())));
        registry.register(Box::new(features::screen_watcher::ScreenWatcher::new()));
        registry.register(Box::new(features::project_consciousness::ProjectConsciousness::new()));

        // Initialize Skill Registry
        let skills_dir = {
            // Check /opt/yantrik/skills/ first (deployed), then relative to binary
            let deployed = std::path::PathBuf::from("/opt/yantrik/skills");
            if deployed.exists() {
                deployed
            } else {
                // Development fallback: relative to cwd
                std::env::current_dir().unwrap_or_default().join("skills")
            }
        };
        let skill_db_path = {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            format!("{}/.config/yantrik/skills.db", home)
        };
        // Ensure parent directory exists
        if let Some(parent) = std::path::Path::new(&skill_db_path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let skill_registry = match rusqlite::Connection::open(&skill_db_path) {
            Ok(conn) => {
                let mut reg = SkillRegistry::init(&conn, &skills_dir);
                // Auto-enable skills matching config.enabled_services on first run
                reg.auto_enable_for_services(&conn, &enabled_services);
                tracing::info!(
                    skills = reg.count(),
                    enabled = reg.enabled_count(),
                    dir = %skills_dir.display(),
                    "Skill Registry initialized"
                );
                Rc::new(RefCell::new(reg))
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to open skills.db — using empty registry");
                // Create a minimal in-memory registry
                let conn = rusqlite::Connection::open_in_memory().unwrap();
                Rc::new(RefCell::new(SkillRegistry::init(&conn, &skills_dir)))
            }
        };

        // Initialize entity graph (cross-app object model)
        let entity_graph: SharedEntityGraph = {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            let graph_path = format!("{}/.config/yantrik/entity_graph.db", home);
            match yantrik_os::EntityGraph::open(&graph_path) {
                Ok(g) => {
                    tracing::info!(path = %graph_path, "Entity graph initialized");
                    Arc::new(std::sync::Mutex::new(g))
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to open entity graph — using in-memory");
                    Arc::new(std::sync::Mutex::new(
                        yantrik_os::EntityGraph::in_memory().unwrap(),
                    ))
                }
            }
        };

        // Initialize AppState (persistent KV store)
        let app_state = {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            let state_path = format!("{}/.config/yantrik/app_state.db", home);
            match AppState::open(&state_path) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to open app_state — using in-memory");
                    AppState::in_memory().unwrap()
                }
            }
        };

        // Initialize background job manager
        let job_manager = {
            let bus = event_bus.clone();
            BackgroundJobManager::start(move |job_id, app, result| {
                tracing::info!(job_id = %job_id, app = %app, "Background job completed");
                let _ = bus.emit(
                    yantrik_os::EventKind::ToolCompleted {
                        tool_name: format!("bg_job:{app}:{job_id}"),
                        outcome: yantrik_os::ToolOutcome::Verified,
                        duration_ms: 0,
                        result_preview: result.chars().take(200).collect(),
                    },
                    yantrik_os::EventSource::Background,
                );
            })
        };

        Self {
            bridge,
            event_bus,
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
            notification_store: Rc::new(RefCell::new(notifications::NotificationStore::load())),
            voice_config,
            image_viewer_state: Rc::new(RefCell::new(ImageViewerState::default())),
            editor_file_path: Rc::new(RefCell::new(String::new())),
            editor_tabs: Rc::new(RefCell::new(None)),
            editor_active_tab: Rc::new(RefCell::new(None)),
            media_player: Rc::new(RefCell::new(None)),
            frecency: Rc::new(RefCell::new(FrecencyStore::load())),
            browser_history_back: Rc::new(RefCell::new(Vec::new())),
            browser_history_forward: Rc::new(RefCell::new(Vec::new())),
            browser_sort_field: Rc::new(RefCell::new("name".to_string())),
            browser_sort_ascending: Rc::new(RefCell::new(true)),
            browser_filter: Rc::new(RefCell::new(String::new())),
            summary_timer: Rc::new(RefCell::new(None)),
            browser_multi_selection: Rc::new(RefCell::new(BTreeSet::new())),
            telegram: None, // Legacy poller replaced by chat bridge
            terminals: Rc::new(RefCell::new(Vec::new())),
            terminal_active: Rc::new(RefCell::new(0)),
            terminal_split_handle: Rc::new(RefCell::new(None)),
            user_name,
            config_path,
            skill_registry,
            i18n: I18n::load(&I18n::detect_locale()),
            entity_graph,
            app_state,
            job_manager,
        }
    }
}

/// Get local time components using libc::localtime_r (respects /etc/localtime).
fn local_time() -> (u32, u32, u32, u32, u32, u32) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now as *const i64, &mut tm) };
    (
        tm.tm_hour as u32,
        tm.tm_min as u32,
        tm.tm_sec as u32,
        tm.tm_wday as u32,  // 0=Sun, 1=Mon, ..., 6=Sat
        tm.tm_mon as u32,   // 0-11
        tm.tm_mday as u32,  // 1-31
    )
}

/// Get current time as HH:MM string (local timezone).
pub fn current_time_hhmm() -> String {
    let (hours, minutes, _, _, _, _) = local_time();
    format!("{:02}:{:02}", hours, minutes)
}

/// Generate a time-of-day greeting (local timezone).
pub fn time_of_day_greeting() -> String {
    let (hour, _, _, _, _, _) = local_time();
    match hour {
        5..=11 => "Good morning".to_string(),
        12..=17 => "Good afternoon".to_string(),
        18..=21 => "Good evening".to_string(),
        _ => "Good night".to_string(),
    }
}

/// Get current date as a human-readable string, e.g. "Tuesday, March 4" (local timezone).
pub fn current_date_text() -> String {
    let (_, _, _, wday, mon, mday) = local_time();

    let day_names = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    let day_name = day_names[wday as usize];

    let month_names = [
        "January", "February", "March", "April", "May", "June",
        "July", "August", "September", "October", "November", "December",
    ];
    let month_name = month_names[mon as usize];

    format!("{}, {} {}", day_name, month_name, mday)
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
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
