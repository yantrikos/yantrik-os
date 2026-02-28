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
use crate::{App, BondData, FileEntry, OpinionData, SharedRefData, UrgeCardData};

/// Wire on_navigate callback.
pub fn wire(ui: &App, ctx: &AppContext) {
    let bridge = ctx.bridge.clone();
    let ui_weak = ui.as_weak();
    let nav_timer: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));
    let timer_inner = nav_timer.clone();
    let browser_path = ctx.browser_path.clone();

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
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_file_browser_path(slint::SharedString::from(&path));
                    let entries = filebrowser::list_dir(&path);
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
                }
            }
            _ => {}
        }
    });
}
