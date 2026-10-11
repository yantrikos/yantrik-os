//! Power menu — wire power actions (lock, log out, suspend, hibernate, restart, shut down), from
//! the bar's power popover and the launcher's footer alike.
//!
//! The confirmation for restart and shut down lives in the popover (`components/power_menu.slint`):
//! by the time `power-action` arrives here, a person has pressed the red button. A mind reaches
//! the same acts through `power_sleep` and `power_off` (`control_power`), which are graded and
//! refuse while minds are working.

use slint::ComponentHandle;

use crate::app_context::AppContext;
use crate::control_power::{self, Verb};
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
            // Restart and shut down go straight to the system. They used to post "Save my current
            // workspace" into the companion's chat as if the person had typed it, then hold the
            // whole desktop still for two seconds on the UI thread: a message the person never
            // wrote, in their own conversation, and a frozen screen, for a save nothing performed.
            other => match Verb::parse(other) {
                Some(verb) => {
                    tracing::info!(action = other, "Power action from the popover");
                    request(verb);
                }
                None => tracing::warn!(action = other, "Unknown power action"),
            },
        }
    });

    // The popover opens, or starts a confirmation: say how many minds are working now, and ask
    // logind whether it would hibernate. The count is a store read; the bus call is not, so it
    // waits on a worker and the row appears when logind has answered.
    let ui_weak = ui.as_weak();
    ui.on_power_refresh(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        ui.set_power_minds_working(control_power::minds_working() as i32);
        refresh_hibernate(ui.as_weak());
    });
    refresh_hibernate(ui.as_weak());
}

/// Ask logind off the UI thread, put the answer on the screen.
fn refresh_hibernate(weak: slint::Weak<App>) {
    std::thread::spawn(move || {
        let yes = yantrik_os::login1::can_hibernate();
        let _ = weak.upgrade_in_event_loop(move |ui| ui.set_power_can_hibernate(yes));
    });
}

/// Do it on a worker, and say in the log when logind refuses rather than dropping it. These used
/// to be `sudo zzz` (an Alpine command that does not exist on the Debian this OS ships, so
/// Suspend did nothing), `sudo reboot` and `sudo poweroff`, which needed the account to have
/// passwordless sudo for everything (#397).
fn request(verb: Verb) {
    std::thread::spawn(move || {
        if let Err(why) = control_power::run(verb) {
            tracing::warn!(?verb, error = %why, "the power action was refused");
        }
    });
}
