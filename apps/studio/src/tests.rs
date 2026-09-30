//! The joins between the parts, which is where an app either works or does not.
//!
//! Every module here carries its own tests beside it: the configuration's parsing, the workflow's
//! templating, the sidecar's round trip, the Trash's. What follows is what only the whole app can
//! show, at three levels:
//!
//! * **The published surface.** `surface()` and `view()` are the two functions the socket answers
//!   with. Driving them by hand runs the whole `generate → gallery → describe` path with no socket,
//!   no event loop and no window — which is what makes it runnable on a machine that has nothing
//!   installed, including the one CI runs on.
//! * **The wire.** A ComfyUI server and an OpenAI-compatible one, both in this process, answering
//!   real HTTP on a real socket to the same `ureq` client the app ships. What that tests is the
//!   protocol this app speaks, rather than this app's idea of the protocol.
//! * **The window.** Headless, with Slint's own software renderer and only the window-system event
//!   queue substituted, so the callbacks a person presses, the strings that reach the screen and the
//!   PNG of the result are the real ones.
//!
//! `App::serve` is never called here, and that is a decision rather than an omission. It binds a
//! socket in the machine's runtime directory, so a test that called it would answer
//! `yos describe studio` for as long as it ran, and would leave a socket behind pointing at nothing
//! afterwards. What not calling it skips is the runtime's own envelope — the ceiling, the
//! required-argument check, the revision guard, a regrade while the app runs — and that envelope
//! belongs to `crates/yantrik-app-runtime`, where it is tested against this app's exact shape.

use super::*;

use slint::platform::{
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    EventLoopProxy, Platform, WindowAdapter,
};
use slint::Model as _;
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

// ── a world of its own ────────────────────────────────────────────────────

/// A gallery, a Trash folder and a settings file, all inside one temporary directory.
///
/// `engine::Places` exists so this can be handed to the app rather than arranged through the
/// environment: `HOME`, `XDG_DATA_HOME` and `XDG_CONFIG_HOME` are process-wide and the tests in one
/// binary run at the same time, so a test that moved one of them would change what every other test
/// saw — including which folder a picture was written into.
fn world(name: &str) -> (PathBuf, engine::Places) {
    let dir = std::env::temp_dir().join(format!(
        "studio-{name}-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    // Canonicalised, because `resolve` canonicalises what it finds and the Trash records the
    // canonical parent: on a machine whose /tmp is a symlink, comparing the two would otherwise
    // compare different spellings of one place.
    let dir = dir.canonicalize().unwrap();
    let found = engine::Places {
        gallery: dir.join("gallery"),
        trash: dir.join("trash-v2"),
        settings: Some(dir.join("studio.json")),
    };
    (dir, found)
}

fn an_engine(config_text: &str, found: &engine::Places) -> Engine {
    Engine::open(config::Config::parse(config_text).unwrap(), found.clone())
}

/// Wait for the queue to empty the way a caller watching `describe` would, and return the notice it
/// left behind. An empty notice means nothing went wrong: once a job is out of the queue, the notice
/// is the only place a failure has to go.
fn settled(engine: &Engine) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let snapshot = engine.snapshot();
        if snapshot.jobs.is_empty() {
            return snapshot.notice;
        }
        assert!(Instant::now() < deadline, "the job never finished: {:?}", snapshot.jobs);
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// One call over the surface, the way `app.act` would make it.
///
/// The runtime checks the ceiling, the required arguments and the revision before a handler is
/// reached; this is the handler underneath, which is where this app's own refusals and its own
/// answers come from.
fn call(actions: &[(Action, Handler)], name: &str, args: Value) -> Result<Value, String> {
    let (_, run) = actions
        .iter()
        .find(|(spec, _)| spec.name == name)
        .unwrap_or_else(|| panic!("Studio does not publish `{name}`"));
    run(&args)
}

fn spec_of<'a>(actions: &'a [(Action, Handler)], name: &str) -> &'a Action {
    &actions
        .iter()
        .find(|(spec, _)| spec.name == name)
        .unwrap_or_else(|| panic!("Studio does not publish `{name}`"))
        .0
}

fn required_of(actions: &[(Action, Handler)], name: &str) -> Vec<String> {
    spec_of(actions, name)
        .params
        .iter()
        .filter(|param| param.required)
        .map(|param| param.name.clone())
        .collect()
}

/// Everything `yos describe studio` would print, built by the same function the socket uses.
fn described(engine: &Engine, actions: &[(Action, Handler)]) -> Value {
    let specs: Vec<Action> = actions.iter().map(|(spec, _)| spec.clone()).collect();
    control::describe_json("studio", &view(engine), &specs)
}

/// The paths in a gallery, newest first, as the state reports them.
fn paths(state: &Value) -> Vec<String> {
    state["gallery"]["newest"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["path"].as_str().unwrap().to_string())
        .collect()
}

/// One picture, made by the fake backend, so the tests that are about something else start from a
/// gallery that has one.
fn one_picture(actions: &[(Action, Handler)], engine: &Engine, prompt: &str) -> String {
    call(
        actions,
        "generate",
        json!({ "prompt": prompt, "seed": 11, "width": 256, "height": 256, "steps": 4 }),
    )
    .unwrap();
    assert_eq!(settled(engine), "", "the setup picture was not made");
    let name = engine
        .snapshot()
        .gallery
        .first()
        .expect("the picture is in the gallery")
        .name
        .clone();
    name
}

// ── what a caller is shown ────────────────────────────────────────────────

