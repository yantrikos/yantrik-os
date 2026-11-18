//! The whole path on the backend every machine has: a sentence becomes a file, a record and an
//! answer a Mind can read back; the actions on one picture; and one run from start to finish.

use std::path::PathBuf;

use serde_json::json;

use super::{fake, unconfigured, Fixture};
use crate::backend;
use crate::config;
use crate::gallery::{self, Sidecar};
use crate::model::display;
use crate::plan;
use crate::trash;

#[test]
fn a_sentence_becomes_a_file_a_record_and_an_answer_a_mind_can_read_back() {
    let f = fake("path");
    let pending = f
        .app
        .act(
            "generate",
            json!({
                "prompt": "a lighthouse in fog",
                "negative": "blurry, text",
                "seed": 7,
                "count": 2,
                // A string, because that is how a number arrives about half the time, and a
                // refusal about JSON would be a refusal about nothing.
                "width": "1025",
                "height": 700,
                "steps": 25,
            }),
        )
        .unwrap();

    // While it runs, the queue is the only thing that says so: the worker has not run yet.
    let busy = f.describe();
    let running = busy["state"]["queue"]["running"].as_array().unwrap();
    assert_eq!(running.len(), 1, "{busy}");
    assert_eq!(running[0]["kind"], json!("generate"));
    assert_eq!(running[0]["status"], json!("running"));
    assert_eq!(running[0]["progress"], json!("0 of 2"));
    assert!(busy["summary"].as_str().unwrap().contains("1 picture being made"), "{}", busy["summary"]);

    // The call starts the work and says so, rather than pretending to have finished it.
    let answer = f.settle(pending).unwrap();
    assert_eq!(answer["queued"], json!(true));
    assert_eq!(answer["count"], json!(2));
    assert_eq!(answer["backend"], json!("fake"));
    assert_eq!(answer["where"], json!("this machine"));
    assert_eq!(answer["prompt_leaves_this_machine"], json!(false));
    assert!(answer["job"].as_i64().unwrap() >= 1, "{answer}");
    assert!(
        answer["read_back"].as_str().unwrap().contains("gallery.newest"),
        "the answer has to say where the result will turn up: {answer}"
    );
    assert_eq!(f.settled(), "", "a generation that worked left a notice behind");

    // ── describe, which is what a mind reads ──
    let rendered = f.describe();
    let summary = rendered["summary"].as_str().unwrap();
    assert!(summary.starts_with("Studio \u{2014} "), "{summary}");
    assert!(summary.contains("2 pictures in"), "{summary}");
    assert!(summary.contains("placeholder images"), "{summary}");

    let state = &rendered["state"];
    assert_eq!(state["backend"]["kind"], json!("fake"));
    assert_eq!(state["backend"]["where"], json!("this machine"));
    assert_eq!(state["backend"]["configured"], json!(true));
    assert_eq!(state["backend"]["generate_is_graded"], json!("standard"));
    assert_eq!(state["backend"]["prompt_leaves_this_machine"], json!(false));
    assert_eq!(state["output_folder"], json!(display(&f.gallery())));
    assert_eq!(state["gallery"]["count"], json!(2));
    // An empty queue is still shown: "nothing is running" and "I could not read it" have to be
    // different answers.
    assert_eq!(state["queue"]["running"], json!([]));
    assert_eq!(state["queue"]["pending"], json!([]));
    assert!(state.get("notice").is_none(), "a success left a notice: {state}");
    assert_eq!(rendered["revision"].as_str().unwrap().len(), 16);

    let newest = state["gallery"]["newest"].as_array().unwrap();
    assert_eq!(newest.len(), 2);
    // Seeds run on from the one given, so two pictures from `seed=7` are 8 and 7, newest first.
    assert_eq!(newest[0]["seed"], json!(8));
    assert_eq!(newest[1]["seed"], json!(7));
    for shot in newest {
        assert_eq!(shot["prompt"], json!("a lighthouse in fog"));
        assert_eq!(shot["backend"], json!("fake"));
        assert_eq!(shot["model"], json!("fake"));
        // The placeholder backend caps its own long edge at 1024 and keeps the shape, so a
        // 1025x700 ask comes back 1024x699, and the record says what was actually sent rather
        // than what was hoped for.
        assert_eq!(shot["size"], json!("1024x699"), "{shot}");
        assert!(shot["seconds"].as_f64().is_some(), "{shot}");
        assert!(shot["path"].as_str().unwrap().ends_with(".png"), "{shot}");
        assert!(shot["created"].as_str().unwrap().contains('T'), "{shot}");
    }

    // ── the files, which are the real thing ──
    let path = PathBuf::from(newest[0]["path"].as_str().unwrap());
    assert!(path.starts_with(f.gallery()), "{path:?}");
    assert!(path.is_file(), "describe names a file that is not there");
    assert_eq!(
        path.parent().unwrap().file_name().unwrap().to_string_lossy(),
        chrono::Local::now().format("%Y-%m-%d").to_string(),
        "pictures are filed by day, so a folder of thousands stays navigable"
    );
    assert_eq!(gallery::dimensions(&path), (Some(1024), Some(699)));

    let sidecar = Sidecar::read(&path).expect("no record beside the picture");
    assert_eq!(sidecar.prompt, "a lighthouse in fog");
    assert_eq!(sidecar.negative, "blurry, text");
    assert_eq!(sidecar.seed, 8);
    assert_eq!(sidecar.backend, "fake");
    assert_eq!(sidecar.model, "fake");
    assert_eq!(sidecar.width, 1024);
    assert_eq!(sidecar.height, 699);
    assert_eq!(sidecar.sent, "1024x699");
    assert_eq!(sidecar.steps, Some(25));
    assert_eq!(sidecar.cfg, Some(plan::DEFAULT_CFG));
    assert!(sidecar.made_from.is_empty());
    assert_eq!(sidecar.made_by, "yantrik-studio");
    assert!(sidecar.seconds.is_finite() && sidecar.seconds >= 0.0);
    assert!(Sidecar::path_for(&path).is_file(), "the record is a file of its own");
    // The bytes are the placeholder the fake backend draws from the prompt and the seed, which is
    // what makes "deterministic" a claim that can be checked rather than a word in a comment.
    assert_eq!(
        std::fs::read(&path).unwrap(),
        backend::draw("a lighthouse in fog", 8, 1024, 699),
        "the file on disk is not the picture the backend drew"
    );

    // ── and a second app over the same folder has to see the same thing ──
    //
    // A restart, or the poll, re-reads the gallery from the filesystem. Two pictures made inside
    // one second have the same `created`, so the order of a batch is exactly where a re-read and
    // the model could disagree, and then `gallery.newest` would name a different picture as the
    // newest depending on whether the folder had been read again since.
    let reopened = f.reopen();
    assert_eq!(reopened.paths(), f.paths(), "a re-read reordered the gallery");

    // `refresh` reads the folder again and answers with what it found.
    let refreshed = f.act("refresh", json!({})).unwrap();
    assert_eq!(refreshed["refreshed"], json!(true));
    assert_eq!(refreshed["gallery_count"], json!(2));
}

