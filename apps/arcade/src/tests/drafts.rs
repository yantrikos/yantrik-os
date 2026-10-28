//! The editor's text is never lost: it is drafted while the library does not hold it, written at
//! once when the app ends, put back in the editor at the next start, and let go only when a save
//! of it has landed.

use serde_json::json;
use yantrik_app::app::WindowId;
use yantrik_app::drafts::path_in;

use super::{open, PIP};

fn draft(f: &super::Fixture) -> Option<String> {
    std::fs::read_to_string(path_in(f.paths.state_dir(), "spec")).ok()
}

/// Typing no button kept: the old app lost it when the window closed.
#[test]
fn typing_left_at_close_is_back_in_the_editor_at_the_next_start() {
    let f = open();
    f.type_spec("{ \"name\": \"Half a thought\" }");
    f.app.close_requested(WindowId::PRIMARY);
    assert!(f.app.exit_reason().is_some());
    f.app.run_writes();
    assert!(draft(&f).unwrap().contains("Half a thought"));

    let f = f.restart();
    assert_eq!(f.app.ui(|ui| ui.get_spec_text().to_string()), "{ \"name\": \"Half a thought\" }");
    assert!(f.notice().contains("back in the editor"), "{}", f.notice());
}

/// It is let go when a save of that very text lands, and not before.
#[test]
fn a_draft_goes_when_its_text_is_kept_and_not_before() {
    let f = open();
    f.type_spec(PIP);
    f.app.advance(yantrik_app::drafts::DEBOUNCE);
    f.app.run_writes();
    assert!(draft(&f).is_some(), "typed, so drafted");
    // A refused save keeps the draft: nothing holds the text.
    f.type_spec(&PIP.replace("critter", "dragon"));
    f.press("new-character");
    assert!(f.app.model(|m| m.unsaved()));
    f.type_spec(PIP);
    f.press("new-character");
    f.app.run_writes();
    assert!(!f.app.model(|m| m.unsaved()));
    assert!(draft(&f).is_none(), "the library holds it now");
    assert_eq!(f.library().load_character("pip").unwrap().name, "Pip");
}

/// A mind's save of some other text does not touch the person's draft.
#[test]
fn a_minds_save_does_not_take_the_persons_draft() {
    let f = open();
    f.type_spec("{ \"typing\": true }");
    f.app.advance(yantrik_app::drafts::DEBOUNCE);
    f.app.run_writes();
    f.act("new_character", json!({ "spec": PIP })).unwrap();
    f.app.run_writes();
    assert!(draft(&f).unwrap().contains("typing"));
}

#[test]
fn an_emptied_editor_has_no_draft() {
    let f = open();
    f.type_spec("{ \"a\": 1 }");
    f.app.advance(yantrik_app::drafts::DEBOUNCE);
    f.app.run_writes();
    assert!(draft(&f).is_some());
    f.type_spec("   ");
    f.app.run_writes();
    assert!(draft(&f).is_none());
}
