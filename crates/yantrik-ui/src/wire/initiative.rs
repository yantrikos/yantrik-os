//! Settings → Minds → Initiative on screen (design/initiative-inbox-2026-10-16.md, "Controls").
//!
//! The settings live in the notifications store, beside the cards they govern, because the store
//! makes the grant decision and must read what the person set rather than what a screen last drew.
//! Each change is a call to `notifications.initiative_set`, which only the desktop may make; what
//! the store answers is drawn, and the shell's poll carries it to every attached mind within a
//! second (`notice_desk`).

use std::cell::RefCell;

use slint::{ComponentHandle, ModelRc, VecModel};
use yantrik_ipc_contracts::notifications::notice::{Family, Initiative, QuietHours};
use yantrik_ipc_contracts::notifications::INITIATIVE_SET;
use yantrik_ipc_transport::SyncRpcClient;

use crate::{App, FamilyRowData, InitiativeState};

/// The quiet hours Settings cycles through, `None` first: off.
pub const QUIET_HOURS: &[Option<QuietHours>] = &[
    None,
    Some(QuietHours { start: 22, end: 7 }),
    Some(QuietHours { start: 23, end: 7 }),
    Some(QuietHours { start: 21, end: 6 }),
];

/// What each row says about its family, in the person's words.
fn description(family: &Family) -> &'static str {
    match family {
        Family::Reminder => "Reminders you asked a mind for. Never off: you asked for them.",
        Family::Delegated => "The results of jobs you handed to a mind. Never off: you asked for them.",
        Family::Machine => "Something about this machine a mind thinks you should know.",
        Family::Calendar => "Your events, before they start.",
        Family::Discovery => "Something a mind came across that it thinks you would want.",
        Family::StartOfDay => "A short note as your day begins.",
        Family::Other(_) => "",
    }
}

thread_local! {
    /// What the screen last drew, so a second that changed nothing rebuilds nothing.
    static DRAWN: RefCell<Option<Initiative>> = const { RefCell::new(None) };
}

/// The rows the screen draws for these settings.
pub fn rows(settings: &Initiative) -> Vec<FamilyRowData> {
    Family::ALL
        .iter()
        .map(|family| FamilyRowData {
            family: family.as_str().into(),
            label: family.label().into(),
            description: description(family).into(),
            setting: settings.setting(family).as_str().into(),
            can_be_off: !family.is_obligation(),
        })
        .collect()
}

/// The label of the quiet-hours button.
pub fn quiet_hours_label(hours: Option<QuietHours>) -> String {
    hours.map_or_else(|| "Off".to_string(), |h| h.label())
}

/// The choice after `current` in the cycle, back to off after the last.
pub fn next_quiet_hours(current: Option<QuietHours>) -> Option<QuietHours> {
    let at = QUIET_HOURS.iter().position(|q| *q == current);
    at.map_or(QUIET_HOURS[1], |i| QUIET_HOURS[(i + 1) % QUIET_HOURS.len()])
}

pub fn wire(ui: &App) {
    let state = ui.global::<InitiativeState>();
    let weak = ui.as_weak();
    state.on_set_quiet(move |on| change(&weak, serde_json::json!({ "quiet": on })));
    let weak = ui.as_weak();
    state.on_set_family(move |family, setting| {
        change(&weak, serde_json::json!({ "family": family.as_str(), "setting": setting.as_str() }))
    });
    let weak = ui.as_weak();
    state.on_next_quiet_hours(move || {
        let next = next_quiet_hours(crate::notice_desk::initiative().quiet_hours);
        change(&weak, serde_json::json!({ "quiet_hours": next }));
    });
    refresh(ui);
}

/// Draw the settings as the store last said them.
pub fn refresh(ui: &App) {
    let settings = crate::notice_desk::initiative();
    let unchanged = DRAWN.with(|d| d.borrow().as_ref() == Some(&settings));
    if unchanged {
        return;
    }
    let state = ui.global::<InitiativeState>();
    state.set_quiet(settings.quiet);
    state.set_quiet_hours(quiet_hours_label(settings.quiet_hours).into());
    state.set_families(ModelRc::new(VecModel::from(rows(&settings))));
    DRAWN.with(|d| *d.borrow_mut() = Some(settings));
}

/// Ask the store for one change, off the UI thread, and draw what it answers.
fn change(weak: &slint::Weak<App>, params: serde_json::Value) {
    let weak = weak.clone();
    let _ = std::thread::Builder::new().name("yos-initiative".into()).spawn(move || {
        let answer = yantrik_app_runtime::service::ensure("notifications")
            .map_err(|e| e.to_string())
            .and_then(|_| {
                SyncRpcClient::for_service("notifications")
                    .with_timeout(std::time::Duration::from_secs(3))
                    .call(INITIATIVE_SET, params)
                    .map_err(|e| e.message)
            })
            .and_then(|v| serde_json::from_value::<Initiative>(v).map_err(|e| e.to_string()));
        let _ = weak.upgrade_in_event_loop(move |ui| {
            let state = ui.global::<InitiativeState>();
            match answer {
                Ok(settings) => {
                    crate::notice_desk::set_initiative(settings);
                    state.set_notice("".into());
                    refresh(&ui);
                }
                Err(why) => state.set_notice(format!("Not changed: {why}").into()),
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_ipc_contracts::notifications::notice::FamilySetting;

    #[test]
    fn every_family_has_a_row_and_obligations_offer_no_off() {
        let mut settings = Initiative::default();
        settings.set(&Family::Discovery, FamilySetting::Off).unwrap();
        let rows = rows(&settings);
        assert_eq!(rows.len(), Family::ALL.len());
        for row in &rows {
            let obligation = matches!(row.family.as_str(), "reminder" | "delegated");
            assert_eq!(row.can_be_off, !obligation, "{}", row.family);
            assert!(!row.description.is_empty(), "{}", row.family);
        }
        assert_eq!(rows.iter().find(|r| r.family == "discovery").unwrap().setting, "off");
    }

    #[test]
    fn quiet_hours_cycle_through_the_offered_windows_and_back_to_off() {
        let mut at = None;
        let mut seen = vec![quiet_hours_label(at)];
        for _ in 0..QUIET_HOURS.len() {
            at = next_quiet_hours(at);
            seen.push(quiet_hours_label(at));
        }
        assert_eq!(seen, ["Off", "22:00–07:00", "23:00–07:00", "21:00–06:00", "Off"]);
        // A window set some other way steps to the first offered one.
        assert_eq!(next_quiet_hours(Some(QuietHours { start: 1, end: 2 })), QUIET_HOURS[1]);
    }
}