#[test]
fn a_machine_with_no_backend_configured_draws_placeholders_and_says_so() {
    // `Config::unconfigured` is what `load` returns when there is no file, which is the state every
    // machine that has not set Studio up is in. The app has to open, work, and be honest about it.
    let f = unconfigured("unconfigured");
    let rendered = f.describe();
    assert_eq!(rendered["state"]["backend"]["configured"], json!(false));
    assert_eq!(rendered["state"]["backend"]["kind"], json!("fake"));
    let note = rendered["state"]["backend"]["note"].as_str().unwrap();
    assert!(note.contains("No backend is configured"), "{note}");
    assert!(note.contains(config::FILE_NAME), "the note has to name the file that would fix it: {note}");
    assert!(note.contains("set_backend"), "{note}");
    assert!(rendered["summary"].as_str().unwrap().contains("no backend configured"), "{}", rendered["summary"]);
    // An unconfigured machine is still a local one: nothing it draws goes anywhere.
    assert_eq!(rendered["state"]["backend"]["generate_is_graded"], json!("standard"));
    assert_eq!(rendered["state"]["backend"]["prompt_leaves_this_machine"], json!(false));

    f.act("generate", json!({ "prompt": "a harbour at dusk", "width": 64, "height": 64 })).unwrap();
    assert_eq!(f.settled(), "");
    let state = f.state();
    assert_eq!(state["gallery"]["count"], json!(1));
    assert_eq!(state["gallery"]["newest"][0]["backend"], json!("fake"));
}

