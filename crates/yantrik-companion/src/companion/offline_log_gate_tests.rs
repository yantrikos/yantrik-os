//! The tests of the offline log gate, which logs the language model's reachability once per state
//! change. Separate from companion.rs, which is over the file-size limit, so that file does not
//! grow.

use super::OfflineLogGate;

#[test]
fn a_dead_backend_logs_once_per_state_change_not_every_call() {
    // #30: a desktop with nothing listening on 8341 wrote "LLM offline" for
    // every message and every think cycle — 650+ identical lines in a day —
    // while the shell showed nothing. The log should carry the transitions,
    // not the repeats.
    let mut gate = OfflineLogGate::default();

    assert!(gate.note(true), "the first failure is a state change — it warns");
    assert!(!gate.note(true), "the next failure changes nothing — debug");
    assert!(!gate.note(true), "a backend dead all day stays one line");

    assert!(gate.note(false), "a model answering again is a state change — it infos");
    assert!(!gate.note(false), "continued success does not re-log");

    assert!(gate.note(true), "and a new outage warns again");
}

#[test]
fn a_healthy_backend_never_announces_itself() {
    // The gate starts online: the first successful call must not log a
    // "reachable again" recovery that never happened.
    let mut gate = OfflineLogGate::default();
    assert!(!gate.note(false));
    assert!(!gate.note(false));
}
