//! One picture: its record in the right-hand pane, the Trash, opening it, the folder as it
//! changes, and which files an agent may name.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::json;
use yantrik_app::app::WindowId;
use yantrik_app::desktop::DesktopOutcome;
use yantrik_app::effects::Request;

use super::{apply, apply_as_agent, bare, fake};
use crate::backend;
use crate::command::{Command, Found, Next};
use crate::gallery::{self, Listed, Sidecar};
use crate::model::file_name;
use crate::update::POLL_EVERY;

fn detail(f: &super::Fixture) -> String {
    f.app.ui(|ui| ui.get_detail().to_string())
}

#[test]
fn choosing_a_row_reads_its_record_back_and_choosing_it_again_puts_the_pane_back() {
    let f = fake("choose");
    f.act("generate", json!({ "prompt": "a lighthouse in fog", "seed": 3, "width": 256, "height": 192, "steps": 4 }))
        .unwrap();
    assert_eq!(f.settled(), "");
    assert_eq!(detail(&f), "", "the right pane starts as an invitation, not as a record");

    f.app.run(Command::Choose(0));
    // The row shows as chosen at once; its record is read by a worker and drawn when it lands.
    assert!(f.rows()[0].selected);
    f.quiet();
    let shown = detail(&f);
    assert!(shown.contains("Prompt: a lighthouse in fog"), "{shown}");
    assert!(shown.contains("Kept out: "), "{shown}");
    assert!(shown.contains("Seed: 3"), "{shown}");
    assert!(shown.contains("Size: 256\u{d7}192"), "{shown}");
    assert!(shown.contains("Steps: 4"), "{shown}");
    assert!(shown.contains("Made by: placeholder"), "{shown}");
    assert!(!shown.contains("fake"), "{shown}");
    assert!(shown.contains("Took: "), "{shown}");
    assert!(shown.contains("When: "), "{shown}");

    // A second click on a selected row is what a person does to undo it.
    f.app.run(Command::Choose(0));
    f.quiet();
    assert_eq!(detail(&f), "");
    assert!(!f.rows()[0].selected);
}

/// A record that arrives for a row the person has since left is not drawn over the row they chose.
#[test]
fn a_record_for_a_row_the_person_has_left_is_not_drawn() {
    let f = fake("choose-late");
    f.act("generate", json!({ "prompt": "first", "seed": 1, "count": 2, "width": 64, "height": 64 })).unwrap();
    assert_eq!(f.settled(), "");
    let prompts: Vec<String> = f.app.model(|m| m.gallery.iter().map(|row| row.prompt.clone()).collect());
    assert_eq!(prompts.len(), 2);
    // Two reads in flight, for two rows, answered in order: the later choice is the one that stays.
    f.app.run(Command::Choose(0));
    f.app.run(Command::Choose(1));
    // The first read lands first, for a row that is no longer the chosen one: nothing is drawn.
    assert!(f.app.run_one_job());
    assert_eq!(detail(&f), "", "a record for a row the person had left was drawn");
    f.quiet();
    let seed_of_row_1 = f.app.model(|m| m.gallery[1].seed);
    assert!(detail(&f).contains(&format!("Seed: {seed_of_row_1}")), "{}", detail(&f));
    assert!(f.rows()[1].selected);
}

#[test]
fn a_chosen_picture_that_is_deleted_leaves_the_pane_empty() {
    let f = fake("choose-delete");
    let name = f.one_picture("a harbour");
    f.app.run(Command::Choose(0));
    f.quiet();
    assert!(!detail(&f).is_empty());
    f.act("delete", json!({ "path": name })).unwrap();
    assert_eq!(detail(&f), "");
    assert!(f.rows().is_empty(), "the deleted picture is still on screen");
}

