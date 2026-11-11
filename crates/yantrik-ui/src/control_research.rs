//! The question "may the companion search the web on its own?", and the switch that holds the
//! answer (#1171).
//!
//! An instinct that searches sends terms drawn from the person's interests and place to a search
//! service while nobody is at the keyboard, so `background_research` in the machine's config is
//! off until the person says so. The companion's first wish to search with the person at the desk
//! raises this action as an approval card (`yantrik_companion_tools::research_offer`), the same
//! mechanism as `allow_location_lookup` (`control_location.rs`): graded `sensitive`, with a
//! sentence that says what is sent, to whom, and that it cannot be undone, so every mode raises
//! the card and none (full bypass included, `gate::always_asked`) runs it unasked.
//!
//! The handler is the one place that turns the switch on: it writes `background_research: true`
//! into the config file and tells the running companion. The dispatch runs it only with a grant
//! the person's click created, and the source-scan test keeps `config_file::write_background_research`
//! and `background_research::set_on` out of every other file in the shell. The switch can still be
//! set by hand in `config.yaml`.
//!
//! The way back needs no card, because it lets less out: `turn_off_background_research` (and the
//! Settings button, `wire::research`) writes `false`. A No said to the card is kept in a file
//! (`background_research::decline`), so the card does not come back after a restart;
//! `ask_about_background_research_again` forgets it, and Settings' Allow button forgets it and
//! raises the card, which is the person asking to be asked.
//!
//! `describe shell` shows where it stands under `settings.background_research`.

use yantrik_app_runtime::control::{Action, App as ControlSurface};
use yantrik_companion::background_research;
use yantrik_companion::config_file;

/// The action's name, as the offer asks for it.
pub const ACTION: &str = "allow_background_research";

/// Turning it off, and being asked again after a No.
pub const OFF_ACTION: &str = "turn_off_background_research";
pub const ASK_AGAIN_ACTION: &str = "ask_about_background_research_again";

/// The grade it is published at: above `standard`, so a `tool_permission: standard` ceiling refuses it.
pub const GRADE: &str = "sensitive";

pub fn actions(surface: ControlSurface) -> ControlSurface {
    surface
        .action(Action::new(ACTION, background_research::OFFER).risk(GRADE), |_| allow())
        .action(
            Action::new(
                OFF_ACTION,
                "Turn background research off: the companion stops searching the web on its own. It sends nothing and asks nothing.",
            )
            .risk("standard"),
            |_| turn_off(),
        )
        .action(
            Action::new(
                ASK_AGAIN_ACTION,
                "Forget a No said to the background-research card, so the card may come back the next time the companion wants to                  search. It does not turn research on, sends nothing and asks nothing.",
            )
            .risk("standard"),
            |_| ask_again().map(|()| serde_json::json!({ "background_research": status() })),
        )
}

/// Turn research on: in the config file, then in the running companion.
fn allow() -> Result<serde_json::Value, String> {
    let path = config_file::machine_config_path(background_research::config_path().as_deref());
    let saved = turn_on_at(&path);
    Ok(serde_json::json!({
        "background_research": status(),
        "saved": saved.is_ok(),
        "note": match saved {
            Ok(()) => "It is on from now on.".to_string(),
            Err(why) => format!("It is on for this session, but it could not be saved ({why}), so after a restart it is off again."),
        },
    }))
}

/// Write the config line, then the running switch. The person has allowed it, so the running
/// switch goes on even where the file cannot be written.
fn turn_on_at(path: &std::path::Path) -> Result<(), String> {
    let saved = config_file::write_background_research(path, true).map_err(|e| e.to_string());
    background_research::set_on();
    // An Allow answers an earlier No.
    let _ = background_research::forgive();
    tracing::warn!(saved = saved.is_ok(), "Background research: a person allowed it on the shell's card");
    saved
}

/// Turn research off, in the config file and in the running companion.
pub fn turn_off() -> Result<serde_json::Value, String> {
    let path = config_file::machine_config_path(background_research::config_path().as_deref());
    let saved = config_file::write_background_research(&path, false).map_err(|e| e.to_string());
    background_research::set_off();
    tracing::warn!(saved = saved.is_ok(), "Background research: a person turned it off");
    Ok(serde_json::json!({
        "background_research": status(),
        "saved": saved.is_ok(),
        "note": match saved {
            Ok(()) => "It is off from now on.".to_string(),
            Err(why) => format!("It is off for this session, but that could not be saved ({why}), so after a restart the config's setting applies."),
        },
    }))
}

