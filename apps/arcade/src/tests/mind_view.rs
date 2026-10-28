//! Drawn for a Mind (Mind View, `Boot::for_mind`): the window is read-only, because its buttons run
//! as the person with no consent card (the `delete` button skips the grade its intent has), and
//! the person's unsaved text is not recovered into it. The Mind's own way in is the intents.

use serde_json::json;
use yantrik_app::drafts::path_in;
use yantrik_app::for_mind::READ_ONLY;
use yantrik_app::{App as _, Paths};
use yantrik_testing::TestApp;

use super::{meadow_run, open, seeded, PIP};
use crate::Arcade;

fn start_for_mind(paths: Paths) -> TestApp<Arcade> {
    let app = TestApp::<Arcade>::start_for_mind(paths, Vec::new(), std::sync::Arc::new(yantrik_app::mind::fake::FakeMind::new()));
    app.tick();
    app
}

#[test]
fn the_windows_buttons_and_editor_are_read_only_to_a_mind() {
    let f = open();
    seeded(&f);
    let app = start_for_mind(f.paths.clone());
    app.ui(|ui| ui.set_game_name("meadow-run".into()));
    let notice = |app: &TestApp<Arcade>| app.ui(|ui| ui.get_notice().to_string());
    for id in ["delete", "build", "new-game", "play"] {
        app.ui(|ui| ui.set_notice("".into()));
        app.ui(move |ui| ui.invoke_action(id.into()));
        app.tick();
        assert_eq!(notice(&app), READ_ONLY, "{id}: the window says so");
    }
    // A typed edit is refused and does not stay in the field: it is drawn again from the model.
    app.ui(|ui| {
        ui.set_spec_text("typed by a mind".into());
        ui.invoke_spec_edited("typed by a mind".into());
    });
    app.tick();
    assert_eq!(app.ui(|ui| ui.get_spec_text().to_string()), "", "the refused text is gone from the field");
    assert_eq!(notice(&app), READ_ONLY);
    app.ui(|ui| ui.set_notice("".into()));
    app.ui(|ui| ui.invoke_pick_character("pip".into()));
    app.tick();
    assert_eq!(notice(&app), READ_ONLY);
    assert_eq!(app.ui(|ui| ui.get_spec_text().to_string()), "");
    assert!(f.library().game_dir("meadow-run").exists(), "the game is where it was");
}

#[test]
fn the_intents_stay_open_to_a_mind_at_their_grades() {
    let paths = Paths::temporary(Arcade::ID);
    let app = start_for_mind(paths);
    let pending = app.act("new_character", json!({ "spec": PIP })).unwrap();
    app.tick();
    app.run_jobs();
    app.tick();
    assert!(pending.settle().is_ok());
    let _ = meadow_run("x");
}

#[test]
fn the_persons_unsaved_text_is_not_put_in_a_minds_editor() {
    let f = open();
    f.type_spec("{ \"private\": \"draft\" }");
    f.app.close_requested(yantrik_app::app::WindowId::PRIMARY);
    f.app.run_writes();
    assert!(path_in(f.paths.state_dir(), "spec").exists());
    let mind = start_for_mind(f.paths.clone());
    assert_eq!(mind.ui(|ui| ui.get_spec_text().to_string()), "", "nothing of it in the Mind's copy");
    assert_eq!(mind.ui(|ui| ui.get_notice().to_string()), "");
    // The person's own copy still gets it back.
    let f = f.restart();
    assert!(f.app.ui(|ui| ui.get_spec_text().to_string()).contains("draft"));
}