#[test]
fn deleting_moves_the_picture_and_its_record_to_the_trash_and_nothing_is_read_on_the_ui_thread() {
    let f = fake("delete");
    f.act("generate", json!({ "prompt": "a picture to regret", "width": 64, "height": 64 })).unwrap();
    assert_eq!(f.settled(), "");
    let path = f.app.model(|m| m.gallery[0].path.clone());
    // The command only asks a worker: until it runs, the picture is where it was.
    f.app.run(Command::Delete { of: path.display().to_string() });
    assert!(path.exists(), "the update moved a file itself");
    f.quiet();
    assert!(!path.exists());
    assert!(!Sidecar::path_for(&path).exists(), "the record was left beside a picture that is gone");
    assert!(f.app.model(|m| m.gallery.is_empty()), "the gallery still shows it");
    assert!(f.notice().contains("Trash"), "{:?}", f.notice());
    let listed = crate::trash::items(&f.world.places.trash).unwrap();
    assert_eq!(listed.len(), 2, "{listed:?}");
}

#[test]
fn this_app_will_not_delete_a_file_outside_its_own_gallery() {
    let f = fake("outside");
    let elsewhere = f.world.dir.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let picture = elsewhere.join("important.png");
    std::fs::write(&picture, backend::draw("x", 1, 16, 16)).unwrap();
    f.app.run(Command::Delete { of: picture.display().to_string() });
    f.quiet();
    assert!(f.notice().contains("not in Studio's gallery"), "{}", f.notice());
    assert!(f.notice().contains("Files can delete anything"), "{}", f.notice());
    assert!(picture.exists(), "it was deleted anyway");
}

#[test]
fn a_bare_filename_means_the_gallery_and_a_path_means_itself() {
    let f = fake("names");
    f.act("generate", json!({ "prompt": "a lighthouse", "width": 64, "height": 64 })).unwrap();
    assert_eq!(f.settled(), "");
    let path = f.app.model(|m| m.gallery[0].path.clone());
    let name = file_name(&path);
    let canonical = std::fs::canonicalize(&path).unwrap();
    for named in [name.clone(), path.display().to_string()] {
        let found = f.act("upscale", json!({ "path": named, "factor": 2 })).unwrap();
        assert_eq!(found["of"], json!(named));
        assert_eq!(f.settled(), "");
    }
    let made: Vec<String> = f.app.model(|m| m.gallery.iter().map(|row| row.made_from.clone()).collect());
    assert_eq!(made.iter().filter(|each| **each == format!("upscale of {name}")).count(), 2, "{made:?}");
    assert!(canonical.exists());
}

#[test]
fn a_file_that_is_not_a_picture_is_not_treated_as_one() {
    let f = fake("not-a-picture");
    std::fs::write(f.world.dir.join("notes.txt"), "hello").unwrap();
    f.app.run(Command::Delete { of: f.world.dir.join("notes.txt").display().to_string() });
    f.quiet();
    assert!(f.notice().contains("not a picture"), "{}", f.notice());
    assert!(f.world.dir.join("notes.txt").exists(), "it was moved anyway");
    f.app.run(Command::Delete { of: f.world.dir.display().to_string() });
    f.quiet();
    assert!(f.notice().contains("a folder"), "{}", f.notice());
}

