//! Power menu — wire power actions (lock, suspend, restart, shutdown).
//! Auto-saves workspace before destructive actions (restart, shutdown).

use slint::ComponentHandle;

use crate::app_context::AppContext;
use crate::App;

pub fn wire(ui: &App, ctx: &AppContext) {
    let ui_weak = ui.as_weak();
    let bridge = ctx.bridge.clone();
    ui.on_power_action(move |action| {
        let Some(ui) = ui_weak.upgrade() else { return };
        match action.as_str() {
            "lock" => {
                // Through the one way in, so the vault's key goes with it: this path used to set
                // the screen and leave the key in memory.
                crate::lock::engage(&ui, crate::lock::LOCK_SCREEN);
                tracing::info!("Screen locked via power menu");
            }
            "suspend" => {
                tracing::info!("Suspending via power menu");
                let _ = std::process::Command::new("sudo")
                    .args(["zzz"])
                    .spawn();
            }
            "restart" => {
                // Auto-save workspace before restart
                tracing::info!("Auto-saving workspace before restart");
                bridge.send_message(
                    "Save my current workspace — I'm restarting.".to_string(),
                );
                std::thread::sleep(std::time::Duration::from_secs(2));
                tracing::info!("Restarting via power menu");
                let _ = std::process::Command::new("sudo")
                    .args(["reboot"])
                    .spawn();
            }
            "shutdown" => {
                // Auto-save workspace before shutdown
                tracing::info!("Auto-saving workspace before shutdown");
                bridge.send_message(
                    "Save my current workspace — I'm shutting down.".to_string(),
                );
                std::thread::sleep(std::time::Duration::from_secs(2));
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
