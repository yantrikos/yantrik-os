//! Yantrik OS — AI-native desktop shell.
//!
//! The desktop's primary interface. Embeds CompanionService in-process
//! on a worker thread, renders via Slint on the main thread.
//!
//! Layout: boot animation → desktop (particle field, orb, Intent Lens).
//! The Intent Lens is the primary interaction — search, ask, launch, control.
//!
//! Usage:
//!   yantrik-ui [config.yaml]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::{Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use yantrikdb_companion::CompanionConfig;

mod bridge;
// NOTE: #[allow(dead_code)] required to avoid rustc 1.93.1 ICE in check_mod_deathness.
// Remove once rustc is updated past the fix.
#[allow(dead_code)]
mod features;
mod voice;

slint::include_modules!();

/// Known apps that can be launched via the Intent Lens or dock.
const KNOWN_APPS: &[(&str, &str, &str)] = &[
    ("terminal", "foot", "Open terminal emulator"),
    ("browser", "firefox-esr", "Open web browser"),
    ("files", "thunar", "Open file manager"),
];

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
    let config = load_config(config_path);
    let voice_config = config.voice.clone();

    // Create Slint UI
    let ui = App::new().unwrap();

    // Set boot status
    ui.set_boot_status("remembering...".into());

    // Set initial greeting based on time of day
    ui.set_greeting_text(time_of_day_greeting().into());

    // Start companion bridge (spawns worker thread)
    let bridge = Arc::new(bridge::CompanionBridge::start(config, ui.as_weak()));

    // ── Wire callbacks ──

    // Send message — used by Intent Lens chat mode
    let bridge_send = bridge.clone();
    let ui_weak_send = ui.as_weak();
    let stream_timer: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));
    let stream_timer_inner = stream_timer.clone();
    ui.on_send_message(move |text| {
        let text = text.to_string();
        if text.is_empty() {
            return;
        }

        let ui_weak = ui_weak_send.clone();

        // Add user message to the list
        if let Some(ui) = ui_weak.upgrade() {
            let messages = ui.get_messages();
            let model = messages
                .as_any()
                .downcast_ref::<VecModel<MessageData>>()
                .unwrap();
            model.push(MessageData {
                role: "user".into(),
                content: SharedString::from(&text),
                is_streaming: false,
            });
            // Add empty assistant bubble for streaming
            model.push(MessageData {
                role: "assistant".into(),
                content: "".into(),
                is_streaming: true,
            });
            ui.set_is_generating(true);
            // Switch Lens to chat mode
            ui.set_lens_chat_mode(true);
        }

        // Start streaming from companion
        let token_rx = bridge_send.send_message(text);
        let ui_weak_stream = ui_weak.clone();
        let timer_handle = stream_timer_inner.clone();

        // Poll tokens at 16ms (60fps)
        let timer = Timer::default();
        timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
            let mut done = false;
            while let Ok(token) = token_rx.try_recv() {
                if token == "__DONE__" {
                    done = true;
                    break;
                }
                if let Some(ui) = ui_weak_stream.upgrade() {
                    let messages = ui.get_messages();
                    let model = messages
                        .as_any()
                        .downcast_ref::<VecModel<MessageData>>()
                        .unwrap();
                    let count = model.row_count();
                    if count > 0 {
                        let mut last = model.row_data(count - 1).unwrap();
                        let mut content = last.content.to_string();
                        content.push_str(&token);
                        last.content = SharedString::from(&content);
                        model.set_row_data(count - 1, last);
                    }
                }
            }
            if done {
                if let Some(ui) = ui_weak_stream.upgrade() {
                    ui.set_is_generating(false);
                    let messages = ui.get_messages();
                    let model = messages
                        .as_any()
                        .downcast_ref::<VecModel<MessageData>>()
                        .unwrap();
                    let count = model.row_count();
                    if count > 0 {
                        let mut last = model.row_data(count - 1).unwrap();
                        last.is_streaming = false;
                        model.set_row_data(count - 1, last);
                    }
                }
                *timer_handle.borrow_mut() = None;
            }
        });
        *stream_timer_inner.borrow_mut() = Some(timer);
    });

    // ── Intent Lens: submit query ──
    // When user presses Enter in the Lens, route the query.
    let bridge_lens = bridge.clone();
    let ui_weak_lens = ui.as_weak();
    let lens_stream_timer: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));
    let lens_stream_inner = lens_stream_timer.clone();
    ui.on_lens_submit(move |query| {
        let query = query.to_string();
        if query.is_empty() {
            return;
        }

        tracing::info!(query = %query, "Lens submit");

        // Check if this is an app launch command
        let lower = query.to_lowercase();
        for (app_id, cmd, _) in KNOWN_APPS {
            if lower.contains(&format!("open {}", app_id))
                || lower.contains(app_id)
                || lower.contains(cmd)
            {
                tracing::info!(cmd, "Launching app from Lens");
                match std::process::Command::new(cmd).spawn() {
                    Ok(_) => tracing::info!(cmd, "App started"),
                    Err(e) => tracing::error!(cmd, error = %e, "Failed to launch app"),
                }
                if let Some(ui) = ui_weak_lens.upgrade() {
                    ui.set_lens_open(false);
                }
                return;
            }
        }

        // Otherwise treat as an AI conversation — send to companion
        if let Some(ui) = ui_weak_lens.upgrade() {
            let messages = ui.get_messages();
            let model = messages
                .as_any()
                .downcast_ref::<VecModel<MessageData>>()
                .unwrap();
            model.push(MessageData {
                role: "user".into(),
                content: SharedString::from(&query),
                is_streaming: false,
            });
            model.push(MessageData {
                role: "assistant".into(),
                content: "".into(),
                is_streaming: true,
            });
            ui.set_is_generating(true);
            ui.set_lens_chat_mode(true);
        }

        let token_rx = bridge_lens.send_message(query);
        let ui_weak_stream = ui_weak_lens.clone();
        let timer_handle = lens_stream_inner.clone();

        let timer = Timer::default();
        timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
            let mut done = false;
            while let Ok(token) = token_rx.try_recv() {
                if token == "__DONE__" {
                    done = true;
                    break;
                }
                if let Some(ui) = ui_weak_stream.upgrade() {
                    let messages = ui.get_messages();
                    let model = messages
                        .as_any()
                        .downcast_ref::<VecModel<MessageData>>()
                        .unwrap();
                    let count = model.row_count();
                    if count > 0 {
                        let mut last = model.row_data(count - 1).unwrap();
                        let mut content = last.content.to_string();
                        content.push_str(&token);
                        last.content = SharedString::from(&content);
                        model.set_row_data(count - 1, last);
                    }
                }
            }
            if done {
                if let Some(ui) = ui_weak_stream.upgrade() {
                    ui.set_is_generating(false);
                    let messages = ui.get_messages();
                    let model = messages
                        .as_any()
                        .downcast_ref::<VecModel<MessageData>>()
                        .unwrap();
                    let count = model.row_count();
                    if count > 0 {
                        let mut last = model.row_data(count - 1).unwrap();
                        last.is_streaming = false;
                        model.set_row_data(count - 1, last);
                    }
                }
                *timer_handle.borrow_mut() = None;
            }
        });
        *lens_stream_inner.borrow_mut() = Some(timer);
    });

    // ── Intent Lens: query changed (live search) ──
    let ui_weak_query = ui.as_weak();
    ui.on_lens_query(move |text| {
        let query = text.to_string();
        if let Some(ui) = ui_weak_query.upgrade() {
            if query.is_empty() {
                ui.set_lens_results(ModelRc::new(VecModel::<LensResult>::default()));
                ui.set_lens_chat_mode(false);
                return;
            }

            // Generate results based on query — keyword routing
            let lower = query.to_lowercase();
            let mut results = Vec::new();

            // App matches: "open terminal", "browser", "files"
            for (app_id, _cmd, desc) in KNOWN_APPS {
                if app_id.contains(&lower) || lower.contains(app_id) || lower.contains("open") {
                    results.push(LensResult {
                        result_type: "do".into(),
                        title: SharedString::from(format!("Open {}", capitalize(app_id))),
                        subtitle: SharedString::from(*desc),
                        icon_char: "▶".into(),
                        action_id: SharedString::from(format!("launch:{}", app_id)),
                    });
                }
            }

            // Web search: "search for X", "google X", "look up X"
            let search_prefixes = ["search for ", "search ", "google ", "look up ", "find online "];
            for prefix in &search_prefixes {
                if let Some(rest) = lower.strip_prefix(prefix) {
                    if !rest.is_empty() {
                        let search_url = format!(
                            "https://duckduckgo.com/?q={}",
                            rest.replace(' ', "+")
                        );
                        results.push(LensResult {
                            result_type: "do".into(),
                            title: SharedString::from(format!("Search: \"{}\"", rest)),
                            subtitle: "Open in browser".into(),
                            icon_char: "🔍".into(),
                            action_id: SharedString::from(format!("url:{}", search_url)),
                        });
                        break;
                    }
                }
            }

            // URL: "go to example.com", pasted URLs
            if lower.starts_with("http://") || lower.starts_with("https://")
                || lower.starts_with("go to ")
            {
                let url = if let Some(rest) = lower.strip_prefix("go to ") {
                    let rest = rest.trim();
                    if rest.contains('.') {
                        format!("https://{}", rest)
                    } else {
                        String::new()
                    }
                } else {
                    query.clone()
                };
                if !url.is_empty() {
                    results.push(LensResult {
                        result_type: "do".into(),
                        title: SharedString::from(format!("Open {}", &url)),
                        subtitle: "Open in browser".into(),
                        icon_char: "🌐".into(),
                        action_id: SharedString::from(format!("url:{}", url)),
                    });
                }
            }

            // Clipboard: "copy X", "paste", "clipboard"
            if lower == "paste" || lower == "clipboard" || lower.starts_with("what's on clipboard")
                || lower.starts_with("what did i copy")
            {
                results.push(LensResult {
                    result_type: "do".into(),
                    title: "Read clipboard".into(),
                    subtitle: "Show clipboard contents".into(),
                    icon_char: "📋".into(),
                    action_id: "clipboard:read".into(),
                });
            }

            // File operations: "show downloads", "list files", "what's in ~/X"
            if lower.starts_with("show ") || lower.starts_with("list ") || lower.contains("downloads")
                || lower.starts_with("what's in ")
            {
                let dir = if lower.contains("downloads") {
                    "~/Downloads"
                } else if lower.contains("documents") {
                    "~/Documents"
                } else if lower.contains("desktop") {
                    "~/Desktop"
                } else {
                    ""
                };
                if !dir.is_empty() {
                    results.push(LensResult {
                        result_type: "find".into(),
                        title: SharedString::from(format!("Browse {}", dir)),
                        subtitle: "List directory contents".into(),
                        icon_char: "📁".into(),
                        action_id: SharedString::from(format!("files:{}", dir)),
                    });
                }
            }

            // System info: "battery", "memory", "disk space", "uptime"
            if lower.contains("battery") || lower.contains("memory") || lower.contains("ram")
                || lower.contains("disk") || lower.contains("uptime") || lower.contains("system")
            {
                results.push(LensResult {
                    result_type: "find".into(),
                    title: "System status".into(),
                    subtitle: "Battery, memory, disk, uptime".into(),
                    icon_char: "📊".into(),
                    action_id: "system:status".into(),
                });
            }

            // Setting matches: "focus", "timer", "settings"
            if lower.contains("focus") || lower.contains("timer") {
                results.push(LensResult {
                    result_type: "setting".into(),
                    title: "Start focus mode".into(),
                    subtitle: "Dim desktop, suppress notifications".into(),
                    icon_char: "◎".into(),
                    action_id: "setting:focus".into(),
                });
            }

            // Memory search: "remember", "what do you know about"
            if lower.starts_with("remember") || lower.contains("you know about")
                || lower.starts_with("recall ")
            {
                results.push(LensResult {
                    result_type: "memory".into(),
                    title: SharedString::from(format!("Search memories: \"{}\"", &query)),
                    subtitle: "Search Yantrik's memory".into(),
                    icon_char: "🧠".into(),
                    action_id: SharedString::from(format!("memory:{}", &query)),
                });
            }

            // Always offer AI conversation as the last option
            if !lower.is_empty() {
                results.push(LensResult {
                    result_type: "ask".into(),
                    title: SharedString::from(format!("Ask: \"{}\"", &query)),
                    subtitle: "Send to Yantrik AI".into(),
                    icon_char: "?".into(),
                    action_id: SharedString::from(format!("ask:{}", &query)),
                });
            }

            ui.set_lens_results(ModelRc::new(VecModel::from(results)));
        }
    });

    // ── Intent Lens: result selected ──
    let ui_weak_result = ui.as_weak();
    ui.on_lens_result_selected(move |action_id| {
        let action = action_id.to_string();
        tracing::info!(action = %action, "Lens result selected");

        if action.starts_with("launch:") {
            let app_id = &action[7..];
            for (_id, cmd, _) in KNOWN_APPS {
                if app_id == *_id {
                    match std::process::Command::new(cmd).spawn() {
                        Ok(_) => tracing::info!(cmd, "App started"),
                        Err(e) => tracing::error!(cmd, error = %e, "Failed to launch"),
                    }
                    break;
                }
            }
            if let Some(ui) = ui_weak_result.upgrade() {
                ui.set_lens_open(false);
            }
        } else if action.starts_with("url:") {
            let url = &action[4..];
            tracing::info!(url, "Opening URL from Lens");
            match std::process::Command::new("xdg-open").arg(url).spawn() {
                Ok(_) => tracing::info!(url, "URL opened"),
                Err(e) => tracing::error!(url, error = %e, "Failed to open URL"),
            }
            if let Some(ui) = ui_weak_result.upgrade() {
                ui.set_lens_open(false);
            }
        } else if action == "clipboard:read" || action.starts_with("files:")
            || action == "system:status" || action.starts_with("memory:")
        {
            // Route these through the AI as natural language queries
            let query = match action.as_str() {
                "clipboard:read" => "What's on my clipboard?".to_string(),
                "system:status" => "Show me system status — battery, memory, disk.".to_string(),
                a if a.starts_with("files:") => format!("List the files in {}", &a[6..]),
                a if a.starts_with("memory:") => a[7..].to_string(),
                _ => action.clone(),
            };
            if let Some(ui) = ui_weak_result.upgrade() {
                ui.invoke_lens_submit(SharedString::from(&query));
            }
        } else if action.starts_with("ask:") {
            let query = &action[4..];
            if let Some(ui) = ui_weak_result.upgrade() {
                ui.invoke_lens_submit(SharedString::from(query));
            }
        }
    });

    // ── Lens open/close ──
    let ui_weak_open = ui.as_weak();
    ui.on_open_lens(move || {
        tracing::debug!("Lens opened");
        // Could trigger pre-population of suggestions here
        let _ = ui_weak_open.upgrade();
    });

    let ui_weak_close = ui.as_weak();
    ui.on_close_lens(move || {
        tracing::debug!("Lens closed");
        if let Some(ui) = ui_weak_close.upgrade() {
            // Reset lens state on close
            ui.set_lens_results(ModelRc::new(VecModel::<LensResult>::default()));
            ui.set_lens_chat_mode(false);
        }
    });

    // Navigation callback — load data when entering certain screens.
    let bridge_nav = bridge.clone();
    let ui_weak_nav = ui.as_weak();
    let nav_timer: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));
    let nav_timer_inner = nav_timer.clone();
    ui.on_navigate(move |screen| {
        tracing::debug!(screen, "Navigate to screen");

        match screen {
            // Desktop — load pending urges
            1 => {
                let reply_rx = bridge_nav.request_pending_urges();
                let weak = ui_weak_nav.clone();
                let handle = nav_timer_inner.clone();
                let timer = Timer::default();
                timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
                    if let Ok(urges) = reply_rx.try_recv() {
                        if let Some(ui) = weak.upgrade() {
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_secs_f64();

                            let cards: Vec<UrgeCardData> = urges
                                .iter()
                                .map(|u| UrgeCardData {
                                    urge_id: u.urge_id.clone().into(),
                                    instinct_name: u.instinct_name.clone().into(),
                                    reason: u.reason.clone().into(),
                                    urgency: u.urgency as f32,
                                    suggested_message: u.suggested_message.clone().into(),
                                    time_ago: bridge::format_time_ago(now - u.created_at).into(),
                                    border_color: bridge::instinct_color(&u.instinct_name),
                                })
                                .collect();

                            // Set pending count for Quiet Queue badge
                            ui.set_pending_count(cards.len() as i32);
                            ui.set_urges(ModelRc::new(VecModel::from(cards)));
                        }
                        *handle.borrow_mut() = None;
                    }
                });
                *nav_timer_inner.borrow_mut() = Some(timer);
            }
            // Bond screen
            4 => {
                let reply_rx = bridge_nav.request_bond();
                let weak = ui_weak_nav.clone();
                let handle = nav_timer_inner.clone();
                let timer = Timer::default();
                timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
                    if let Ok(bond) = reply_rx.try_recv() {
                        if let Some(ui) = weak.upgrade() {
                            ui.set_bond_data(BondData {
                                bond_score: bond.bond_score as f32,
                                bond_level: bond.bond_level.into(),
                                total_interactions: bond.total_interactions as i32,
                                days_together: bond.days_together as i32,
                                current_streak: bond.current_streak as i32,
                                humor_rate: bond.humor_rate as f32,
                                vulnerability_events: bond.vulnerability_events as i32,
                                shared_references: bond.shared_references as i32,
                            });
                        }
                        *handle.borrow_mut() = None;
                    }
                });
                *nav_timer_inner.borrow_mut() = Some(timer);
            }
            // Personality screen
            5 => {
                let reply_rx = bridge_nav.request_evolution();
                let weak = ui_weak_nav.clone();
                let handle = nav_timer_inner.clone();
                let timer = Timer::default();
                timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
                    if let Ok(evo) = reply_rx.try_recv() {
                        if let Some(ui) = weak.upgrade() {
                            ui.set_formality(evo.formality as f32);
                            ui.set_humor_ratio(evo.humor_ratio as f32);
                            ui.set_opinion_strength(evo.opinion_strength as f32);
                            ui.set_question_ratio(evo.question_ratio as f32);

                            let opinions: Vec<OpinionData> = evo
                                .opinions
                                .iter()
                                .map(|o| OpinionData {
                                    topic: o.topic.clone().into(),
                                    stance: o.stance.clone().into(),
                                    confidence: o.confidence as f32,
                                })
                                .collect();
                            ui.set_opinions(ModelRc::new(VecModel::from(opinions)));

                            let refs: Vec<SharedRefData> = evo
                                .shared_refs
                                .iter()
                                .map(|r| SharedRefData {
                                    text: r.text.clone().into(),
                                    times_used: r.times_used as i32,
                                })
                                .collect();
                            ui.set_shared_refs(ModelRc::new(VecModel::from(refs)));
                        }
                        *handle.borrow_mut() = None;
                    }
                });
                *nav_timer_inner.borrow_mut() = Some(timer);
            }
            _ => {}
        }
    });

    // Memory search callback
    let bridge_search = bridge.clone();
    let ui_weak_search = ui.as_weak();
    let search_timer: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));
    let search_timer_inner = search_timer.clone();
    ui.on_search_memories(move |query| {
        let query = query.to_string();
        if query.is_empty() {
            return;
        }

        if let Some(ui) = ui_weak_search.upgrade() {
            ui.set_is_searching_memories(true);
        }

        let reply_rx = bridge_search.recall_memories(query);
        let weak = ui_weak_search.clone();
        let handle = search_timer_inner.clone();
        let timer = Timer::default();
        timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
            if let Ok(results) = reply_rx.try_recv() {
                if let Some(ui) = weak.upgrade() {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs_f64();

                    let items: Vec<MemoryItem> = results
                        .iter()
                        .map(|r| MemoryItem {
                            rid: r.rid.clone().into(),
                            text: r.text.clone().into(),
                            memory_type: r.memory_type.clone().into(),
                            importance: r.importance as f32,
                            valence: r.valence as f32,
                            score: r.score as f32,
                            time_ago: bridge::format_time_ago(now - r.created_at).into(),
                        })
                        .collect();
                    ui.set_memory_results(ModelRc::new(VecModel::from(items)));
                    ui.set_is_searching_memories(false);
                }
                *handle.borrow_mut() = None;
            }
        });
        *search_timer_inner.borrow_mut() = Some(timer);
    });

    // App launch callback (from dock)
    ui.on_launch_app(move |app_id| {
        let app = app_id.to_string();
        tracing::info!(app = %app, "Launching app");

        let cmd = match app.as_str() {
            "terminal" => "foot",
            "browser" => "firefox-esr",
            "files" => "thunar",
            "settings" => return,
            _ => {
                tracing::warn!(app = %app, "Unknown app");
                return;
            }
        };

        match std::process::Command::new(cmd).spawn() {
            Ok(_) => tracing::info!(cmd, "App started"),
            Err(e) => tracing::error!(cmd, error = %e, "Failed to launch app"),
        }
    });

    // Voice mode
    let voice_session: Arc<Mutex<Option<voice::VoiceSession>>> =
        Arc::new(Mutex::new(None));

    let voice_session_mic = voice_session.clone();
    let bridge_mic = bridge.clone();
    let ui_weak_mic = ui.as_weak();
    let voice_config_mic = voice_config.clone();
    ui.on_mic_pressed(move || {
        let mut session = voice_session_mic.lock().unwrap();
        if session.is_none() {
            if let Some(ui) = ui_weak_mic.upgrade() {
                ui.set_voice_active(true);
                ui.set_voice_state(0);
                ui.set_voice_transcribed("".into());
                ui.set_voice_response("".into());
            }
            *session = Some(voice::VoiceSession::start(
                bridge_mic.clone(),
                ui_weak_mic.clone(),
                voice_config_mic.clone(),
            ));
            tracing::info!("Voice mode started");
        }
    });

    let voice_session_cancel = voice_session.clone();
    let ui_weak_cancel = ui.as_weak();
    ui.on_cancel_voice(move || {
        let mut session = voice_session_cancel.lock().unwrap();
        if let Some(mut s) = session.take() {
            s.stop();
        }
        if let Some(ui) = ui_weak_cancel.upgrade() {
            ui.set_voice_active(false);
        }
        tracing::info!("Voice mode cancelled");
    });

    // Set up message model
    let messages_model = VecModel::<MessageData>::default();
    ui.set_messages(ModelRc::new(messages_model));

    // Set up urges model
    let urges_model = VecModel::<UrgeCardData>::default();
    ui.set_urges(ModelRc::new(urges_model));

    // Clock timer (updates every 30 seconds)
    let ui_weak_clock = ui.as_weak();
    let clock_timer = Timer::default();
    clock_timer.start(TimerMode::Repeated, Duration::from_secs(30), move || {
        if let Some(ui) = ui_weak_clock.upgrade() {
            ui.set_clock_text(current_time_hhmm().into());
            ui.set_greeting_text(time_of_day_greeting().into());
        }
    });
    // Set initial clock
    ui.set_clock_text(current_time_hhmm().into());

    // Background cognition timer (every 60s)
    let bridge_think = bridge.clone();
    let think_timer = Timer::default();
    think_timer.start(TimerMode::Repeated, Duration::from_secs(60), move || {
        bridge_think.think();
    });

    // ── System Observer + Proactive Features ──

    // Load system observer config from same YAML
    let sys_config = load_system_config(std::env::args().nth(1).map(PathBuf::from));

    // Start the system observer (spawns monitor threads)
    let observer = yantrik_os::SystemObserver::start(&sys_config);

    // Create feature registry and register all v1 features
    let mut registry = features::FeatureRegistry::new();
    registry.register(Box::new(features::resource_guardian::ResourceGuardian::new()));
    registry.register(Box::new(features::process_sentinel::ProcessSentinel::new()));
    registry.register(Box::new(features::focus_flow::FocusFlow::new()));

    let scorer = features::UrgencyScorer::new();
    let system_snapshot = yantrik_os::SystemSnapshot::default();

    // Wrap in Rc<RefCell> for shared main-thread access
    let observer = Rc::new(observer);
    let registry = Rc::new(RefCell::new(registry));
    let scorer = Rc::new(RefCell::new(scorer));
    let system_snapshot = Rc::new(RefCell::new(system_snapshot));

    // System poll timer — drains events, runs features, updates UI, records in brain (every 3s)
    let ui_weak_sys = ui.as_weak();
    let observer_poll = observer.clone();
    let registry_poll = registry.clone();
    let scorer_poll = scorer.clone();
    let snapshot_poll = system_snapshot.clone();
    let bridge_sys = bridge.clone();
    let system_timer = Timer::default();
    system_timer.start(TimerMode::Repeated, Duration::from_secs(3), move || {
        // 1. Drain all pending system events
        let events = observer_poll.drain();
        if events.is_empty() {
            // Still tick features (for time-based logic like FocusFlow)
            let snap = snapshot_poll.borrow();
            let ctx = features::FeatureContext {
                system: &snap,
                clock: std::time::SystemTime::now(),
            };
            let tick_urges = registry_poll.borrow_mut().tick(&ctx);
            if !tick_urges.is_empty() {
                let scored = scorer_poll.borrow_mut().score(tick_urges);
                if !scored.is_empty() {
                    push_whisper_cards(&ui_weak_sys, &scored);
                }
            }
            return;
        }

        // 2. Process each event
        let mut all_urges = Vec::new();
        for event in &events {
            // Update system snapshot
            snapshot_poll.borrow_mut().apply(event);

            // Route through features
            let snap = snapshot_poll.borrow();
            let ctx = features::FeatureContext {
                system: &snap,
                clock: std::time::SystemTime::now(),
            };
            let event_urges = registry_poll.borrow_mut().process_event(event, &ctx);
            all_urges.extend(event_urges);
        }

        // Tick features too
        {
            let snap = snapshot_poll.borrow();
            let ctx = features::FeatureContext {
                system: &snap,
                clock: std::time::SystemTime::now(),
            };
            all_urges.extend(registry_poll.borrow_mut().tick(&ctx));
        }

        // 3. Forward significant events to the brain (companion memory)
        for event in &events {
            if let Some((text, domain, importance)) = event_to_memory(event) {
                bridge_sys.record_system_event(text, domain, importance);
            }
        }

        // 4. Update status bar from snapshot
        let snap = snapshot_poll.borrow();
        if let Some(ui) = ui_weak_sys.upgrade() {
            ui.set_battery_level(snap.battery_level as i32);
            ui.set_battery_charging(snap.battery_charging);
            ui.set_wifi_connected(snap.network_connected);
        }

        // 4b. Update system context for LLM prompt injection
        bridge_sys.set_system_context(format_system_context(&snap));

        // 5. Score and display urges
        if !all_urges.is_empty() {
            let scored = scorer_poll.borrow_mut().score(all_urges);
            if !scored.is_empty() {
                tracing::info!(
                    count = scored.len(),
                    top_pressure = scored[0].pressure,
                    top_title = %scored[0].urge.title,
                    "Whisper cards generated"
                );
                push_whisper_cards(&ui_weak_sys, &scored);
            }
        }
    });

    // Run the Slint event loop
    tracing::info!("Starting Yantrik OS desktop shell");
    ui.run().unwrap();

    tracing::info!("Yantrik OS shutting down");
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

