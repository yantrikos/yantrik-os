//! A Mind's window beside the person's, in the one instance (#963): the Mind's window on Mind
//! View's display holds no mail text, the person's beside it shows their mail, a launch goes to
//! the window of whoever asked, and the intents answer as they do with the person's window alone.

use serde_json::json;
use yantrik_app::for_mind::HIDDEN;
use yantrik_app::WindowId;

use super::mind_view::{mailbox, window_text_in, with_message, SENTINEL};
use super::{act, quiet};

#[test]
fn only_the_minds_window_is_masked_and_the_persons_beside_it_is_not() {
    let app = with_message(&mailbox(), false);
    let mind = app.open_for_mind("wayland-1");
    quiet(&app);
    let theirs = window_text_in(&app, mind);
    assert!(!theirs.contains(SENTINEL), "{theirs}");
    assert!(theirs.contains(HIDDEN));
    assert!(window_text_in(&app, WindowId::PRIMARY).contains(SENTINEL), "the person's window is not masked");
    // The person goes on reading: their window draws it, the Mind's masks it.
    app.run(crate::command::Command::Open(crate::command::Pick::Key(0)));
    quiet(&app);
    assert!(window_text_in(&app, WindowId::PRIMARY).contains(SENTINEL));
    assert!(!window_text_in(&app, mind).contains(SENTINEL));
}

#[test]
fn a_launch_goes_to_the_window_of_whoever_asked() {
    let app = with_message(&mailbox(), false);
    let mind = app.open_for_mind("wayland-1");
    app.launch_from_person().unwrap();
    assert_eq!(app.windows(), vec![WindowId::PRIMARY, mind], "no window of the Mind's handed the person's launch");
    assert_eq!(app.open_for_mind("wayland-1"), mind, "the Mind's launch raises its own window");
    assert_eq!(app.person_windows(), vec![WindowId::PRIMARY]);
    assert!(window_text_in(&app, WindowId::PRIMARY).contains(SENTINEL));
}

#[test]
fn the_intents_answer_the_same_with_a_minds_window_open() {
    let mut got = Vec::new();
    for beside in [false, true] {
        let app = with_message(&mailbox(), false);
        if beside {
            app.open_for_mind("wayland-1");
            quiet(&app);
        }
        let search = act(&app, "search", json!({ "query": "subject" })).unwrap();
        quiet(&app);
        let compose = act(&app, "compose", json!({ "to": "m@x", "subject": "hi", "body": "there" })).unwrap();
        quiet(&app);
        let d = app.describe();
        got.push(json!([d["summary"], d["state"], search, compose, app.model(|m| m.composer.text.clone()).body]));
    }
    assert!(got[0].to_string().contains(SENTINEL), "the intents read the person's mail");
    assert_eq!(got[0], got[1]);
}
