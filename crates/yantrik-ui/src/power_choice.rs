//! Choosing a power mode: the one way in, for the person's tap and for a caller's
//! `set_power_profile` alike.
//!
//! The daemon answers later (a D-Bus call with a two-second timeout), so the choice is shown
//! pending — "Switching…", no second press taken — from the moment it is asked until the daemon
//! answers, and then the tile shows what the daemon reads back, never what was asked (toggle_truth).
//! The control action used to call the daemon on its own and skip the pending state the tap set,
//! so a caller's choice was drawn as nothing happening until it suddenly had.

use yantrik_os::power_profile::PowerProfiles;
use yantrik_os::PowerProfileInfo;

use crate::toggle_truth::Settled;
use crate::App;

/// The part of the shell a power choice draws on: the pending mark and the profile shown.
/// [`App`] is the real one; the tests use a fake to watch the whole path.
pub(crate) trait PowerTile {
    fn pending(&self) -> String;
    fn set_pending(&self, profile: &str);
    fn show(&self, now: Option<&PowerProfileInfo>);
}

impl PowerTile for App {
    fn pending(&self) -> String {
        self.get_power_profile_pending().to_string()
    }
    fn set_pending(&self, profile: &str) {
        self.set_power_profile_pending(profile.into());
    }
    fn show(&self, now: Option<&PowerProfileInfo>) {
        crate::power_status::apply_profile(self, now);
    }
}

/// On the UI thread: mark `profile` pending, or refuse while another choice is still waiting on
/// the daemon — the tile takes no press then, and a caller is told the same.
pub(crate) fn begin(tile: &impl PowerTile, profile: &str) -> Result<(), String> {
    let waiting = tile.pending();
    if !waiting.is_empty() {
        return Err(format!("the power mode is still switching to `{waiting}`"));
    }
    tile.set_pending(profile);
    Ok(())
}

/// On the UI thread, once the daemon has answered: clear the pending mark, show what the daemon
/// reads as in effect, and say why when that is not what was asked.
pub(crate) fn finish(tile: &impl PowerTile, out: Settled<Option<PowerProfiles>>, say: impl FnOnce(&str, &str)) {
    tile.set_pending("");
    let shown = out.shown.map(|p| PowerProfileInfo { active: p.active, offered: p.offered });
    tile.show(shown.as_ref());
    if let Some(why) = &out.refused {
        say("Power mode", why);
    }
}

/// Off the UI thread: ask the daemon through `set`, hand what to show to `settle` (which takes it
/// back to the UI thread), and answer with the daemon's own result.
pub(crate) fn ask(
    profile: &str,
    set: impl FnOnce(&str) -> Result<PowerProfiles, String>,
    read: impl FnOnce() -> Option<PowerProfiles>,
    settle: impl FnOnce(Settled<Option<PowerProfiles>>),
) -> Result<PowerProfiles, String> {
    let answer = set(profile);
    settle(crate::power_status::settled(profile, answer.clone(), read));
    answer
}

/// The tap's and the control surface's choice. Marks it pending now, on the UI thread, and returns
/// the bus work for the caller to run off it; that work clears the pending mark whatever the
/// daemon says. Refused, with nothing marked, while another choice is pending.
pub(crate) fn choose(
    ui: &App,
    profile: &str,
) -> Result<impl FnOnce() -> Result<PowerProfiles, String> + Send + 'static, String> {
    use slint::ComponentHandle;
    begin(ui, profile)?;
    let weak = ui.as_weak();
    let profile = profile.to_string();
    Ok(move || {
        ask(&profile, yantrik_os::power_profile::set, yantrik_os::power_profile::read, move |out| {
            let _ = weak.upgrade_in_event_loop(move |ui| finish(&ui, out, crate::toggle_truth::say_refused));
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Tile {
        pending: RefCell<String>,
        shown: RefCell<Option<String>>,
    }

    impl PowerTile for Tile {
        fn pending(&self) -> String {
            self.pending.borrow().clone()
        }
        fn set_pending(&self, profile: &str) {
            *self.pending.borrow_mut() = profile.into();
        }
        fn show(&self, now: Option<&PowerProfileInfo>) {
            *self.shown.borrow_mut() = now.map(|p| p.active.clone());
        }
    }

    fn profiles(active: &str) -> PowerProfiles {
        PowerProfiles { active: active.into(), offered: ["power-saver", "balanced"].map(String::from).to_vec() }
    }

    /// The whole path but the thread hop, with a daemon that refuses: pending while it is asked,
    /// then the profile still in effect, the pending mark gone, the reason said, and the caller
    /// told it failed.
    #[test]
    fn a_refused_choice_is_pending_then_shows_what_is_in_effect_and_says_why() {
        let tile = Tile { shown: RefCell::new(Some("balanced".into())), ..Default::default() };
        begin(&tile, "power-saver").unwrap();
        assert_eq!(tile.pending(), "power-saver", "the tile says Switching… before the daemon is asked");
        assert!(begin(&tile, "balanced").is_err(), "a second choice is refused while one is pending");

        let said = RefCell::new(Vec::new());
        let answer = ask(
            "power-saver",
            |_| {
                assert_eq!(tile.pending(), "power-saver", "still pending while the daemon is asked");
                Err("the daemon did not take it".into())
            },
            || Some(profiles("balanced")),
            |out| finish(&tile, out, |what, why| said.borrow_mut().push(format!("{what}: {why}"))),
        );
        assert!(answer.is_err(), "the caller is not told it worked");
        assert_eq!(tile.pending(), "", "pending is cleared on a refusal");
        assert_eq!(tile.shown.borrow().as_deref(), Some("balanced"), "the profile still in effect is shown");
        assert_eq!(*said.borrow(), ["Power mode: the daemon did not take it"]);
    }

    #[test]
    fn a_taken_choice_clears_pending_and_says_nothing() {
        let tile = Tile::default();
        begin(&tile, "power-saver").unwrap();
        let answer = ask("power-saver", |_| Ok(profiles("power-saver")), || panic!("not read"), |out| {
            finish(&tile, out, |what, why| panic!("{what} said {why} on success"))
        });
        assert_eq!(answer.unwrap().active, "power-saver");
        assert_eq!(tile.pending(), "");
        assert_eq!(tile.shown.borrow().as_deref(), Some("power-saver"));
    }

    /// The tap and `set_power_profile` both go through [`choose`], and neither asks the daemon on
    /// its own: there is one place that sets and clears pending.
    #[test]
    fn the_tap_and_the_control_action_share_one_way_in() {
        let tap = include_str!("power_status.rs").split("#[cfg(test)]").next().unwrap();
        assert!(tap.contains("crate::power_choice::choose("), "the tap does not go through choose");
        assert!(!tap.contains("power_profile::set("), "the tap asks the daemon on its own");
        let control = include_str!("control.rs").split("#[cfg(test)]").next().unwrap();
        assert!(control.contains("crate::power_choice::choose("), "set_power_profile does not go through choose");
        assert!(!control.contains("power_profile::set("), "set_power_profile asks the daemon on its own");
    }
}
