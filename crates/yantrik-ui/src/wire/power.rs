//! Power menu — wire power actions (lock, suspend, restart, shutdown), from the status bar's power
//! button and the launcher's footer alike.

use slint::ComponentHandle;

use crate::app_context::AppContext;
use crate::App;

pub fn wire(ui: &App, _ctx: &AppContext) {
    let ui_weak = ui.as_weak();
    ui.on_power_action(move |action| {
        let Some(ui) = ui_weak.upgrade() else { return };
        match action.as_str() {
            "lock" => {
                // The one lock path: the shell's screen and the compositor's session lock (#313).
                ui.invoke_lock_screen();
                tracing::info!("Screen locked via power menu");
            }
            "suspend" => {
                tracing::info!("Suspending via power menu");
                power("suspend");
            }
            // Restart and shut down go straight to the system. They used to post "Save my current
            // workspace" into the companion's chat as if the person had typed it, then hold the
            // whole desktop still for two seconds on the UI thread: a message the person never
            // wrote, in their own conversation, and a frozen screen, for a save nothing performed.
            "restart" => {
                tracing::info!("Restarting via power menu");
                power("reboot");
            }
            "shutdown" => {
                tracing::info!("Shutting down via power menu");
                power("poweroff");
            }
            _ => {
                tracing::warn!(action = action.as_str(), "Unknown power action");
            }
        }
    });
}

/// Suspend, restart or power off through logind, which lets the person at this machine's active
/// session do so without sudo. These used to be `sudo zzz` (an Alpine command that does not exist
/// on the Debian this OS ships, so Suspend did nothing), `sudo reboot` and `sudo poweroff`, which
/// needed the account to have passwordless sudo for everything (#397). Said in the log when it
/// fails, rather than dropped.
fn power(verb: &'static str) {
    std::thread::spawn(move || match std::process::Command::new("systemctl").arg(verb).output() {
        Ok(out) if out.status.success() => {}
        Ok(out) => tracing::warn!(
            verb,
            error = %String::from_utf8_lossy(&out.stderr).trim(),
            "systemctl refused the power action"
        ),
        Err(e) => tracing::warn!(verb, error = %e, "could not run systemctl"),
    });
}
