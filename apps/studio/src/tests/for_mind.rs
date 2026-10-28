//! A window drawn for a Mind (Mind View): the sentences and the pictures are hidden everywhere they
//! could be read, and nothing is typed, asked for, filed or moved through it. The window says why,
//! as well as the caller, and what was refused does not stay in the field.

use std::sync::Arc;

use serde_json::json;
use yantrik_app::for_mind::{HIDDEN, READ_ONLY};
use yantrik_mind::fake::FakeMind;
use yantrik_testing::TestApp;

use super::window::{press, type_into};
use super::{apply, bare, world, Fixture};
use crate::command::{BackendAsk, Command, Which};
use crate::config::Config;
use crate::gallery::{self, Sidecar};
use crate::model::Field;
use crate::plan::Plan;
use crate::{backend, describe, words, Studio, StudioApp};

const SENTINEL: &str = "SENTINEL-951";

/// A gallery that already holds a picture whose sentence is the sentinel, as the person made it
/// before the Mind looks, and an app drawn for a Mind over it.
fn mind_copy() -> Fixture {
    let world = world::make("for-mind", Config::parse(r#"{"backend":{"kind":"fake"}}"#).unwrap());
    let day = gallery::day_folder(&world.places.gallery, chrono::Local::now());
    std::fs::create_dir_all(&day).unwrap();
    let picture = day.join("made-before.png");
    std::fs::write(&picture, backend::draw(SENTINEL, 3, 64, 64)).unwrap();
    Sidecar {
        prompt: format!("a picture of {SENTINEL}"),
        negative: format!("not {SENTINEL}"),
        seed: 3,
        backend: "fake".into(),
        model: "fake".into(),
        seconds: 0.1,
        width: 64,
        height: 64,
        sent: String::new(),
        created: chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false),
        made_from: String::new(),
        steps: Some(4),
        cfg: None,
        made_by: "yantrik-studio".into(),
    }
    .write(&picture)
    .unwrap();
    let mind = Arc::new(FakeMind::new().answering("never asked"));
    let app = TestApp::<Studio>::start_for_mind(world.paths.clone(), Vec::new(), mind);
    let f = Fixture { app, world };
    f.until("the grade to be published", |f| f.app.model(|m| m.graded));
    f
}

fn leaks(text: &str) -> bool {
    text.contains(SENTINEL)
}

#[test]
fn the_sentences_and_the_pictures_are_hidden_everywhere_they_could_be_read() {
    let f = mind_copy();
    assert_eq!(f.app.model(|m| m.gallery.len()), 1, "the person's picture is in the gallery");

    // `describe` keeps what a Mind may know of any app of the person's, and masks the sentence.
    let d = f.describe();
    assert!(!leaks(&d.to_string()), "{d}");
    assert_eq!(d["state"]["gallery"]["newest"][0]["prompt"], json!(HIDDEN));
    assert_eq!(d["state"]["gallery"]["newest"][0]["seed"], json!(3), "a seed is not the person's sentence");
    assert_eq!(d["state"]["gallery"]["count"], json!(1));

    // The window shows the one sentence where a sentence would be, and draws no picture.
    let row = f.rows().remove(0);
    assert_eq!(row.prompt, HIDDEN);
    assert!(!row.has_thumb, "a thumbnail is the person's picture");
    assert!(!leaks(&row.meta) && !leaks(&row.name), "{row:?}");

    // Choosing a row reads nothing, and the pane holds the mask.
    f.app.ui(|ui: &StudioApp| ui.invoke_choose(0));
    f.quiet();
    assert!(f.rows()[0].selected);
    let detail = f.app.ui(|ui: &StudioApp| ui.get_detail().to_string());
    assert_eq!(detail, HIDDEN);

    let everything = f.app.ui(|ui: &StudioApp| {
        [
            ui.get_backend_line().to_string(),
            ui.get_facts().to_string(),
            ui.get_status().to_string(),
            ui.get_notice().to_string(),
            ui.get_prompt().to_string(),
            ui.get_negative().to_string(),
        ]
        .join("\n")
    });
    assert!(!leaks(&everything), "{everything}");
}

#[test]
fn nothing_is_typed_asked_for_filed_or_moved_through_it_and_the_window_says_why() {
    let f = mind_copy();
    let before = std::fs::read_dir(gallery::day_folder(&f.gallery(), chrono::Local::now())).unwrap().count();

    // Typing: the text does not stay in the box, and the model never held it.
    type_into(&f, "prompt", SENTINEL);
    assert_eq!(f.notice(), READ_ONLY, "the window says it, not only the log");
    assert_eq!(f.app.ui(|ui: &StudioApp| ui.get_prompt().to_string()), "", "the refused text stayed in the field");
    assert_eq!(f.app.model(|m| m.form.prompt.clone()), "");
    for field in ["negative", "width", "height", "steps", "count"] {
        let before = f.app.ui(|ui: &StudioApp| (ui.get_negative().to_string(), ui.get_want_width().to_string()));
        type_into(&f, field, SENTINEL);
        let after = f.app.ui(|ui: &StudioApp| (ui.get_negative().to_string(), ui.get_want_width().to_string()));
        assert_eq!(before, after, "{field}: the refused text stayed in the field");
    }

    let ask = BackendAsk {
        kind: "comfyui".into(),
        base_url: "http://203.0.113.7:8188".into(),
        model: String::new(),
        api_key_env: String::new(),
        workflow: String::new(),
    };
    let path = f.app.model(|m| m.gallery[0].path.display().to_string());
    let refused = [
        Command::Edit(Field::Prompt, SENTINEL.into()),
        Command::Generate(Some(Box::new(Plan::new(SENTINEL)))),
        Command::Generate(None),
        Command::Variations { of: path.clone(), count: 1, fill: true },
        Command::Upscale { of: path.clone(), factor: 2 },
        Command::Open { of: path.clone() },
        Command::OpenFolder,
        Command::Delete { of: path.clone() },
        Command::SetBackend(Box::new(ask)),
        // Stopping the person's own render changes their work.
        Command::Cancel(Which::All),
        Command::Cancel(Which::One(1)),
    ];
    for command in refused {
        // A refresh clears the notice, so each refusal is its own.
        f.app.run(Command::Refresh);
        f.quiet();
        f.app.run(command);
        f.quiet();
        assert_eq!(f.notice(), READ_ONLY, "the window says it, not only the log");
    }
    // The buttons say it too.
    for id in ["generate", "folder", &format!("open:{path}"), &format!("delete:{path}"), &format!("upscale:{path}")] {
        f.app.run(Command::Refresh);
        f.quiet();
        press(&f, id);
        f.quiet();
        assert_eq!(f.app.ui(|ui: &StudioApp| ui.get_notice().to_string()), READ_ONLY, "{id}");
    }

    // Nothing was made, filed, moved or written, and no job was started.
    assert!(f.app.model(|m| m.queue.is_empty()));
    assert_eq!(std::fs::read_dir(gallery::day_folder(&f.gallery(), chrono::Local::now())).unwrap().count(), before);
    assert!(std::path::Path::new(&path).exists(), "a picture was moved");
    assert!(!f.world.places.settings.as_ref().unwrap().exists(), "the settings were written");
    assert!(!f.world.places.trash.exists(), "the Trash was touched");
}

#[test]
fn a_mind_calling_the_actions_is_refused_with_the_same_sentence() {
    let f = mind_copy();
    let path = f.app.model(|m| m.gallery[0].path.display().to_string());
    for (name, args) in [
        ("generate", json!({ "prompt": SENTINEL })),
        ("variations", json!({ "of": path.clone() })),
        ("upscale", json!({ "path": path.clone() })),
        ("open", json!({ "path": path.clone() })),
        ("delete", json!({ "path": path.clone() })),
        ("cancel", json!({ "job": 1 })),
        ("cancel_all", json!({})),
    ] {
        assert_eq!(f.agent(name, args), Err(READ_ONLY.to_string()), "{name}");
    }
    // `set_backend` is the person's alone: an agent is refused it before Mind View is asked.
    assert_eq!(f.agent("set_backend", json!({ "kind": "fake" })), Err(crate::update::AGENT_REFUSED.to_string()));
    // Reading is not refused.
    assert!(f.agent("refresh", json!({})).is_ok());
}

/// A job's label is the person's sentence: the queue shows its shape and not its words.
#[test]
fn the_queue_shows_its_shape_and_not_the_sentence() {
    let (_world, mut m) = bare("mind-queue", r#"{"backend":{"kind":"fake"}}"#);
    apply(&mut m, Command::Generate(Some(Box::new(Plan { count: 2, ..Plan::new(SENTINEL) }))));
    assert!(leaks(&describe::state(&m).to_string()), "the person's own copy shows the label");
    m.for_mind = true;
    let state = describe::state(&m);
    assert!(!leaks(&state.to_string()), "{state}");
    assert_eq!(state["queue"]["running"][0]["label"], json!(HIDDEN));
    assert_eq!(state["queue"]["running"][0]["progress"], json!("0 of 2"));
    assert!(!leaks(&describe::summary(&m)));
    assert!(!leaks(&words::status_line(&m)), "{}", words::status_line(&m));
    assert!(words::status_line(&m).contains(HIDDEN));
}
