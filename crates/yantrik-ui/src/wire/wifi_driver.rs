//! Keeps the first-run screens told about a Wi-Fi chip still waiting for its driver.
//!
//! `yantrik-broadcom-wifi` (a system service) fetches the Mac BCM4331's driver once the machine
//! is online and writes its progress to a status file; crate::wifi_note turns that into one line
//! for the installer's Welcome screen and the first-boot hardware scan (`onboard-hw-note`).
//! Polled every few seconds, because the state moves (waiting for a cable, installing, working)
//! while the person is looking at those screens, and stopped once it is settled.

use crate::app_context::AppContext;
use crate::wifi_note;
use crate::App;
use slint::ComponentHandle;

const POLL: std::time::Duration = std::time::Duration::from_secs(5);

pub fn wire(ui: &App, _ctx: &AppContext) {
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let mut last = String::new();
        loop {
            let status = std::fs::read_to_string(wifi_note::STATUS_FILE).unwrap_or_default();
            let note = wifi_note::note_from(&status);
            if note != last {
                last = note.clone();
                let weak = weak.clone();
                let alive = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = weak.upgrade() {
                        ui.set_onboard_hw_note(note.into());
                    }
                });
                if alive.is_err() {
                    return;
                }
            }
            if wifi_note::settled(&status) {
                return;
            }
            std::thread::sleep(POLL);
        }
    });
}
