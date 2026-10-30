//! The desk follows the shell: what it draws, when it asks, and what it does while nothing
//! happens.

use std::time::Duration;

use yantrik_ipc_contracts::mind_desk::MindState;
use yantrik_services::mind_desk::FakeMindDesk;
use yantrik_services::ServiceError;

use super::{hermes, snapshot, start, top_words};
use crate::model::Link;
use crate::{chrome, Command};

fn down() -> ServiceError {
    ServiceError::Unreachable { service: "app-shell" }
}

#[test]
fn it_opens_one_window_and_asks_the_shell_for_the_first_snapshot() {
    let (app, shell) = start(FakeMindDesk::new().answering(snapshot(1, Some(hermes(MindState::RunningTool)))));
    assert_eq!(app.ui(|ui| ui.get_bar_height()), crate::metrics::BAR_HEIGHT as f32, "one window, the bar's height");
    assert_eq!(app.model(|m| m.link), Link::Connecting);
    assert!(top_words(&app).contains("CONNECTING"), "{}", top_words(&app));

    app.run_jobs();
    assert_eq!(shell.asked(), [None], "the first ask has nothing to say it has already");
    assert_eq!(app.model(|m| m.link), Link::Live);
    let words = top_words(&app);
    assert!(words.contains("HERMES · RUNNING A TOOL"), "{words}");
    assert!(words.contains("Running blender.add_object"), "{words}");
    assert!(words.contains("RUNNING TOOL | blender.add_object | ◇ Reported · Running"), "{words}");
    assert!(words.contains("4:32 elapsed at 14:32 · 12 calls · 1 failed · 1 interrupted"), "{words}");
    assert_eq!(app.ui(|ui| ui.get_tone()), 1, "teal: the Mind at work");
    assert!(app.ui(|ui| ui.get_can_stop()));
}

/// The compositor reads the height at map, and the heights the three strips share out are the
/// bar's, with the pixel over the apps.
#[test]
fn the_bar_says_how_tall_it_is_and_how_it_is_shared_out_before_it_is_shown() {
    use crate::metrics::{BAR_HEIGHT, FOOTER_HEIGHT, HEADER_HEIGHT, MARGIN_TOP, OVERLAP, STRIP_HEIGHT};
    let (app, _) = start(FakeMindDesk::new());
    app.ui(|ui| {
        assert_eq!(ui.get_bar_height(), BAR_HEIGHT as f32);
        assert_eq!(ui.get_header_height(), HEADER_HEIGHT as f32);
        assert_eq!(ui.get_strip_height(), STRIP_HEIGHT as f32);
    });
    assert_eq!(HEADER_HEIGHT + STRIP_HEIGHT + FOOTER_HEIGHT, MARGIN_TOP);
    assert_eq!(BAR_HEIGHT, MARGIN_TOP + OVERLAP);
    // The footer's words, by the same chrome the window is drawn from.
    let footer = app.model(chrome::footer);
    assert_eq!(footer.apps, "Apps unavailable", "nothing is claimed before the shell has said");
}

#[test]
fn the_footer_says_the_apps_the_account_and_the_model_from_the_snapshot() {
    let (app, _) = start(FakeMindDesk::new().answering(snapshot(1, Some(hermes(MindState::Thinking)))));
    app.run_jobs();
    let b = app.model(chrome::footer);
    assert_eq!(b.apps, "Blender / Terminal / Files");
    assert_eq!(b.safety, "Runs as you · OS actions ask first · Not isolated (planned)");
    assert_eq!(b.model, "deepseek-v4.1-flash via ollama-cloud");
    assert_eq!(b.model_mark, "◇ Harness-reported model");

    // The built-in Mind's account; and a harness that gave no model.
    let mut own = hermes(MindState::Idle);
    own.name = "Yantrik Mind".into();
    own.account = yantrik_ipc_contracts::mind_desk::Account::Own;
    own.model = None;
    app.run(Command::Heard(Ok(snapshot(2, Some(own)))));
    let b = app.model(chrome::footer);
    assert_eq!(b.safety, "Own account · Network checked · OS actions ask first");
    assert_eq!((b.model.as_str(), b.model_mark.as_str()), ("Model unavailable", ""));
}

#[test]
fn a_new_snapshot_is_asked_for_by_the_tag_of_the_last_one_drawn() {
    let (app, shell) = start(FakeMindDesk::new().answering(snapshot(1, Some(hermes(MindState::Thinking)))));
    app.run_jobs();
    shell.push(Ok(snapshot(2, Some(hermes(MindState::WaitingForYou)))));
    app.run_jobs();
    assert_eq!(shell.asked()[..2], [None, Some(1)]);
    assert_eq!(app.ui(|ui| ui.get_tone()), 2, "amber: waiting for the person, and only that");
    assert!(top_words(&app).contains("WAITING FOR YOU"));
}

#[test]
fn every_state_is_said_in_words_and_failed_is_red_and_done_is_green() {
    let (app, _) = start(FakeMindDesk::new());
    for (state, label, tone) in [
        (MindState::Thinking, "THINKING", 1),
        (MindState::RunningTool, "RUNNING A TOOL", 1),
        (MindState::WaitingForYou, "WAITING FOR YOU", 2),
        (MindState::Idle, "IDLE", 0),
        (MindState::Done, "DONE", 4),
        (MindState::Failed, "FAILED", 3),
        (MindState::HarnessGone, "CONNECTION LOST", 0),
    ] {
        app.run(Command::Heard(Ok(snapshot(1 + state as u64, Some(hermes(state))))));
        let words = top_words(&app);
        assert!(words.contains(&format!("HERMES · {label}")), "{state:?}: {words}");
        assert_eq!(app.ui(|ui| ui.get_tone()), tone, "{state:?}");
    }
}

