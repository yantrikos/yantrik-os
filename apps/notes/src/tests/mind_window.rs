//! A Mind's window beside the person's, in the one Notes (#963): the Mind's window on Mind
//! View's display holds no note text and runs nothing as the person, the person's beside it shows
//! the note and takes their typing, a launch goes to the window of whoever asked, and the calls
//! answer as they do with the person's window alone.

use yantrik_app::for_mind::{HIDDEN, READ_ONLY};
use yantrik_app::WindowId;
use yantrik_testing::TestApp;

use super::mind_view::{kept, open_first, what_a_caller_gets, window_text_in, SENTINEL};
use super::settle;
use crate::command::{Command, WindowAction};
use crate::Notes;

/// The person's Notes over a kept library, its first note open.
fn persons() -> TestApp<Notes> {
    persons_at(kept())
}

fn persons_at(paths: yantrik_app::Paths) -> TestApp<Notes> {
    let app = TestApp::<Notes>::start_at(paths, Vec::new());
    settle(&app);
    open_first(&app);
    app
}

fn from(window: WindowId, command: Command) -> Command {
    Command::From { window, command: Box::new(command) }
}

#[test]
fn only_the_minds_window_is_masked_and_the_persons_beside_it_is_not() {
    let app = persons();
    let mind = app.open_for_mind("wayland-1");
    settle(&app);
    let theirs = window_text_in(&app, mind);
    assert!(!theirs.contains(SENTINEL), "{theirs}");
    assert!(theirs.contains("Plain title"), "titles stay: {theirs}");
    assert_eq!(app.window(mind, |ui| ui.get_content().to_string()), Some(HIDDEN.to_string()));
    assert_eq!(app.window(mind, |ui| ui.get_read_only()), Some(true), "the Mind's editor takes no typing");
    assert!(window_text_in(&app, WindowId::PRIMARY).contains(SENTINEL), "the person's window is not masked");
    assert!(!app.ui(|ui| ui.get_read_only()));
    let d = app.describe();
    let listed = d["state"]["windows"].as_array().cloned().unwrap_or_default();
    assert!(listed.iter().any(|w| w["window"] == mind.0 && w["mind_view"] == true), "{listed:?}");
}

#[test]
fn the_minds_window_runs_nothing_as_the_person_and_the_persons_window_does() {
    let app = persons();
    let mind = app.open_for_mind("wayland-1");
    settle(&app);
    for command in [
        Command::Edit(HIDDEN.into()),
        Command::Window(WindowAction::Copy),
        Command::Window(WindowAction::Save),
        Command::Window(WindowAction::NewWindow),
    ] {
        app.run(from(mind, command));
        assert_eq!(app.last_refusal().map(|r| r.sentence), Some(READ_ONLY.to_string()));
    }
    assert_eq!(app.windows(), vec![WindowId::PRIMARY, mind], "no window was opened at the Mind's press");
    app.run(from(WindowId::PRIMARY, Command::Edit(format!("# Plain title\n\nTyped by the person {SENTINEL}.\n"))));
    assert!(app.last_refusal().is_none(), "the person types in their own window");
    settle(&app);
    assert!(window_text_in(&app, WindowId::PRIMARY).contains("Typed by the person"));
    assert!(!window_text_in(&app, mind).contains("Typed by the person"), "the edit is masked in the Mind's");
}

#[test]
fn a_launch_goes_to_the_window_of_whoever_asked() {
    let app = persons();
    let mind = app.open_for_mind("wayland-1");
    app.launch_from_person().unwrap();
    assert_eq!(app.windows(), vec![WindowId::PRIMARY, mind]);
    assert_eq!(app.open_for_mind("wayland-1"), mind, "the Mind's launch raises its own window");
    assert!(window_text_in(&app, WindowId::PRIMARY).contains(SENTINEL));
    // The person closes their window; the Mind's keeps Notes running, and the person's next
    // launch gives them their own window back, unmasked.
    app.close_requested(WindowId::PRIMARY);
    settle(&app);
    assert_eq!(app.windows(), vec![mind]);
    app.launch_from_person().unwrap();
    settle(&app);
    assert_eq!(app.person_windows(), vec![WindowId::PRIMARY]);
    assert!(!app.ui(|ui| ui.get_read_only()));
}

#[test]
fn the_calls_answer_the_same_with_a_minds_window_open() {
    let paths = kept();
    let alone = persons_at(paths.clone());
    let expected = what_a_caller_gets(&alone);
    let _ = alone.shutdown();
    drop(alone);
    let beside = persons_at(paths);
    beside.open_for_mind("wayland-1");
    settle(&beside);
    let mut got = what_a_caller_gets(&beside);
    let mut expected = expected;
    // The one difference: `describe.windows` lists the Mind's window too.
    for answer in [&mut got, &mut expected] {
        if let Some(state) = answer["state"].as_object_mut() {
            state.remove("windows");
        }
    }
    assert!(expected.to_string().contains(SENTINEL), "the calls read the person's notes");
    assert_eq!(got, expected);
}
