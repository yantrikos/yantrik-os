//! #1353: in plain Bypass a command runs unasked, and "for this session" outlives the bypass.

use super::*;

const AGENT_RUN: Declared = Declared { cannot_be_undone: false, open_ended: true };
const GONE: Declared = Declared { cannot_be_undone: true, open_ended: false };
const MODES: [Mode; 5] = [Mode::Plan, Mode::Ask, Mode::Auto, Mode::Bypass, Mode::BypassAll];

fn in_mode(mode: Mode, now: Instant) -> Modes {
    let mut modes = Modes::new(Mode::Ask);
    modes.person_set_mode(mode, Bypass::Hour, now, 0);
    modes
}

/// Each mode × a command, an unrecoverable action and an always-asked one.
#[test]
fn mind_mode_bypass_runs_commands_unasked_and_still_asks_the_rest() {
    let now = Instant::now();
    for mode in MODES {
        let modes = in_mode(mode, now);
        let command = modes.decide("sensitive", "shell", "agent_run", AGENT_RUN, "dangerous", now);
        let gone = modes.decide("sensitive", "calendar", "delete_event", GONE, "dangerous", now);
        let (app, action) = yantrik_ipc_transport::always_asked::ALWAYS_ASKED[0];
        let always = modes.decide("sensitive", app, action, Declared::default(), "dangerous", now);
        match mode {
            Mode::Plan => assert!(matches!(command, Decision::Refuse { .. }), "plan"),
            Mode::Ask | Mode::Auto => assert_eq!(command, Decision::Ask, "{mode:?} asks before a command"),
            Mode::Bypass | Mode::BypassAll => {
                assert_eq!(command, Decision::Run { unasked: true }, "{mode:?} runs it, and it is audited")
            }
        }
        match mode {
            Mode::Plan => {}
            Mode::BypassAll => assert_eq!(gone, Decision::Run { unasked: true }),
            _ => assert_eq!(gone, Decision::Ask, "{mode:?} asks before what cannot be undone"),
        }
        if mode != Mode::Plan {
            assert_eq!(always, Decision::Ask, "{mode:?}: an always-asked action asks in every mode");
        }
    }
}

/// The session rule given in auto stays when a bypass runs out back to auto; it goes when the
/// mode is lowered to ask or plan.
#[test]
fn mind_mode_a_session_rule_survives_a_bypass_expiring_back_to_auto() {
    let now = Instant::now();
    let mut modes = in_mode(Mode::Auto, now);
    modes.person_add_rule_at("shell", "agent_run", "sensitive", "What it runs can do anything you can.", now).unwrap();
    modes.person_set_mode(Mode::Bypass, Bypass::Minutes15, now, 0);
    let after = now + Duration::from_secs(15 * 60 + 1);
    assert!(modes.lapse(after));
    assert_eq!(modes.mode(after), Mode::Auto);
    assert_eq!(modes.rules().len(), 1, "the rule outlived the bypass");
    assert_eq!(modes.decide("sensitive", "shell", "agent_run", AGENT_RUN, "dangerous", after), Decision::Run { unasked: true });
    modes.person_set_mode(Mode::Ask, Bypass::Hour, after, 0);
    assert!(modes.rules().is_empty(), "lowered to ask: the rule goes");
    // And a bypass that runs out back to ask clears them too.
    let mut from_ask = in_mode(Mode::Ask, now);
    from_ask.person_add_rule_at("shell", "agent_run", "sensitive", "x", now).unwrap();
    from_ask.person_set_mode(Mode::Bypass, Bypass::Minutes15, now, 0);
    assert!(from_ask.lapse(after));
    assert!(from_ask.rules().is_empty());
}