/// Get current time as HH:MM string.
fn current_time_hhmm() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let hours = (now / 3600) % 24;
    let minutes = (now / 60) % 60;
    format!("{:02}:{:02}", hours, minutes)
}

/// Generate a time-of-day greeting.
fn time_of_day_greeting() -> String {
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

/// Capitalize first letter of a string.
fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

/// Load system observer config from the YAML file.
/// Falls back to defaults (mock mode) if not present.
fn load_system_config(path: Option<PathBuf>) -> yantrik_os::SystemObserverConfig {
    let Some(p) = path else {
        return yantrik_os::SystemObserverConfig {
            mock: true,
            ..Default::default()
        };
    };

    // Parse the YAML and extract the "system" key
    let contents = match std::fs::read_to_string(&p) {
        Ok(c) => c,
        Err(_) => {
            return yantrik_os::SystemObserverConfig {
                mock: true,
                ..Default::default()
            };
        }
    };

    let yaml: serde_yaml::Value = match serde_yaml::from_str(&contents) {
        Ok(v) => v,
        Err(_) => {
            return yantrik_os::SystemObserverConfig {
                mock: true,
                ..Default::default()
            };
        }
    };

    match yaml.get("system") {
        Some(sys_val) => {
            serde_yaml::from_value(sys_val.clone()).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "Invalid system config, using defaults");
                yantrik_os::SystemObserverConfig {
                    mock: true,
                    ..Default::default()
                }
            })
        }
        None => {
            tracing::info!("No 'system' section in config, using mock mode");
            yantrik_os::SystemObserverConfig {
                mock: true,
                ..Default::default()
            }
        }
    }
}