/// `open` hands a resolved picture to the desktop, and says so only once the desktop has: nothing
/// is spawned by the update, and the answer is the desktop's own outcome.
#[test]
fn open_is_answered_by_the_desktop_not_by_the_update() {
    let (world, mut m) = bare("open", r#"{"backend":{"kind":"fake"}}"#);
    let path = world.dir.join("seen.png");
    std::fs::write(&path, backend::draw("x", 1, 16, 16)).unwrap();

    let fx = apply_as_agent(&mut m, Command::Resolved { next: Next::Open, result: Ok(Found { path: path.clone(), sidecar: None }) });
    let asked: Vec<_> = fx.requests().iter().map(Request::name).collect();
    assert_eq!(asked, ["spawn"], "the desktop is asked on a worker, and nothing is answered yet");
    assert_eq!(m.answer, serde_json::Value::Null);

    let fx = apply_as_agent(&mut m, Command::Opened { shown: "seen.png".into(), outcome: DesktopOutcome::Done });
    assert_eq!(m.answer, json!({ "opened": "seen.png" }));
    assert!(fx.requests().iter().any(|r| r.name() == "settle"));

    let fx = apply_as_agent(
        &mut m,
        Command::Opened { shown: "seen.png".into(), outcome: DesktopOutcome::Failed("no opener".into()) },
    );
    assert_eq!(m.notice, "no opener");
    assert!(matches!(fx.requests().last(), Some(Request::Settle(Err(_)))));
}

#[test]
fn a_picture_somebody_else_puts_in_the_folder_turns_up_in_the_gallery() {
    let f = fake("poll");
    assert!(f.app.model(|m| m.gallery.is_empty()));
    let day = gallery::day_folder(&f.gallery(), chrono::Local::now());
    std::fs::create_dir_all(&day).unwrap();
    std::fs::write(day.join("copied-in.png"), backend::draw("a harbour", 5, 32, 32)).unwrap();

    // The poll's timer comes round; the folder is read on a worker.
    f.app.advance(POLL_EVERY + Duration::from_millis(10));
    f.quiet();
    let rows = f.app.model(|m| m.gallery.clone());
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].name, "copied-in.png");
    assert!(!rows[0].has_sidecar, "a record was invented for a file that has none");
    assert_eq!(rows[0].json()["sidecar"], json!("missing"));
}

#[test]
fn being_brought_to_the_front_looks_at_the_folder_again() {
    let f = fake("activated");
    let day = gallery::day_folder(&f.gallery(), chrono::Local::now());
    std::fs::create_dir_all(&day).unwrap();
    std::fs::write(day.join("while-away.png"), backend::draw("a harbour", 5, 32, 32)).unwrap();
    f.app.deactivate(WindowId::PRIMARY);
    f.app.activate(WindowId::PRIMARY);
    f.quiet();
    assert_eq!(f.app.model(|m| m.gallery.len()), 1, "what arrived while the window was behind was not noticed");
}

