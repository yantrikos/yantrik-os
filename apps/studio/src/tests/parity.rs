//! The parity pin: the action table and `describe` exactly as Studio published them before the
//! framework. The table below was extracted mechanically from the old `surface()` in
//! `apps/studio/src/main.rs` (at origin/main 493f1ef4, before the port): a script read that
//! function's source and evaluated every string literal the way rustc reads it (a backslash at the
//! end of a line joining it to the next), and the rows are what it wrote, not words copied by hand
//! from the new `intents.rs`, so a word changed there fails here.
//!
//! The `describe` half pins the keys of the state in the four shapes the old tests drove it
//! through, and the summary line of each.

use serde_json::{json, Value};
use yantrik_app::intents::Intents;

use super::{fake, started, unconfigured};
use crate::Studio;

/// What `set_backend` says first since agents may not call it (the hotfix #992 on main put the same
/// words before the description it always had, naming the file the person edits).
const PERSONS_ALONE: &str = "The person's alone: an agent that calls this is refused and should ask the person to edit ~/.config/yantrik/studio.json. ";

struct Golden {
    name: &'static str,
    risk: &'static str,
    description: &'static str,
    deferred: bool,
    explains: bool,
    /// Name, JSON type, required, description.
    params: Vec<(&'static str, &'static str, bool, &'static str)>,
}

fn before() -> Vec<Golden> {
    vec![
        Golden {
            name: "generate",
            risk: "standard",
            description: r####"Make pictures from a sentence and save them as files in the output folder, each with a small JSON record beside it naming the prompt, the seed, the model, the backend and how many seconds it took. This starts the work rather than finishing it: the answer carries the job's id, and `describe` lists that job under `queue.running` until the pictures land in `gallery.newest`. Graded `sensitive` while a hosted backend is configured, because the sentence is sent to a machine you do not own and the service may charge for each picture; graded `standard` on a ComfyUI server on your own network and on `fake`, the placeholder backend, where the prompt never leaves the building."####,
            deferred: true,
            explains: false,
            params: vec![
                ("prompt", "string", true, r####"The sentence the picture is made from. Up to 4000 characters, and the only required argument."####),
                ("negative", "string", false, r####"What to keep out of the picture. Left out, Studio sends its own default negative prompt; sent as an empty string, no negative prompt is sent at all."####),
                ("width", "number", false, r####"Pixels wide. Rounded onto the 8-pixel grid the models want and kept between 64 and 4096. Left out means 1024. A hosted service may only accept a few sizes, in which case the nearest one is used and the record beside the picture names the size that was actually sent."####),
                ("height", "number", false, r####"Pixels high. Same rounding and same limits as `width`. Left out means 1024."####),
                ("steps", "number", false, r####"Sampling steps: more is slower and usually sharper. Between 1 and 150, left out means 30. A hosted service may ignore this entirely."####),
                ("seed", "number", false, r####"The seed to start from. Left out, one is chosen at random and written into the record beside the picture, so any picture can be asked for again. Asking for several pictures with one seed runs the seeds on: 7 makes 7, 8 and 9."####),
                ("count", "number", false, r####"How many pictures to make, 1 to 4. A hosted backend charges for each one, which is why this stops at four."####),
            ],
        },
        Golden {
            name: "variations",
            risk: "standard",
            description: r####"Make more pictures of one that already exists, taken from the record beside it: the same sentence, the same size, the same step count, new seeds. `of` is the picture's path, or just its filename if it is in the gallery. Starts the work and answers with the job's id. Graded the same way as `generate`, because it sends the same prompt to the same backend."####,
            deferred: true,
            explains: false,
            params: vec![
                ("of", "string", true, r####"The picture to vary: its path, or its bare filename if it is in Studio's gallery."####),
                ("count", "number", false, r####"How many variations, 1 to 4. Left out means 1."####),
            ],
        },
        Golden {
            name: "upscale",
            risk: "standard",
            description: r####"Make a bigger copy of a picture by resampling it on this machine, with no model and no network. It is a resize and not an upscaler: it interpolates the pixels that are already there and cannot add detail that was never generated. The copy is saved into the gallery with a record naming the picture it came from. Nothing is sent anywhere, whatever backend is configured, and no backend is asked."####,
            deferred: true,
            explains: false,
            params: vec![
                ("path", "string", true, r####"The picture to enlarge: a path, or a bare filename from the gallery."####),
                ("factor", "number", false, r####"How many times bigger on each edge, 2 to 8. Left out means 2, so a 1024×1024 picture becomes 2048×2048."####),
            ],
        },
        Golden {
            name: "open",
            risk: "standard",
            description: r####"Open a picture — or a folder, if a folder is named — in whatever this machine opens it with. The file is not changed and nothing leaves the machine."####,
            deferred: false,
            explains: false,
            params: vec![
                ("path", "string", true, r####"What to open: a picture's path, a folder's path, or a bare filename from the gallery."####),
            ],
        },
        Golden {
            name: "delete",
            risk: "standard",
            description: r####"Move a picture, and the record beside it, to the Trash. It is recoverable: the files are moved into the OS's own trash folder, not destroyed, and Files can put them back. Only files inside Studio's output folder can be deleted this way — this is not a way to remove an arbitrary file."####,
            deferred: false,
            explains: false,
            params: vec![
                ("path", "string", true, r####"The picture to move to the Trash: a path, or a bare filename from the gallery."####),
            ],
        },
        Golden {
            name: "set_backend",
            risk: "sensitive",
            description: r####"Choose where pictures are made from now on, and write that choice down in the configuration file. Graded `sensitive` because it decides where every later prompt goes: naming a hosted service means the sentences typed into this app will leave this machine and may cost money. `generate` and `variations` are regraded the moment this lands, so a caller cannot point Studio at a service and generate in the same breath under the old, local grade. No key is taken here — only the NAME of an environment variable that holds one, which is read at call time and never stored, logged or shown."####,
            deferred: false,
            explains: true,
            params: vec![
                ("kind", "string", true, r####"Which backend: `comfyui` for a ComfyUI server (your own GPU), `openai-images` for any OpenAI-compatible /v1/images/generations endpoint (a hosted service), or `fake` for the placeholder that draws from the prompt's hash on this machine."####),
                ("base_url", "string", false, r####"Where the backend is. Left out, a kind takes its own default: http://127.0.0.1:8188 for ComfyUI, https://api.openai.com/v1 for a hosted service."####),
                ("model", "string", false, r####"Which model to ask for: a ComfyUI checkpoint filename, or a hosted service's image model. Required for `openai-images`, because a guess here spends money on the wrong thing."####),
                ("api_key_env", "string", false, r####"The NAME of the environment variable holding the API key for a hosted service — for example OPENAI_API_KEY. Only the name is written down; the value is read from the environment each time and never stored."####),
                ("workflow", "string", false, r####"A path to a ComfyUI workflow in API format, to use instead of the built-in SDXL text-to-image graph. A file saved from ComfyUI's editor rather than its 'Save (API Format)' is refused with an explanation, not silently misread."####),
            ],
        },
        Golden {
            name: "cancel",
            risk: "standard",
            description: r####"Stop one job. A picture already saved stays saved, and there is no half-written file to clean up: Studio writes nothing until a whole picture has arrived. To stop everything in the queue, ask for `cancel_all`."####,
            deferred: false,
            explains: false,
            params: vec![
                ("job", "number", true, r####"The job id from `generate`, `variations` or `upscale`, as `describe` lists under `queue`: a whole number of 1 or more."####),
            ],
        },
        Golden {
            name: "cancel_all",
            risk: "standard",
            description: r####"Stop every job in the queue, which is what the window's Cancel button asks for. A picture already saved stays saved, as with `cancel`: Studio writes nothing until a whole picture has arrived."####,
            deferred: false,
            explains: false,
            params: vec![
            ],
        },
        Golden {
            name: "refresh",
            risk: "safe",
            description: r####"Read the output folder again. Pictures this app made are noticed on their own within a few seconds; this is for one copied in by hand, or for a folder that changed while Studio was not looking. Read-only: it changes nothing and sends nothing."####,
            deferred: false,
            explains: false,
            params: vec![
            ],
        },
    ]
}

fn table() -> Vec<yantrik_app::control::Action> {
    let mut table = Intents::<Studio>::new();
    <Studio as yantrik_app::App>::intents(&mut table);
    table.specs()
}

#[test]
fn the_nine_actions_are_published_as_they_were() {
    let specs = table();
    assert_eq!(
        specs.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["generate", "variations", "upscale", "open", "delete", "set_backend", "cancel", "cancel_all", "refresh"],
        "the surface a mind is offered changed shape"
    );
    for (spec, golden) in specs.iter().zip(before()) {
        let name = golden.name;
        assert_eq!(spec.name, name);
        // The one word changed on purpose: `set_backend` says an agent cannot call it.
        let description = if name == "set_backend" { format!("{PERSONS_ALONE}{}", golden.description) } else { golden.description.to_string() };
        assert_eq!(spec.description, description, "{name}: description");
        assert_eq!(spec.permission, golden.risk, "{name}: grade");
        assert_eq!(spec.expected_seconds, None, "{name}: expected seconds");
        assert_eq!(spec.deferred, golden.deferred, "{name}: deferred");
        assert!(!spec.stateless, "{name}");
        assert_eq!(spec.explainer.is_some(), golden.explains, "{name}: explains");
        let params: Vec<(&str, &str, bool, &str)> =
            spec.params.iter().map(|p| (p.name.as_str(), p.kind, p.required, p.description.as_str())).collect();
        assert_eq!(params, golden.params, "{name}: parameters");
        assert!(spec.params.iter().all(|p| p.default.is_none() && !p.enumerated), "{name}");
    }
}

#[test]
fn only_the_three_that_start_work_say_they_settle_later() {
    for spec in table() {
        let starts_work = matches!(spec.name.as_str(), "generate" | "variations" | "upscale");
        assert_eq!(spec.schema()["settles"] == json!("later"), starts_work, "{}", spec.name);
        // Nothing here is open-ended: no action runs whatever it is given.
        assert!(!spec.description.contains(yantrik_app::control::OPEN_ENDED), "{}", spec.name);
    }
}

/// What a caller sees in `describe`: the nine, and the framework's `launch`, which every app on the
/// framework publishes.
#[test]
fn describe_lists_the_nine_and_the_frameworks_launch() {
    let f = fake("parity-list");
    let actions = f.describe()["actions"].as_array().cloned().unwrap_or_default();
    let mut names: Vec<String> = actions.iter().filter_map(|a| a["name"].as_str().map(String::from)).collect();
    let mut expected: Vec<String> = before().iter().map(|g| g.name.to_string()).collect();
    expected.push("launch".to_string());
    names.sort();
    expected.sort();
    assert_eq!(names, expected);
    for golden in before() {
        let action = actions.iter().find(|a| a["name"] == json!(golden.name)).expect("published");
        assert_eq!(action["permission"], json!(golden.risk), "{}", golden.name);
        let required: Vec<&str> = golden.params.iter().filter(|p| p.2).map(|p| p.0).collect();
        assert_eq!(action["parameters"]["required"], json!(required), "{}", golden.name);
    }
    // The fact that `set_backend` can explain one call of itself is published; the sentence never.
    let explains = |name: &str| actions.iter().find(|a| a["name"] == name).unwrap().get("explains").cloned();
    assert_eq!(explains("set_backend"), Some(json!(true)));
    for golden in before().iter().filter(|g| g.name != "set_backend") {
        assert_eq!(explains(golden.name), None, "`{}` publishes what it always did", golden.name);
    }
}

/// docs/app-control.md's rows for Studio say what the app is and what it publishes.
#[test]
fn the_guides_rows_are_still_true() {
    let doc = include_str!("../../../../docs/app-control.md");
    let surface = doc.lines().find(|l| l.starts_with("| `studio` |")).expect("the surfaces table names it");
    let cells: Vec<&str> = surface.split('|').map(str::trim).collect();
    assert_eq!(cells[2], "`app-studio.sock`", "the socket is the id's: {surface}");
    assert_eq!(cells[3], "\u{2014}", "{surface}");
    assert_eq!(
        cells[4],
        "make pictures from a sentence, on your own GPU or a hosted service; they land as files",
        "{surface}"
    );

    // The grades table: every action once, at the grade the code holds.
    let rows: Vec<Vec<&str>> = doc
        .lines()
        .filter(|l| l.starts_with("| studio |"))
        .map(|l| l.split('|').map(str::trim).collect())
        .collect();
    assert_eq!(rows.len(), 4, "{rows:?}");
    let listed = |row: &[&str]| -> Vec<String> { row[2].split(',').map(|n| n.trim().trim_matches('`').to_string()).collect() };
    let grades = [
        (vec!["refresh"], "safe"),
        (vec!["generate", "variations"], "standard \u{21c4} sensitive"),
        (vec!["set_backend"], "sensitive"),
        (vec!["upscale", "open", "delete", "cancel", "cancel_all"], "standard"),
    ];
    for (row, (names, grade)) in rows.iter().zip(grades) {
        assert_eq!(listed(row), names, "{row:?}");
        assert_eq!(row[3], grade, "{row:?}");
    }
    // Every action is in exactly one row, and the rows' grades are the ones the code publishes
    // (`generate` and `variations` are declared `standard` and move with the backend).
    let mut in_rows: Vec<String> = rows.iter().flat_map(|row| listed(row)).collect();
    let mut published: Vec<String> = before().iter().map(|g| g.name.to_string()).collect();
    in_rows.sort();
    published.sort();
    assert_eq!(in_rows, published);
    for golden in before() {
        let row = rows.iter().find(|row| listed(row).contains(&golden.name.to_string())).unwrap();
        let grade = if matches!(golden.name, "generate" | "variations") { "standard \u{21c4} sensitive" } else { golden.risk };
        assert_eq!(row[3], grade, "{}", golden.name);
    }
}

fn keys(value: &Value, prefix: &str, out: &mut Vec<String>) {
    if let Some(map) = value.as_object() {
        for (key, inner) in map {
            let path = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
            out.push(path.clone());
            keys(inner, &path, out);
        }
    }
}

fn strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

fn key_paths(state: &Value) -> Vec<String> {
    let mut out = Vec::new();
    keys(state, "", &mut out);
    out.sort();
    out
}

#[test]
fn a_machine_nobody_set_up_is_described_as_it_was() {
    let f = unconfigured("parity-unconfigured");
    let d = f.describe();
    assert_eq!(
        key_paths(&d["state"]),
        strings(&[
            "backend",
            "backend.configured",
            "backend.generate_is_graded",
            "backend.kind",
            "backend.note",
            "backend.prompt_leaves_this_machine",
            "backend.where",
            "gallery",
            "gallery.count",
            "gallery.newest",
            "output_folder",
            "queue",
            "queue.pending",
            "queue.running",
        ])
    );
    assert_eq!(d["summary"], json!("Studio \u{2014} no backend configured, so pictures are placeholders; nothing in the gallery yet"));
    assert_eq!(d["state"]["backend"]["kind"], json!("fake"));
    assert_eq!(d["state"]["backend"]["where"], json!("this machine"));
    assert_eq!(d["state"]["backend"]["configured"], json!(false));
    assert_eq!(d["state"]["backend"]["generate_is_graded"], json!("standard"));
    assert_eq!(d["state"]["backend"]["prompt_leaves_this_machine"], json!(false));
    assert_eq!(d["state"]["gallery"], json!({ "count": 0, "newest": [] }));
    assert_eq!(d["state"]["queue"], json!({ "running": [], "pending": [] }));
    assert_eq!(d["state"]["output_folder"], json!(crate::model::display(&f.gallery())));
}

#[test]
fn two_pictures_and_a_notice_are_described_as_they_were() {
    let f = fake("parity-two");
    f.act("generate", json!({ "prompt": "a lighthouse in fog", "seed": 7, "count": 2, "width": 64, "height": 64, "steps": 4 }))
        .unwrap();
    assert_eq!(f.settled(), "");
    let d = f.describe();
    let folder = crate::model::display(&f.gallery());
    assert_eq!(d["summary"], json!(format!("Studio \u{2014} placeholder images, because no image model is configured; 2 pictures in {folder}")));
    let paths = key_paths(&d["state"]);
    for expected in [
        "backend.kind",
        "gallery.count",
        "gallery.newest",
        "queue.running",
        "queue.pending",
        "output_folder",
    ] {
        assert!(paths.contains(&expected.to_string()), "{expected} is gone: {paths:?}");
    }
    assert!(!paths.contains(&"notice".to_string()), "an empty notice is not a fact");
    assert!(!paths.contains(&"backend.note".to_string()) || d["state"]["backend"]["note"].as_str().unwrap().contains("placeholder"));
    let row = &d["state"]["gallery"]["newest"][0];
    let mut row_keys: Vec<&str> = row.as_object().unwrap().keys().map(String::as_str).collect();
    row_keys.sort();
    assert_eq!(row_keys, ["backend", "created", "model", "path", "prompt", "seconds", "seed", "size"]);

    // A refusal leads the summary, and is the state's `notice`.
    f.act("cancel_all", json!({})).unwrap_err();
    let d = f.describe();
    assert_eq!(
        d["summary"],
        json!(format!("Studio \u{2014} nothing was queued or running; placeholder images, because no image model is configured; 2 pictures in {folder}"))
    );
    assert_eq!(d["state"]["notice"], json!("nothing was queued or running"));
}

#[test]
fn each_kind_of_backend_is_described_as_it_was() {
    std::env::remove_var("STUDIO_ABSENT_KEY");
    let lan = started("parity-lan", r#"{"backend":{"kind":"comfyui","base_url":"http://192.168.4.35:8188","model":"mine.safetensors"}}"#);
    let d = lan.describe();
    assert_eq!(d["summary"], json!("Studio \u{2014} ComfyUI at http://192.168.4.35:8188; nothing in the gallery yet"));
    assert_eq!(
        key_paths(&d["state"]["backend"]),
        strings(&["configured", "generate_is_graded", "kind", "model", "prompt_leaves_this_machine", "where"])
    );
    assert_eq!(d["state"]["backend"]["where"], json!("a ComfyUI server you reach at http://192.168.4.35:8188"));
    assert_eq!(d["state"]["backend"]["model"], json!("mine.safetensors"));

    let away = started("parity-away", r#"{"backend":{"kind":"comfyui","base_url":"http://203.0.113.7:8188"}}"#);
    let d = away.describe();
    assert_eq!(d["summary"], json!("Studio \u{2014} ComfyUI away from your network at http://203.0.113.7:8188; nothing in the gallery yet"));
    assert_eq!(d["state"]["backend"]["generate_is_graded"], json!("sensitive"));
    assert_eq!(d["state"]["backend"]["prompt_leaves_this_machine"], json!(true));

    let hosted = started(
        "parity-hosted",
        r#"{"backend":{"kind":"openai-images","model":"gpt-image-1","api_key_env":"STUDIO_ABSENT_KEY"}}"#,
    );
    let d = hosted.describe();
    assert_eq!(d["summary"], json!("Studio \u{2014} a hosted service at https://api.openai.com/v1; nothing in the gallery yet"));
    let backend = &d["state"]["backend"];
    // The key is there, and holds the name only when that variable is set (it is not).
    assert_eq!(backend["api_key_env"], json!(null));
    assert_eq!(backend["api_key_is_set"], json!(false));
    assert_eq!(backend["generate_is_graded"], json!("sensitive"));
    assert!(backend["note"].as_str().unwrap().starts_with("Prompts cannot be sent yet:"), "{backend}");
    assert_eq!(
        key_paths(backend),
        strings(&[
            "api_key_env",
            "api_key_is_set",
            "configured",
            "generate_is_graded",
            "kind",
            "model",
            "note",
            "prompt_leaves_this_machine",
            "where"
        ])
    );
}