/// Format a SystemSnapshot into a compact string for LLM context injection.
/// Kept short (~100 tokens) to fit the token budget.
fn format_system_context(snap: &yantrik_os::SystemSnapshot) -> String {
    let mut parts = Vec::new();

    // Battery
    let charge_str = if snap.battery_charging { " (charging)" } else { "" };
    parts.push(format!("Battery: {}%{}", snap.battery_level, charge_str));

    // Network
    if snap.network_connected {
        let ssid = snap.network_ssid.as_deref().unwrap_or("connected");
        parts.push(format!("WiFi: {}", ssid));
    } else {
        parts.push("WiFi: disconnected".to_string());
    }

    // CPU & memory
    if snap.cpu_usage_percent > 0.0 {
        parts.push(format!("CPU: {:.0}%", snap.cpu_usage_percent));
    }
    if snap.memory_total_bytes > 0 {
        let used_mb = snap.memory_used_bytes / (1024 * 1024);
        let total_mb = snap.memory_total_bytes / (1024 * 1024);
        parts.push(format!("RAM: {}/{}MB ({:.0}%)", used_mb, total_mb, snap.memory_usage_percent()));
    }

    // Running processes (top 5 by name)
    if !snap.running_processes.is_empty() {
        let names: Vec<&str> = snap.running_processes.iter().take(5).map(|p| p.name.as_str()).collect();
        parts.push(format!("Apps: {}", names.join(", ")));
    }

    // User idle
    if snap.user_idle && snap.idle_seconds > 60 {
        parts.push(format!("User idle: {}m", snap.idle_seconds / 60));
    }

    parts.join("\n")
}