#[test]
fn the_actions_on_one_picture_vary_it_enlarge_it_and_put_it_in_the_trash() {
    let f = fake("row");
    let name = f.one_picture("a lighthouse in fog");
    let original = f.app.model(|m| m.gallery[0].path.clone());
    let original_seed = Sidecar::read(&original).unwrap().seed;

    // ── variations: the same sentence, new seeds ──
    // Named by its bare filename rather than its path, because "the lighthouse one" is how a
    // person refers to a picture and a path is how a machine does. Both have to work.
    let answer = f.act("variations", json!({ "of": &name, "count": 2 })).unwrap();
    assert_eq!(answer["queued"], json!(true));
    assert_eq!(answer["count"], json!(2));
    assert_eq!(answer["of"], json!(name));
    assert_eq!(f.settled(), "");

    let state = f.state();
    assert_eq!(state["gallery"]["count"], json!(3));
    let newest = state["gallery"]["newest"].as_array().unwrap();
    assert_eq!(newest[0]["made_from"], json!(format!("variation of {name}")));
    assert_eq!(newest[1]["made_from"], json!(format!("variation of {name}")));
    for shot in &newest[..2] {
        assert_eq!(shot["prompt"], json!("a lighthouse in fog"), "a variation changes the seed, not the sentence");
        assert_eq!(shot["size"], json!("256x256"));
        assert_ne!(shot["seed"], json!(original_seed), "{shot}");
    }
    assert_ne!(newest[0]["seed"], newest[1]["seed"], "two variations of one ask came out the same");
    let varied = PathBuf::from(newest[0]["path"].as_str().unwrap());
    assert_eq!(Sidecar::read(&varied).unwrap().steps, Some(4), "a variation keeps the step count it was told about");

    // ── upscale: a resample on this machine, and a record that does not claim otherwise ──
    let answer = f.act("upscale", json!({ "path": &name, "factor": 2 })).unwrap();
    assert_eq!(answer["factor"], json!(2));
    assert_eq!(answer["done_on"], json!("this machine"));
    assert_eq!(f.settled(), "");

    let state = f.state();
    assert_eq!(state["gallery"]["count"], json!(4));
    let bigger = &state["gallery"]["newest"][0];
    assert_eq!(bigger["made_from"], json!(format!("upscale of {name}")));
    assert_eq!(bigger["size"], json!("512x512"));
    assert_eq!(bigger["backend"], json!("studio-resample"), "nothing was asked of a backend");
    assert!(bigger.get("model").is_none(), "a resample has no model to name: {bigger}");
    let bigger_path = PathBuf::from(bigger["path"].as_str().unwrap());
    assert_eq!(gallery::dimensions(&bigger_path), (Some(512), Some(512)));
    let bigger_record = Sidecar::read(&bigger_path).unwrap();
    assert_eq!(bigger_record.prompt, "a lighthouse in fog", "the sentence travels with the picture");
    assert_eq!(bigger_record.seed, original_seed);
    assert_eq!(bigger_record.backend, "studio-resample");
    assert!(bigger_record.model.is_empty());

    // ── delete: into the Trash, and back out again ──
    let answer = f.act("delete", json!({ "path": &name })).unwrap();
    assert_eq!(answer["recoverable"], json!(true));
    assert_eq!(answer["gallery_count"], json!(3));
    let moved = answer["moved_to_trash"].as_array().unwrap();
    assert_eq!(moved.len(), 2, "the picture and its record go together: {moved:?}");
    assert_eq!(moved[0]["name"], json!(name));
    assert_eq!(moved[1]["name"], json!(name.replace(".png", ".json")));

    let notice = f.notice();
    assert!(notice.contains("Moved"), "{notice}");
    assert!(notice.contains("Files can put them back"), "{notice}");
    assert!(!original.exists(), "the picture is still in the gallery");
    assert!(!Sidecar::path_for(&original).exists(), "the record was left behind");

    // "Recoverable" is a promise the app makes in its purpose, in its notice and in its answer, so
    // it is checked the only way that counts: read the Trash the way Files reads it, and put the
    // picture back.
    let trash_root = f.world.places.trash.clone();
    let listed = trash::items(&trash_root).unwrap();
    assert_eq!(listed.len(), 2, "{listed:?}");
    assert!(listed.iter().any(|item| item.original == original));
    for item in &listed {
        trash::restore(item, &trash_root).unwrap();
    }
    assert!(original.exists(), "the picture did not come back");
    assert_eq!(Sidecar::read(&original).unwrap().prompt, "a lighthouse in fog", "the record did not come back with it");
}

