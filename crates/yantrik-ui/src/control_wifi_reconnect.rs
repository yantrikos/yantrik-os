//! The two buttons on the "Wi-Fi disconnected" notification, as actions on the shell's surface.
//!
//! A notification the shell sends as `Yantrik` has its buttons carried out on the shell's own
//! control surface (`wire::notifications::forward_to_app`), the way "See what it did" is
//! `show_mind_audit`. Without these two, the buttons would be controls that do nothing.
//!
//! * `reconnect_wifi` is the tile's "Tap to reconnect": rejoin the strongest saved network in
//!   range, and when there is none or the join fails, open Network Manager on its list. It is
//!   `standard` and not `sensitive` like `connect_wifi`, because it chooses no network: it only
//!   rejoins one this machine already has saved, which NetworkManager would rejoin by itself had
//!   it not stopped trying, and it does nothing at all while a network is joined, so it can never
//!   move the machine from one network to another. `standard` is also what runs unasked in `ask`
//!   mode, which is what a button the person just pressed needs.
//! * `choose_wifi_network` opens Network Manager, as `open_app network-manager` would. `standard`,
//!   like `open_app`.
//!
//! Both answer at once: the rejoin runs on a worker thread and its outcome shows in the network
//! state (`describe shell` network.state, network.ssid).

use slint::ComponentHandle;
use yantrik_app_runtime::control::{Action, App as ControlSurface};

use crate::App;

/// Refused on a screen the desktop is waiting at (lock, login, boot, first run): a window opened
/// there would be over the lock.
fn unlocked(ui: &App) -> Result<(), String> {
    if crate::control::locked_screen(ui.get_current_screen()) {
        return Err("the desktop is waiting for the person to sign in, so nothing was opened or joined".to_string());
    }
    Ok(())
}

pub fn actions(surface: ControlSurface, ui: &App) -> ControlSurface {
    let reconnect_weak = ui.as_weak();
    let choose_weak = ui.as_weak();
    surface
        .action(
            Action::new(
                "reconnect_wifi",
                "Rejoin Wi-Fi after a drop: the strongest network in range that this machine has \
                 already saved, so nothing has to be typed. Does nothing while a Wi-Fi network is joined, \
                 never switches the radio, and never joins a network that is not saved; when no \
                 saved network is in range or the join fails, it opens Network Manager on its \
                 list for the person. Answers at once; network.state in `describe shell` shows \
                 the outcome",
            )
            .risk("standard"),
            move |_args| {
                let ui = reconnect_weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
                unlocked(&ui)?;
                // It may open Network Manager, which must not cover an approval card.
                crate::card_watch::hold_windows("reconnect_wifi")?;
                let latest = yantrik_os::network::latest();
                let snapshot = latest.as_ref().ok_or("the network state is not known yet")?;
                if !snapshot.wifi_present {
                    return Err("this machine has no Wi-Fi device".to_string());
                }
                if !snapshot.radio_on {
                    return Err("Wi-Fi is switched off; turn it on first".to_string());
                }
                if snapshot.wifi_joined() {
                    return Ok(serde_json::json!({ "rejoining": false, "note": "Wi-Fi is already joined; nothing was done" }));
                }
                crate::wire::wifi_reconnect::reconnect_for_person(ui.as_weak());
                Ok(serde_json::json!({
                    "rejoining": snapshot.best_saved_in_range().map(|p| p.ssid.clone()),
                    "note": "the rejoin runs now and takes a few seconds; with no saved network in \
                             range, or if it fails, Network Manager opens on its list",
                }))
            },
        )
        .action(
            Action::new(
                "choose_wifi_network",
                "Open Network Manager, which opens on its Wi-Fi list when no cable carries the \
                 traffic, so the person can choose a network",
            )
            .risk("standard"),
            move |_args| {
                let ui = choose_weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
                unlocked(&ui)?;
                crate::card_watch::hold_windows("choose_wifi_network")?;
                crate::wire::wifi_reconnect::choose_network(ui.as_weak());
                Ok(serde_json::json!({ "opening": "network-manager" }))
            },
        )
}

#[cfg(test)]
mod tests {
    /// The notification's buttons name actions this surface really has, and the surface is on the
    /// shell: a button with no action behind it is a control that does nothing.
    #[test]
    fn the_drop_notices_buttons_are_actions_on_the_shells_surface() {
        use crate::features::network_watcher::{CHOOSE_ACTION, RECONNECT_ACTION};
        let control: String = include_str!("control.rs").split("#[cfg(test)]").next().unwrap().split_whitespace().collect();
        assert!(control.contains("crate::control_wifi_reconnect::actions(surface,ui)"));
        let here: String = include_str!("control_wifi_reconnect.rs").split_whitespace().collect();
        for name in [RECONNECT_ACTION, CHOOSE_ACTION] {
            assert!(here.contains(&format!("Action::new(\"{name}\",")), "`{name}` is not declared here");
        }
    }

    /// Neither button carries a secret, and neither can switch the radio off.
    #[test]
    fn neither_action_takes_an_argument_or_touches_the_radio() {
        let code = include_str!("control_wifi_reconnect.rs").split("#[cfg(test)]").next().unwrap();
        assert!(!code.contains("Param::"), "no arguments, so nothing a password could ride in");
        assert!(!code.contains("set_wifi_enabled"));
    }
}