/// Convert a system event into a memory record (text, domain, importance).
/// Returns None for events that aren't worth remembering (routine resource polls).
fn event_to_memory(event: &yantrik_os::SystemEvent) -> Option<(String, String, f64)> {
    use yantrik_os::SystemEvent;
    match event {
        SystemEvent::BatteryChanged { level, charging, .. } => {
            // Only record significant battery events
            if *charging {
                Some((
                    format!("Battery started charging at {}%", level),
                    "system/battery".into(),
                    0.3,
                ))
            } else if *level <= 20 {
                Some((
                    format!("Battery low at {}%", level),
                    "system/battery".into(),
                    0.6,
                ))
            } else {
                None // Don't record every battery tick
            }
        }
        SystemEvent::NetworkChanged { connected, ssid, .. } => {
            let text = if *connected {
                format!("Connected to network{}", ssid.as_ref().map(|s| format!(" '{}'", s)).unwrap_or_default())
            } else {
                "Network disconnected".into()
            };
            Some((text, "system/network".into(), 0.4))
        }
        SystemEvent::NotificationReceived { app, summary, .. } => {
            Some((
                format!("Notification from {}: {}", app, summary),
                "system/notification".into(),
                0.5,
            ))
        }
        SystemEvent::FileChanged { path, kind } => {
            let action = match kind {
                yantrik_os::FileChangeKind::Created => "created",
                yantrik_os::FileChangeKind::Modified => "modified",
                yantrik_os::FileChangeKind::Deleted => "deleted",
                yantrik_os::FileChangeKind::Renamed { to } => {
                    return Some((
                        format!("File renamed: {} → {}", path, to),
                        "system/files".into(),
                        0.3,
                    ));
                }
            };
            Some((
                format!("File {}: {}", action, path),
                "system/files".into(),
                0.3,
            ))
        }
        SystemEvent::ProcessStarted { name, .. } => {
            Some((
                format!("App opened: {}", name),
                "system/process".into(),
                0.2,
            ))
        }
        SystemEvent::ProcessStopped { name, .. } => {
            Some((
                format!("App closed: {}", name),
                "system/process".into(),
                0.2,
            ))
        }
        SystemEvent::UserIdle { idle_seconds } if *idle_seconds > 300 => {
            Some((
                format!("User idle for {} minutes", idle_seconds / 60),
                "system/presence".into(),
                0.2,
            ))
        }
        SystemEvent::UserResumed => {
            Some((
                "User returned".into(),
                "system/presence".into(),
                0.3,
            ))
        }
        // Skip routine resource pressure events (too noisy for memory)
        _ => None,
    }
}

