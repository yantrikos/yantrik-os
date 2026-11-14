//! The question "may this machine find where it is from its internet address?", and the switch
//! that holds the answer (#1171).
//!
//! An IP-based lookup tells a third party this machine's public address, so it is off until the
//! person says so. A feature that needs a place with none known (the weather tool, asked about
//! "here") asks with `allow_location_lookup`: this action, graded `sensitive`, whose sentence says
//! the address cannot be taken back, which is what makes every mode raise a card (`gate::decide`, the
//! same reading as `send_mail`) and plan mode refuse. Full bypass asks too: the action is in
//! `gate::always_asked`, because the switch it writes outlasts the hour.
//!
//! Unlike `send_mail`, the shell itself acts on the Allow. The caller does not write anything:
//! the app's dispatch runs this handler only with a grant the person's click created, and the
//! handler is the one place that writes `~/.config/yantrik/location-lookup`
//! (`location_consent::turn_on`, which the source-scan test keeps out of every other file). So
//! no tool, mind or model can set the switch; it is set by an Allow on this card, or by hand.
//!
//! `describe shell` shows where it stands under `settings.location_lookups`
//! ("Location lookups: on (since 2026-11-10 09:12)"), and the shell logs the moment it is turned on.

use yantrik_app_runtime::control::{Action, App as ControlSurface};
use yantrik_ipc_contracts::location_consent::{self, Verdict};

/// The action's name, as callers ask about it.
pub const ACTION: &str = "allow_location_lookup";

/// The grade it is published at: above `standard`, so a `tool_permission: standard` ceiling refuses it.
pub const GRADE: &str = "sensitive";

/// What a person reads on the card. "Cannot be undone" is the phrase `gate::unrecoverable` reads.
pub const PURPOSE: &str = "Allow this machine to find where it is from its internet address, from now on: \
    wttr.in (a weather service) when the weather is asked about with no city named, and ipinfo.io (a location \
    service) for this desktop's own place and timezone. Each sees your internet address, and an address once \
    sent cannot be undone. To turn it off again, delete ~/.config/yantrik/location-lookup.";

pub fn actions(surface: ControlSurface) -> ControlSurface {
    surface.action(Action::new(ACTION, PURPOSE).risk(GRADE), |_| turn_on())
}

/// Write the switch, unless the environment forbids lookups outright.
fn turn_on() -> Result<serde_json::Value, String> {
    if location_consent::env_forbids() {
        return Err("YANTRIK_NO_GEOLOCATE is set on this machine, so location lookups stay off whatever is answered here.".into());
    }
    location_consent::turn_on().map_err(|e| format!("The switch could not be written: {e}"))?;
    let now = location_consent::verdict();
    tracing::warn!("{}: a person allowed it on the shell's card", now.line());
    Ok(serde_json::json!({ "location_lookups": now.line() }))
}

/// The line `describe shell` carries, and the log when the file is there but not believed.
pub fn status() -> String {
    let verdict = location_consent::verdict();
    if let Verdict::Refused(why) = &verdict {
        tracing::warn!(reason = %why, "a location-lookup switch is present and was not believed");
    }
    verdict.line()
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use yantrik_ipc_transport::gate::{self, Authority, Mode as GateMode};

    use super::*;
    use crate::mind_mode::{Decision, Mode, Modes};

    /// The grade and sentence the surface publishes, read from the declaration itself.
    fn published() -> (String, String) {
        let production = include_str!("control_location.rs").split("#[cfg(test)]").next().unwrap();
        let declaration = production.split("Action::new(").nth(1).expect("the declaration");
        assert!(declaration.starts_with("ACTION, PURPOSE)"), "the action is not declared with ACTION and PURPOSE");
        assert!(declaration.contains(".risk(GRADE)"), "the action is not published at GRADE");
        (GRADE.to_string(), PURPOSE.to_string())
    }

    fn authority(mode: &str) -> Authority {
        Authority { ceiling: "dangerous".into(), mode: GateMode::named(mode), granted: false, asks_above: None }
    }

    #[test]
    fn it_is_sensitive_and_says_the_address_cannot_be_taken_back() {
        let (grade, said) = published();
        assert_eq!(grade, "sensitive");
        assert!(gate::unrecoverable(&said), "{said}");
        assert!(crate::approvals::shown_whole(&grade, &said), "the card shows it whole");
        assert!(said.contains("wttr.in") && said.contains("ipinfo.io") && said.contains("location-lookup"));
    }

    /// Without the person's Allow nothing runs: every mode asks, full bypass included, plan refuses,
    /// and the wire gate refuses an ungranted call in each of them.
    #[test]
    fn the_shell_asks_in_every_mode_full_bypass_included_and_never_runs_it_ungranted() {
        let (grade, said) = published();
        let cannot_be_undone = gate::unrecoverable(&said);
        let decide = |mode| Modes::new(mode).decide(&grade, "shell", ACTION, cannot_be_undone, "dangerous", Instant::now());
        for mode in [Mode::Ask, Mode::Auto, Mode::Bypass, Mode::BypassAll] {
            assert!(matches!(decide(mode), Decision::Ask), "{mode:?} did not ask");
        }
        assert!(matches!(decide(Mode::Plan), Decision::Refuse { .. }), "plan");
        for mode in ["ask", "auto", "bypass", "bypass_all", "plan"] {
            assert!(gate::decide(&authority(mode), "shell", ACTION, &grade, &said).is_err(), "{mode}");
        }
        let granted = Authority { granted: true, ..authority("auto") };
        assert!(gate::decide(&granted, "shell", ACTION, &grade, &said).is_ok(), "a person's Allow");
    }

    /// Only this file may write the switch.
    #[test]
    fn no_other_shell_code_writes_the_switch() {
        for (file, src) in [
            ("control.rs", include_str!("control.rs")),
            ("control_approvals.rs", include_str!("control_approvals.rs")),
            ("wire/location.rs", include_str!("wire/location.rs")),
            ("channels.rs", include_str!("channels.rs")),
        ] {
            assert!(!src.contains("location_consent::turn_on"), "{file} must not write the location switch");
        }
    }

    #[test]
    fn the_status_line_is_the_one_describe_carries() {
        let line = status();
        assert!(line.starts_with("Location lookups: "), "{line}");
        let section = include_str!("control_settings.rs").split("#[cfg(test)]").next().unwrap();
        assert!(section.contains("crate::control_location::status()"), "describe shell must carry it");
    }
}