/// The brief's "one real run": the whole app driven from a binary, with the fake backend, on a
/// machine that has no GPU, no API key and no ComfyUI. It is a test rather than a transcript
/// because a transcript goes stale and a test does not.
#[test]
fn a_whole_session_runs_from_one_sentence_to_a_filed_picture() {
    let f = fake("session");

    // 1. What is this app?
    let rendered = f.describe();
    assert!(rendered["summary"].as_str().unwrap().contains("nothing in the gallery yet"));
    // The nine, and the framework's `launch`.
    assert_eq!(rendered["actions"].as_array().unwrap().len(), 10);

    // 2. Make two pictures from one sentence.
    f.act("generate", json!({ "prompt": "a lighthouse in fog", "seed": 99, "count": 2, "width": 512, "height": 384 }))
        .unwrap();
    assert_eq!(f.settled(), "");

    // 3. Read them back.
    let rendered = f.describe();
    let newest = rendered["state"]["gallery"]["newest"].as_array().unwrap().clone();
    assert_eq!(newest.len(), 2);
    assert!(rendered["summary"].as_str().unwrap().contains("2 pictures in"));

    // 4. Vary the one a person would pick, and enlarge it.
    let name =
        PathBuf::from(newest[0]["path"].as_str().unwrap()).file_name().unwrap().to_string_lossy().to_string();
    f.act("variations", json!({ "of": &name })).unwrap();
    assert_eq!(f.settled(), "");
    f.act("upscale", json!({ "path": &name })).unwrap();
    assert_eq!(f.settled(), "");

    // 5. Decide against one and take it back out of the gallery.
    let rendered = f.describe();
    assert_eq!(rendered["state"]["gallery"]["count"], json!(4));
    let oldest = rendered["state"]["gallery"]["newest"].as_array().unwrap().last().unwrap()["path"]
        .as_str()
        .unwrap()
        .to_string();
    let answer = f.act("delete", json!({ "path": &oldest })).unwrap();
    assert_eq!(answer["recoverable"], json!(true));
    assert_eq!(f.describe()["state"]["gallery"]["count"], json!(3));

    // 6. And the folder is the real thing: three pictures, three records, all of them readable by
    //    anything else on this machine.
    let day = gallery::day_folder(&f.gallery(), chrono::Local::now());
    let files: Vec<String> = std::fs::read_dir(&day)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(files.iter().filter(|name| name.ends_with(".png")).count(), 3, "{files:?}");
    assert_eq!(files.iter().filter(|name| name.ends_with(".json")).count(), 3, "{files:?}");
    for name in &files {
        if !name.ends_with(".png") {
            continue;
        }
        let record = Sidecar::read(&day.join(name)).unwrap_or_else(|| panic!("{name} has no record"));
        assert!(!record.prompt.is_empty(), "{name}");
        assert_eq!(record.made_by, "yantrik-studio");
        assert!(["fake", "studio-resample"].contains(&record.backend.as_str()), "{name}: {}", record.backend);
    }
    // The Trash holds the one that was deleted, where Files can see it.
    assert_eq!(trash::items(&f.world.places.trash).unwrap().len(), 2);
}