/// Push scored urges as Whisper Cards to the Slint UI.
fn push_whisper_cards(ui_weak: &slint::Weak<App>, scored: &[features::ScoredUrge]) {
    let cards: Vec<UrgeCardData> = scored
        .iter()
        .filter(|s| s.tier != features::UrgeTier::Drop)
        .map(|s| {
            let color = match s.urge.category {
                features::UrgeCategory::Resource => {
                    slint::Color::from_rgb_u8(0xD4, 0xA5, 0x74) // amber
                }
                features::UrgeCategory::Security => {
                    slint::Color::from_rgb_u8(0xE8, 0x6B, 0x6B) // red
                }
                features::UrgeCategory::FileManagement => {
                    slint::Color::from_rgb_u8(0x5A, 0xC8, 0xD4) // cyan
                }
                features::UrgeCategory::Focus => {
                    slint::Color::from_rgb_u8(0xC4, 0x8B, 0xD4) // purple
                }
                features::UrgeCategory::Celebration => {
                    slint::Color::from_rgb_u8(0x8B, 0xE8, 0x6B) // green
                }
            };

            UrgeCardData {
                urge_id: s.urge.id.clone().into(),
                instinct_name: s.urge.source.clone().into(),
                reason: s.urge.body.clone().into(),
                urgency: s.pressure,
                suggested_message: s.urge.title.clone().into(),
                time_ago: "just now".into(),
                border_color: color,
            }
        })
        .collect();

    if cards.is_empty() {
        return;
    }

    let weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            // Merge with existing urges (append new ones)
            let existing = ui.get_urges();
            let model = existing
                .as_any()
                .downcast_ref::<VecModel<UrgeCardData>>();

            if let Some(model) = model {
                for card in cards {
                    model.push(card);
                }
                ui.set_pending_count(model.row_count() as i32);
            } else {
                let count = cards.len();
                ui.set_urges(ModelRc::new(VecModel::from(cards)));
                ui.set_pending_count(count as i32);
            }
        }
    });
}