/// Forget a No, so the offer may be raised again.
pub fn ask_again() -> Result<(), String> {
    background_research::forgive().map_err(|e| format!("The No could not be forgotten: {e}"))
}

/// The line `describe shell` carries.
pub fn status() -> String {
    if background_research::is_on() { "Background research: on".into() } else { "Background research: off".into() }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use yantrik_ipc_transport::gate::{self, Authority, Mode as GateMode};

    use super::*;
    use crate::mind_mode::{Decision, Mode, Modes};

    /// The switch is process-wide: the tests that set it take turns.
    static SWITCH: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn authority(mode: &str) -> Authority {
        Authority { ceiling: "dangerous".into(), mode: GateMode::named(mode), granted: false, asks_above: None }
    }

    /// Every mode but plan asks, full bypass included; plan refuses; the wire gate refuses an
    /// ungranted call in each, and a person's Allow lets it through.
    #[test]
    fn the_shell_asks_in_every_mode_and_never_runs_it_ungranted() {
        let said = background_research::OFFER;
        let cannot_be_undone = gate::unrecoverable(said);
        assert!(cannot_be_undone, "the card says it cannot be undone");
        let decide = |mode| Modes::new(mode).decide(GRADE, "shell", ACTION, cannot_be_undone, "dangerous", Instant::now());
        for mode in [Mode::Ask, Mode::Auto, Mode::Bypass, Mode::BypassAll] {
            assert!(matches!(decide(mode), Decision::Ask), "{mode:?} did not ask");
        }
        assert!(matches!(decide(Mode::Plan), Decision::Refuse { .. }), "plan");
        for mode in ["ask", "auto", "bypass", "bypass_all", "plan"] {
            assert!(gate::decide(&authority(mode), "shell", ACTION, GRADE, said).is_err(), "{mode}");
        }
        let granted = Authority { granted: true, ..authority("auto") };
        assert!(gate::decide(&granted, "shell", ACTION, GRADE, said).is_ok(), "a person's Allow");
        assert!(crate::approvals::shown_whole(GRADE, said), "the card shows it whole");
    }

    /// Full bypass asks about all three consents: the two switches and the public report. The
    /// shell's table and the wire gate agree, and a session rule does not answer them.
    #[test]
    fn full_bypass_asks_for_all_three_and_a_session_rule_does_not_answer_them() {
        use yantrik_ipc_transport::always_asked::ALWAYS_ASKED;
        assert_eq!(ALWAYS_ASKED.len(), 3);
        let now = Instant::now();
        for (app, action) in ALWAYS_ASKED {
            let mut modes = Modes::new(Mode::Ask);
            modes.person_set_mode(Mode::BypassAll, crate::mind_mode::Bypass::Hour, now, 0);
            modes.person_add_rule_at(app, action, "sensitive", "Do it.", now).ok();
            let decided = modes.decide("sensitive", app, action, crate::mind_mode::Declared::default(), "dangerous", now);
            assert_eq!(decided, Decision::Ask, "{app}.{action} under Mode::BypassAll");
            assert!(gate::decide(&authority("bypass_all"), app, action, "sensitive", "Do it.").is_err(), "{action} on the wire gate");
        }
        let mut full = Modes::new(Mode::Ask);
        full.person_set_mode(Mode::BypassAll, crate::mind_mode::Bypass::Hour, now, 0);
        let other = full.decide("sensitive", "shell", "set_theme", crate::mind_mode::Declared::default(), "dangerous", now);
        assert!(matches!(other, Decision::Run { .. }), "every other action still runs in full bypass");
    }

    #[test]
    fn the_declaration_is_this_actions_and_the_card_names_what_is_sent_and_to_whom() {
        let production = include_str!("control_research.rs").split("#[cfg(test)]").next().unwrap();
        assert!(production.contains("Action::new(ACTION, background_research::OFFER).risk(GRADE)"));
        for word in ["search terms", "SearXNG", "DuckDuckGo", "internet address"] {
            assert!(background_research::OFFER.contains(word), "{word}");
        }
    }

    /// Only this file may turn it on: not the other control surfaces, the wire code, the channels
    /// or the tools. The card is the only thing that writes the switch.
    #[test]
    fn no_other_shell_code_writes_the_switch() {
        for (file, src) in [
            ("control.rs", include_str!("control.rs")),
            ("control_approvals.rs", include_str!("control_approvals.rs")),
            ("control_location.rs", include_str!("control_location.rs")),
            ("wire/location.rs", include_str!("wire/location.rs")),
            ("channels.rs", include_str!("channels.rs")),
            ("bridge.rs", include_str!("bridge.rs")),
            ("main.rs", include_str!("main.rs")),
            ("wire/research.rs", include_str!("wire/research.rs")),
        ] {
            for writer in ["write_background_research", "background_research::set_on", "set_on()", "background_research::set_off", "background_research::forgive"] {
                assert!(!src.contains(writer), "{file} must not write the research switch ({writer})");
            }
        }
    }

    #[test]
    fn allowing_writes_the_config_line_and_turns_the_running_switch_on() {
        let _turn = SWITCH.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("yantrik-research-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.yaml");
        std::fs::write(&path, "user_name: \"Ada\"\nbackground_research: false\n").unwrap();
        background_research::init(false, Some(path.clone()));
        assert_eq!(status(), "Background research: off");
        let answer = allow().unwrap();
        assert_eq!(answer["saved"], true);
        assert_eq!(status(), "Background research: on");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "user_name: \"Ada\"\nbackground_research: true\n");
        // A file that cannot be written: on for now, and the answer says it was not saved.
        background_research::init(false, Some(dir.join("missing/config.yaml")));
        let answer = allow().unwrap();
        assert_eq!(answer["saved"], false);
        assert_eq!(status(), "Background research: on");
        background_research::init(false, None);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The way back: off needs no card and writes `false`; a No is forgotten only on request.
    #[test]
    fn the_person_can_turn_it_off_and_ask_to_be_asked_again() {
        let _turn = SWITCH.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("yantrik-research-off-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.yaml");
        std::fs::write(&path, "user_name: \"Ada\"
background_research: true
").unwrap();
        background_research::init(true, Some(path.clone()));
        assert_eq!(status(), "Background research: on");
        let answer = turn_off().unwrap();
        assert_eq!(answer["saved"], true);
        assert_eq!(status(), "Background research: off");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "user_name: \"Ada\"
background_research: false
");
        background_research::init(false, None);
        let _ = std::fs::remove_dir_all(dir);
        let production = include_str!("control_research.rs").split("#[cfg(test)]").next().unwrap();
        assert!(production.contains(".risk(\"standard\")") && production.contains("OFF_ACTION") && production.contains("ASK_AGAIN_ACTION"));
        assert!(!production.contains("OFF_ACTION, background_research::OFFER"), "off asks nothing: no card");
    }

    /// The worker marks a turn as the person's only when the prompt is not one the machine sent
    /// itself, and before the model runs: everything else (the think timer's EXECUTE urges,
    /// queued tasks, a program on the socket) is unattended and never reaches the browser.
    #[test]
    fn only_a_turn_the_person_started_is_marked_as_theirs() {
        let src = include_str!("bridge.rs");
        let marker = "(!is_system_generated).then(yantrik_companion::background_research::person_turn)";
        assert_eq!(src.matches("background_research::person_turn").count(), 1, "one place marks a turn");
        let defined = src.find("let is_system_generated = outside").expect("the worker decides who started a turn");
        let marked = src.find(marker).expect("the person's turn is marked");
        let answered = src.find("companion.handle_message_streaming(&text").expect("the turn the model answers");
        assert!(defined < marked && marked < answered, "marked after it is known who started it, before the model runs");
    }

    #[test]
    fn describe_carries_the_status_line() {
        let section = include_str!("control_settings.rs").split("#[cfg(test)]").next().unwrap();
        assert!(section.contains("crate::control_research::status()"));
    }
}
