//! A Mind's window beside the person's, in the one instance (#963): the Mind's window holds no
//! document text and the person's beside it shows their document, a launch goes to the window of
//! whoever asked, and the calls answer as they do with the person's window alone.

use yantrik_app::for_mind::HIDDEN;
use yantrik_app::WindowId;

use super::mind_view::{window_text_in, with_document, SENTINEL};
use super::{act, none, settle};

#[test]
fn only_the_minds_window_is_masked_and_the_persons_beside_it_is_not() {
    let (app, _) = with_document(false);
    let mind = app.open_for_mind("wayland-1");
    settle(&app);
    let theirs = window_text_in(&app, mind);
    assert!(!theirs.contains(SENTINEL), "{theirs}");
    // A file outside the home is one no agent is shown, so its name and path are withheld from
    // the Mind's window as `describe` withholds them (#972 M2); the person's window names it.
    assert!(!theirs.contains("plan.rs") && theirs.contains(crate::model::HIDDEN_TAB), "{theirs}");
    assert!(window_text_in(&app, WindowId::PRIMARY).contains("plan.rs"));
    assert_eq!(app.window(mind, |ui| ui.get_content().to_string()), Some(HIDDEN.to_string()));
    assert_eq!(app.window(mind, |ui| ui.get_masked()), Some(true), "the Mind's field takes no keys");
    assert!(window_text_in(&app, WindowId::PRIMARY).contains(SENTINEL), "the person's window is not masked");
    assert!(!app.ui(|ui| ui.get_masked()));
    assert!(app.ui(|ui| ui.get_line_count()) > 0);
}

#[test]
fn a_launch_goes_to_the_window_of_whoever_asked() {
    let (app, _) = with_document(false);
    let mind = app.open_for_mind("wayland-1");
    app.launch_from_person().unwrap();
    assert_eq!(app.windows(), vec![WindowId::PRIMARY, mind]);
    assert_eq!(app.open_for_mind("wayland-1"), mind);
    assert!(window_text_in(&app, WindowId::PRIMARY).contains(SENTINEL));
    assert!(!window_text_in(&app, mind).contains(SENTINEL));
}

#[test]
fn the_calls_answer_the_same_with_a_minds_window_open() {
    let mut got = Vec::new();
    for beside in [false, true] {
        let (app, path) = with_document(false);
        if beside {
            app.open_for_mind("wayland-1");
            settle(&app);
        }
        let d = app.describe();
        let read = act(&app, "read", none()).unwrap();
        let find = act(&app, "find", serde_json::json!({ "text": SENTINEL })).unwrap();
        let text = serde_json::json!([d["summary"], d["state"], read, find]).to_string();
        got.push(text.replace(&path.parent().unwrap().display().to_string(), "<dir>"));
    }
    assert!(got[0].contains(SENTINEL), "the calls read the person's document");
    assert_eq!(got[0], got[1]);
}
