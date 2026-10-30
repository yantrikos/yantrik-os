//! Stop, and the Agents screen: the two things the desk may ask of the shell, and what it claims.
//!
//! The desk stops nothing. Stop… asks the shell, which asks the person on their own desktop, and
//! the desk says "Stopped" only when a snapshot carries the shell's word that it did.

use yantrik_ipc_contracts::mind_desk::{MindState, StopAck, StopStatus};
use yantrik_services::mind_desk::FakeMindDesk;
use yantrik_services::ServiceError;

use super::{hermes, snapshot, start, top_words};
use crate::model::Stop;
use crate::Command;

/// A shell that says Hermes is at work, and says it again when asked again.
fn working() -> FakeMindDesk {
    FakeMindDesk::new()
        .answering(snapshot(1, Some(hermes(MindState::RunningTool))))
        .answering(snapshot(1, Some(hermes(MindState::RunningTool))))
}

fn with_stop(tag: u64, state: MindState, stop: Option<StopStatus>) -> yantrik_ipc_contracts::mind_desk::DeskSnapshot {
    let mut mind = hermes(state);
    mind.stop = stop;
    snapshot(tag, Some(mind))
}

#[test]
fn stop_is_offered_to_a_mind_at_work_and_to_none_other() {
    let (app, _) = start(FakeMindDesk::new());
    for (state, offered) in [
        (MindState::Thinking, true),
        (MindState::RunningTool, true),
        (MindState::WaitingForYou, true),
        (MindState::Idle, false),
        (MindState::Done, false),
        (MindState::Failed, false),
        // No backend to stop it with.
        (MindState::HarnessGone, false),
    ] {
        app.run(Command::Heard(Ok(snapshot(10 + state as u64, Some(hermes(state))))));
        assert_eq!(app.ui(|ui| ui.get_can_stop()), offered, "{state:?}");
    }
    // And asking for a stop of one that is not at work is refused, not queued.
    app.run(Command::StopAsked);
    assert!(app.last_refusal().is_some());
    assert_eq!(app.model(|m| m.stop.clone()), Stop::Resting);
}

/// Pressing Stop… asks the shell and nothing else: there is no question in the room, because the
/// room is where a Mind can answer it. The desk says it is asking, and says nothing is stopped.
#[test]
fn stop_asks_the_shell_and_the_desk_claims_nothing() {
    let (app, shell) = start(working());
    app.run_jobs();
    shell.stop_answers(Ok(StopAck { agent: "hermes:c-1".into() }));
    app.run(Command::StopAsked);

    assert_eq!(app.ui(|ui| ui.get_stop_note()), "Asking…");
    assert!(!app.ui(|ui| ui.get_can_stop()), "no second press while one is out");
    assert!(!top_words(&app).contains("Stopped"), "{}", top_words(&app));
    assert!(shell.asked_to_stop().is_empty(), "the job has not run yet");

    app.run_jobs();
    assert_eq!(shell.asked_to_stop(), ["hermes:c-1"]);
    // The shell took the question. That is all it said: nothing has stopped, and the desk does not
    // say it has.
    assert_eq!(app.model(|m| m.stop.clone()), Stop::Resting);
    assert!(!top_words(&app).contains("Stopped"), "{}", top_words(&app));
}

/// What the desk says of a stop is the shell's word in a snapshot, and only that.
#[test]
fn stopped_is_said_only_when_the_shell_says_it_stopped_the_mind() {
    let (app, _) = start(FakeMindDesk::new());
    let said = |tag, state, stop| {
        app.run(Command::Heard(Ok(with_stop(tag, state, stop))));
        app.ui(|ui| ui.get_stop_note()).to_string()
    };
    assert_eq!(said(1, MindState::RunningTool, None), "");
    assert_eq!(said(2, MindState::RunningTool, Some(StopStatus::Asked)), "Waiting for your answer on the desktop");
    assert!(!app.ui(|ui| ui.get_can_stop()), "while the person is being asked");
    assert_eq!(said(3, MindState::Idle, Some(StopStatus::Stopped)), "Stopped");
    assert_eq!(said(4, MindState::RunningTool, Some(StopStatus::NotStopped)), "Not stopped");
    assert!(app.ui(|ui| ui.get_can_stop()), "and it may be asked again");
}

#[test]
fn a_question_the_shell_would_not_take_claims_nothing() {
    for answer in [
        Err(ServiceError::Unreachable { service: "app-shell" }),
        Err(ServiceError::Timeout { service: "app-shell", seconds: 10 }),
        Err(ServiceError::Refused { code: -32000, sentence: "no".into() }),
        // An acknowledgement of some other agent is not one for this.
        Ok(StopAck { agent: "pi:c-9".into() }),
    ] {
        let (app, shell) = start(working());
        app.run_jobs();
        shell.stop_answers(answer.clone());
        app.run(Command::StopAsked);
        app.run_jobs();
        assert_eq!(app.ui(|ui| ui.get_stop_note()), "The shell did not take the request", "{answer:?}");
        assert!(!top_words(&app).contains("Stopped"), "{answer:?}: {}", top_words(&app));
        assert!(matches!(app.model(|m| m.stop.clone()), Stop::NotTaken { .. }));
        assert!(app.ui(|ui| ui.get_can_stop()), "and it may be asked again");
    }
}

#[test]
fn the_desk_has_no_confirmation_of_its_own() {
    // The question is on the person's desktop, and nothing here can answer it.
    let source = include_str!("../model.rs");
    let commands = source.split("pub enum Command").nth(1).unwrap();
    assert!(!commands.contains("StopConfirmed") && !commands.contains("StopCancelled"));
    let ui = format!("{}{}", include_str!("../../ui/app.slint"), include_str!("../../ui/desk.slint"));
    assert!(!ui.contains("Keep going") && !ui.contains("confirm"), "no question is asked in the room");
}

#[test]
fn open_in_agents_asks_the_shell_for_the_persons_own_screen() {
    let (app, shell) = start(working());
    app.run_jobs();
    assert!(app.ui(|ui| ui.get_agents_offered()));
    shell.show_answers(Ok(()));
    app.run(Command::AgentsAsked);
    app.run_jobs();
    assert_eq!(shell.showed(), 1);
    assert_eq!(app.ui(|ui| ui.get_stop_note()), "");

    shell.show_answers(Err(ServiceError::Unreachable { service: "app-shell" }));
    app.run(Command::AgentsAsked);
    app.run_jobs();
    assert_eq!(app.ui(|ui| ui.get_stop_note()), "Agents did not open");
}

/// The shell holds the Agents screen while an approval card waits, and rate limits it: the person
/// is told in the desk's words and not the shell's.
#[test]
fn what_the_shell_says_when_agents_will_not_open_is_said_in_the_desks_words() {
    for (sentence, said) in [
        ("`mind_desk_open_agents` was not run: an approval card is waiting for the person", "An approval card is waiting on your desktop"),
        ("Agents was opened from the Mind desk a moment ago; it opens at most once every ten seconds.", "Agents was opened a moment ago"),
        ("Agents opens from the Mind desk only while Mind View is in front.", "Agents opens while Mind View is in front"),
    ] {
        let (app, shell) = start(working());
        app.run_jobs();
        shell.show_answers(Err(ServiceError::Refused { code: -32000, sentence: sentence.into() }));
        app.run(Command::AgentsAsked);
        app.run_jobs();
        assert_eq!(app.ui(|ui| ui.get_stop_note()), said, "{sentence}");
    }
}
