//! Navigation wiring — on_navigate screen dispatch.
//!
//! Loads screen-specific data when entering a screen.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::bridge;
use crate::filebrowser;
use crate::notifications;
use crate::{
    App, BondData, BreadcrumbSegment, FileEntry, OpinionData, ProcessData, SharedRefData,
    TerminalTabData, UrgeCardData,
};

/// Wire on_navigate callback.
pub fn wire(ui: &App, ctx: &AppContext) {
    let bridge = ctx.bridge.clone();
    let ui_weak = ui.as_weak();
    let nav_timer: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));
    let timer_inner = nav_timer.clone();
    let browser_path = ctx.browser_path.clone();
    let browser_show_hidden = ctx.browser_show_hidden.clone();
    let notification_store = ctx.notification_store.clone();
    let system_snapshot = ctx.system_snapshot.clone();
    let terminals = ctx.terminals.clone();
    let terminal_active = ctx.terminal_active.clone();
    let term_poll_timer: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));

    ui.on_navigate(move |screen| {
        tracing::debug!(screen, "Navigate to screen");

        match screen {
            // Desktop — load pending urges
            1 => {
                let reply_rx = bridge.request_pending_urges();
                let weak = ui_weak.clone();
                let handle = timer_inner.clone();
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

                            ui.set_pending_count(cards.len() as i32);
                            ui.set_urges(ModelRc::new(VecModel::from(cards)));
                        }
                        *handle.borrow_mut() = None;
                    }
                });
                *timer_inner.borrow_mut() = Some(timer);
            }
            // Bond screen
            4 => {
                let reply_rx = bridge.request_bond();
                let weak = ui_weak.clone();
                let handle = timer_inner.clone();
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
                *timer_inner.borrow_mut() = Some(timer);
            }
            // Personality screen
            5 => {
                let reply_rx = bridge.request_evolution();
                let weak = ui_weak.clone();
                let handle = timer_inner.clone();
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
                *timer_inner.borrow_mut() = Some(timer);
            }
            // File browser screen
            8 => {
                let path = browser_path.borrow().clone();
                let show_hidden = *browser_show_hidden.borrow();
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_file_browser_path(slint::SharedString::from(&path));
                    let entries = filebrowser::list_dir_filtered(&path, show_hidden);
                    let items: Vec<FileEntry> = entries
                        .into_iter()
                        .map(|e| FileEntry {
                            name: e.name.into(),
                            is_dir: e.is_dir,
                            size_text: e.size_text.into(),
                            modified_text: e.modified_text.into(),
                            icon_char: e.icon_char.into(),
                        })
                        .collect();
                    ui.set_file_browser_entries(ModelRc::new(VecModel::from(items)));

                    // Update breadcrumbs
                    let segments = filebrowser::breadcrumb_segments(&path);
                    let crumbs: Vec<BreadcrumbSegment> = segments
                        .into_iter()
                        .map(|(label, full_path)| BreadcrumbSegment {
                            label: label.into(),
                            full_path: full_path.into(),
                        })
                        .collect();
                    ui.set_file_breadcrumbs(ModelRc::new(VecModel::from(crumbs)));
                }
            }
            // Notification Center — sync from store
            9 => {
                let store = notification_store.borrow();
                notifications::sync_to_ui(&store, &ui_weak);
            }
            // System Dashboard — populate from snapshot
            10 => {
                let snap = system_snapshot.borrow();
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_sys_cpu_usage(snap.cpu_usage_percent);
                    ui.set_sys_memory_usage(snap.memory_usage_percent());
                    ui.set_sys_memory_text(format_memory(snap.memory_used_bytes, snap.memory_total_bytes).into());
                    ui.set_sys_wifi_ssid(snap.network_ssid.clone().unwrap_or_default().into());
                    ui.set_sys_wifi_signal(snap.network_signal.unwrap_or(0) as i32);
                    ui.set_sys_uptime_text(format_uptime().into());

                    let procs: Vec<ProcessData> = snap
                        .running_processes
                        .iter()
                        .take(15)
                        .map(|p| ProcessData {
                            name: p.name.clone().into(),
                            pid: p.pid as i32,
                            cpu_percent: p.cpu_percent,
                        })
                        .collect();
                    ui.set_sys_top_processes(ModelRc::new(VecModel::from(procs)));
                }
            }
            // Terminal screen — spawn first tab if no tabs exist, start poll timer
            14 => {
                // Spawn first terminal tab if none exist
                {
                    let mut tabs = terminals.borrow_mut();
                    if tabs.is_empty() {
                        match crate::terminal::TerminalHandle::spawn(24, 80) {
                            Ok(th) => {
                                tabs.push(th);
                                *terminal_active.borrow_mut() = 0;
                                tracing::info!("Terminal tab 1 spawned");
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "Failed to spawn terminal");
                            }
                        }
                    }
                }

                // Sync tab UI state and start poll timer
                if let Some(ui) = ui_weak.upgrade() {
                    {
                        let tabs = terminals.borrow();
                        let active = *terminal_active.borrow();
                        // Build tab data for UI
                        let tab_data: Vec<TerminalTabData> = tabs
                            .iter()
                            .enumerate()
                            .map(|(i, th)| TerminalTabData {
                                title: slint::format!("Shell {}", i + 1),
                                is_active: i == active,
                                is_alive: th.is_alive(),
                            })
                            .collect();
                        ui.set_terminal_tab_count(tabs.len() as i32);
                        ui.set_terminal_active_tab(active as i32);
                        ui.set_terminal_tabs(ModelRc::new(VecModel::from(tab_data)));
                    }

                    super::terminal::start_poll_timer(
                        &ui,
                        &terminals,
                        &terminal_active,
                        &bridge,
                        &term_poll_timer,
                    );
                }
            }
            // Notes editor — load notes list
            15 => {
                if let Some(ui) = ui_weak.upgrade() {
                    super::notes::load_notes_list(&ui);
                }
            }
            _ => {}
        }
    });
}

/// Format memory as human-readable text.
fn format_memory(used_bytes: u64, total_bytes: u64) -> String {
    let used_mb = used_bytes / (1024 * 1024);
    let total_mb = total_bytes / (1024 * 1024);
    if total_mb >= 1024 {
        format!(
            "{:.1} / {:.1} GB",
            used_mb as f64 / 1024.0,
            total_mb as f64 / 1024.0
        )
    } else {
        format!("{} / {} MB", used_mb, total_mb)
    }
}

/// Read /proc/uptime and format as human-readable text.
fn format_uptime() -> String {
    std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|content| content.split_whitespace().next().map(String::from))
        .and_then(|s| s.parse::<f64>().ok())
        .map(|secs| {
            let total = secs as u64;
            let days = total / 86400;
            let hours = (total % 86400) / 3600;
            let mins = (total % 3600) / 60;
            if days > 0 {
                format!("{}d {}h", days, hours)
            } else if hours > 0 {
                format!("{}h {}m", hours, mins)
            } else {
                format!("{}m", mins)
            }
        })
        .unwrap_or_else(|| "—".to_string())
}