/// Two calls in flight at once are each answered from their own picture: nothing the second does
/// can become the first one's answer, and nothing the person chooses meanwhile can either.
#[test]
fn each_call_is_answered_from_its_own_picture() {
    let f = fake("own");
    let first = f.one_picture("a lighthouse in fog");
    let second = f.one_picture("a harbour at dusk");
    let by_name = |name: &str| f.app.model({
        let name = name.to_string();
        move |m| m.gallery.iter().position(|row| row.name == name).unwrap()
    });

    let upscaling = f.app.act("upscale", json!({ "path": &first, "factor": 3 })).unwrap();
    let varying = f.app.act("variations", json!({ "of": &second, "count": 2 })).unwrap();
    // The person chooses the other row while both are on workers.
    f.app.run(crate::Command::Choose(by_name(&first) as i32));

    let varied = f.settle(varying).unwrap();
    let enlarged = f.settle(upscaling).unwrap();
    assert_eq!(varied["of"], json!(second), "{varied}");
    assert_eq!(varied["count"], json!(2));
    assert_eq!(enlarged["of"], json!(first), "{enlarged}");
    assert_eq!(enlarged["factor"], json!(3));
    assert_ne!(varied["job"], enlarged["job"]);
    assert_eq!(f.settled(), "");

    let made: Vec<String> = f.app.model(|m| m.gallery.iter().map(|row| row.made_from.clone()).collect());
    assert!(made.contains(&format!("upscale of {first}")), "{made:?}");
    assert_eq!(made.iter().filter(|each| **each == format!("variation of {second}")).count(), 2, "{made:?}");
}

/// A picture's own record, read back through the surface rather than through the filesystem, is the
/// same record. This is the join the whole app rests on: a mind that reads `describe` and a person
/// who opens the folder have to be looking at one thing.
#[test]
fn the_gallery_the_window_and_the_files_agree() {
    let f = fake("agree");
    let name = f.one_picture("a harbour at dusk");
    let path = f.app.model({
        let name = name.clone();
        move |m| m.gallery.iter().find(|row| row.name == name).unwrap().path.clone()
    });

    let from_the_surface = f.state()["gallery"]["newest"][0].clone();
    let from_the_disk = Sidecar::read(&path).unwrap();
    assert_eq!(from_the_surface["prompt"], json!(from_the_disk.prompt));
    assert_eq!(from_the_surface["seed"], json!(from_the_disk.seed));
    assert_eq!(from_the_surface["backend"], json!(from_the_disk.backend));
    assert_eq!(from_the_surface["model"], json!(from_the_disk.model));
    assert_eq!(from_the_surface["size"], json!(format!("{}x{}", from_the_disk.width, from_the_disk.height)));
    assert_eq!(from_the_surface["path"].as_str().unwrap(), path.display().to_string());

    // And the row a person sees is built from the same record, beside the JSON rather than far away
    // from it.
    let shown = f.rows().remove(0);
    assert_eq!(shown.prompt, from_the_disk.prompt);
    assert!(shown.meta.contains(&format!("seed {}", from_the_disk.seed)), "{}", shown.meta);
    assert!(shown.meta.starts_with("placeholder \u{b7} "), "{}", shown.meta);
    assert!(shown.meta.contains("256\u{d7}256"), "{}", shown.meta);
    assert_eq!(shown.name, name);
    assert!(!shown.selected);

    // A picture with no record beside it says so in all three places at once.
    std::fs::remove_file(Sidecar::path_for(&path)).unwrap();
    f.act("refresh", json!({})).unwrap();
    let stale = f.app.model({
        let path = path.clone();
        move |m| m.gallery.iter().find(|each| each.path == path).cloned().unwrap()
    });
    assert!(!stale.has_sidecar, "the gallery never noticed the record was gone");
    assert_eq!(stale.json()["sidecar"], json!("missing"));
    let painted = f.rows().remove(0);
    assert!(painted.meta.contains("no record beside it"), "{}", painted.meta);
    assert_eq!(painted.prompt, "(no prompt recorded beside this file)");
    assert!(
        crate::words::detail(&path, None).contains("There is no record beside this picture"),
        "the pane says it too"
    );
}

impl Fixture {
    /// A second app over the same folder, as a restart would be.
    pub fn reopen(&self) -> Fixture {
        let paths = yantrik_app::Paths::temporary("studio");
        let (config, places) = super::world::take(&self.world.paths);
        super::world::register(&paths, config, places.clone());
        let app = yantrik_testing::TestApp::<crate::Studio>::start_at(paths.clone(), Vec::new());
        let world = super::World { dir: self.world.dir.join("reopened-not-owned"), places, paths };
        Fixture { app, world }
    }
}
