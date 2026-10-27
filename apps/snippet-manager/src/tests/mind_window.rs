//! A Mind's window beside the person's, in the one instance (#963): the Mind's window holds no
//! code and no search of the person's, the person's beside it shows both, a launch goes to the
//! window of whoever asked, and the calls answer as they do with the person's window alone.

use serde_json::json;
use yantrik_app::for_mind::HIDDEN;
use yantrik_app::WindowId;

use super::{open, Fixture};
use crate::Command;

const SENTINEL: &str = "SENTINEL-mind-window-77b2";

/// Every string `window` holds that a screen reader (or OCR) could see.
fn text_in(f: &Fixture, window: WindowId) -> String {
    f.app
        .window(window, |ui| {
            let mut all = vec![
                ui.get_detail_title().to_string(),
                ui.get_detail_code().to_string(),
                ui.get_notice().to_string(),
                ui.get_search_query().to_string(),
            ];
            let rows = ui.get_snippets();
            for i in 0..slint::Model::row_count(&rows) {
                let r = slint::Model::row_data(&rows, i).unwrap();
                all.extend([r.title, r.code, r.preview].map(|s| s.to_string()));
            }
            all.join("\n")
        })
        .expect("the window is open")
}

/// A snippet holding the sentinel, open in the person's window.
fn with_snippet() -> (Fixture, i32) {
    let f = open();
    let id = f.keep("Deploy key", "Shell", &format!("export TOKEN={SENTINEL}"), "deploy");
    f.app.run(Command::Select(id));
    f.quiet();
    (f, id)
}

#[test]
fn only_the_minds_window_is_masked_and_the_persons_beside_it_is_not() {
    let (f, _) = with_snippet();
    let mind = f.app.open_for_mind("wayland-1");
    f.quiet();
    let theirs = text_in(&f, mind);
    assert!(!theirs.contains(SENTINEL), "{theirs}");
    assert!(theirs.contains("Deploy key") && theirs.contains(HIDDEN), "{theirs}");
    assert_eq!(f.app.window(mind, |ui| ui.get_detail_read_only()), Some(true));
    assert!(text_in(&f, WindowId::PRIMARY).contains(SENTINEL), "the person's window is not masked");
    assert!(!f.app.ui(|ui| ui.get_detail_read_only()));
    // The person's search, which matches code, is theirs alone.
    f.app.run(Command::Search(SENTINEL.into()));
    f.quiet();
    assert!(!text_in(&f, mind).contains(SENTINEL));
    assert_eq!(f.app.ui(|ui| ui.get_search_query().to_string()), SENTINEL);
}

#[test]
fn a_launch_goes_to_the_window_of_whoever_asked() {
    let (f, _) = with_snippet();
    let mind = f.app.open_for_mind("wayland-1");
    f.app.launch_from_person().unwrap();
    assert_eq!(f.app.windows(), vec![WindowId::PRIMARY, mind]);
    assert_eq!(f.app.open_for_mind("wayland-1"), mind);
    assert!(text_in(&f, WindowId::PRIMARY).contains(SENTINEL));
}

#[test]
fn the_calls_answer_the_same_with_a_minds_window_open() {
    let mut got = Vec::new();
    for beside in [false, true] {
        let (f, id) = with_snippet();
        if beside {
            f.app.open_for_mind("wayland-1");
            f.quiet();
        }
        let args = json!({ "id": id.to_string() });
        let copied = f.act("copy", args.clone());
        let found = f.act("search", json!({ "query": "TOKEN" }));
        let d = f.app.describe();
        // Each run keeps its store in a folder of its own; the rest must be the same.
        let text = json!([d["summary"], d["state"], copied, found, f.clipboard.texts()]).to_string();
        got.push(text.replace(&f.paths.data_dir().display().to_string(), "<dir>"));
    }
    assert!(got[0].contains(SENTINEL), "the calls reach the person's code");
    assert_eq!(got[0], got[1]);
}
