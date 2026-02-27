//! Yantrik OS — native Slint UI shell.
//!
//! The phone's primary interface. Embeds CompanionService in-process
//! on a worker thread, renders via Slint on the main thread.
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
mod voice;

slint::include_modules!();

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

    // Start companion bridge (spawns worker thread)
    let bridge = Arc::new(bridge::CompanionBridge::start(config, ui.as_weak()));

    // ── Wire callbacks ──

    // Send message — timer must outlive the callback, so we store it in an Rc.
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
        }

        // Start streaming from companion
        let token_rx = bridge_send.send_message(text);
        let ui_weak_stream = ui_weak.clone();
        let timer_handle = stream_timer_inner.clone();

        // Poll tokens at 16ms (60fps) — stored in Rc so it survives this closure
        let timer = Timer::default();
        timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
            let mut done = false;
            // Drain all available tokens this frame
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
                    // Mark last message as not streaming
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
                // Stop the timer now that generation is done
                *timer_handle.borrow_mut() = None;
            }
        });
        // Store timer so it lives beyond this callback
        *stream_timer_inner.borrow_mut() = Some(timer);
    });

    // Navigation callback — load data when entering certain screens.
    // Timer stored in Rc so it survives the callback.
    let bridge_nav = bridge.clone();
    let ui_weak_nav = ui.as_weak();
    let nav_timer: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));
    let nav_timer_inner = nav_timer.clone();
    ui.on_navigate(move |screen| {
        tracing::debug!(screen, "Navigate to screen");

        match screen {
            // Home screen — load pending urges
            3 => {
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
                                    time_ago: format_time_ago(now - u.created_at).into(),
                                    border_color: bridge::instinct_color(&u.instinct_name),
                                })
                                .collect();
                            ui.set_urges(ModelRc::new(VecModel::from(cards)));
                        }
                        *handle.borrow_mut() = None;
                    }
                });
                *nav_timer_inner.borrow_mut() = Some(timer);
            }
            // Bond screen — request bond data
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
            // Personality screen — request evolution data
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

    // Memory search callback — timer stored in Rc so it survives the callback.
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
                            time_ago: format_time_ago(now - r.created_at).into(),
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

    // Background cognition timer (every 60s)
    let bridge_think = bridge.clone();
    let think_timer = Timer::default();
    think_timer.start(TimerMode::Repeated, Duration::from_secs(60), move || {
        bridge_think.think();
    });

    // Run the Slint event loop
    tracing::info!("Starting Yantrik OS shell");
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

/// Format seconds-ago into a human-readable string.
fn format_time_ago(seconds: f64) -> String {
    if seconds < 60.0 {
        "just now".to_string()
    } else if seconds < 3600.0 {
        format!("{}m ago", (seconds / 60.0) as i64)
    } else if seconds < 86400.0 {
        format!("{}h ago", (seconds / 3600.0) as i64)
    } else {
        format!("{}d ago", (seconds / 86400.0) as i64)
    }
}
