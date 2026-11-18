//! Snippets drawn for a Mind (Mind View): `read_mind_view` reads the window as text, so the window
//! holds no code. Titles, tags and language stay; the editor takes no typing; Copy puts nothing on
//! the clipboard.

use serde_json::json;

use super::{open, start_for_mind, Fixture};
use yantrik_app::for_mind::HIDDEN;
use crate::Command;

const SENTINEL: &str = "SENTINEL-mind-view-5c1d";

/// Every string the window holds that a screen reader (or OCR) could see: the editor, the rows
/// and the notice.
fn window_text(f: &Fixture) -> String {
    f.app.ui(|ui| {
        let mut all = vec![
            ui.get_detail_title().to_string(),
            ui.get_detail_language().to_string(),
            ui.get_detail_code().to_string(),
            ui.get_detail_tags().to_string(),
            ui.get_notice().to_string(),
        ];
        let rows = ui.get_snippets();
        for i in 0..slint::Model::row_count(&rows) {
            let r = slint::Model::row_data(&rows, i).unwrap();
            all.extend([r.title, r.language, r.code, r.tags, r.preview].map(|s| s.to_string()));
        }
        all.join("\n")
    })
}

/// A snippet kept in a normal run, and the paths it is kept at.
fn kept() -> (yantrik_app::Paths, i32) {
    let f = open();
    let id = f.keep("Deploy key", "Shell", &format!("export TOKEN={SENTINEL}\n# {SENTINEL}"), "deploy, secrets");
    let paths = f.paths.clone();
    let _ = f.app.shutdown();
    (paths, id)
}

#[test]
fn a_mind_view_window_holds_no_code_but_keeps_titles_tags_and_language() {
    let (paths, id) = kept();
    let f = start_for_mind(paths);
    assert!(!window_text(&f).contains(SENTINEL), "the list");
    f.app.run(Command::Select(id));
    let text = window_text(&f);
    assert!(!text.contains(SENTINEL), "the editor: {text}");
    assert_eq!(f.app.ui(|ui| ui.get_detail_code().to_string()), HIDDEN);
    assert!(f.app.ui(|ui| ui.get_detail_read_only()), "the editor takes no typing");
    for kept in ["Deploy key", "Shell", "deploy, secrets"] {
        assert!(text.contains(kept), "{kept} stays: {text}");
    }
    let preview = f.app.ui(|ui| slint::Model::row_data(&ui.get_snippets(), 0).unwrap().preview.to_string());
    assert_eq!(preview, HIDDEN);
}

#[test]
fn a_normal_window_still_shows_the_code() {
    let (paths, id) = kept();
    let f = super::start_at(paths, yantrik_app::mind::fake::FakeMind::new(), Vec::new());
    f.app.run(Command::Select(id));
    assert!(f.app.ui(|ui| ui.get_detail_code().to_string()).contains(SENTINEL));
    assert!(window_text(&f).contains(SENTINEL));
    assert!(!f.app.ui(|ui| ui.get_detail_read_only()));
}

#[test]
fn typing_and_saving_in_a_mind_view_window_change_nothing() {
    let (paths, id) = kept();
    let f = start_for_mind(paths);
    f.app.run(Command::Select(id));
    f.type_into("Deploy key", "overwritten");
    f.app.ui(move |ui| {
        ui.invoke_snip_save(id, "Deploy key".into(), "Shell".into(), HIDDEN.into(), "deploy".into())
    });
    f.quiet();
    assert!(!f.app.model(|m| m.editor_dirty()), "the masked text never becomes the editor's");
    let code = f.on_disk().get(id).unwrap().code.clone();
    assert!(code.contains(SENTINEL) && !code.contains(HIDDEN), "the file keeps the real code");
}

#[test]
fn copy_in_a_mind_view_window_puts_nothing_on_the_clipboard() {
    let (paths, id) = kept();
    let f = start_for_mind(paths);
    f.app.run(Command::Select(id));
    f.app.ui(move |ui| ui.invoke_snip_copy(id));
    f.quiet();
    assert!(f.clipboard.texts().is_empty(), "{:?}", f.clipboard.texts());
    let by_name = f.act("copy", json!({ "id": id.to_string() }));
    f.quiet();
    assert_eq!(by_name.unwrap_err(), yantrik_app::for_mind::READ_ONLY);
    assert!(f.clipboard.texts().is_empty());
    assert_eq!(f.on_disk().get(id).unwrap().use_count, 0, "and nothing was counted");
}

