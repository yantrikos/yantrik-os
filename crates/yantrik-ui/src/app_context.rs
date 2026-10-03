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
use crate::terminal::TerminalHandle;
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
    /// The installed apps, live. Not a snapshot: something installed while the shell is
    /// running has to become launchable without restarting it.
    pub installed_apps: crate::apps::Catalogue,
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
    /// Configured LLM endpoint, so first-boot probes test the endpoint the
    /// companion will actually use rather than assuming localhost.
    pub llm_base_url: Option<String>,
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
        ui.set_settings_auto_lock_secs(crate::wire::settings::auto_lock_choice(user_settings.auto_lock_secs));
        // The only place do-not-disturb comes back after a restart. Everything that suppresses a
        // toast reads `dnd_mode` off the window (see `wire::notifications::maybe_toast`), so this
        // one line is what makes "held until I say otherwise" mean anything across a reboot.
        ui.set_dnd_mode(user_settings.dnd_mode);

        // Accent color (persisted)
        let accent_idx = crate::wire::settings::accent_name_to_index(&user_settings.accent_color);
        ui.global::<AccentPreset>().set_index(accent_idx);
        ui.set_settings_accent_color(user_settings.accent_color.into());

        // Wallpaper (persisted)
        //
        // A machine that has never been told otherwise gets a scene rather than a flat wash.
        // That is a look decision and also a structural one: the translucent surfaces this OS
        // draws need something with depth behind them, and a smooth gradient gives them
        // nothing to be in front of.
        if user_settings.wallpaper.is_empty() {
            ui.set_wallpaper_path(crate::wire::settings::DEFAULT_WALLPAPER.into());
        }
        if !user_settings.wallpaper.is_empty() {
            let wp = &user_settings.wallpaper;
            ui.set_wallpaper_path(wp.as_str().into());
            // For custom file paths (not presets), load the image
            if !crate::wire::settings::WALLPAPER_PRESETS.contains(&wp.as_str()) {
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

        // The saved theme's palette and its files on the machine (wire/theme.rs), then the
        // community override file on top: a person's own file is the last word.
        crate::wire::theme::restore(ui);
        // The lock screen's blurred wallpaper, made if it is missing or is for another wallpaper.
        crate::lock_wallpaper::refresh_if_stale(&ui.get_wallpaper_path());
        load_theme_overrides(ui);

        // Boot status + greeting (personalized with user name)
        // No boot status word. The stages say what is happening, one line each, and a fixed
        // "remembering..." underneath them was the last thing on that screen still pretending.
        ui.set_greeting_text(
            format!("{}, {}", time_of_day_greeting(), config.user_name).into(),
        );

        // First-boot onboarding check
        if !crate::onboarding::marker_path().exists() {
            ui.set_onboarding_step(1);
            tracing::info!("First boot detected — onboarding enabled");
            // Installed by the graphical installer, which already made the account: open on
            // the optional-setup welcome (phase 3), past the animation and the name question.
            if crate::onboarding::after_install_marker_path().exists() {
                ui.set_onboard_after_install(true);
                ui.set_onboard_phase(3);
                tracing::info!("First boot after install — offering the optional setup");
            }
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
        // So Settings → AI can ask the companion's endpoint the way the companion does.
        crate::wire::ai_status::set_config_key(config.llm.api_key.clone());

        // ── What is answering, in the status bar ──
        //
        // The chip up there has always been able to show the model and never had one to show:
        // ai-provider-label was declared, read by status_bar.slint, and set by nothing, so it
        // fell through to the word "Local" on every machine. Which is true, and is also the
        // least interesting true thing available — "Local" is a property of the arrangement,
        // and the arrangement is the whole point of this OS, so it deserves to say WHICH mind
        // is answering and where it runs.
        //
        // Read from the configuration rather than from whatever answered last: this is set
        // before the first question is asked, and a chip that is blank until you talk to the
        // machine is a chip that is blank when you most want to check.
        let backend = config.llm.backend.to_ascii_lowercase();
        let model_label = config
            .llm
            .api_model
            .clone()
            .filter(|m| !m.trim().is_empty())
            .or_else(|| {
                // llama.cpp and the built-in path name a repository rather than a model id;
                // the last path segment is the part a person would recognise.
                let repo = config.llm.hub_repo.trim();
                (!repo.is_empty()).then(|| {
                    repo.rsplit('/').next().unwrap_or(repo).to_string()
                })
            });

        if let Some(model) = model_label {
            // Cloud backends are named, because "which cloud" is the question you are actually
            // asking when you look. Local ones are not, because the lock icon beside this
            // already says where it runs and repeating it costs width the bar does not have.
            let label = match backend.as_str() {
                "claude-cli" | "claude_cli" => format!("Claude · {model}"),
                "api" => model,
                _ => model,
            };
            ui.set_ai_active_provider_label(label.into());
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

        // Scan installed apps
        let installed_apps = crate::apps::Catalogue::shared();

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
        let llm_base_url = config.llm.api_base_url.clone();
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
        // One board for the whole process: the worker reports to it and the RPC layer reads it,
        // so what a caller is told about the queue is the queue.
        let board = crate::jobs::Board::new();
        let bridge =
            Arc::new(CompanionBridge::start(config, ui.as_weak(), event_bus.clone(), board));

        // Start multi-provider chat system (Discord, Matrix, IRC, Slack, Signal, etc.)
        // This also handles Telegram if configured, replacing the legacy poller.
        let _chat_handle = yantrik_companion::chat_bridge::start_chat(
            &chat_config_snapshot,
            // AI callback: the person's message to the mind answering, and its reply. The router
            // asks only about a direct message from one of `chat.people` (`ChatRouter::set_people`).
            Box::new(move |text: &str, context: &[String], policy: &yantrik_chat::policy::ConversationPolicy, asker: &yantrik_chat::router::Asker, outbox: &yantrik_chat::router::Outbox| {
                // The mind answering, from the phone, answered later through the outbox; or an
                // answer to a card on the phone, answered now. See `channels`.
                crate::channels::from_phone(text, context, policy.max_reply_length, asker, outbox)
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
            // Private mode: the channels keep nothing while it is on.
            Box::new(crate::private_mode::is_on),
            // An answer to a card on the phone is never kept: its code is not context.
            Box::new(crate::channels::is_card_answer),
        );
        // What the shell sends to a channel unasked (a card on the phone), and which channels the
        // person trusts with an Allow though their operator can read them.
        if let Some(handle) = &_chat_handle {
            crate::channels::configure(handle.outbox(), chat_config_snapshot.chat.phone_approvals.clone());
        }

        // Set up UI models
        ui.set_messages(ModelRc::new(VecModel::<MessageData>::default()));
        ui.set_urges(ModelRc::new(VecModel::<UrgeCardData>::default()));
        ui.set_whisper_cards(ModelRc::new(VecModel::<WhisperCardItem>::default()));

        // Set initial clock
        ui.set_clock_text(current_time_hhmm().into());
        ui.set_date_text(current_date_short().into());

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
        // `NotificationRelay` is not registered any more. It turned every notification into an
        // urge, and an urge is drawn as a whisper card at the top right — so since the toasts
        // arrived, each notification was on screen twice, and the second copy sat exactly where
        // the approval card goes: the card asking "allow this?" came up with a grey ghost of its
        // own notice behind it. The toast is the one place a notification is shown. A mind still
        // learns of it: `system_context` and the activity feed read the same event.
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
            // A mirror of the notifications service's store, not a store of its own. It starts
            // empty and the first poll fills it; the service owns the file.
            notification_store: Rc::new(RefCell::new(notifications::NotificationMirror::new())),
            voice_config,
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
            llm_base_url,
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

/// Short date for the status bar: "Thu 4 Sep". The long form belongs on the lock screen,
/// where it is the only thing to read; on a 32px bar it is three words too many.
pub fn current_date_short() -> String {
    let (_, _, _, wday, mon, mday) = local_time();
    let day = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][wday as usize];
    let month = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
        [mon as usize];
    format!("{day} {mday} {month}")
}

/// The `clock` object of `describe shell`: the top bar's time told in full —
/// `{"date": "2026-09-23", "weekday": "Wednesday", "time": "18:31",
/// "utc_offset": "-05:00", "zone": "America/Chicago"}`.
///
/// A mind that needed today's date used to run `shell.agent_run` with `date`, which is graded
/// sensitive, so learning what day it is raised an approval card for the person (#207). The
/// shell already knows the time — it draws the clock in the top bar — so here it is, said in
/// full: the top bar's `clock` ("18:31") and `date` ("Wed 23 Sep") leave out the year, the
/// offset and the zone, and a caller working out "today" cannot do without them.
///
/// The zone name reveals location. That is what the clock on the status bar already shows
/// anyone at the screen, and describe only reaches local callers.
pub fn clock_for_describe() -> serde_json::Value {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&secs as *const i64, &mut tm) };
    clock_object(
        &ClockParts {
            year: i64::from(tm.tm_year) + 1900,
            month: tm.tm_mon as u32 + 1,
            day: tm.tm_mday as u32,
            weekday: tm.tm_wday as u32,
            hour: tm.tm_hour as u32,
            minute: tm.tm_min as u32,
            offset_secs: tm.tm_gmtoff as i64,
        },
        &zone_name(),
    )
}

/// One instant as libc broke it down: the local date and time `localtime_r` resolved for a
/// timestamp, and the offset it resolved along with them.
struct ClockParts {
    year: i64,
    month: u32,
    day: u32,
    weekday: u32,
    hour: u32,
    minute: u32,
    /// Seconds east of UTC, `tm_gmtoff`'s own convention. Read off the instant, never
    /// derived from the zone name: the same zone is -05:00 in September and -06:00 in
    /// January, and only libc knows which side of the switch a timestamp falls on.
    offset_secs: i64,
}

/// The `clock` object from a timestamp's parts and a zone. Pure — nothing but the instant
/// and the zone go in — so its shape is testable against a table of machines without
/// waiting for a minute to come around on any of them.
fn clock_object(parts: &ClockParts, zone: &str) -> serde_json::Value {
    let day_names = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    serde_json::json!({
        "date": format!("{:04}-{:02}-{:02}", parts.year, parts.month, parts.day),
        "weekday": day_names[(parts.weekday % 7) as usize],
        "time": format!("{:02}:{:02}", parts.hour, parts.minute),
        "utc_offset": offset_text(parts.offset_secs),
        "zone": zone,
    })
}

/// "±HH:MM" from seconds east of UTC — `tm_gmtoff`'s own sign convention, negative west of
/// Greenwich. The minutes are computed rather than assumed zero because half-hour and
/// three-quarter-hour zones are real: Asia/Kolkata is +05:30 and Asia/Kathmandu +05:45.
fn offset_text(offset_secs: i64) -> String {
    let sign = if offset_secs < 0 { '-' } else { '+' };
    let magnitude = offset_secs.unsigned_abs();
    format!("{sign}{:02}:{:02}", magnitude / 3600, (magnitude % 3600) / 60)
}

/// The IANA zone name this machine's clock runs on, or "" when the machine does not name
/// one.
fn zone_name() -> String {
    let tz = std::env::var("TZ").ok();
    let link = std::fs::read_link("/etc/localtime")
        .ok()
        .map(|target| target.to_string_lossy().into_owned());
    zone_name_from(tz.as_deref(), link.as_deref())
}

/// Which of the machine's zone sayings wins — or that none does, and the answer is "".
///
/// The two sources are the ones libc itself resolves in order: `TZ` first, then the
/// zoneinfo target of the `/etc/localtime` link, read the way `wire::location` reads it.
/// Nothing else is asked, because nothing else says what the clock is on: `/etc/timezone`
/// can lag — the live VM this issue was found on had it saying Etc/UTC while the link
/// pointed at America/Chicago — and the abbreviation libc resolved ("CDT") is no IANA
/// name. When the machine names no zone the honest answer is "", and the `utc_offset`
/// beside it already tells a reader what the clock is on.
fn zone_name_from(tz_env: Option<&str>, localtime_target: Option<&str>) -> String {
    if let Some(tz) = tz_env {
        // A leading ':' is how a shell says the rest is an IANA name; libc strips it, so
        // this does too. A TZ may also point at a zone file rather than name one.
        let name = tz.trim().trim_start_matches(':');
        if name.is_empty() {
            // glibc reads a set-but-empty TZ as UTC: the clock runs on UTC, the machine
            // still names no zone, and the link beside it would name one the clock is not
            // on.
            return String::new();
        }
        return name.split("zoneinfo/").nth(1).unwrap_or(name).to_string();
    }
    localtime_target
        .and_then(|target| target.split("zoneinfo/").nth(1))
        .filter(|name| !name.is_empty())
        .unwrap_or_default()
        .to_string()
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

#[cfg(test)]
mod clock_tests {
    use super::{clock_object, offset_text, zone_name_from, ClockParts};
    use serde_json::json;

    fn parts(
        year: i64,
        month: u32,
        day: u32,
        weekday: u32,
        hour: u32,
        minute: u32,
        offset_secs: i64,
    ) -> ClockParts {
        ClockParts { year, month, day, weekday, hour, minute, offset_secs }
    }

    /// The object the issue asked for (#207), across a table of machines: one running UTC
    /// and naming no zone, Chicago in summer and in winter — the same zone on two offsets,
    /// because the offset is what libc resolved for the instant and never derived from the
    /// name — and a half-hour zone. A mind reads this instead of running `date` through a
    /// sensitive `agent_run` and raising a card to learn the day.
    #[test]
    fn the_clock_says_the_date_the_weekday_the_time_the_offset_and_the_zone() {
        assert_eq!(
            clock_object(&parts(2026, 9, 23, 3, 18, 31, 0), ""),
            json!({
                "date": "2026-09-23",
                "weekday": "Wednesday",
                "time": "18:31",
                "utc_offset": "+00:00",
                "zone": "",
            }),
            "a machine with no zone of its own says so rather than claiming one"
        );
        assert_eq!(
            clock_object(&parts(2026, 9, 23, 3, 18, 31, -18_000), "America/Chicago"),
            json!({
                "date": "2026-09-23",
                "weekday": "Wednesday",
                "time": "18:31",
                "utc_offset": "-05:00",
                "zone": "America/Chicago",
            })
        );
        assert_eq!(
            clock_object(&parts(2026, 1, 4, 0, 9, 5, -21_600), "America/Chicago"),
            json!({
                "date": "2026-01-04",
                "weekday": "Sunday",
                "time": "09:05",
                "utc_offset": "-06:00",
                "zone": "America/Chicago",
            }),
            "the same zone, an hour further west in winter: single-digit months, days and \
             hours keep their zeroes, and the offset follows the instant, not the name"
        );
        assert_eq!(
            clock_object(&parts(2026, 9, 24, 4, 5, 1, 19_800), "Asia/Kolkata"),
            json!({
                "date": "2026-09-24",
                "weekday": "Thursday",
                "time": "05:01",
                "utc_offset": "+05:30",
                "zone": "Asia/Kolkata",
            }),
            "half-hour zones are real, so the offset's minutes are computed"
        );
    }

    #[test]
    fn the_offset_carries_its_sign_and_its_minutes() {
        assert_eq!(offset_text(-18000), "-05:00");
        assert_eq!(offset_text(19800), "+05:30", "Asia/Kolkata");
        assert_eq!(offset_text(20700), "+05:45", "Asia/Kathmandu");
        assert_eq!(offset_text(0), "+00:00");
    }

    /// The name has to tell the same zone the clock beside it is on, and when the machine
    /// names none the object says "" rather than guess: the live VM this issue was found on
    /// had /etc/localtime pointing at America/Chicago while /etc/timezone still said
    /// Etc/UTC, so anything past the two sources libc resolves would have been a coin flip.
    #[test]
    fn the_zone_is_the_one_the_machine_names_and_never_a_guess() {
        let chicago_link = Some("../usr/share/zoneinfo/America/Chicago");
        assert_eq!(
            zone_name_from(Some("Asia/Kolkata"), chicago_link),
            "Asia/Kolkata",
            "TZ is what libc consults first, so the name has to match it"
        );
        assert_eq!(
            zone_name_from(Some(":America/Denver"), None),
            "America/Denver",
            "a leading ':' introduces an IANA name and is stripped"
        );
        assert_eq!(
            zone_name_from(Some("/usr/share/zoneinfo/Asia/Kolkata"), None),
            "Asia/Kolkata",
            "a TZ pointing at a zone file names the zone it points at"
        );
        assert_eq!(
            zone_name_from(None, chicago_link),
            "America/Chicago",
            "the zoneinfo target of the /etc/localtime link"
        );
        assert_eq!(
            zone_name_from(Some(""), chicago_link),
            "",
            "an empty TZ runs the clock on UTC and names no zone; the link beside it \
             would name a zone the clock is not on"
        );
        assert_eq!(zone_name_from(None, None), "", "a machine with no zone file says so");
    }
}
