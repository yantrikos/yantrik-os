//! Stop, and the Agents screen: the two things the desk may ask of the shell, and what it claims.

use yantrik_ipc_contracts::mind_desk::{MindState, StopAck};
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

#[test]
fn stop_asks_first_and_the_answer_is_the_persons() {
    let (app, shell) = start(working());
    app.run_jobs();
    app.run(Command::StopAsked);
    assert!(app.ui(|ui| ui.get_confirming()));
    assert_eq!(app.ui(|ui| ui.get_confirm_question()), "Stop Hermes?");
    assert!(!app.ui(|ui| ui.get_can_stop()), "the question stands where the button was");
    assert!(shell.stopped().is_empty(), "nothing is stopped by asking");

    // Keep going: nothing is stopped, and the button is back.
    app.run(Command::StopCancelled);
    assert!(!app.ui(|ui| ui.get_confirming()) && app.ui(|ui| ui.get_can_stop()));
    assert!(shell.stopped().is_empty());
}

#[test]
fn a_stop_that_the_shell_acknowledged_is_said_stopped_and_not_before() {
    let (app, shell) = start(working());
    app.run_jobs();
    shell.stop_answers(Ok(StopAck { agent: "hermes:c-1".into() }));
    app.run(Command::StopAsked);
    app.run(Command::StopConfirmed);

    // Asked, and not answered: the desk says it is stopping, and says nothing is stopped.
    assert_eq!(app.ui(|ui| ui.get_stop_note()), "Stopping…");
    assert!(!top_words(&app).contains("Stopped"), "{}", top_words(&app));
    assert_eq!(app.model(|m| m.stop.clone()), Stop::Stopping { agent: "hermes:c-1".into() });

    // The shell's acknowledgement names the agent it stopped. Only then.
    app.run_jobs();
    assert_eq!(shell.stopped(), ["hermes:c-1"]);
    assert_eq!(app.ui(|ui| ui.get_stop_note()), "Stopped");
    assert!(!app.ui(|ui| ui.get_can_stop()), "nothing to stop again");
}

#[test]
fn a_stop_the_shell_did_not_acknowledge_claims_nothing() {
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
        app.run(Command::StopConfirmed);
        app.run_jobs();
        let note = app.ui(|ui| ui.get_stop_note());
        assert_eq!(note, "The shell did not confirm the stop", "{answer:?}");
        assert!(!top_words(&app).contains("Stopped"), "{answer:?}: {}", top_words(&app));
        assert!(matches!(app.model(|m| m.stop.clone()), Stop::NotConfirmed { .. }));
        assert!(app.ui(|ui| ui.get_can_stop()), "and it may be asked again");
    }
}

/// A stop is for the Mind that was showing when it was asked.
#[test]
fn a_confirmation_does_not_outlive_its_mind() {
    let (app, shell) = start(working());
    app.run_jobs();
    app.run(Command::StopAsked);
    // The shell now says a different Mind needs the person.
    let mut other = hermes(MindState::WaitingForYou);
    other.agent = "pi:c-2".into();
    other.name = "Pi".into();
    app.run(Command::Heard(Ok(snapshot(2, Some(other)))));
    assert!(!app.ui(|ui| ui.get_confirming()), "the question was about Hermes");
    app.run(Command::StopConfirmed);
    assert!(app.last_refusal().is_some(), "a yes with no question is refused");
    assert!(shell.stopped().is_empty());
}

#[test]
fn an_acknowledged_stop_is_said_until_the_mind_is_at_work_again() {
    let (app, shell) = start(working());
    app.run_jobs();
    shell.stop_answers(Ok(StopAck { agent: "hermes:c-1".into() }));
    app.run(Command::StopAsked);
    app.run(Command::StopConfirmed);
    app.run_jobs();
    // The shell reports the Mind at rest: Stopped stays.
    app.run(Command::Heard(Ok(snapshot(2, Some(hermes(MindState::Idle))))));
    assert_eq!(app.ui(|ui| ui.get_stop_note()), "Stopped");
    // A new run, which is a state change: the word is gone.
    let mut again = hermes(MindState::Thinking);
    again.since = 500;
    app.run(Command::Heard(Ok(snapshot(3, Some(again)))));
    assert_eq!(app.ui(|ui| ui.get_stop_note()), "");
    assert!(app.ui(|ui| ui.get_can_stop()));
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

/// The shell holds the Agents screen while an approval card waits: the person is told where the
/// card is, in the desk's words and not the shell's.
#[test]
fn a_card_waiting_is_said_when_agents_will_not_open() {
    let (app, shell) = start(working());
    app.run_jobs();
    shell.show_answers(Err(ServiceError::Refused {
        code: -32000,
        sentence: "`show_workroom` was not run: an approval card is waiting for the person at this machine".into(),
    }));
    app.run(Command::AgentsAsked);
    app.run_jobs();
    assert_eq!(app.ui(|ui| ui.get_stop_note()), "An approval card is waiting on your desktop");
}
