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
    UrgeCardData,
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
    let terminal_split_handle = ctx.terminal_split_handle.clone();
    let term_split_poll_timer: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));

    ui.on_navigate(move |screen| {
        tracing::debug!(screen, "Navigate to screen");

        // Every path that shows a screen ends here — the control surface, the command palette,
        // a notification, the Lens, cross-app requests and app.slint's own buttons all set the
        // screen and then call this. While the desktop is locked none of them is shown and
        // nothing is loaded for it: the lock's screen is put back. See `crate::lock`.
        if let Some(ui) = ui_weak.upgrade() {
            if crate::lock::refuse_screen(&ui, screen) {
                return;
            }
        }

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
                                loaded: true,
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
            // Directory I/O is owned by the asynchronous Files controller.
            8 => { if let Some(ui)=ui_weak.upgrade() { ui.invoke_file_refresh(); } }
            // Notification Center — draw the mirror at once, and mark what is showing as read.
            //
            // Read on open, because opening this screen IS reading them: leaving the badge at
            // eleven after somebody has looked at all eleven is how a badge stops meaning
            // anything. The service is told off-thread and its answer arrives on the next poll.
            9 => {
                {
                    let store = notification_store.borrow();
                    notifications::sync_to_ui(&store, &ui_weak);
                }
                super::notifications::mark_showing_read(&ui_weak);
            }
            // System Dashboard — populate from snapshot
            10 => {
                let snap = system_snapshot.borrow();
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_sys_cpu_usage(snap.cpu_usage_percent);
                    // The breakdown rows (Used / Cached / Free, swap) come from
                    // the same function the live poll feeds them through. This
                    // entry path used to set only the headline figure, so the
                    // labels rendered with nothing beside them until an
                    // observer event happened to arrive.
                    super::system_poll::update_memory_readouts(&ui, &snap);
                    // The network row is fed from the network service on the
                    // system poll's own cadence, on every screen — there is
                    // nothing about it to populate on entry.
                    //
                    // Uptime is the About screen's reader, not a second copy
                    // of it: this path had its own formatter that printed
                    // "3d 1h" where About printed "3d 1h 2m".
                    ui.set_sys_uptime_text(super::about::read_uptime().into());

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
            // Package Manager — auto-refresh on open
            21 => {
                tracing::debug!("Navigated to package manager");
                if let Some(ui) = ui_weak.upgrade() {
                    ui.invoke_pkg_refresh();
                }
            }
            // Device Dashboard
            27 => {
                tracing::debug!("Navigated to device dashboard");
            }
            // Permission Dashboard
            28 => {
                tracing::debug!("Navigated to permission dashboard");
            }
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_system_screen_states_the_about_screen_s_uptime() {
        // This screen used to read /proc/uptime through a formatter of its
        // own that stopped at hours. Two screens, one boot, two answers:
        // About said "3d 1h 2m" and System said "3d 1h" for 262922 seconds,
        // the reading taken off the live machine.
        assert_eq!(crate::wire::about::format_uptime(262922), "3d 1h 2m");
    }
}
