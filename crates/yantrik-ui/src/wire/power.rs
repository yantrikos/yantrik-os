//! Power menu — wire power actions (lock, suspend, restart, shutdown).

use slint::ComponentHandle;

use crate::app_context::AppContext;
use crate::App;

pub fn wire(ui: &App, _ctx: &AppContext) {
    let ui_weak = ui.as_weak();
    ui.on_power_action(move |action| {
        let Some(ui) = ui_weak.upgrade() else { return };
        match action.as_str() {
            "lock" => {
                ui.set_current_screen(3);
                ui.set_lock_error("".into());
                tracing::info!("Screen locked via power menu");
            }
            "suspend" => {
                tracing::info!("Suspending via power menu");
                let _ = std::process::Command::new("sudo")
                    .args(["zzz"])
                    .spawn();
            }
            "restart" => {
                tracing::info!("Restarting via power menu");
                let _ = std::process::Command::new("sudo")
                    .args(["reboot"])
                    .spawn();
            }
            "shutdown" => {
                tracing::info!("Shutting down via power menu");
                let _ = std::process::Command::new("sudo")
                    .args(["poweroff"])
                    .spawn();
            }
            _ => {
                tracing::warn!(action = action.as_str(), "Unknown power action");
            }
        }
    });
}