/// The desk quotes nobody: what a Mind says it is doing is not on a window every Mind can read.
#[test]
fn the_desk_quotes_nobody() {
    let (app, _) = start(FakeMindDesk::new().answering(snapshot(1, Some(hermes(MindState::Thinking)))));
    app.run_jobs();
    assert_eq!(app.ui(|ui| ui.get_headline()), "Thinking");
    assert!(!top_words(&app).contains('“'));
    assert!(!top_words(&app).contains("Reported ·") || top_words(&app).contains("Reported · Running"), "only a tool's own mark");
}

#[test]
fn with_no_mind_known_the_desk_says_so_and_offers_nothing_to_stop() {
    let (app, _) = start(FakeMindDesk::new().answering(snapshot(1, None)));
    app.run_jobs();
    assert!(top_words(&app).contains("MIND VIEW · IDLE | No Mind is at work here"), "{}", top_words(&app));
    assert!(!app.ui(|ui| ui.get_can_stop()));
    assert!(app.ui(|ui| ui.get_agents_offered()));
}

#[test]
fn a_shell_that_does_not_answer_is_said_and_asked_again_with_a_longer_pause_each_time() {
    let (app, shell) = start(FakeMindDesk::new().then(Err(down())));
    app.run_jobs();
    assert_eq!(app.model(|m| m.link), Link::Lost);
    let words = top_words(&app);
    assert!(words.contains("NOT CONNECTED") && words.contains("The shell is not answering"), "{words}");
    assert!(!app.ui(|ui| ui.get_can_stop()) && !app.ui(|ui| ui.get_agents_offered()), "nothing it could not do");
    let footer = app.model(chrome::footer);
    assert_eq!((footer.apps.as_str(), footer.safety.as_str()), ("Apps unavailable", "Account unavailable"));

    // 3 s, then 6, then 12: never a loop.
    assert_eq!(shell.asked().len(), 1);
    app.advance(Duration::from_secs(2));
    app.run_jobs();
    assert_eq!(shell.asked().len(), 1, "not yet");
    app.advance(Duration::from_secs(1));
    app.run_jobs();
    assert_eq!(shell.asked().len(), 2);
    app.advance(Duration::from_secs(5));
    app.run_jobs();
    assert_eq!(shell.asked().len(), 2, "the pause doubled");
    app.advance(Duration::from_secs(1));
    app.run_jobs();
    assert_eq!(shell.asked().len(), 3);

    // And it comes back when the shell does.
    shell.push(Ok(snapshot(1, Some(hermes(MindState::Idle)))));
    app.advance(Duration::from_secs(12));
    app.run_jobs();
    assert_eq!(app.model(|m| m.link), Link::Live);
    assert!(top_words(&app).contains("HERMES · IDLE"));
}

/// A shell that answers at once with what the desk has is not a reason to ask in a loop.
#[test]
fn an_answer_that_did_not_wait_and_said_nothing_new_is_asked_again_slowly() {
    let same = snapshot(1, Some(hermes(MindState::Thinking)));
    let (app, shell) = start(FakeMindDesk::new().answering(same.clone()).answering(same.clone()).answering(same));
    // The first answer is news, so the next ask goes out at once.
    assert!(app.run_one_job());
    assert_eq!((shell.asked().len(), app.jobs_waiting(), app.timers_armed()), (1, 1, 0));
    // The second says what the desk has, and came back at once: it is not asked again at once.
    assert!(app.run_one_job());
    assert_eq!((shell.asked().len(), app.jobs_waiting(), app.timers_armed()), (2, 0, 1), "a timer, and no job");
    // After its pause it is asked once more, and the pause is not shorter.
    app.advance(Duration::from_secs(4));
    assert_eq!(app.jobs_waiting(), 0, "four seconds is not five");
    app.advance(Duration::from_secs(1));
    assert_eq!(app.jobs_waiting(), 1);
    assert!(app.run_one_job());
    assert_eq!((shell.asked().len(), app.timers_armed()), (3, 1));
}

/// The desk at rest draws nothing and fires nothing: its one ask is a job blocked on the shell's
/// socket, and there is no timer to wake it.
#[test]
fn a_desk_left_alone_costs_nothing() {
    let (app, shell) = start(FakeMindDesk::new().answering(snapshot(1, Some(hermes(MindState::Thinking)))));
    app.run_jobs();
    assert_eq!(shell.asked().len(), 1);
    assert_eq!(app.jobs_waiting(), 1, "the next ask is out, waiting on the shell, as in life");
    app.resize(1184, 161);
    assert!(app.settle_frames());
    app.advance(Duration::from_secs(600));
    assert_eq!(app.timers_armed(), 0, "no timer while the shell answers");
    assert_eq!(app.draw(), 0, "ten minutes: no frame");
    assert_eq!(app.jobs_waiting(), 1, "and still the one ask");
}

/// The same words arriving again redraw nothing.
#[test]
fn the_same_snapshot_again_changes_nothing_on_screen() {
    let (app, _) = start(FakeMindDesk::new().answering(snapshot(1, Some(hermes(MindState::Thinking)))));
    app.run_jobs();
    app.resize(1184, 161);
    assert!(app.settle_frames());
    app.run(Command::Heard(Ok(snapshot(1, Some(hermes(MindState::Thinking))))));
    assert_eq!(app.draw(), 0, "the wait ran its course with the same words");
}

#[test]
fn a_window_that_is_asked_to_close_stays() {
    let (app, _) = start(FakeMindDesk::new());
    app.event(yantrik_app::app::Event::CloseRequested(yantrik_app::app::WindowId::PRIMARY));
    assert!(app.exit_reason().is_none(), "the bars are walls, not windows a person closes");
}