#[test]
fn explain_in_a_mind_view_window_does_not_send_the_code_anywhere() {
    let (paths, id) = kept();
    let f = start_for_mind(paths);
    f.app.run(Command::Select(id));
    f.app.ui(|ui| ui.invoke_ai_explain_pressed());
    f.quiet();
    assert!(f.mind.prompts().is_empty(), "no prompt was made of the code");
}

/// A Mind View window with the Mind reachable, so a refusal is the app's and not the reach's.
fn ready_for_mind(paths: yantrik_app::Paths) -> Fixture {
    let f = start_for_mind(paths);
    f.mind.set_reach(yantrik_app::mind::Reach::Ready);
    f.app.seen(yantrik_app::watch::Seen::Reach(yantrik_app::mind::Reach::Ready));
    f
}

#[test]
fn the_rail_offers_and_sends_nothing_in_a_mind_view_window() {
    let (paths, id) = kept();
    let f = ready_for_mind(paths);
    f.app.run(Command::Select(id));
    f.app.advance(std::time::Duration::from_millis(300));
    assert!(f.app.model(|m| m.rail.suggestions().is_empty()), "no chip is offered");
    f.app.ui(|ui| ui.invoke_agent_suggestion_activated("explain".into()));
    f.app.ui(|ui| ui.invoke_ai_explain_pressed());
    f.app.run(Command::Rail(yantrik_app::rail::RailCommand::Activate("explain".into())));
    f.quiet();
    assert!(f.mind.prompts().is_empty(), "no prompt was made of the code: {:?}", f.mind.prompts());
    assert!(f.app.model(|m| m.rail.proposal().is_none()), "and no card is drawn on Mind View");
}

fn shown_rows(f: &Fixture) -> usize {
    f.app.ui(|ui| slint::Model::row_count(&ui.get_snippets()))
}

#[test]
fn the_search_box_in_a_mind_view_window_matches_labels_not_code() {
    let (paths, _) = kept();
    let f = start_for_mind(paths);
    f.app.run(Command::Search("TOKEN=SENTINEL-mind-view-5c1d".into()));
    f.quiet();
    assert_eq!(shown_rows(&f), 0, "the code is not searched");
    assert_eq!(f.app.model(|m| m.rows().len()), 0);
    for label in ["deploy key", "shell", "secrets"] {
        f.app.run(Command::Search(label.into()));
        f.quiet();
        assert_eq!(shown_rows(&f), 1, "{label} still finds it");
    }
}

#[test]
fn a_call_the_masked_copy_refuses_answers_with_the_refusal() {
    let (paths, id) = kept();
    let f = start_for_mind(paths);
    let args = || json!({ "id": id.to_string() });
    let sentence = yantrik_app::for_mind::READ_ONLY;
    // Answered at once, or by the settled call, never by a timeout.
    let answer = f.app.act("copy", args()).and_then(|p| p.settle());
    assert_eq!(answer.unwrap_err(), sentence);
    assert_eq!(f.act("copy", args()).unwrap_err(), sentence);
    f.app.run(Command::CopyNamed(id.to_string()));
    f.quiet();
    assert!(f.clipboard.texts().is_empty());
}

#[test]
fn export_all_in_a_mind_view_window_writes_no_file() {
    let (paths, _) = kept();
    let f = start_for_mind(paths);
    f.app.ui(|ui| ui.invoke_snip_export_all());
    f.quiet();
    assert_eq!(f.app.last_refusal().unwrap().sentence, yantrik_app::for_mind::READ_ONLY);
}

#[test]
fn collections_in_a_mind_view_window_are_not_changed() {
    let (paths, _) = kept();
    let f = start_for_mind(paths);
    for command in [
        Command::CollectionCreate("new".into()),
        Command::CollectionRename(0, "renamed".into()),
        Command::CollectionDelete(0),
    ] {
        f.app.run(command);
        assert_eq!(f.app.last_refusal().unwrap().sentence, yantrik_app::for_mind::READ_ONLY);
    }
}