#[test]
fn the_surface_publishes_nine_actions_and_says_what_each_one_costs() {
    let (dir, found) = world("surface");
    let engine = an_engine(r#"{"backend":{"kind":"fake"}}"#, &found);
    let actions = surface(engine.clone());

    let names: Vec<&str> = actions.iter().map(|(spec, _)| spec.name.as_str()).collect();
    assert_eq!(
        names,
        ["generate", "variations", "upscale", "open", "delete", "set_backend", "cancel", "cancel_all", "refresh"],
        "the surface a mind is offered changed shape"
    );

    // The grades, which are what an approval card reads before a person agrees to anything.
    // `generate` and `variations` are published at the grade a local backend deserves and move to
    // `sensitive` the moment a hosted one is configured — at startup and on every `set_backend` —
    // which is what `control::regrade` is for.
    for name in ["generate", "variations", "upscale", "open", "delete", "cancel", "cancel_all"] {
        assert_eq!(spec_of(&actions, name).permission, "standard", "`{name}`");
    }
    assert_eq!(spec_of(&actions, "set_backend").permission, "sensitive");
    assert_eq!(spec_of(&actions, "refresh").permission, "safe");

    // The three that hand work to a worker say so, or a caller would report a render as finished at
    // the moment it began.
    for name in ["generate", "variations", "upscale"] {
        assert!(spec_of(&actions, name).deferred, "`{name}` settles later and does not say so");
    }
    for name in ["open", "delete", "set_backend", "cancel", "cancel_all", "refresh"] {
        assert!(!spec_of(&actions, name).deferred, "`{name}` is finished when it returns");
    }

    // One required argument on the whole surface besides the two that name a file, the one that
    // names a backend and the one that names a job. Everything else has a default worth having,
    // and an action that demands seven arguments is an action a mind gets wrong often enough to
    // matter.
    assert_eq!(required_of(&actions, "generate"), ["prompt"]);
    assert_eq!(required_of(&actions, "variations"), ["of"]);
    assert_eq!(required_of(&actions, "upscale"), ["path"]);
    assert_eq!(required_of(&actions, "open"), ["path"]);
    assert_eq!(required_of(&actions, "delete"), ["path"]);
    assert_eq!(required_of(&actions, "set_backend"), ["kind"]);
    assert_eq!(required_of(&actions, "cancel"), ["job"]);
    assert!(required_of(&actions, "cancel_all").is_empty());
    assert!(required_of(&actions, "refresh").is_empty());

    // The purposes. Each of these is a fact the caller has to be told, not decoration: what is sent
    // where, what can be undone, and what the words in the box actually mean.
    let generate = &spec_of(&actions, "generate").description;
    assert!(generate.contains("`sensitive` while a hosted backend"), "{generate}");
    assert!(generate.contains("may charge"), "{generate}");
    assert!(generate.contains("queue.running"), "{generate}");
    let delete = &spec_of(&actions, "delete").description;
    assert!(delete.contains("It is recoverable"), "{delete}");
    assert!(delete.contains("not a way to remove an arbitrary file"), "{delete}");
    let upscale = &spec_of(&actions, "upscale").description;
    assert!(upscale.contains("It is a resize and not an upscaler"), "{upscale}");
    assert!(upscale.contains("Nothing is sent anywhere"), "{upscale}");
    let set_backend = &spec_of(&actions, "set_backend").description;
    assert!(set_backend.contains("NAME of an environment variable"), "{set_backend}");
    assert!(set_backend.contains("never stored"), "{set_backend}");

    // And the JSON that carries all of it, rendered by the function the socket answers with.
    let rendered = described(&engine, &actions);
    assert_eq!(rendered["app"], json!("studio"));
    assert_eq!(rendered["actions"].as_array().unwrap().len(), 9);
    assert_eq!(rendered["actions"][0]["name"], json!("generate"));
    assert_eq!(rendered["actions"][0]["permission"], json!("standard"));
    assert_eq!(rendered["actions"][0]["settles"], json!("later"));
    assert_eq!(rendered["actions"][0]["parameters"]["required"], json!(["prompt"]));
    assert_eq!(rendered["actions"][8]["settles"], json!("on return"));
    // Every argument carries a sentence. A schema handed to a model with an empty description is a
    // schema the model guesses at, and the guess is what the person gets.
    for action in rendered["actions"].as_array().unwrap() {
        assert!(action["description"].as_str().unwrap().len() >= 40, "{action}");
        for (name, param) in action["parameters"]["properties"].as_object().unwrap() {
            let sentence = param["description"].as_str().unwrap();
            assert!(sentence.len() >= 15, "{name}: {sentence:?}");
            assert!(
                ["string", "number", "boolean"].contains(&param["type"].as_str().unwrap()),
                "{name} is typed {:?}",
                param["type"]
            );
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// #137: the two cards for `set_backend` carried the same paragraph — the action's published
/// purpose, the same for every call of it — and the person had to work out for themselves that
/// `kind: fake` brings the prompts back onto this machine. The purpose states the condition
/// ("naming a hosted service means…"); the per-call sentence says which side of it these
/// arguments fall on, and nothing beyond what they establish.
#[test]
fn set_backend_explains_the_call_not_the_action() {
    let (dir, found) = world("explain");
    let engine = an_engine(r#"{"backend":{"kind":"fake"}}"#, &found);
    let actions = surface(engine.clone());

    let set_backend = spec_of(&actions, "set_backend");
    let explains = |args: Value| {
        set_backend
            .explainer
            .as_ref()
            .expect("set_backend explains one call of itself")
            .sentence(&args)
    };
    // The issue's two sentences, for the issue's two cards.
    assert_eq!(explains(json!({ "kind": "fake" })), "After this, prompts stay on this machine.");
    assert_eq!(
        explains(json!({
            "kind": "openai-images", "model": "gpt-image-1", "api_key_env": "OPENAI_API_KEY"
        })),
        "After this, prompts go to api.openai.com and may cost money."
    );
    // A call that named where to go: the sentence names the HOST that URL actually reaches —
    // never the raw argument, which is caller-supplied text the shell would vouch for.
    assert_eq!(
        explains(json!({ "kind": "openai-images", "base_url": "https://images.example/v1" })),
        "After this, prompts go to images.example and may cost money."
    );
    // The userinfo trick: this URL reads like api.openai.com while the prompts and the key go
    // to evil.example. The sentence must name where the call goes, not what it looks like.
    assert_eq!(
        explains(json!({
            "kind": "openai-images", "base_url": "https://api.openai.com@evil.example/v1"
        })),
        "After this, prompts go to evil.example and may cost money."
    );
    // A base_url that is not a URL says nothing rather than guessing a destination.
    assert_eq!(explains(json!({ "kind": "openai-images", "base_url": "not a url" })), "");
    // Every spelling `set_backend` accepts is explained as that backend: a spelling the
    // explainer did not know used to leave the card unexplained, and an unexplained card still
    // offers "Allow for this session" — a mind could pick the spelling that bought it one.
    assert_eq!(
        explains(json!({ "kind": "OpenAI" })),
        "After this, prompts go to api.openai.com and may cost money."
    );
    assert_eq!(explains(json!({ "kind": "off" })), "After this, prompts stay on this machine.");
    assert_eq!(
        explains(json!({ "kind": "comfy" })),
        "After this, prompts go to the ComfyUI server on this machine."
    );
    assert_eq!(
        explains(json!({ "kind": "comfyui", "base_url": "http://gpu-box.lan:8188" })),
        "After this, prompts go to the ComfyUI server at gpu-box.lan."
    );
    // A kind `set_backend` would refuse says nothing, and the card reads as it did before.
    assert_eq!(explains(json!({ "kind": "midjourney" })), "");
    assert_eq!(explains(json!({})), "");

    // The rendered describe carries the FACT — this action can explain one call of itself — and
    // never a sentence, because the sentence depends on arguments `describe` does not see.
    let rendered = described(&engine, &actions);
    let flag = |name: &str| {
        rendered["actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == name)
            .unwrap()
            .get("explains")
            .cloned()
    };
    assert_eq!(flag("set_backend"), Some(json!(true)));
    for name in
        ["generate", "variations", "upscale", "open", "delete", "cancel", "cancel_all", "refresh"]
    {
        assert_eq!(flag(name), None, "`{name}` publishes what it always did");
    }
    std::fs::remove_dir_all(&dir).ok();
}

// ── the whole path, on the backend every machine has ──────────────────────

#[test]
fn a_sentence_becomes_a_file_a_record_and_an_answer_a_mind_can_read_back() {
    let (dir, found) = world("path");
    let engine = an_engine(r#"{"backend":{"kind":"fake"}}"#, &found);
    let actions = surface(engine.clone());

    let answer = call(
        &actions,
        "generate",
        json!({
            "prompt": "a lighthouse in fog",
            "negative": "blurry, text",
            "seed": 7,
            "count": 2,
            // A string, because that is how a number arrives about half the time, and a refusal
            // about JSON would be a refusal about nothing.
            "width": "1025",
            "height": 700,
            "steps": 25,
        }),
    )
    .unwrap();

    // The call starts the work and says so, rather than pretending to have finished it.
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

    // While it runs, the queue is the only thing that says so. The fake backend can also finish
    // before this line runs, so the queue being empty is allowed — being empty and having said so
    // in `gallery` is not: that is checked below.
    let busy = described(&engine, &actions);
    let running = busy["state"]["queue"]["running"].as_array().unwrap();
    if !running.is_empty() {
        assert_eq!(running[0]["kind"], json!("generate"));
        assert_eq!(running[0]["status"], json!("running"));
        assert!(
            running[0]["progress"].as_str().unwrap().ends_with("of 2"),
            "{running:?}"
        );
        assert!(busy["summary"].as_str().unwrap().contains("being made"), "{}", busy["summary"]);
    }

    assert_eq!(settled(&engine), "", "a generation that worked left a notice behind");

    // ── describe, which is what a mind reads ──
    let rendered = described(&engine, &actions);
    let summary = rendered["summary"].as_str().unwrap();
    assert!(summary.starts_with("Studio — "), "{summary}");
    assert!(summary.contains("2 pictures in"), "{summary}");
    assert!(summary.contains("the fake backend"), "{summary}");

    let state = &rendered["state"];
    assert_eq!(state["backend"]["kind"], json!("fake"));
    assert_eq!(state["backend"]["where"], json!("this machine"));
    assert_eq!(state["backend"]["configured"], json!(true));
    assert_eq!(state["backend"]["generate_is_graded"], json!("standard"));
    assert_eq!(state["backend"]["prompt_leaves_this_machine"], json!(false));
    assert_eq!(state["output_folder"], json!(engine::display(&found.gallery)));
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
        // 1025×700 ask comes back 1024×699, and the record says what was actually sent rather
        // than what was hoped for.
        assert_eq!(shot["size"], json!("1024x699"), "{shot}");
        assert!(shot["seconds"].as_f64().is_some(), "{shot}");
        assert!(shot["path"].as_str().unwrap().ends_with(".png"), "{shot}");
        assert!(shot["created"].as_str().unwrap().contains('T'), "{shot}");
    }

    // ── the files, which are the real thing ──
    let path = PathBuf::from(newest[0]["path"].as_str().unwrap());
    assert!(path.starts_with(&found.gallery), "{path:?}");
    assert!(path.is_file(), "describe names a file that is not there");
    assert_eq!(
        path.parent().unwrap().file_name().unwrap().to_string_lossy(),
        chrono::Local::now().format("%Y-%m-%d").to_string(),
        "pictures are filed by day, so a folder of thousands stays navigable"
    );
    assert_eq!(gallery::dimensions(&path), (Some(1024), Some(699)));

    let sidecar = gallery::Sidecar::read(&path).expect("no record beside the picture");
    assert_eq!(sidecar.prompt, "a lighthouse in fog");
    assert_eq!(sidecar.negative, "blurry, text");
    assert_eq!(sidecar.seed, 8);
    assert_eq!(sidecar.backend, "fake");
    assert_eq!(sidecar.model, "fake");
    assert_eq!(sidecar.width, 1024);
    assert_eq!(sidecar.height, 699);
    assert_eq!(sidecar.sent, "1024x699");
    assert_eq!(sidecar.steps, Some(25));
    assert_eq!(sidecar.cfg, Some(engine::DEFAULT_CFG));
    assert!(sidecar.made_from.is_empty());
    assert_eq!(sidecar.made_by, "yantrik-studio");
    assert!(sidecar.seconds.is_finite() && sidecar.seconds >= 0.0);
    assert!(gallery::Sidecar::path_for(&path).is_file(), "the record is a file of its own");
    // The bytes are the placeholder the fake backend draws from the prompt and the seed, which is
    // what makes "deterministic" a claim that can be checked rather than a word in a comment.
    assert_eq!(
        std::fs::read(&path).unwrap(),
        backend::draw("a lighthouse in fog", 8, 1024, 699),
        "the file on disk is not the picture the backend drew"
    );

    // ── and a second app over the same folder has to see the same thing ──
    //
    // A restart, or the poll timer, re-reads the gallery from the filesystem. Two pictures made
    // inside one second have the same `created`, so the order of a batch is exactly where a re-read
    // and the engine could disagree — and then `gallery.newest` would name a different picture as
    // the newest depending on whether the folder had been read again since.
    let reopened = an_engine(r#"{"backend":{"kind":"fake"}}"#, &found);
    assert_eq!(paths(&reopened.snapshot().state()), paths(state), "a re-read reordered the gallery");
    assert_eq!(reopened.snapshot().gallery.len(), 2);

    // `refresh` reads the folder again and answers with what it found.
    let refreshed = call(&actions, "refresh", json!({})).unwrap();
    assert_eq!(refreshed["refreshed"], json!(true));
    assert_eq!(refreshed["gallery_count"], json!(2));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_machine_with_no_backend_configured_draws_placeholders_and_says_so() {
    let (dir, found) = world("unconfigured");
    // `Config::unconfigured` is what `load` returns when there is no file, which is the state every
    // machine that has not set Studio up is in. The app has to open, work, and be honest about it.
    let engine = Engine::open(config::Config::unconfigured(), found.clone());
    let actions = surface(engine.clone());

    let rendered = described(&engine, &actions);
    assert_eq!(rendered["state"]["backend"]["configured"], json!(false));
    assert_eq!(rendered["state"]["backend"]["kind"], json!("fake"));
    let note = rendered["state"]["backend"]["note"].as_str().unwrap();
    assert!(note.contains("No backend is configured"), "{note}");
    assert!(note.contains(config::FILE_NAME), "the note has to name the file that would fix it: {note}");
    assert!(note.contains("set_backend"), "{note}");
    assert!(
        rendered["summary"].as_str().unwrap().contains("no backend configured"),
        "{}",
        rendered["summary"]
    );
    // An unconfigured machine is still a local one: nothing it draws goes anywhere.
    assert_eq!(rendered["state"]["backend"]["generate_is_graded"], json!("standard"));
    assert_eq!(rendered["state"]["backend"]["prompt_leaves_this_machine"], json!(false));

    call(&actions, "generate", json!({ "prompt": "a harbour at dusk", "width": 64, "height": 64 })).unwrap();
    assert_eq!(settled(&engine), "");
    let state = engine.snapshot().state();
    assert_eq!(state["gallery"]["count"], json!(1));
    assert_eq!(state["gallery"]["newest"][0]["backend"], json!("fake"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_actions_on_one_picture_vary_it_enlarge_it_and_put_it_in_the_trash() {
    let (dir, found) = world("row");
    let engine = an_engine(r#"{"backend":{"kind":"fake"}}"#, &found);
    let actions = surface(engine.clone());
    let name = one_picture(&actions, &engine, "a lighthouse in fog");
    let original = engine.snapshot().gallery[0].path.clone();
    let original_seed = gallery::Sidecar::read(&original).unwrap().seed;

    // ── variations: the same sentence, new seeds ──
    // Named by its bare filename rather than its path, because "the lighthouse one" is how a person
    // refers to a picture and a path is how a machine does. Both have to work.
    let answer = call(&actions, "variations", json!({ "of": &name, "count": 2 })).unwrap();
    assert_eq!(answer["queued"], json!(true));
    assert_eq!(answer["count"], json!(2));
    assert_eq!(settled(&engine), "");

    let state = engine.snapshot().state();
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
    let varied_record = gallery::Sidecar::read(&varied).unwrap();
    assert_eq!(varied_record.steps, Some(4), "a variation keeps the step count it was told about");

    // ── upscale: a resample on this machine, and a record that does not claim otherwise ──
    let answer = call(&actions, "upscale", json!({ "path": &name, "factor": 2 })).unwrap();
    assert_eq!(answer["factor"], json!(2));
    assert_eq!(answer["done_on"], json!("this machine"));
    assert_eq!(settled(&engine), "");

    let state = engine.snapshot().state();
    assert_eq!(state["gallery"]["count"], json!(4));
    let bigger = &state["gallery"]["newest"][0];
    assert_eq!(bigger["made_from"], json!(format!("upscale of {name}")));
    assert_eq!(bigger["size"], json!("512x512"));
    assert_eq!(bigger["backend"], json!("studio-resample"), "nothing was asked of a backend");
    assert!(bigger.get("model").is_none(), "a resample has no model to name: {bigger}");
    let bigger_path = PathBuf::from(bigger["path"].as_str().unwrap());
    assert_eq!(gallery::dimensions(&bigger_path), (Some(512), Some(512)));
    let bigger_record = gallery::Sidecar::read(&bigger_path).unwrap();
    assert_eq!(bigger_record.prompt, "a lighthouse in fog", "the sentence travels with the picture");
    assert_eq!(bigger_record.seed, original_seed);
    assert_eq!(bigger_record.backend, "studio-resample");
    assert!(bigger_record.model.is_empty());

    // ── delete: into the Trash, and back out again ──
    let answer = call(&actions, "delete", json!({ "path": &name })).unwrap();
    assert_eq!(answer["recoverable"], json!(true));
    assert_eq!(answer["gallery_count"], json!(3));
    let moved = answer["moved_to_trash"].as_array().unwrap();
    assert_eq!(moved.len(), 2, "the picture and its record go together: {moved:?}");
    assert_eq!(moved[0]["name"], json!(name));
    assert_eq!(moved[1]["name"], json!(name.replace(".png", ".json")));

    let notice = engine.snapshot().notice;
    assert!(notice.contains("Moved"), "{notice}");
    assert!(notice.contains("Files can put them back"), "{notice}");
    assert!(!original.exists(), "the picture is still in the gallery");
    assert!(!gallery::Sidecar::path_for(&original).exists(), "the record was left behind");

    // "Recoverable" is a promise the app makes in its purpose, in its notice and in its answer, so
    // it is checked the only way that counts: read the Trash the way Files reads it, and put the
    // picture back.
    let listed = trash::items(&found.trash).unwrap();
    assert_eq!(listed.len(), 2, "{listed:?}");
    assert!(listed.iter().any(|item| item.original == original));
    for item in &listed {
        trash::restore(item, &found.trash).unwrap();
    }
    assert!(original.exists(), "the picture did not come back");
    assert_eq!(
        gallery::Sidecar::read(&original).unwrap().prompt,
        "a lighthouse in fog",
        "the record did not come back with it"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_call_that_cannot_work_is_refused_in_words_a_caller_can_act_on() {
    let (dir, found) = world("refusals");
    let engine = an_engine(r#"{"backend":{"kind":"fake"}}"#, &found);
    let actions = surface(engine.clone());

    // An empty prompt. The runtime refuses a *missing* required argument before the handler runs;
    // this is the other half, one that is present and blank.
    let problem = call(&actions, "generate", json!({ "prompt": "   " })).unwrap_err();
    assert!(problem.contains("a prompt is needed"), "{problem}");
    assert!(problem.contains("generate prompt="), "the refusal has to show the shape that works: {problem}");
    // Said twice: to whoever asked, and in the notice bar the person is looking at.
    assert_eq!(engine.snapshot().notice, problem);

    // A count that would spend real money on a hosted backend.
    let problem = call(&actions, "generate", json!({ "prompt": "a cliff", "count": 9 })).unwrap_err();
    assert!(problem.contains("more than the 4"), "{problem}");
    assert!(problem.contains("charges for each one"), "{problem}");

    // A prompt nobody's model would read.
    let problem = call(&actions, "generate", json!({ "prompt": "x".repeat(4001) })).unwrap_err();
    assert!(problem.contains("4001 characters"), "{problem}");

    // Numbers, by contrast, are brought onto the range rather than refused: a mind that asks for
    // 9000 pixels wants a picture, and the record beside it says what was actually made. Two
    // shapings happen: the engine clamps the ask to 4096×64 (edges live in 64..=4096), and the
    // fake backend then caps its own long edge at 1024 and keeps the aspect, so the file on disk
    // is 1024×16. The record says what is really there, not what was asked for.
    call(&actions, "generate", json!({ "prompt": "a wide sea", "width": 9000, "height": 3, "steps": 900 }))
        .unwrap();
    assert_eq!(settled(&engine), "");
    let shot = &engine.snapshot().state()["gallery"]["newest"][0];
    assert_eq!(shot["size"], json!("1024x16"), "{shot}");
    assert_eq!(gallery::Sidecar::read(&PathBuf::from(shot["path"].as_str().unwrap())).unwrap().steps, Some(150));

    // Missing and empty arguments are named.
    let problem = call(&actions, "variations", json!({})).unwrap_err();
    assert!(problem.contains("`variations` needs `of`"), "{problem}");
    let problem = call(&actions, "delete", json!({ "path": " " })).unwrap_err();
    assert!(problem.contains("`delete` was given an empty `path`"), "{problem}");

    // A picture that is not there, and one that is not a picture.
    let problem = call(&actions, "variations", json!({ "of": "nothing-like-it.png" })).unwrap_err();
    assert!(problem.contains("nothing-like-it.png"), "{problem}");
    assert!(problem.contains("there is no"), "{problem}");
    let problem = call(&actions, "upscale", json!({ "path": "/tmp/studio-no-such-file.png" })).unwrap_err();
    assert!(problem.contains("is not there. Nothing was changed."), "{problem}");
    std::fs::write(dir.join("notes.txt"), "not a picture").unwrap();
    let problem =
        call(&actions, "upscale", json!({ "path": dir.join("notes.txt").display().to_string() })).unwrap_err();
    assert!(problem.contains("is not a picture this OS recognises"), "{problem}");

    // `delete` is not a way to remove a file from somewhere else. This is the refusal the grade of
    // `standard` rests on.
    std::fs::write(dir.join("outside.png"), backend::draw("elsewhere", 1, 32, 32)).unwrap();
    let problem = call(
        &actions,
        "delete",
        json!({ "path": dir.join("outside.png").display().to_string() }),
    )
    .unwrap_err();
    assert!(problem.contains("only deletes its own pictures"), "{problem}");
    assert!(problem.contains("Files can delete anything"), "{problem}");
    assert!(dir.join("outside.png").exists(), "the refusal happened after the move");
    // The same file is readable through `upscale`, which changes nothing outside the gallery and
    // writes only inside it: reading a picture is not deleting one.
    call(&actions, "upscale", json!({ "path": dir.join("outside.png").display().to_string() })).unwrap();
    assert_eq!(settled(&engine), "");
    assert!(dir.join("outside.png").exists());

    // A picture with no record beside it cannot be varied, and the refusal says what to do instead.
    let day = gallery::day_folder(&found.gallery, chrono::Local::now());
    std::fs::create_dir_all(&day).unwrap();
    let stray = day.join("stray.png");
    std::fs::write(&stray, backend::draw("copied in by hand", 3, 64, 64)).unwrap();
    // A bare name is looked up in the gallery and then in today's folder, so this finds the file
    // whether or not the `refresh` below has finished re-reading — and either way the answer has
    // to be that there is nothing to vary, not a guess.
    let problem = call(&actions, "variations", json!({ "of": "stray.png" })).unwrap_err();
    assert!(problem.contains("there is no record beside"), "{problem}");
    assert!(problem.contains("Generate from a sentence instead"), "{problem}");
    call(&actions, "refresh", json!({})).unwrap();
    // And the gallery says which of its rows are unaccounted for, rather than presenting a guess.
    let mut waited = 0;
    let listed_row = loop {
        let state = engine.snapshot().state();
        if let Some(each) = state["gallery"]["newest"]
            .as_array()
            .unwrap()
            .iter()
            .find(|each| each["path"].as_str().unwrap().ends_with("stray.png"))
        {
            break each.clone();
        }
        assert!(waited < 100, "the picture copied in by hand never reached the gallery");
        waited += 1;
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(listed_row["sidecar"], json!("missing"), "{listed_row}");
    assert!(
        listed_row.get("seed").is_none(),
        "a row with no record must not invent a seed: {listed_row}"
    );

    // Cancelling an empty queue is a refusal, not a success: a mind that reports "cancelled" when
    // nothing was running has been told something false.
    let problem = call(&actions, "cancel_all", json!({})).unwrap_err();
    assert_eq!(problem, "nothing was queued or running");
    let problem = call(&actions, "cancel", json!({ "job": 999 })).unwrap_err();
    assert!(problem.contains("there is no job 999"), "{problem}");
    // And an id nobody was ever issued is refused as that, with the action that really does stop
    // everything named: `-1` once read as 0 through a float cast and cancelled the whole queue.
    for not_a_job in [json!({}), json!({ "job": -1 }), json!({ "job": 0 }), json!({ "job": 2.5 }), json!({ "job": "two" })] {
        let problem = call(&actions, "cancel", not_a_job.clone()).unwrap_err();
        assert!(problem.contains("cancel_all"), "{not_a_job}: {problem}");
    }

    // A backend this app does not have, and a hosted one with no model named.
    let problem = call(&actions, "set_backend", json!({ "kind": "midjourney" })).unwrap_err();
    assert!(problem.contains("`midjourney` is not a backend Studio has"), "{problem}");
    assert!(problem.contains(config::KINDS[0]), "the refusal lists what would work: {problem}");
    let problem = call(&actions, "set_backend", json!({ "kind": "openai-images" })).unwrap_err();
    assert!(problem.contains("`model` is required"), "{problem}");

    // `open`'s success path is not exercised here: it spawns `xdg-open` on whatever desktop the
    // tests happen to run on. `resolve` is the part that holds the logic, and it is tested from both
    // sides above.
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn set_backend_moves_where_prompts_go_and_what_that_is_graded() {
    let (dir, found) = world("backend");
    let engine = an_engine(r#"{"backend":{"kind":"fake"},"output_folder":"~/Pictures/Studio"}"#, &found);
    let actions = surface(engine.clone());
    assert_eq!(engine.grade(), "standard");

    // A ComfyUI server on the person's own LAN — the brief's own GPU box — is a local generation.
    let answer = call(
        &actions,
        "set_backend",
        json!({ "kind": "comfyui", "base_url": "http://192.168.4.35:8188/", "model": "mine.safetensors" }),
    )
    .unwrap();
    assert_eq!(answer["backend"], json!("comfyui"));
    assert_eq!(answer["configured"], json!(true));
    assert_eq!(answer["generate_is_now_graded"], json!("standard"));
    assert_eq!(answer["prompt_leaves_this_machine"], json!(false));
    assert_eq!(answer["where"], json!("a ComfyUI server you reach at http://192.168.4.35:8188"));
    assert_eq!(engine.grade(), "standard");

    // The choice is written down where this instance was started with, so it survives a restart, and
    // the folder the person already had stays theirs.
    let written = std::fs::read_to_string(found.settings.as_ref().unwrap()).unwrap();
    let on_disk = config::Config::parse(&written).unwrap();
    assert_eq!(on_disk.backend.kind, config::Kind::ComfyUi);
    assert_eq!(on_disk.backend.base_url, "http://192.168.4.35:8188", "the trailing slash was kept");
    assert_eq!(on_disk.backend.model, "mine.safetensors");
    assert_eq!(on_disk.output_folder, "~/Pictures/Studio");

    // The same kind, pointed somewhere the person does not own, is a different act.
    let answer = call(
        &actions,
        "set_backend",
        json!({ "kind": "comfyui", "base_url": "http://203.0.113.7:8188" }),
    )
    .unwrap();
    assert_eq!(answer["generate_is_now_graded"], json!("sensitive"));
    assert_eq!(answer["prompt_leaves_this_machine"], json!(true));
    assert!(answer["where"].as_str().unwrap().contains("away from your network"), "{answer}");
    assert_eq!(engine.grade(), "sensitive");

    // A hosted service is `sensitive` however cheap or trustworthy it is: the words leave the
    // machine and the service may bill for them.
    let answer = call(
        &actions,
        "set_backend",
        json!({ "kind": "openai-images", "model": "gpt-image-1", "api_key_env": "STUDIO_ABSENT_KEY" }),
    )
    .unwrap();
    assert_eq!(answer["generate_is_now_graded"], json!("sensitive"));
    assert_eq!(answer["prompt_leaves_this_machine"], json!(true));
    assert_eq!(answer["api_key_env"], json!("STUDIO_ABSENT_KEY"), "the variable's name is the person's to know");
    assert!(answer["note"].as_str().unwrap().contains("is not set"), "{answer}");
    assert_eq!(engine.grade(), "sensitive");
    // A generation that cannot be sent says so rather than failing quietly in a worker.
    call(&actions, "generate", json!({ "prompt": "a lighthouse" })).unwrap();
    let notice = settled(&engine);
    assert!(notice.contains("openai-images could not make the picture"), "{notice}");
    assert!(notice.contains("STUDIO_ABSENT_KEY is not set"), "{notice}");
    assert!(engine.snapshot().state()["backend"]["api_key_is_set"] == json!(false));

    // And back to the placeholder, which is local again.
    let answer = call(&actions, "set_backend", json!({ "kind": "fake" })).unwrap();
    assert_eq!(answer["generate_is_now_graded"], json!("standard"));
    assert!(answer["note"].as_str().unwrap().contains("placeholder"), "{answer}");
    assert_eq!(engine.grade(), "standard");

    // The registry half of this — a published action's grade actually moving, and the ceiling
    // following it — needs a socket, so it is tested in `crates/yantrik-app-runtime`, against this
    // app's exact shape. What is left here without one is a `tracing::error!`.
    std::fs::remove_dir_all(&dir).ok();
}

// ── the wire ──────────────────────────────────────────────────────────────

/// What the fake server saw. Everything a test asserts about the protocol is recorded here rather
/// than inferred from what came back.
#[derive(Default)]
struct Recorded {
    next: usize,
    /// prompt id → the graph that was posted with it.
    graphs: HashMap<String, Value>,
    /// prompt id → how many times `/history` has been asked about it.
    polls: HashMap<String, usize>,
    /// "subfolder/filename" → prompt id, so `/view` can render the graph it was asked for.
    views: HashMap<String, String>,
    viewed: Vec<String>,
    prompts: Vec<Value>,
    interrupts: usize,
    authorizations: Vec<String>,
    image_requests: Vec<Value>,
}

/// How the fake server misbehaves, for the tests that are about failure.
///
/// `pub(crate)` because `engine`'s own tests drive a ComfyUI generation through it too, and a
/// second fake server answering the same routes a second way would be two things to keep in step.
#[derive(Clone, Default)]
pub(crate) struct Manners {
    /// Answer `/prompt` with this status and body instead of taking the graph.
    refuse: Option<(u16, String)>,
    /// Never report a render as finished, so the only way out is the cancel flag.
    never_finish: bool,
}

/// A ComfyUI server, and an OpenAI-compatible images endpoint, in this process.
///
/// One server for both because both are HTTP and the question the second one answers — whether an
/// API key can be used without ever being repeated — needs a service that receives it.
///
/// Real sockets and real HTTP, to the same `ureq` client the app ships. A hand-written trait object
/// standing in for `Backend` would test this app's idea of the protocol; this tests the protocol.
pub(crate) struct Server {
    pub(crate) base_url: String,
    shared: Arc<Mutex<Recorded>>,
    stop: Arc<AtomicBool>,
    accept: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    pub(crate) fn start(manners: Manners) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        // Non-blocking so the accept loop can notice it is being shut down. A test that leaked a
        // thread blocked in `accept` would leak a socket with it, and the port would stay taken for
        // the rest of the run.
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let shared = Arc::new(Mutex::new(Recorded::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let accept = {
            let shared = shared.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let shared = shared.clone();
                            let manners = manners.clone();
                            // One thread per connection rather than a pool: these are tests, and a
                            // connection that is served inline would deadlock the moment a handler
                            // waited on the client that is waiting on it.
                            std::thread::spawn(move || serve(stream, &shared, &manners));
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2))
                        }
                        Err(_) => return,
                    }
                }
            })
        };
        Server { base_url: format!("http://127.0.0.1:{port}"), shared, stop, accept: Some(accept) }
    }

    fn config(&self, kind: &str) -> String {
        format!(r#"{{"backend":{{"kind":"{kind}","base_url":"{}"}}}}"#, self.base_url)
    }

    fn with<T>(&self, read: impl FnOnce(&Recorded) -> T) -> T {
        read(&self.shared.lock().unwrap())
    }

    fn last_graph(&self) -> Value {
        self.with(|recorded| {
            recorded
                .graphs
                .iter()
                // Job ids are "prompt-1", "prompt-2", … so (length, text) is numeric order, and a
                // HashMap's iteration order is not something to build an expectation on.
                .max_by_key(|(id, _)| (id.len(), (*id).clone()))
                .map(|(_, graph)| graph.clone())
                .expect("no graph was posted to the server")
        })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
    }
}

/// Serve one connection until the client closes it.
///
/// Every reply says `Connection: close`, and the backend builds a fresh `ureq` agent per call, so in
/// practice this is one request per connection. The loop is here because "in practice" is not a
/// contract, and a server that hung up on a client that reused a connection would look exactly like
/// a bug in the app.
fn serve(mut stream: TcpStream, shared: &Arc<Mutex<Recorded>>, manners: &Manners) {
    let peer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(peer);
    loop {
        let mut request_line = String::new();
        match reader.read_line(&mut request_line) {
            Ok(0) => return,
            Ok(_) => {}
            Err(_) => return,
        }
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or_default().to_string();
        let target = parts.next().unwrap_or_default().to_string();
        let mut length = 0usize;
        let mut authorization = String::new();
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {}
                Err(_) => return,
            }
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    length = value.trim().parse().unwrap_or(0);
                }
                if name.eq_ignore_ascii_case("authorization") {
                    // Recorded verbatim, and asserted on by exactly one test: this is the string
                    // that must never appear anywhere the app writes or says.
                    authorization = value.trim().to_string();
                }
            }
        }
        let mut body = vec![0u8; length];
        if length > 0 && reader.read_exact(&mut body).is_err() {
            return;
        }
        let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let (status, kind, payload) = route(&method, &target, body, authorization, shared, manners);
        let reason = match status {
            200 => "OK",
            400 => "Bad Request",
            404 => "Not Found",
            500 => "Internal Server Error",
            _ => "Status",
        };
        let mut head = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        )
        .into_bytes();
        head.extend_from_slice(&payload);
        if stream.write_all(&head).is_err() || stream.flush().is_err() {
            return;
        }
    }
}

fn route(
    method: &str,
    target: &str,
    body: Value,
    authorization: String,
    shared: &Arc<Mutex<Recorded>>,
    manners: &Manners,
) -> (u16, &'static str, Vec<u8>) {
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path, query.to_string()),
        None => (target, String::new()),
    };
    let json = |status: u16, value: Value| (status, "application/json", value.to_string().into_bytes());

    match (method, path) {
        ("POST", "/prompt") => {
            if let Some((status, text)) = &manners.refuse {
                return (*status, "application/json", text.as_bytes().to_vec());
            }
            let Some(graph) = body.get("prompt") else {
                return json(400, json!({ "error": { "message": "no graph was posted" } }));
            };
            let mut recorded = shared.lock().unwrap();
            recorded.next += 1;
            let id = format!("prompt-{}", recorded.next);
            recorded.graphs.insert(id.clone(), graph.clone());
            recorded.prompts.push(body.clone());
            drop(recorded);
            json(200, json!({ "prompt_id": id, "number": 1, "node_errors": [] }))
        }
        ("GET", path) if path.starts_with("/history/") => {
            let id = path.trim_start_matches("/history/").to_string();
            let mut recorded = shared.lock().unwrap();
            let polls = recorded.polls.entry(id.clone()).or_insert(0);
            *polls += 1;
            let first = *polls == 1;
            // The entry appears as soon as the graph is queued and grows its `outputs` when the
            // render finishes, so the first poll answers with an entry that has none. That is the
            // "still working" branch, and a server that answered with the picture straight away
            // would never exercise it.
            if first && !manners.never_finish {
                let graph = recorded.graphs.get(&id).cloned().unwrap_or(Value::Null);
                drop(recorded);
                let (width, height) = latent_size(&graph);
                let key = format!("Studio/{}/Studio_{:05}_.png", today(), width * height % 1000);
                shared.lock().unwrap().views.insert(key.clone(), id.clone());
                return json(
                    200,
                    json!({ id: { "status": { "status_str": "running", "completed": false } } }),
                );
            }
            if manners.never_finish {
                drop(recorded);
                return json(200, json!({ id: { "status": { "status_str": "running", "completed": false } } }));
            }
            let key = recorded
                .views
                .iter()
                .find(|(_, owner)| *owner == &id)
                .map(|(key, _)| key.clone())
                .unwrap_or_else(|| format!("Studio/{}/Studio_00001_.png", today()));
            drop(recorded);
            json(
                200,
                json!({
                    id: {
                        "status": { "status_str": "success", "completed": true },
                        "outputs": {
                            "9": {
                                "images": [{
                                    "filename": key.rsplit('/').next().unwrap_or_default(),
                                    "subfolder": key.rsplit_once('/').map(|(head, _)| head).unwrap_or_default(),
                                    "type": "output",
                                }]
                            }
                        }
                    }
                }),
            )
        }
        ("GET", "/view") => {
            let mut filename = String::new();
            let mut subfolder = String::new();
            for pair in query.split('&') {
                if let Some((name, value)) = pair.split_once('=') {
                    match name {
                        "filename" => filename = unescape(value),
                        "subfolder" => subfolder = unescape(value),
                        _ => {}
                    }
                }
            }
            let mut recorded = shared.lock().unwrap();
            recorded.viewed.push(target.to_string());
            let key = format!("{subfolder}/{filename}");
            let Some(graph) = recorded.views.get(&key).and_then(|id| recorded.graphs.get(id)).cloned() else {
                drop(recorded);
                return json(404, json!({ "error": { "message": format!("no file called {key}") } }));
            };
            drop(recorded);
            // Rendered from the graph that was actually posted: the prompt is taken by following the
            // sampler's own `positive` link, exactly the link `fill` wrote the prompt through. A
            // graph that wired the negative prompt into the positive input would draw a different
            // picture, and the test that compares these bytes to the ones the backend drew would
            // fail — which is the point of rendering rather than returning a fixed image.
            let (prompt, seed) = ask_of(&graph);
            let (width, height) = latent_size(&graph);
            (200, "image/png", backend::draw(&prompt, seed, width.max(8), height.max(8)))
        }
        ("POST", "/interrupt") => {
            shared.lock().unwrap().interrupts += 1;
            json(200, json!({}))
        }
        ("GET", "/system_stats") => json(
            200,
            json!({ "system": { "comfyui_version": "0.3.0-fake" }, "devices": [] }),
        ),
        ("POST", "/images/generations") => {
            let mut recorded = shared.lock().unwrap();
            recorded.authorizations.push(authorization.clone());
            recorded.image_requests.push(body.clone());
            drop(recorded);
            let prompt = body["prompt"].as_str().unwrap_or_default().to_string();
            let size = body["size"].as_str().unwrap_or("1024x1024").to_string();
            let (width, height) = size
                .split_once('x')
                .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
                .unwrap_or((1024, 1024));
            use base64::Engine as _;
            let encoded = base64::engine::general_purpose::STANDARD.encode(backend::draw(&prompt, 1, width, height));
            json(200, json!({ "created": 1_780_000_000u64, "data": [{ "b64_json": encoded }] }))
        }
        _ => json(404, json!({ "error": { "message": format!("the fake server has no {method} {path}") } })),
    }
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Undo the percent-encoding this app writes by hand, so the query it built can be read back.
fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' && at + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[at + 1..at + 3]).unwrap_or_default();
            match u8::from_str_radix(hex, 16) {
                Ok(byte) => {
                    out.push(byte);
                    at += 3;
                    continue;
                }
                Err(_) => {}
            }
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn node_of<'a>(graph: &'a Value, class: &str) -> Option<&'a Value> {
    graph
        .as_object()?
        .values()
        .find(|node| node["class_type"].as_str() == Some(class))
}

/// The text a link points at. A link in the API format is `["6", 0]`; a literal is a string.
fn text_of_link(graph: &Value, link: &Value) -> String {
    if let Some(text) = link.as_str() {
        return text.to_string();
    }
    let id = link.as_array().and_then(|pair| pair.first()).and_then(|id| id.as_str()).unwrap_or_default();
    graph[id]["inputs"]["text"].as_str().unwrap_or_default().to_string()
}

/// What a posted graph asks for, read the way a server would read it.
fn ask_of(graph: &Value) -> (String, u64) {
    let sampler = node_of(graph, "KSampler").or_else(|| node_of(graph, "KSamplerAdvanced"));
    let prompt = sampler
        .and_then(|node| node["inputs"].get("positive"))
        .map(|link| text_of_link(graph, link))
        .unwrap_or_default();
    let seed = sampler
        .and_then(|node| node["inputs"].get("seed").or_else(|| node["inputs"].get("noise_seed")))
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    (prompt, seed)
}

fn latent_size(graph: &Value) -> (u32, u32) {
    let latent = node_of(graph, "EmptyLatentImage");
    let read = |key: &str| {
        latent
            .and_then(|node| node["inputs"].get(key))
            .and_then(|value| value.as_u64())
            .unwrap_or(64) as u32
    };
    (read("width"), read("height"))
}

#[test]
fn a_comfyui_server_is_handed_a_graph_and_the_picture_it_renders_is_saved_with_its_record() {
    let server = Server::start(Manners::default());
    let (dir, found) = world("comfy");
    let engine = an_engine(
        &format!(
            r#"{{"backend":{{"kind":"comfyui","base_url":"{}","model":"sdxl_test.safetensors"}}}}"#,
            server.base_url
        ),
        &found,
    );
    let actions = surface(engine.clone());

    call(
        &actions,
        "generate",
        json!({
            "prompt": "a lighthouse in fog",
            "negative": "blurry, text",
            "seed": 4242,
            // 1025 and 700: the ask is clamped onto the range but not onto the grid, so what proves
            // the graph was filled in is the latent coming out at 1024 and 704.
            "width": 1025,
            "height": 700,
            "steps": 20,
        }),
    )
    .unwrap();
    assert_eq!(settled(&engine), "", "the render was reported as failed");

    // ── the graph that went out ──
    let posted = server.with(|recorded| recorded.prompts.clone());
    assert_eq!(posted.len(), 1, "one picture, one POST /prompt");
    assert_eq!(posted[0]["client_id"], json!("yantrik-studio"));
    let graph = server.last_graph();
    let nodes = graph.as_object().unwrap();
    assert!(nodes.keys().all(|id| !id.starts_with('_')), "the explanation was sent as a node: {graph}");
    assert_eq!(nodes.len(), 7, "the shipped graph is seven nodes: {graph}");

    let sampler = &nodes["3"];
    assert_eq!(sampler["class_type"], json!("KSampler"));
    assert_eq!(sampler["inputs"]["seed"].as_u64(), Some(4242));
    assert_eq!(sampler["inputs"]["steps"].as_u64(), Some(20));
    assert_eq!(sampler["inputs"]["cfg"].as_f64(), Some(engine::DEFAULT_CFG));
    assert_eq!(sampler["inputs"]["denoise"].as_f64(), Some(1.0));
    let latent = &nodes["5"];
    assert_eq!(latent["inputs"]["width"].as_u64(), Some(1024), "1025 was not snapped onto the grid");
    assert_eq!(latent["inputs"]["height"].as_u64(), Some(704), "700 was not snapped onto the grid");
    assert_eq!(latent["inputs"]["batch_size"].as_u64(), Some(1));
    assert_eq!(nodes["4"]["inputs"]["ckpt_name"], json!("sdxl_test.safetensors"));
    assert_eq!(
        nodes["9"]["inputs"]["filename_prefix"],
        json!(format!("Studio/{}", today())),
        "the server's own copy is filed under the same date as Studio's"
    );
    // Following the sampler's links, which is the part that cannot be checked by looking at one
    // node: swapped here, and every picture this app makes is the negative prompt.
    assert_eq!(text_of_link(&graph, &sampler["inputs"]["positive"]), "a lighthouse in fog");
    assert_eq!(text_of_link(&graph, &sampler["inputs"]["negative"]), "blurry, text");

    // ── the fetch ──
    let viewed = server.with(|recorded| recorded.viewed.clone());
    assert_eq!(viewed.len(), 1, "{viewed:?}");
    assert!(viewed[0].starts_with("/view?filename="), "{viewed:?}");
    assert!(viewed[0].contains("type=output"), "{viewed:?}");
    assert!(
        viewed[0].contains(&format!("subfolder=Studio%2F{}", today())),
        "the subfolder has to be percent-encoded, or the `&` in a filename would change what the next request asks for: {viewed:?}"
    );
    assert_eq!(server.with(|recorded| recorded.interrupts), 0, "nothing was cancelled");

    // ── and what landed ──
    let state = engine.snapshot().state();
    assert_eq!(state["backend"]["kind"], json!("comfyui"));
    assert_eq!(state["backend"]["generate_is_graded"], json!("standard"), "a server on this machine is local");
    assert_eq!(state["backend"]["prompt_leaves_this_machine"], json!(false));
    assert_eq!(state["gallery"]["count"], json!(1));
    let shot = &state["gallery"]["newest"][0];
    assert_eq!(shot["backend"], json!("comfyui"));
    assert_eq!(shot["model"], json!("sdxl_test.safetensors"));
    assert_eq!(shot["seed"], json!(4242));
    assert_eq!(shot["size"], json!("1024x704"));

    let path = PathBuf::from(shot["path"].as_str().unwrap());
    let sidecar = gallery::Sidecar::read(&path).unwrap();
    assert_eq!(sidecar.prompt, "a lighthouse in fog");
    assert_eq!(sidecar.negative, "blurry, text");
    assert_eq!(sidecar.seed, 4242);
    assert_eq!(sidecar.backend, "comfyui");
    assert_eq!(sidecar.sent, "1024x704", "the record names the size the graph was told to draw");
    assert_eq!(sidecar.steps, Some(20));
    // The bytes are what the server drew from the graph it was handed, computed here from the ask
    // rather than from the reply. This is the assertion that the whole round trip carried the
    // prompt, the seed and the size, and not merely that a file appeared.
    assert_eq!(
        std::fs::read(&path).unwrap(),
        backend::draw("a lighthouse in fog", 4242, 1024, 704),
        "the saved file is not the picture the server rendered from the graph"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_server_that_refuses_the_graph_passes_its_own_explanation_back() {
    let server = Server::start(Manners {
        refuse: Some((
            400,
            r#"{"error":{"message":"Value not in list: ckpt_name: 'sdxl_test.safetensors'","type":"invalid_prompt"}}"#.to_string(),
        )),
        never_finish: false,
    });
    let (dir, found) = world("refused");
    let engine = an_engine(&server.config("comfyui"), &found);
    let actions = surface(engine.clone());

    call(&actions, "generate", json!({ "prompt": "a lighthouse in fog", "width": 64, "height": 64 })).unwrap();
    let notice = settled(&engine);
    assert!(notice.contains("comfyui could not make the picture"), "{notice}");
    assert!(notice.contains("refused the graph (HTTP 400)"), "{notice}");
    // The server's own sentence is the useful part, so it is passed through rather than summarised
    // away: "Value not in list: ckpt_name" tells a person which file to name.
    assert!(notice.contains("Value not in list: ckpt_name"), "{notice}");
    assert!(!notice.contains("invalid_prompt"), "the whole envelope is noise next to its sentence: {notice}");
    assert_eq!(engine.snapshot().gallery.len(), 0, "a refused render left a file behind");
    // The refusal reaches a caller too, and not only the notice bar.
    let rendered = described(&engine, &actions);
    assert!(rendered["summary"].as_str().unwrap().contains("refused the graph"), "{}", rendered["summary"]);
    assert_eq!(rendered["state"]["gallery"]["count"], json!(0));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn cancelling_tells_the_server_to_stop_rendering() {
    let server = Server::start(Manners { never_finish: true, ..Manners::default() });
    let (dir, found) = world("cancel");
    let engine = an_engine(&server.config("comfyui"), &found);
    let actions = surface(engine.clone());

    let answer = call(&actions, "generate", json!({ "prompt": "a long render", "count": 2, "width": 64, "height": 64 }))
        .unwrap();
    let job = answer["job"].as_i64().unwrap();

    // The queue is how a caller watches work it cannot wait for.
    let mut waited = 0;
    let running = loop {
        let state = engine.snapshot().state();
        let running = state["queue"]["running"].as_array().unwrap().clone();
        if !running.is_empty() {
            break running;
        }
        assert!(waited < 200, "the job never started");
        waited += 1;
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(running[0]["id"], json!(job));
    assert_eq!(running[0]["kind"], json!("generate"));
    assert_eq!(running[0]["label"], json!("2 pictures of “a long render”"));
    assert_eq!(running[0]["progress"], json!("0 of 2"));
    assert!(engine.snapshot().summary().contains("1 picture being made"), "{}", engine.snapshot().summary());

    // A bad id must not reach the cancel-everything path: `-1` used to read as 0 through a float
    // cast, and 0 meant "no job was named". The running job is still running after each refusal.
    for not_a_job in [json!({}), json!({ "job": -1 }), json!({ "job": 0 }), json!({ "job": "two" })] {
        let problem = call(&actions, "cancel", not_a_job.clone()).unwrap_err();
        assert!(problem.contains("cancel_all"), "{not_a_job}: {problem}");
        assert!(
            !engine.snapshot().state()["queue"]["running"].as_array().unwrap().is_empty(),
            "{not_a_job} stopped a job it did not name"
        );
    }

    let answer = call(&actions, "cancel", json!({ "job": job })).unwrap();
    assert_eq!(answer["cancelled"], json!(format!("asked job {job} to stop")));
    // "cancelling" rather than gone: the worker is still in the middle of something, and a queue
    // that said nothing while a server was still rendering would be describing a busy machine as
    // idle. The worker checks the flag once a poll, so it can also have stopped before this
    // snapshot was taken; then the job is out of the queue and the notice is already the
    // cancelled one. Both are the truth, and a lie would be a queue that shows neither.
    let state = engine.snapshot().state();
    let queue = &state["queue"];
    let listed = queue["running"]
        .as_array()
        .unwrap()
        .iter()
        .chain(queue["pending"].as_array().unwrap().iter())
        .collect::<Vec<_>>();
    if listed.is_empty() {
        let notice = state["notice"].as_str().unwrap_or_default();
        assert!(notice.contains("Cancelled"), "{state}");
    } else {
        assert!(listed.iter().any(|each| each["status"] == json!("cancelling")), "{queue}");
    }

    let notice = settled(&engine);
    assert_eq!(notice, "Cancelled before anything was made.");
    // A render nobody is waiting for is still burning a GPU somebody owns, so the server is told.
    let mut waited = 0;
    while server.with(|recorded| recorded.interrupts) == 0 {
        assert!(waited < 200, "the server was never told to stop");
        waited += 1;
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(server.with(|recorded| recorded.interrupts) >= 1);
    // Nothing was written, so there is no half-file to clean up — which is what `cancel`'s purpose
    // promises.
    assert_eq!(engine.snapshot().gallery.len(), 0);
    assert!(!found.gallery.exists() || gallery::listing(&found.gallery, 12, false).is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

/// What the window's Cancel button asks for, published: every job in the queue stops, including
/// the ones that had not started. An empty queue is a refusal, as it is for every other failure —
/// tested above, where the rest of the surface's refusals are.
#[test]
fn cancel_all_stops_every_job_including_the_ones_not_started() {
    let server = Server::start(Manners { never_finish: true, ..Manners::default() });
    let (dir, found) = world("cancel-all");
    let engine = an_engine(&server.config("comfyui"), &found);
    let actions = surface(engine.clone());

    call(&actions, "generate", json!({ "prompt": "one long render", "width": 64, "height": 64 })).unwrap();
    call(&actions, "generate", json!({ "prompt": "another long render", "width": 64, "height": 64 })).unwrap();
    // Wait until one of them is actually RUNNING, not just listed: cancelling while both are still
    // pending asks the server to stop nothing, and the interrupt this test waits for below never
    // comes. That is what failed on CI's slower runner, where neither had started after 2 seconds.
    let mut waited = 0;
    loop {
        let state = engine.snapshot().state();
        let running = state["queue"]["running"].as_array().unwrap().len();
        let listed = running + state["queue"]["pending"].as_array().unwrap().len();
        if listed == 2 && running >= 1 {
            break;
        }
        assert!(waited < 1000, "the two jobs never reached the queue with one running");
        waited += 1;
        std::thread::sleep(Duration::from_millis(10));
    }

    let answer = call(&actions, "cancel_all", json!({})).unwrap();
    assert_eq!(answer["cancelled"], json!("asked 2 jobs to stop"));
    assert!(settled(&engine).contains("Cancelled"), "{}", engine.snapshot().summary());
    // The server is told about the render that was running; the one still pending never began.
    let mut waited = 0;
    while server.with(|recorded| recorded.interrupts) == 0 {
        assert!(waited < 1000, "the server was never told to stop");
        waited += 1;
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(engine.snapshot().gallery.len(), 0);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_server_nobody_is_listening_on_is_reported_as_unreachable() {
    // A port nothing is bound to: the listener is opened and dropped, so the number was real and is
    // now free.
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let (dir, found) = world("unreachable");
    let engine = an_engine(
        &format!(r#"{{"backend":{{"kind":"comfyui","base_url":"http://127.0.0.1:{port}"}}}}"#),
        &found,
    );
    let actions = surface(engine.clone());
    call(&actions, "generate", json!({ "prompt": "a lighthouse in fog", "width": 64, "height": 64 })).unwrap();
    let notice = settled(&engine);
    assert!(notice.contains("comfyui could not make the picture"), "{notice}");
    assert!(notice.contains(&format!("http://127.0.0.1:{port} could not be reached")), "{notice}");
    // The question a person asks next is in the answer, because "could not be reached" on its own
    // sends them to the logs.
    assert!(notice.contains("Is ComfyUI running there"), "{notice}");
    assert!(notice.contains("listening on something other than 127.0.0.1"), "{notice}");
    assert_eq!(engine.snapshot().gallery.len(), 0);
    std::fs::remove_dir_all(&dir).ok();
}

/// A key that is not a key. Named for this test alone, and read by nothing else in the binary, so
/// moving it cannot change another test's answer — which is the bar the rest of this crate's tests
/// hold by being handed their `Places` instead.
const KEY_VARIABLE: &str = "STUDIO_TEST_HOSTED_KEY";
const KEY_VALUE: &str = "sk-studio-test-not-a-real-key-9f3c2b";

#[test]
fn a_key_is_used_and_never_repeated_by_anything_the_app_says_or_writes() {
    std::env::set_var(KEY_VARIABLE, KEY_VALUE);
    let server = Server::start(Manners::default());
    let (dir, found) = world("key");
    let engine = an_engine(
        &format!(
            r#"{{"backend":{{"kind":"openai-images","base_url":"{}","model":"gpt-image-1","api_key_env":"{KEY_VARIABLE}"}}}}"#,
            server.base_url
        ),
        &found,
    );
    let actions = surface(engine.clone());
    assert_eq!(engine.grade(), "sensitive", "a hosted service is `sensitive` however trustworthy it is");

    // A hosted service accepts three sizes; 2000×700 is not one of them.
    call(&actions, "generate", json!({ "prompt": "a wide harbour", "width": 2000, "height": 700, "seed": 5 }))
        .unwrap();
    assert_eq!(settled(&engine), "");

    // The key was read from the environment at the moment of the call — that is the only place it
    // comes from — and it reached the service.
    let sent = server.with(|recorded| recorded.authorizations.clone());
    assert_eq!(sent, vec![format!("Bearer {KEY_VALUE}")]);
    let asked = server.with(|recorded| recorded.image_requests.clone());
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0]["model"], json!("gpt-image-1"));
    assert_eq!(asked[0]["prompt"], json!("a wide harbour"));
    assert_eq!(asked[0]["size"], json!("1536x1024"), "the nearest shape was not chosen");
    assert!(asked[0].get("response_format").is_none(), "gpt-image-1 rejects that field");

    // And it is nowhere else.
    let state = engine.snapshot().state();
    assert_eq!(state["backend"]["api_key_env"], json!(KEY_VARIABLE), "the variable's name is the person's to know");
    assert_eq!(state["backend"]["api_key_is_set"], json!(true));
    assert_eq!(state["backend"]["generate_is_graded"], json!("sensitive"));
    assert_eq!(state["backend"]["prompt_leaves_this_machine"], json!(true));
    let rendered = described(&engine, &actions).to_string();
    assert!(!rendered.contains(KEY_VALUE), "describe repeated the key");
    assert!(rendered.contains(KEY_VARIABLE));

    let snapshot = engine.snapshot();
    for text in [
        headline(&snapshot),
        facts_block(&snapshot),
        status_line(&snapshot),
        snapshot.summary(),
        snapshot.state().to_string(),
    ] {
        assert!(!text.contains(KEY_VALUE), "the window repeated the key: {text}");
    }
    // What the window does say is which variable to export, and whether it worked.
    let facts = facts_block(&snapshot);
    assert!(facts.contains(KEY_VARIABLE), "{facts}");
    assert!(facts.contains("set, so prompts can be sent"), "{facts}");
    assert!(facts.contains("graded `sensitive`"), "{facts}");
    assert!(facts.contains("sent off this machine"), "{facts}");

    let shot = &state["gallery"]["newest"][0];
    let path = PathBuf::from(shot["path"].as_str().unwrap());
    assert_eq!(shot["backend"], json!("openai-images"));
    assert_eq!(shot["model"], json!("gpt-image-1"));
    assert_eq!(shot["size"], json!("1536x1024"));
    let sidecar_text = std::fs::read_to_string(gallery::Sidecar::path_for(&path)).unwrap();
    assert!(!sidecar_text.contains(KEY_VALUE), "the record beside the picture holds the key");
    let sidecar = gallery::Sidecar::read(&path).unwrap();
    assert_eq!(sidecar.prompt, "a wide harbour");
    assert_eq!(sidecar.seed, 5);
    assert_eq!(sidecar.sent, "1536x1024");
    assert_eq!(std::fs::read(&path).unwrap(), backend::draw("a wide harbour", 1, 1536, 1024));

    // A backend that cannot send anything says so without saying why in terms of the key.
    std::env::remove_var(KEY_VARIABLE);
    let answer = call(
        &actions,
        "set_backend",
        json!({ "kind": "openai-images", "model": "gpt-image-1", "api_key_env": KEY_VARIABLE }),
    )
    .unwrap();
    assert!(answer["note"].as_str().unwrap().contains("is not set"), "{answer}");
    assert!(!answer.to_string().contains(KEY_VALUE));

    // The settings file — written by that `set_backend`, since opening an engine from text never
    // touches the disk — holds the variable's name and not its contents, which is the whole
    // reason `api_key_env` is a name.
    let written = std::fs::read_to_string(found.settings.as_ref().unwrap()).unwrap();
    assert!(!written.contains(KEY_VALUE), "the configuration file holds the key");
    assert!(written.contains(KEY_VARIABLE));
    assert!(written.contains("openai-images"));
    std::fs::remove_dir_all(&dir).ok();
}

// ── the window ────────────────────────────────────────────────────────────

type Queue = Arc<Mutex<VecDeque<Box<dyn FnOnce() + Send>>>>;

struct Proxy(Queue);

impl EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), slint::EventLoopError> {
        Ok(())
    }
    fn invoke_from_event_loop(&self, event: Box<dyn FnOnce() + Send>) -> Result<(), slint::EventLoopError> {
        self.0.lock().unwrap().push_back(event);
        Ok(())
    }
}

struct Headless {
    window: Rc<MinimalSoftwareWindow>,
    queue: Queue,
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Proxy(self.queue.clone())))
    }
}

/// Run the event queue, the timers and one draw. Returns whether anything was drawn.
fn tick(queue: &Queue, window: &MinimalSoftwareWindow) -> bool {
    // The lock is dropped before the callbacks run, because a callback enqueues another event.
    let events: Vec<_> = queue.lock().unwrap().drain(..).collect();
    for event in events {
        event();
    }
    slint::platform::update_timers_and_animations();
    let size = window.size();
    let mut buffer = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(size.width, size.height);
    window.draw_if_needed(|renderer| {
        renderer.render(buffer.make_mut_slice(), size.width as usize);
    })
}

fn wait_for(queue: &Queue, window: &MinimalSoftwareWindow, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        tick(queue, window);
        assert!(Instant::now() < deadline, "the window never reached the state it was waited for");
        std::thread::sleep(Duration::from_millis(10));
    }
    tick(queue, window);
}

/// Where the PNGs go.
///
/// The test binary lives in `<target>/<profile>/deps/`, so its own path names the real build
/// directory wherever cargo put it. CI builds this tree from a worktree against a shared target
/// directory outside the checkout, and a path that assumed otherwise would fail with ENOENT before
/// the test had asserted anything — which reads like a missing display and is not one.
fn shot_dir() -> PathBuf {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("ui-screenshots")))
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn screenshot(window: &MinimalSoftwareWindow, name: &str) -> PathBuf {
    let size = window.size();
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(size.width, size.height);
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(pixels.make_mut_slice(), size.width as usize);
    });
    let path = shot_dir().join(name);
    let mut png = png::Encoder::new(std::fs::File::create(&path).unwrap(), size.width, size.height);
    png.set_color(png::ColorType::Rgb);
    png.set_depth(png::BitDepth::Eight);
    png.write_header().unwrap().write_image_data(pixels.as_bytes()).unwrap();
    path
}

/// The real window, headless: Slint's own software renderer, with only the window-system event queue
/// substituted.
///
/// Everything else in this file could run without a display, and does. This is the one test that
/// needs a window, because the window is the half of the app a person uses: the callbacks the
/// buttons fire, the strings that reach the screen, and the rows the gallery draws.
///
/// It is one test rather than several because a process may install exactly one Slint platform —
/// `i-slint-core`'s `EVENTLOOP_PROXY` is a process-wide `OnceCell`, so a second `set_platform` fails
/// wherever it is called from. Every check below needs the same window, so they share one.
#[test]
fn the_window_shows_the_queue_the_gallery_and_the_record_of_the_chosen_picture() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let queue = Queue::default();
    slint::platform::set_platform(Box::new(Headless { window: window.clone(), queue: queue.clone() })).unwrap();

    let (dir, found) = world("window");
    // Unconfigured, which is what a machine with no GPU and no API key gets: the app has to open,
    // draw, and say plainly that the pictures are placeholders.
    let engine = Engine::open(config::Config::unconfigured(), found.clone());
    let ui = StudioApp::new().unwrap();
    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(1280, 860));
    // Held for the life of the window: a dropped Slint timer stops, and a stopped redraw timer is a
    // window that never shows the picture that arrived.
    let (_redraw, _poll) = wire(&ui, &engine);
    tick(&queue, &window);

    // ── empty ──
    assert_eq!(ui.get_backend_line(), "No backend configured — drawing placeholders from your prompt's hash");
    assert!(ui.get_facts().contains("No backend is configured"), "{}", ui.get_facts());
    assert!(ui.get_facts().contains("graded `standard`"), "{}", ui.get_facts());
    assert_eq!(ui.get_output_folder(), engine::display(&found.gallery));
    assert_eq!(ui.get_notice(), "");
    assert!(ui.get_status().contains("Nothing yet"), "{}", ui.get_status());
    assert!(!ui.get_busy());
    assert_eq!(ui.get_shots().row_count(), 0);
    assert_eq!(ui.get_jobs().row_count(), 0);
    assert_eq!(ui.get_detail(), "", "the right pane starts as an invitation, not as a record");
    // The ask fields start at the engine's defaults. A field that looks empty and means 1024 is a
    // field a person cannot trust.
    assert_eq!(ui.get_want_width(), "1024");
    assert_eq!(ui.get_want_steps(), "30");
    assert_eq!(ui.get_negative(), engine::DEFAULT_NEGATIVE);
    let empty = screenshot(&window, "studio-empty.png");
    assert!(std::fs::metadata(&empty).unwrap().len() > 1000, "{} is blank", empty.display());

    // ── a generation, from the button a person presses ──
    ui.set_prompt("a lighthouse in fog".into());
    ui.set_want_width("256".into());
    ui.set_want_height("192".into());
    ui.set_want_steps("4".into());
    ui.set_want_count("2".into());
    ui.invoke_action("generate".into());
    // `on_action` paints before it returns, so the queue is on screen at once — which matters,
    // because a render takes longer than a person's patience and the strip is what tells them it
    // heard them. The fake backend is fast enough that the worker can also have finished before
    // that paint, in which case the pictures are up instead; both are the window telling the
    // truth, so both are accepted here — but one of them has to be on screen at once, with
    // nothing in between.
    if ui.get_busy() {
        assert_eq!(ui.get_jobs().row_count(), 1);
        let job = ui.get_jobs().row_data(0).unwrap();
        assert_eq!(job.label, "2 pictures of “a lighthouse in fog”");
        assert!(job.status == "pending" || job.status == "running", "{}", job.status);
        assert!(job.progress == "0 of 2" || job.progress == "1 of 2", "{}", job.progress);
        // The footer names the running job, or says one is waiting to start; which of the two
        // depends on whether the worker thread has been scheduled yet, and both are true.
        assert!(
            ui.get_status().contains("2 pictures of") || ui.get_status().contains("waiting to start"),
            "the footer does not name the job: {}",
            ui.get_status()
        );
    } else {
        assert_eq!(ui.get_shots().row_count(), 2, "the window shows neither the queue nor the pictures");
    }

    wait_for(&queue, &window, || !ui.get_busy());
    assert_eq!(ui.get_jobs().row_count(), 0, "the queue kept a job that had finished");
    assert_eq!(ui.get_shots().row_count(), 2);
    assert!(ui.get_status().contains("2 pictures in"), "{}", ui.get_status());
    assert!(ui.get_status().contains("pictures are files"), "{}", ui.get_status());

    let top = ui.get_shots().row_data(0).unwrap();
    assert_eq!(top.prompt, "a lighthouse in fog");
    assert!(top.meta.contains("fake"), "{}", top.meta);
    assert!(top.meta.contains("seed "), "{}", top.meta);
    assert!(top.meta.contains("256×192"), "{}", top.meta);
    assert!(top.meta.contains(" s"), "{}", top.meta);
    assert!(top.path.starts_with("~/") || top.path.starts_with(&dir.display().to_string()), "{}", top.path);
    // The thumbnail was decoded on a worker and crossed to this thread as plain RGB, because
    // Slint's own image type is not `Send`.
    assert!(top.has_thumb, "the row has no thumbnail");
    let size = top.thumb.size();
    assert!(size.width > 0 && size.height > 0, "the thumbnail is empty");
    assert!(size.width <= gallery::THUMBNAIL_EDGE && size.height <= gallery::THUMBNAIL_EDGE, "{size:?}");
    assert!(!top.selected);
    let gallery = screenshot(&window, "studio-gallery.png");
    assert!(std::fs::metadata(&gallery).unwrap().len() > 1000, "{} is blank", gallery.display());

    // ── choosing a row reads its record back ──
    ui.invoke_choose(0);
    let detail = ui.get_detail().to_string();
    assert!(detail.contains("Prompt: a lighthouse in fog"), "{detail}");
    assert!(detail.contains("Kept out: "), "{detail}");
    assert!(detail.contains("Seed: "), "{detail}");
    assert!(detail.contains("Size: 256×192"), "{detail}");
    assert!(detail.contains("Steps: 4"), "{detail}");
    assert!(detail.contains("Made by: fake"), "{detail}");
    assert!(detail.contains("Took: "), "{detail}");
    assert!(detail.contains("When: "), "{detail}");
    assert!(ui.get_shots().row_data(0).unwrap().selected, "the chosen row does not show as chosen");
    let chosen = screenshot(&window, "studio-detail.png");
    assert!(std::fs::metadata(&chosen).unwrap().len() > 1000, "{} is blank", chosen.display());

    // Choosing the same row again puts the pane back, because a second click on a selected row is
    // what a person does to undo it.
    ui.invoke_choose(0);
    assert_eq!(ui.get_detail(), "");
    assert!(!ui.get_shots().row_data(0).unwrap().selected);

    // ── the buttons that change nothing on disk ──
    ui.invoke_action("backend".into());
    let notice = ui.get_notice().to_string();
    assert!(notice.contains("set_backend is graded `sensitive`"), "{notice}");
    assert!(notice.contains("studio.json"), "{notice}");
    // `refresh` clears the notice rather than leaving the last thing that went wrong on screen.
    ui.invoke_action("refresh".into());
    assert_eq!(ui.get_notice(), "");

    // A refusal from a button reaches the notice bar, which is where the person is looking, and
    // `describe`, which is where a mind would look.
    ui.set_prompt("   ".into());
    ui.invoke_action("generate".into());
    assert!(ui.get_notice().contains("a prompt is needed"), "{}", ui.get_notice());
    assert!(!ui.get_busy(), "a refused generation started a job");
    assert!(engine.snapshot().notice.contains("a prompt is needed"));

    // ── a row's own buttons ──
    // They hand back "<verb>:<path>" rather than an index, because the index would be an index into
    // the list the click is about to change.
    let name = ui.get_shots().row_data(0).unwrap().name.to_string();
    let path = engine.resolve(&name, false).unwrap();
    ui.invoke_action(format!("upscale:{}", path.display()).into());
    // The wait ends when the queue is empty, and the paint that empties the queue is the same
    // paint that adds the row — the worker saves before it reports, and a snapshot is one lock —
    // so there is no moment at which the window shows neither.
    wait_for(&queue, &window, || !ui.get_busy());
    assert_eq!(ui.get_shots().row_count(), 3);
    let top = ui.get_shots().row_data(0).unwrap();
    assert!(top.meta.contains("made from upscale of"), "{}", top.meta);
    assert!(top.meta.contains("512×384"), "{}", top.meta);

    // A refusal on purpose: a path that is not there must delete nothing, and the window must say
    // so rather than sit there.
    ui.invoke_action(format!("delete:{}/no-such-picture.png", dir.display()).into());
    assert!(ui.get_notice().contains("is not there"), "{}", ui.get_notice());
    assert_eq!(ui.get_shots().row_count(), 3, "a refused delete changed the gallery");

    ui.invoke_action(format!("delete:{}", engine.resolve(&top.name, false).unwrap().display()).into());
    assert_eq!(ui.get_shots().row_count(), 2, "the deleted picture is still on screen");
    assert!(ui.get_notice().contains("Moved"), "{}", ui.get_notice());

    // `open` and `folder` are not pressed here: they spawn `xdg-open`, which on a machine with a
    // desktop would open a window nobody asked for. `engine::open` is one `Command::spawn` and
    // `resolve` — the part with the logic — is exercised from both sides above.
    engine.shutdown();
    std::fs::remove_dir_all(&dir).ok();
}

// ── one run, start to finish, with nothing but this process ───────────────

/// The brief's "one real run": the whole app driven from a binary, with the fake backend, on a
/// machine that has no GPU, no API key and no ComfyUI. It is a test rather than a transcript because
/// a transcript goes stale and a test does not.
#[test]
fn a_whole_session_runs_from_one_sentence_to_a_filed_picture() {
    let (dir, found) = world("session");
    let engine = an_engine(r#"{"backend":{"kind":"fake"}}"#, &found);
    let actions = surface(engine.clone());

    // 1. What is this app?
    let rendered = described(&engine, &actions);
    assert!(rendered["summary"].as_str().unwrap().contains("nothing in the gallery yet"));
    assert_eq!(rendered["actions"].as_array().unwrap().len(), 9);

    // 2. Make two pictures from one sentence.
    call(&actions, "generate", json!({ "prompt": "a lighthouse in fog", "seed": 99, "count": 2, "width": 512, "height": 384 }))
        .unwrap();
    assert_eq!(settled(&engine), "");

    // 3. Read them back.
    let rendered = described(&engine, &actions);
    let newest = rendered["state"]["gallery"]["newest"].as_array().unwrap().clone();
    assert_eq!(newest.len(), 2);
    assert!(rendered["summary"].as_str().unwrap().contains("2 pictures in"));

    // 4. Vary the one a person would pick, and enlarge it.
    let name = PathBuf::from(newest[0]["path"].as_str().unwrap())
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    call(&actions, "variations", json!({ "of": &name })).unwrap();
    assert_eq!(settled(&engine), "");
    call(&actions, "upscale", json!({ "path": &name })).unwrap();
    assert_eq!(settled(&engine), "");

    // 5. Decide against one and take it back out of the gallery.
    let rendered = described(&engine, &actions);
    assert_eq!(rendered["state"]["gallery"]["count"], json!(4));
    let oldest = rendered["state"]["gallery"]["newest"].as_array().unwrap().last().unwrap()["path"]
        .as_str()
        .unwrap()
        .to_string();
    let answer = call(&actions, "delete", json!({ "path": &oldest })).unwrap();
    assert_eq!(answer["recoverable"], json!(true));
    assert_eq!(described(&engine, &actions)["state"]["gallery"]["count"], json!(3));

    // 6. And the folder is the real thing: three pictures, three records, all of them readable by
    //    anything else on this machine.
    let day = gallery::day_folder(&found.gallery, chrono::Local::now());
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
        let record = gallery::Sidecar::read(&day.join(name)).unwrap_or_else(|| panic!("{name} has no record"));
        assert!(!record.prompt.is_empty(), "{name}");
        assert_eq!(record.made_by, "yantrik-studio");
        assert!(["fake", "studio-resample"].contains(&record.backend.as_str()), "{name}: {}", record.backend);
    }
    // The Trash holds the one that was deleted, where Files can see it.
    assert_eq!(trash::items(&found.trash).unwrap().len(), 2);
    std::fs::remove_dir_all(&dir).ok();
}

/// A picture's own record, read back through the surface rather than through the filesystem, is the
/// same record. This is the join the whole app rests on: a mind that reads `describe` and a person
/// who opens the folder have to be looking at one thing.
#[test]
fn the_gallery_the_window_and_the_files_agree() {
    let (dir, found) = world("agree");
    let engine = an_engine(r#"{"backend":{"kind":"fake"}}"#, &found);
    let actions = surface(engine.clone());
    let name = one_picture(&actions, &engine, "a harbour at dusk");
    let path = engine.resolve(&name, false).unwrap();

    let from_the_surface = engine.snapshot().state()["gallery"]["newest"][0].clone();
    let from_the_disk = gallery::Sidecar::read(&path).unwrap();
    assert_eq!(from_the_surface["prompt"], json!(from_the_disk.prompt));
    assert_eq!(from_the_surface["seed"], json!(from_the_disk.seed));
    assert_eq!(from_the_surface["backend"], json!(from_the_disk.backend));
    assert_eq!(from_the_surface["model"], json!(from_the_disk.model));
    assert_eq!(
        from_the_surface["size"],
        json!(format!("{}x{}", from_the_disk.width, from_the_disk.height))
    );
    assert_eq!(from_the_surface["path"].as_str().unwrap(), path.display().to_string());

    // And the row a person sees is built from the same record, beside the JSON rather than far away
    // from it.
    let shown = row(&engine.snapshot().gallery[0], "");
    assert_eq!(shown.prompt, from_the_disk.prompt);
    assert!(shown.meta.contains(&format!("seed {}", from_the_disk.seed)), "{}", shown.meta);
    assert!(shown.meta.contains("fake"), "{}", shown.meta);
    assert!(shown.meta.contains("256×256"), "{}", shown.meta);
    assert_eq!(shown.name, name);
    assert!(!shown.selected);

    // A picture with no record beside it says so in all three places at once.
    std::fs::remove_file(gallery::Sidecar::path_for(&path)).unwrap();
    call(&actions, "refresh", json!({})).unwrap();
    let mut waited = 0;
    let stale = loop {
        let snapshot = engine.snapshot();
        if let Some(each) = snapshot.gallery.iter().find(|each| each.path == path) {
            if !each.has_sidecar {
                break each.clone();
            }
        }
        assert!(waited < 200, "the gallery never noticed the record was gone");
        waited += 1;
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(stale.json()["sidecar"], json!("missing"));
    let painted = row(&stale, "");
    assert!(painted.meta.contains("no record beside it"), "{}", painted.meta);
    assert_eq!(painted.prompt, "(no prompt recorded beside this file)");
    assert!(detail_for(&engine, &name).contains("There is no record beside this picture"));
    std::fs::remove_dir_all(&dir).ok();
}
