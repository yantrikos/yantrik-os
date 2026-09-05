//! Services wire — the out-of-process services and their state, for the machine rail.
//!
//! Unlike the other wire modules this one is not called from `wire_all`: the service manager
//! is created in `main()` AFTER the callbacks are wired, and it is the thing being displayed.
//! `main` hands it over once the services have been started.

use std::time::Duration;

use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};
use yantrik_shell_core::service_manager::{ServiceManager, ServiceStatus};

use crate::{App, ServiceItem};

/// How often the rail re-reads service state. Services change state on the order of
/// seconds (a crash, a restart); anything faster is polling for its own sake.
const REFRESH: Duration = Duration::from_secs(5);

pub fn wire(ui: &App, mgr: ServiceManager) {
    push(ui, &mgr);

    let ui_weak = ui.as_weak();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, REFRESH, move || {
        if let Some(ui) = ui_weak.upgrade() {
            push(&ui, &mgr);
        }
    });
    std::mem::forget(timer);
}

fn push(ui: &App, mgr: &ServiceManager) {
    let mut items: Vec<ServiceItem> = mgr
        .list()
        .into_iter()
        .map(|s| {
            let (status, note): (&str, String) = match s.status {
                ServiceStatus::Running => ("running", "up".into()),
                ServiceStatus::Starting => ("starting", "starting".into()),
                ServiceStatus::Stopped => (
                    "stopped",
                    if s.autostart { "not running".into() } else { "on demand".into() },
                ),
                ServiceStatus::Failed(err) => ("failed", err),
            };
            ServiceItem {
                id: s.id.into(),
                status: status.into(),
                note: note.into(),
            }
        })
        .collect();
    // Stable order so a service does not jump rows when its state changes.
    items.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    ui.set_services(ModelRc::new(VecModel::from(items)));
}
