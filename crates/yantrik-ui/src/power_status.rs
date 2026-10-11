//! The battery and the power profile as the shell says them: in words for the bar and its
//! popover, and as data for `describe shell`.
//!
//! The words are composed here, not in Slint, so they are tested and so the bar's tooltip, the
//! popover and the control surface cannot drift apart. The rule for all of them is the one the
//! indicator follows: a time that nobody measured is left out, never filled in.

use slint::ComponentHandle;
use yantrik_os::{BatteryState, PowerProfileInfo, SystemSnapshot};

use crate::App;

/// The key the shell's `battery-state` property holds, back to the state.
pub(crate) fn state_from_key(key: &str) -> BatteryState {
    [BatteryState::Charging, BatteryState::Discharging, BatteryState::Full, BatteryState::PluggedNotCharging]
        .into_iter()
        .find(|s| s.as_str() == key)
        .unwrap_or(BatteryState::Unknown)
}

/// "2 h 15 min", "40 min", "3 h".
pub(crate) fn duration_text(mins: u32) -> String {
    match (mins / 60, mins % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// What the battery is doing, in words. "" when the source did not say: a wrong verb is worse
/// than none.
pub(crate) fn status_text(state: BatteryState) -> &'static str {
    match state {
        BatteryState::Charging => "Charging",
        BatteryState::Discharging => "Discharging",
        BatteryState::Full => "Fully charged",
        BatteryState::PluggedNotCharging => "Plugged in, not charging",
        BatteryState::Unknown => "",
    }
}

/// "2 h 15 min left" while it runs down, "40 min to full" while it charges, and "" otherwise,
/// or when there is no estimate.
pub(crate) fn time_text(state: BatteryState, to_empty: Option<u32>, to_full: Option<u32>) -> String {
    match (state, to_empty, to_full) {
        (BatteryState::Discharging, Some(m), _) if m > 0 => format!("{} left", duration_text(m)),
        (BatteryState::Charging, _, Some(m)) if m > 0 => format!("{} to full", duration_text(m)),
        _ => String::new(),
    }
}

/// The `battery` object of `describe shell`. `null` with no battery, so a desktop and a VM say
/// there is none rather than a battery at 0%. Times are in seconds, `null` when unknown; they
/// are held in whole minutes, so they are multiples of 60.
pub(crate) fn battery_for_describe(
    available: bool,
    level: i32,
    state: BatteryState,
    to_empty_mins: i32,
    to_full_mins: i32,
    profile: &str,
) -> serde_json::Value {
    if !available {
        return serde_json::Value::Null;
    }
    let secs = |m: i32| if m > 0 { serde_json::json!(m * 60) } else { serde_json::Value::Null };
    serde_json::json!({
        "percent": level,
        // True only while it is taking charge. A battery held at a charge limit is "plugged-not-
        // charging" in `state`, and not charging here.
        "charging": state == BatteryState::Charging,
        "state": state.as_str(),
        "time_to_empty_s": secs(to_empty_mins),
        "time_to_full_s": secs(to_full_mins),
        "power_profile": if profile.is_empty() { serde_json::Value::Null } else { profile.into() },
    })
}

/// Top-level `power_profile` of `describe shell`: whether or not there is a battery, since
/// desktops have profiles too. `null` when there is no daemon, and then `set_power_profile`
/// refuses.
pub(crate) fn profile_for_describe(profile: &str, performance_offered: bool) -> serde_json::Value {
    if profile.is_empty() {
        return serde_json::Value::Null;
    }
    let mut offered = vec!["power-saver", "balanced"];
    if performance_offered || profile == "performance" {
        offered.push("performance");
    }
    serde_json::json!({ "active": profile, "offered": offered, "set_with": "set_power_profile" })
}

/// Put the snapshot's battery on the bar.
pub(crate) fn apply_battery(ui: &App, snap: &SystemSnapshot) {
    let state = snap.battery_state;
    ui.set_battery_available(snap.battery_available);
    ui.set_battery_level(snap.battery_level as i32);
    ui.set_battery_charging(state == BatteryState::Charging);
    ui.set_battery_state(state.as_str().into());
    ui.set_battery_status_text(status_text(state).into());
    ui.set_battery_time_text(
        time_text(state, snap.battery_time_to_empty_mins, snap.battery_time_to_full_mins).into(),
    );
    let mins = |m: Option<u32>| m.map_or(-1, |m| m as i32);
    ui.set_battery_time_to_empty_mins(mins(snap.battery_time_to_empty_mins));
    ui.set_battery_time_to_full_mins(mins(snap.battery_time_to_full_mins));
}

/// Put what the daemon says on the shell: nothing when there is no daemon.
pub(crate) fn apply_profile(ui: &App, profile: Option<&PowerProfileInfo>) {
    match profile {
        Some(p) => {
            ui.set_power_profile(p.active.as_str().into());
            ui.set_power_performance_offered(p.offered.iter().any(|o| o == "performance"));
        }
        None => {
            ui.set_power_profile("".into());
            ui.set_power_performance_offered(false);
        }
    }
}

/// Show what the daemon says is in effect, from any thread.
pub(crate) fn apply_profile_later(weak: slint::Weak<App>, now: Option<yantrik_os::power_profile::PowerProfiles>) {
    let _ = weak.upgrade_in_event_loop(move |ui| {
        apply_profile(&ui, now.map(|p| PowerProfileInfo { active: p.active, offered: p.offered }).as_ref());
    });
}

/// The popover's choice. The daemon is asked off the UI thread (a bus call is milliseconds, but
/// not none) and the shell then shows what the daemon says is in effect, not what was asked.
pub(crate) fn wire(ui: &App) {
    let weak = ui.as_weak();
    ui.on_set_power_profile(move |profile| {
        let weak = weak.clone();
        let profile = profile.to_string();
        std::thread::spawn(move || {
            let answer = yantrik_os::power_profile::set(&profile);
            if let Err(why) = &answer {
                tracing::warn!(%profile, %why, "Power profile not changed");
            }
            // On failure, show what is really in effect, so a refused choice does not stay lit.
            let now = answer.ok().or_else(yantrik_os::power_profile::read);
            apply_profile_later(weak, now);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_as_a_person_says_them() {
        assert_eq!(duration_text(40), "40 min");
        assert_eq!(duration_text(135), "2 h 15 min");
        assert_eq!(duration_text(180), "3 h");
    }

    #[test]
    fn the_time_goes_with_the_direction_and_is_left_out_when_unknown() {
        use BatteryState::*;
        assert_eq!(time_text(Discharging, Some(135), None), "2 h 15 min left");
        assert_eq!(time_text(Charging, None, Some(40)), "40 min to full");
        assert_eq!(time_text(Discharging, None, None), "", "no estimate, no words");
        assert_eq!(time_text(Discharging, Some(0), None), "", "zero minutes is not an estimate");
        // A stale estimate from the other direction is not shown.
        assert_eq!(time_text(Charging, Some(135), None), "");
        assert_eq!(time_text(PluggedNotCharging, Some(135), Some(40)), "");
        assert_eq!(time_text(Full, Some(135), Some(40)), "");
    }

    #[test]
    fn the_four_states_have_four_phrases_and_unknown_has_none() {
        assert_eq!(status_text(BatteryState::Charging), "Charging");
        assert_eq!(status_text(BatteryState::Discharging), "Discharging");
        assert_eq!(status_text(BatteryState::Full), "Fully charged");
        assert_eq!(status_text(BatteryState::PluggedNotCharging), "Plugged in, not charging");
        assert_eq!(status_text(BatteryState::Unknown), "");
    }

    /// A desktop or a VM has no battery, and `describe` says so with null rather than 0%.
    #[test]
    fn describe_has_no_battery_object_without_a_battery() {
        let v = battery_for_describe(false, 0, BatteryState::Unknown, -1, -1, "balanced");
        assert!(v.is_null());
    }

    #[test]
    fn describe_carries_state_times_and_profile() {
        let v = battery_for_describe(true, 64, BatteryState::Discharging, 135, -1, "power-saver");
        assert_eq!(v["percent"], 64);
        assert_eq!(v["charging"], false);
        assert_eq!(v["state"], "discharging");
        assert_eq!(v["time_to_empty_s"], 8100);
        assert!(v["time_to_full_s"].is_null());
        assert_eq!(v["power_profile"], "power-saver");
    }

    /// The charge-limit case: plugged in, and `charging` must not claim otherwise.
    #[test]
    fn a_battery_held_at_its_limit_is_not_reported_as_charging() {
        let v = battery_for_describe(true, 80, BatteryState::PluggedNotCharging, -1, -1, "balanced");
        assert_eq!(v["charging"], false);
        assert_eq!(v["state"], "plugged-not-charging");
    }

    /// Desktops have profiles too: the top-level object stands whether or not there is a
    /// battery, and offers Performance only where the daemon does.
    #[test]
    fn the_profile_is_described_without_a_battery_and_performance_only_when_offered() {
        let v = profile_for_describe("balanced", false);
        assert_eq!(v["active"], "balanced");
        assert_eq!(v["offered"], serde_json::json!(["power-saver", "balanced"]));
        let v = profile_for_describe("balanced", true);
        assert_eq!(v["offered"], serde_json::json!(["power-saver", "balanced", "performance"]));
        assert!(profile_for_describe("", false).is_null(), "no daemon, no profile");
    }
}