/// An unchanged folder is not announced as a change, which is what keeps an idle window from
/// redrawing and an idle app from costing a share of a core.
#[test]
fn a_poll_that_finds_the_same_pictures_redraws_nothing() {
    let (world, mut m) = bare("quiet-poll", r#"{"backend":{"kind":"fake"}}"#);
    let same = Listed { stamp: "x".into(), content: m.content.clone(), rows: m.gallery.clone() };
    let fx = apply(&mut m, Command::Listed(Box::new(same)));
    assert!(fx.changed_parts().is_empty(), "{:?}", fx.changed_parts());
    let fx = apply(&mut m, Command::Unchanged);
    assert!(fx.changed_parts().is_empty());
    drop(world);
}

/// Review of #942, finding 1: an agent names only files in the person's home, outside its protected
/// places. The person is not asked; a bare filename is a picture in the gallery.
#[test]
fn an_agent_names_only_files_in_the_home() {
    let f = fake("agent-files");
    let outside: PathBuf = f.world.dir.join("outside.png");
    std::fs::write(&outside, backend::draw("SENTINEL", 1, 16, 16)).unwrap();
    let outside = outside.display().to_string();

    // The person is not asked.
    f.act("upscale", json!({ "path": &outside })).unwrap();
    assert_eq!(f.settled(), "");

    for (action, key) in [("upscale", "path"), ("open", "path"), ("variations", "of")] {
        for asked in [
            outside.as_str(),
            "~/Pictures/Screenshots/shot.png",
            "~/.local/share/yantrik/snippets/snippets.json",
            "/etc/hostname",
        ] {
            let problem = f.agent(action, json!({ key: asked })).unwrap_err();
            assert!(problem.starts_with("an agent names only a file in the person's home"), "{action} {asked}: {problem}");
        }
    }
    // `set_backend` is the person's alone (tests/backends.rs): an agent naming a workflow is
    // refused before the file is looked at.
    let problem = f
        .agent("set_backend", json!({ "kind": "comfyui", "workflow": "/etc/hostname" }))
        .unwrap_err();
    assert_eq!(problem, crate::update::AGENT_REFUSED);
    assert!(!f.world.places.settings.as_ref().unwrap().exists(), "a refused change was written");

    // A bare name is a picture in the gallery, resolved there; delete keeps to the gallery itself.
    let problem = f.agent("variations", json!({ "of": "studio-1.png" })).unwrap_err();
    assert!(problem.contains("there is no studio-1.png in the gallery"), "{problem}");
    let problem = f.agent("delete", json!({ "path": &outside })).unwrap_err();
    assert!(problem.contains("only deletes its own pictures"), "{problem}");
    assert!(PathBuf::from(&outside).exists());
}

/// A hand-edited `studio.json` that exists and cannot be used says so where the person and a Mind
/// look, not only in the log: which file, and what is wrong. The variable's value is never said.
#[test]
fn a_configuration_that_was_not_used_says_which_file_and_what_is_wrong() {
    const NAME: &str = "STUDIO-BAD-KEY";
    std::env::set_var(NAME, "sk-never-said-5521");
    let dir = std::env::temp_dir().join(format!("studio-refused-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("studio.json");
    std::fs::write(
        &file,
        format!(r#"{{"backend":{{"kind":"openai-images","model":"m","api_key_env":"{NAME}"}}}}"#),
    )
    .unwrap();
    let config = crate::config::Config::read(&file);
    assert_eq!(config.refused.as_deref().map(|s| s.contains("was not used")), Some(true));

    let f = super::with_config("refused-config", config);
    let said = f.notice();
    let window = f.app.ui(|ui: &crate::StudioApp| (ui.get_notice().to_string(), ui.get_facts().to_string()));
    let described = f.describe();
    for (where_, text) in [
        ("the notice", said.clone()),
        ("the window's notice", window.0.clone()),
        ("the pane", window.1.clone()),
        ("describe", described["state"].to_string()),
        ("the summary", f.summary()),
    ] {
        assert!(text.contains("studio.json was not used"), "{where_}: {text}");
        assert!(text.contains("has to be the name of an environment variable"), "{where_}: {text}");
        // What the field held is never repeated, not even a name that is only hyphenated.
        assert!(!text.contains(NAME), "{where_} repeats what the field held: {text}");
        assert!(!text.contains("sk-never-said"), "{where_} says the value: {text}");
    }
    assert_eq!(window.0, said, "the window and the model disagree");
    assert_eq!(described["state"]["backend"]["note"].as_str().unwrap(), described["state"]["notice"].as_str().unwrap());
    // The app still runs, on placeholders.
    assert_eq!(described["state"]["backend"]["kind"], serde_json::json!("fake"));
    std::env::remove_var(NAME);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A key pasted into `api_key_env` (the commonest mistake in this file) is never repeated: not in
/// the window, not in `describe` as the person or as an agent, not in the summary, not in the log,
/// and not in what `set_backend` refuses with.
#[test]
fn a_key_pasted_where_a_variable_name_goes_is_said_nowhere() {
    use std::sync::{Arc, Mutex};
    use yantrik_testing::Call;
    const SECRET: &str = "SECRET123";
    let dir = std::env::temp_dir().join(format!("studio-pasted-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("studio.json");
    std::fs::write(
        &file,
        format!(r#"{{"backend":{{"kind":"openai-images","model":"m","api_key_env":"sk-proj-{SECRET}"}}}}"#),
    )
    .unwrap();

    // What the app logs while it reads the file.
    #[derive(Clone, Default)]
    struct Log(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Log {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let log = Log::default();
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt().with_writer(move || writer.clone()).with_max_level(tracing::Level::TRACE).finish();
    let config = tracing::subscriber::with_default(subscriber, || {
        // Another test may have reached the same log line with nobody listening, and the callsite
        // remembers that; ask it again now that someone is.
        tracing::callsite::rebuild_interest_cache();
        crate::config::Config::read(&file)
    });
    let logged = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
    assert!(logged.contains("is not usable"), "the read logged nothing, so the check below proves nothing: {logged:?}");
    assert!(!logged.contains(SECRET), "the log repeats the key: {logged}");
    let said = config.refused.clone().expect("the file was refused");
    assert!(said.contains("not the key itself") && !said.contains(SECRET) && !said.contains("sk-"), "{said}");

    let f = super::with_config("pasted-key", config);
    let window = f.app.ui(|ui: &crate::StudioApp| format!("{} {} {}", ui.get_notice(), ui.get_facts(), ui.get_backend_line()));
    let as_person = f.describe().to_string();
    // A Mind's account reads describe too: the same text, asked for as an agent.
    let pending = f.app.act_as(Call::new("refresh", json!({})).as_agent(super::AGENT)).unwrap();
    let as_agent = f.settle(pending).unwrap().to_string();
    let after_agent = f.describe().to_string();
    for (where_, text) in [
        ("the window", window),
        ("describe", as_person),
        ("an agent's answer", as_agent),
        ("describe after an agent's call", after_agent),
        ("the summary", f.summary()),
        ("the notice", f.notice()),
    ] {
        assert!(!text.contains(SECRET) && !text.contains("sk-proj"), "{where_} repeats the key: {text}");
    }

    // And what `set_backend` refuses with, for the person and for the file's reader.
    let refused = f
        .act("set_backend", json!({ "kind": "openai-images", "model": "m", "api_key_env": format!("sk-proj-{SECRET}") }))
        .unwrap_err();
    assert!(refused.contains("not the key itself") && !refused.contains(SECRET), "{refused}");
    assert!(!f.notice().contains(SECRET), "{}", f.notice());
    let kind = f.act("set_backend", json!({ "kind": format!("sk-proj-{SECRET}") })).unwrap_err();
    assert!(!kind.contains(SECRET), "{kind}");
    let kind_in_file = crate::config::Config::parse(&format!(r#"{{"backend":{{"kind":"sk-proj-{SECRET}"}}}}"#)).unwrap_err();
    assert!(!kind_in_file.contains(SECRET), "{kind_in_file}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Many real keys (`hf_...`, `ghp_...`) are valid variable names, so a key pasted into `api_key_env`
/// is accepted as a name. It is said nowhere: a name is said only for a variable that is set here.
#[test]
fn a_key_shaped_like_a_variable_name_is_said_nowhere_and_a_set_variable_is_named() {
    use std::sync::{Arc, Mutex};
    use yantrik_testing::Call;
    const KEY: &str = "hf_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789";
    std::env::remove_var(KEY);
    let dir = std::env::temp_dir().join(format!("studio-hf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("studio.json");
    std::fs::write(
        &file,
        format!(r#"{{"backend":{{"kind":"openai-images","base_url":"https://router.example/v1","model":"m","api_key_env":"{KEY}"}}}}"#),
    )
    .unwrap();

    #[derive(Clone, Default)]
    struct Log(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Log {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let log = Log::default();
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt().with_writer(move || writer.clone()).with_max_level(tracing::Level::TRACE).finish();
    // It is a valid name, so the file is accepted as it is written (the person's config still works
    // once they set the variable), and what the app logs about it repeats nothing.
    let config = tracing::subscriber::with_default(subscriber, || {
        tracing::callsite::rebuild_interest_cache();
        let config = crate::config::Config::read(&file);
        tracing::warn!("{}", config.backend.api_key().unwrap_err());
        config
    });
    assert!(config.refused.is_none());
    assert_eq!(config.backend.api_key_env, KEY, "what the person gave is kept");
    let logged = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
    assert!(logged.contains("is not set"), "nothing was logged to check: {logged:?}");
    assert!(!logged.contains("hf_"), "the log repeats the key: {logged}");

    let f = super::with_config("hf-key", config);
    let clean = |where_: &str, text: String| {
        assert!(!text.contains("hf_") && !text.contains("AbCdEf"), "{where_} repeats the key: {text}");
    };
    // A generate that fails for want of the variable.
    f.act("generate", json!({ "prompt": "a lighthouse", "width": 64, "height": 64 })).unwrap();
    let failed = f.settled();
    assert!(failed.contains("could not make the picture"), "{failed}");
    assert!(failed.contains("the environment variable named in studio.json's `api_key_env` is not set"), "{failed}");
    let window = f.app.ui(|ui: &crate::StudioApp| format!("{} {} {}", ui.get_notice(), ui.get_facts(), ui.get_backend_line()));
    let pending = f.app.act_as(Call::new("refresh", json!({})).as_agent(super::AGENT)).unwrap();
    let as_agent = f.settle(pending).unwrap().to_string();
    let (set_backend, _) = crate::intents::actions().into_iter().find(|(a, _)| a.name == "set_backend").unwrap();
    let card = set_backend
        .explainer
        .as_ref()
        .unwrap()
        .sentence(&json!({ "kind": "openai-images", "base_url": "https://router.example/v1", "api_key_env": KEY }));
    // The person's set_backend with the same value: accepted, and nothing it says repeats it.
    let answer = f.act("set_backend", json!({ "kind": "openai-images", "model": "m", "api_key_env": KEY })).unwrap();
    for (where_, text) in [
        ("the notice after a failed generate", failed),
        ("the window", window),
        ("an agent's answer", as_agent),
        ("describe", f.describe().to_string()),
        ("the summary", f.summary()),
        ("the card", card.clone()),
        ("set_backend's answer", answer.to_string()),
        ("the notice after set_backend", f.notice()),
        (
            "the window after set_backend",
            f.app.ui(|ui: &crate::StudioApp| format!("{} {}", ui.get_notice(), ui.get_facts())),
        ),
    ] {
        clean(where_, text);
    }
    assert_eq!(answer["api_key_env"], serde_json::Value::Null);
    assert_eq!(f.state()["backend"]["api_key_env"], serde_json::Value::Null);
    assert_eq!(f.state()["backend"]["api_key_is_set"], json!(false));
    assert!(card.contains("the environment variable you named"), "{card}");

    // A variable that IS set is named: in the card, in describe, in the answer and in the pane.
    const SET: &str = "STUDIO_TEST_SET_VARIABLE_7731";
    std::env::set_var(SET, "a-value-that-is-never-said-8812");
    let answer = f.act("set_backend", json!({ "kind": "openai-images", "model": "m", "api_key_env": SET })).unwrap();
    assert_eq!(answer["api_key_env"], json!(SET));
    assert_eq!(f.state()["backend"]["api_key_env"], json!(SET));
    assert_eq!(f.state()["backend"]["api_key_is_set"], json!(true));
    let pane = f.app.ui(|ui: &crate::StudioApp| ui.get_facts().to_string());
    assert!(pane.contains(&format!("the environment variable {SET}, which is set")), "{pane}");
    let card = set_backend.explainer.as_ref().unwrap().sentence(&json!({ "kind": "openai-images", "api_key_env": SET }));
    assert_eq!(card, format!("After this, prompts and the value of ${SET} go to api.openai.com."));
    for text in [answer.to_string(), f.describe().to_string(), pane, card] {
        assert!(!text.contains("a-value-that-is-never-said"), "the value is said: {text}");
    }
    std::env::remove_var(SET);
    let _ = std::fs::remove_dir_all(&dir);
}
