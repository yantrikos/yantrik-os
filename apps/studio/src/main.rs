//! Yantrik Studio — pictures from a sentence, made where the person says, saved as files.
//!
//! The window and the control surface are two views of one state. Everything a person can press
//! here is an action a mind can name there, and both read the same `Engine`, so an answer given to
//! `yos act studio generate` and the row that appears in the gallery are the same event seen twice.
//!
//! The part that is not like the other apps is that making a picture takes seconds to minutes, and
//! the thread that owns this window has a three-second budget on every control call. So the engine
//! runs generation on workers and this file does no waiting at all: `generate` answers with a job id
//! and declares itself deferred, and a 250 ms timer redraws when the engine's revision moves.
//!
//! Nothing here touches the network. That is `backend.rs`, which this file only ever reaches through
//! the engine, so a test can point the whole app at the `fake` backend or at a socket in this
//! process and exercise the real window without a GPU or an API key.

mod backend;
mod config;
mod engine;
mod gallery;
mod trash;
mod workflow;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use engine::{Engine, Plan, Snapshot};
use gallery::Record;
use serde_json::{json, Value};
use slint::{ComponentHandle, ModelRc, VecModel};
use yantrik_app_runtime::control::{self, Action, App, Param, View};
use yantrik_app_runtime::prelude::*;

slint::include_modules!();

#[cfg(test)]
mod tests;

fn main() {
    init_tracing("yantrik-studio");
    if std::env::var("LIBGL_ALWAYS_SOFTWARE").as_deref() == Ok("1")
        && matches!(
            std::env::var("SLINT_BACKEND").as_deref(),
            Ok("winit") | Err(_)
        )
    {
        std::env::set_var("SLINT_BACKEND", "winit-software");
    }

    // One window per machine. A second Studio would be a second gallery cache over one folder, and
    // the person who launched it would be looking at a queue that is not the one making pictures.
    let Some(_instance) = instance::claim("studio") else {
        return;
    };

    let ui = StudioApp::new().unwrap();
    // The window's title bar is the app's own (#256).
    yantrik_app_runtime::window_chrome!(ui);
    let prefs = theme::load();
    ui.global::<ThemeMode>().set_dark(prefs.dark);
    ui.global::<AccentPreset>().set_index(prefs.accent_index);

    let engine = Engine::new();
    // Held for the life of the window: a dropped Slint timer stops, and a stopped redraw timer is a
    // window that never shows the picture that arrived.
    let (_redraw, _poll) = wire(&ui, &engine);
    publish_control(&engine);

    ui.invoke_focus_prompt();
    run_until_closed(&ui, "yantrik-studio");
    engine.shutdown();
}

/// Which picture the right-hand pane is reading back.
///
/// Held here rather than derived in `paint`, because reading it means opening the sidecar file, and
/// `paint` runs four times a second while a job is going. A click pays for one small read; a redraw
/// pays for none.
#[derive(Clone, Default)]
struct Chosen {
    path: String,
    detail: String,
}

/// How wide the window has to be to carry the record pane on the right. The number lives here
/// rather than in the .slint because the .slint may not read the window's own width inside the
/// layout the window's size is derived from — Slint reports that as a binding loop on
/// `layoutinfo-h` and says a loop there may panic at runtime. Rust measures the window and hands
/// the answer in as a plain property, so the .slint tree has no edge back to the window size.
const WIDE_WINDOW: f32 = 900.0;

fn is_wide(ui: &StudioApp) -> bool {
    let window = ui.window();
    window.size().width as f32 / window.scale_factor() > WIDE_WINDOW
}

/// Wire the window to the engine. Returns the two timers, which the caller must hold.
fn wire(ui: &StudioApp, engine: &Engine) -> (slint::Timer, slint::Timer) {
    // The ask fields start at the engine's defaults rather than at blank. A field that looks empty
    // and means "1024" is a field a person cannot trust, and one that shows 1024 can be edited.
    ui.set_prompt(String::new().into());
    ui.set_negative(engine::DEFAULT_NEGATIVE.into());
    ui.set_want_width(engine::DEFAULT_WIDTH.to_string().into());
    ui.set_want_height(engine::DEFAULT_HEIGHT.to_string().into());
    ui.set_want_steps(engine::DEFAULT_STEPS.to_string().into());
    ui.set_want_count("1".into());
    // Once here, so the first frame is the right shape, and then on every redraw tick, because a
    // resize moves no engine revision and the tick is the only thing that would notice it.
    ui.set_wide(is_wide(ui));

    let chosen = Rc::new(RefCell::new(Chosen::default()));

    // Choosing a picture reads its record back. Choosing the same one again puts the pane back to
    // its invitation, because a second click on a selected row is what a person does to undo it.
    {
        let engine = engine.clone();
        let chosen = chosen.clone();
        let weak = ui.as_weak();
        ui.on_choose(move |index| {
            let snapshot = engine.snapshot();
            let Some(record) = snapshot.gallery.get(index as usize) else {
                return;
            };
            let path = record.path.display().to_string();
            let next = if chosen.borrow().path == path {
                Chosen::default()
            } else {
                Chosen { detail: detail_for(&engine, &path), path }
            };
            *chosen.borrow_mut() = next;
            if let Some(ui) = weak.upgrade() {
                let now = chosen.borrow().clone();
                paint(&ui, &engine, &now);
            }
        });
    }

    // Every button in the window, including the four on a gallery row. A row button hands back
    // "<verb>:<path>" rather than an index, because the index would be the index into the list the
    // click is about to change.
    {
        let engine = engine.clone();
        let chosen = chosen.clone();
        let weak = ui.as_weak();
        ui.on_action(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            let id = id.to_string();
            let (verb, target) = match id.split_once(':') {
                Some((verb, target)) => (verb.to_string(), target.to_string()),
                None => (id.clone(), String::new()),
            };
            let outcome = press(&ui, &engine, &verb, &target);
            if let Err(problem) = outcome {
                // A button that does nothing and says nothing is the failure this OS's own lint
                // exists to catch. The refusal goes to the notice bar, which is where the person
                // is looking, and to `describe`, which is where a mind would look.
                engine.say(problem);
            }
            let now = chosen.borrow().clone();
            paint(&ui, &engine, &now);
        });
    }

    let initial = chosen.borrow().clone();
    paint(ui, engine, &initial);

    // Redraw when the engine says something moved. Comparing one integer rather than repainting
    // unconditionally: `design/home-2026-09-22.md` measured Notes idling at three percent of a core
    // doing exactly that, and this window's rows carry decoded thumbnails.
    let redraw = slint::Timer::default();
    {
        let weak = ui.as_weak();
        let engine = engine.clone();
        let chosen = chosen.clone();
        // Not 0: the first tick has to draw the empty gallery's counters, and revision 0 is a
        // revision the engine really does start at.
        let seen = Rc::new(RefCell::new(u64::MAX));
        redraw.start(slint::TimerMode::Repeated, Duration::from_millis(250), move || {
            let Some(ui) = weak.upgrade() else { return };
            // Before the revision check, which returns early: the window can have been resized
            // with nothing happening in the engine, and the pane split still has to follow.
            if ui.get_wide() != is_wide(&ui) {
                ui.set_wide(is_wide(&ui));
            }
            let revision = engine.revision();
            if *seen.borrow() == revision {
                return;
            }
            *seen.borrow_mut() = revision;
            let now = chosen.borrow().clone();
            paint(&ui, &engine, &now);
        });
    }

    // Ask the folder whether anything arrived from outside. Four seconds, and the ask is two
    // directory timestamps rather than a listing, so this is free while nothing is happening.
    let poll = slint::Timer::default();
    {
        let engine = engine.clone();
        poll.start(slint::TimerMode::Repeated, Duration::from_secs(4), move || engine.poll());
    }

    (redraw, poll)
}

/// What one press of one button means. Shared by the window's own callbacks and nothing else: the
/// control surface has its own handlers, because a mind passes arguments rather than clicking, and
/// the two have to be able to refuse in their own words.
fn press(
    ui: &StudioApp,
    engine: &Engine,
    verb: &str,
    target: &str,
) -> Result<Value, String> {
    match verb {
        "generate" => {
            let plan = ask_from_the_window(ui);
            let job = engine.generate(plan)?;
            Ok(json!({ "job": job }))
        }
        "refresh" => {
            engine.clear_notice();
            engine.spawn_relist();
            Ok(json!({ "refreshed": true }))
        }
        "folder" => {
            let folder = engine.places().gallery;
            let shown = engine::display(&folder);
            engine::open(&folder).map(|()| json!({ "opened": shown }))
        }
        "open" => {
            let path = engine.resolve(target, false)?;
            let shown = engine::display(&path);
            engine::open(&path).map(|()| json!({ "opened": shown }))
        }
        "variations" => {
            // Fill the prompt box with the sentence the chosen picture was made from, so the person
            // who pressed this can see what is about to be asked for again — and change it.
            if let Ok(path) = engine.resolve(target, false) {
                if let Some(sidecar) = gallery::Sidecar::read(&path) {
                    ui.set_prompt(sidecar.prompt.into());
                }
            }
            let job = engine.variations(target, 1)?;
            Ok(json!({ "job": job }))
        }
        "upscale" => {
            let job = engine.upscale(target, 2)?;
            Ok(json!({ "job": job }))
        }
        "delete" => {
            let moved = engine.delete(target)?;
            Ok(json!({ "moved_to_trash": moved.len(), "recoverable": true }))
        }
        "cancel" => engine.cancel(None).map(|line| json!({ "cancelled": line })),
        "backend" => {
            // There is no dialog here on purpose. Where a person's prompts go is a decision worth
            // one file and one sentence, not a form with a field for a secret; and the surface can
            // already make the same change, graded `sensitive`, with an approval card in front of
            // it. So the button says where the decision lives.
            let path = config::config_path()
                .map(|path| engine::display(&path))
                .unwrap_or_else(|| "~/.config/yantrik/studio.json".to_string());
            engine.say(format!(
                "Pictures are made by {} right now. To change that, edit {path}, or ask: \
                 yos act studio set_backend kind=comfyui base_url=http://127.0.0.1:8188 \
                 — set_backend is graded `sensitive`, because it decides where your prompts go.",
                engine.config().facts(false).place
            ));
            Ok(json!({ "config_file": path }))
        }
        other => Err(format!("Studio has no button called `{other}`.")),
    }
}

/// The ask, as the window has it.
///
/// An unparsable number arrives as 0, which `Plan::checked` reads as "the default" for a size and
/// as "not given" for the rest. Steps and count are filled in here instead, because `checked` clamps
/// 0 steps to 1 and refuses a count of 0 outright — and a person who cleared the field wants a
/// picture, not a refusal about a text box.
fn ask_from_the_window(ui: &StudioApp) -> Plan {
    let mut plan = Plan::new(ui.get_prompt().to_string());
    plan.negative = ui.get_negative().to_string();
    plan.width = typed(ui.get_want_width());
    plan.height = typed(ui.get_want_height());
    let steps = typed(ui.get_want_steps());
    plan.steps = if steps == 0 { engine::DEFAULT_STEPS } else { steps };
    let count = typed(ui.get_want_count());
    plan.count = count.clamp(1, engine::MAX_COUNT);
    plan
}

/// A number somebody typed into a text field.
fn typed(value: slint::SharedString) -> u32 {
    value.trim().parse::<u32>().unwrap_or(0)
}

/// Draw one snapshot of the engine onto the window.
///
/// Every string arrives written. The window formats nothing, so the sentence a person reads and the
/// JSON a mind reads are built beside each other in this file and cannot drift apart.
fn paint(ui: &StudioApp, engine: &Engine, chosen: &Chosen) {
    let snapshot = engine.snapshot();

    // A chosen picture that is no longer in the gallery was deleted or moved out from under us.
    // Dropping the selection here rather than in `delete` covers both that and the one a file
    // manager did, and it is twelve rows to look through.
    let nothing = Chosen::default();
    let chosen = if !chosen.path.is_empty()
        && !snapshot.gallery.iter().any(|row| row.path.display().to_string() == chosen.path)
    {
        &nothing
    } else {
        chosen
    };

    ui.set_backend_line(headline(&snapshot).into());
    ui.set_facts(facts_block(&snapshot).into());
    ui.set_output_folder(engine::display(&snapshot.places.gallery).into());
    ui.set_notice(snapshot.notice.clone().into());
    ui.set_status(status_line(&snapshot).into());
    ui.set_busy(!snapshot.jobs.is_empty());
    ui.set_detail(chosen.detail.clone().into());

    let rows: Vec<ShotRow> =
        snapshot.gallery.iter().map(|record| row(record, &chosen.path)).collect();
    ui.set_shots(ModelRc::new(VecModel::from(rows)));

    let jobs: Vec<JobRow> = snapshot
        .jobs
        .iter()
        .map(|job| JobRow {
            label: job.label.clone().into(),
            status: job.status.clone().into(),
            progress: if job.wanted > 1 {
                format!("{} of {}", job.made, job.wanted).into()
            } else {
                String::new().into()
            },
        })
        .collect();
    ui.set_jobs(ModelRc::new(VecModel::from(jobs)));
}

/// The line under the app's own name: which backend, and whether the prompt leaves.
fn headline(snapshot: &Snapshot) -> String {
    let facts = &snapshot.facts;
    if !facts.configured {
        return "No backend configured — drawing placeholders from your prompt's hash".to_string();
    }
    let mut line = format!("{} · {}", facts.kind, facts.place);
    if facts.prompt_leaves {
        line.push_str(" · your prompt leaves this machine");
    }
    line
}

/// The right pane's account of where pictures are made, including the ways it is incomplete.
fn facts_block(snapshot: &Snapshot) -> String {
    let facts = &snapshot.facts;
    let mut lines = vec![
        format!("Backend: {}", facts.kind),
        format!("Where: {}", facts.place),
    ];
    if !facts.model.is_empty() && facts.kind != "fake" {
        lines.push(format!("Model: {}", facts.model));
    }
    if facts.kind == "openai-images" {
        // The variable's name, never its contents. A person has to be able to tell which variable
        // to export; the value is the one thing this window must not show, so the only question
        // asked of it is whether it is there.
        let usable = snapshot.config.backend.api_key().is_ok();
        lines.push(format!(
            "API key: the environment variable {}, which is {}.",
            snapshot.config.backend.api_key_env,
            if usable { "set, so prompts can be sent" } else { "not usable, so nothing can be sent yet" }
        ));
    }
    lines.push(format!(
        "Making a picture is graded `{}` on this configuration.",
        facts.grade
    ));
    if facts.prompt_leaves {
        lines.push("Your prompt is sent off this machine, and the service may charge for it.".to_string());
    } else {
        lines.push("Your prompt does not leave a network you own.".to_string());
    }
    if !facts.note.is_empty() {
        lines.push(facts.note.clone());
    }
    lines.join("\n")
}

/// The footer: what is happening, or what is waiting to.
fn status_line(snapshot: &Snapshot) -> String {
    if let Some(job) = snapshot.jobs.iter().find(|job| job.status == "running") {
        let progress = if job.wanted > 1 {
            format!(" — {} of {}", job.made, job.wanted)
        } else {
            String::new()
        };
        return format!("{}{} · {:.0}s so far", job.label, progress, job.seconds);
    }
    if !snapshot.jobs.is_empty() {
        return format!("{} waiting to start", snapshot.jobs.len());
    }
    format!(
        "{} in {} · pictures are files, and each has a record beside it",
        match snapshot.gallery.len() {
            0 => "Nothing yet".to_string(),
            1 => "1 picture".to_string(),
            n => format!("{n} pictures"),
        },
        engine::display(&snapshot.places.gallery)
    )
}

/// One gallery row.
fn row(record: &Record, chosen: &str) -> ShotRow {
    let path = record.path.display().to_string();
    ShotRow {
        name: record.name.clone().into(),
        prompt: if record.prompt.is_empty() {
            "(no prompt recorded beside this file)".into()
        } else {
            record.prompt.clone().into()
        },
        meta: meta_line(record).into(),
        path: engine::display(&record.path).into(),
        has_thumb: record.thumbnail.is_some(),
        thumb: record.thumbnail.as_ref().map(pixels).unwrap_or_default(),
        selected: path == chosen,
    }
}

/// The one line under a thumbnail: everything the brief says a gallery should show, in the order a
/// person reads it.
fn meta_line(record: &Record) -> String {
    let size = format!("{}×{}", record.width, record.height);
    if !record.has_sidecar {
        // Not a guess presented as a record. A picture with no sidecar is a picture this app cannot
        // account for, and saying so is the difference between a gallery and a pile of files.
        return format!("{size} · no record beside it, so nothing is known about how it was made");
    }
    let mut parts = vec![record.backend.clone()];
    if !record.model.is_empty() {
        parts.push(record.model.clone());
    }
    parts.push(format!("seed {}", record.seed));
    parts.push(size);
    parts.push(format!("{:.1} s", record.seconds));
    if !record.made_from.is_empty() {
        parts.push(format!("made from {}", record.made_from));
    }
    parts.join(" · ")
}

/// The right pane, for one chosen picture. Read from the sidecar, not from the listing: the sidecar
/// is the record, and the pane is where a person checks that the record is real.
fn detail_for(engine: &Engine, named: &str) -> String {
    let Ok(path) = engine.resolve(named, false) else {
        return String::new();
    };
    let Some(sidecar) = gallery::Sidecar::read(&path) else {
        return format!(
            "{}\n\nThere is no record beside this picture, so nothing can be said about how it was \
             made. It may have been copied into the folder by hand.",
            engine::display(&path)
        );
    };
    let mut lines = vec![
        engine::display(&path),
        String::new(),
        format!("Prompt: {}", sidecar.prompt),
    ];
    if !sidecar.negative.is_empty() {
        lines.push(format!("Kept out: {}", sidecar.negative));
    }
    lines.push(format!("Seed: {}", sidecar.seed));
    lines.push(format!("Size: {}×{}", sidecar.width, sidecar.height));
    if !sidecar.sent.is_empty() {
        lines.push(format!("Asked the backend for: {}", sidecar.sent));
    }
    if let Some(steps) = sidecar.steps {
        lines.push(format!(
            "Steps: {}{}",
            steps,
            sidecar.cfg.map(|cfg| format!(", guidance {cfg}")).unwrap_or_default()
        ));
    }
    lines.push(format!("Made by: {}", sidecar.backend));
    if !sidecar.model.is_empty() {
        lines.push(format!("Model: {}", sidecar.model));
    }
    lines.push(format!("Took: {:.1} seconds", sidecar.seconds));
    lines.push(format!("When: {}", sidecar.created));
    if !sidecar.made_from.is_empty() {
        lines.push(format!("Made from: {}", sidecar.made_from));
    }
    lines.join("\n")
}

/// Decoded pixels into the one image type Slint will draw.
///
/// The bytes cross from the worker as plain RGB, because Slint's `Image` is not `Send` and a
/// thumbnail decoded on a render thread cannot be handed to the UI thread as an image. Copying into
/// the buffer's own bytes keeps the row order Slint expects and needs no unsafe.
fn pixels(thumb: &gallery::Thumbnail) -> slint::Image {
    let mut buffer = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(thumb.width, thumb.height);
    let bytes = buffer.make_mut_bytes();
    let take = bytes.len().min(thumb.rgb.len());
    bytes[..take].copy_from_slice(&thumb.rgb[..take]);
    slint::Image::from_rgb8(buffer)
}

// ── the control surface ──

fn publish_control(engine: &Engine) {
    let described = engine.clone();
    let mut app = App::new("studio").describe(move || view(&described));
    for (spec, run) in surface(engine.clone()) {
        app = app.action(spec, run);
    }
    app.serve();

    // An action's grade is fixed when the surface is published, and the configuration was read
    // before that. So the first thing this app does after publishing is bring `generate` and
    // `variations` to the grade the backend on disk actually deserves: a machine already pointed at
    // a hosted service must not offer to send a prompt at the local grade, not even for the first
    // call. `set_backend` does the same thing on every later change.
    let grade = engine.grade();
    for action in ["generate", "variations"] {
        if let Err(problem) = control::regrade(action, grade) {
            tracing::error!("`{action}` could not be graded `{grade}` at startup: {problem}");
        }
    }
}

/// What `yos describe studio` prints. The engine holds all of it, so no window is needed and a
/// caller can read the app while its window is minimised.
fn view(engine: &Engine) -> View {
    let snapshot = engine.snapshot();
    View::new(snapshot.summary()).state(snapshot.state())
}

/// One published action: what it says it does, and the code that does it.
///
/// Built as a list rather than pushed straight into `App`, so the tests can read exactly what a
/// mind is shown and can run a handler with no socket, no event loop and no window.
///
/// Not one of these handlers reaches for the window, which is a rule and not an oversight. The
/// surface has to answer while the window is minimised, and everything it can say or do comes from
/// the engine — the same state the window is painted from. A handler that grabbed the window would
/// work on a desktop and fail over SSH, and every test below would need a display to run.
type Handler = Box<dyn Fn(&Value) -> Result<Value, String>>;

fn surface(engine: Engine) -> Vec<(Action, Handler)> {
    let mut out: Vec<(Action, Handler)> = Vec::new();
    let mut add = |spec: Action, run: fn(&Engine, &Value) -> Result<Value, String>| {
        let engine = engine.clone();
        out.push((spec, Box::new(move |args: &Value| run(&engine, args)) as Handler));
    };

    add(
        act(
            "generate",
            "Make pictures from a sentence and save them as files in the output folder, each with a \
             small JSON record beside it naming the prompt, the seed, the model, the backend and \
             how many seconds it took. This starts the work rather than finishing it: the answer \
             carries the job's id, and `describe` lists that job under `queue.running` until the \
             pictures land in `gallery.newest`. Graded `sensitive` while a hosted backend is \
             configured, because the sentence is sent to a machine you do not own and the service \
             may charge for each picture; graded `standard` on a ComfyUI server on your own network \
             and on the fake backend, where the prompt never leaves the building.",
        )
        .defers()
        .arg(arg(
            "prompt",
            "The sentence the picture is made from. Up to 4000 characters, and the only required \
             argument.",
        ))
        .arg(
            arg("negative", "What to keep out of the picture. Left out, Studio sends its own \
             default negative prompt; sent as an empty string, no negative prompt is sent at all.")
                .optional(),
        )
        .arg(num("width", "Pixels wide. Rounded onto the 8-pixel grid the models want and kept \
             between 64 and 4096. Left out means 1024. A hosted service may only accept a few \
             sizes, in which case the nearest one is used and the record beside the picture names \
             the size that was actually sent.").optional())
        .arg(num("height", "Pixels high. Same rounding and same limits as `width`. Left out means \
             1024.").optional())
        .arg(num("steps", "Sampling steps: more is slower and usually sharper. Between 1 and 150, \
             left out means 30. A hosted service may ignore this entirely.").optional())
        .arg(num("seed", "The seed to start from. Left out, one is chosen at random and written \
             into the record beside the picture, so any picture can be asked for again. Asking for \
             several pictures with one seed runs the seeds on: 7 makes 7, 8 and 9.").optional())
        .arg(num("count", "How many pictures to make, 1 to 4. A hosted backend charges for each \
             one, which is why this stops at four.").optional()),
        do_generate,
    );

    add(
        act(
            "variations",
            "Make more pictures of one that already exists, taken from the record beside it: the \
             same sentence, the same size, the same step count, new seeds. `of` is the picture's \
             path, or just its filename if it is in the gallery. Starts the work and answers with \
             the job's id. Graded the same way as `generate`, because it sends the same prompt to \
             the same backend.",
        )
        .defers()
        .arg(arg(
            "of",
            "The picture to vary: its path, or its bare filename if it is in Studio's gallery.",
        ))
        .arg(num("count", "How many variations, 1 to 4. Left out means 1.").optional()),
        do_variations,
    );

    add(
        act(
            "upscale",
            "Make a bigger copy of a picture by resampling it on this machine, with no model and no \
             network. It is a resize and not an upscaler: it interpolates the pixels that are \
             already there and cannot add detail that was never generated. The copy is saved into \
             the gallery with a record naming the picture it came from. Nothing is sent anywhere, \
             whatever backend is configured, and no backend is asked.",
        )
        .defers()
        .arg(arg("path", "The picture to enlarge: a path, or a bare filename from the gallery."))
        .arg(
            num("factor", "How many times bigger on each edge, 2 to 8. Left out means 2, so a \
             1024×1024 picture becomes 2048×2048.")
                .optional(),
        ),
        do_upscale,
    );

    add(
        act(
            "open",
            "Open a picture — or a folder, if a folder is named — in whatever this machine opens it \
             with. The file is not changed and nothing leaves the machine.",
        )
        .arg(arg("path", "What to open: a picture's path, a folder's path, or a bare filename from \
             the gallery.")),
        do_open,
    );

    add(
        act(
            "delete",
            "Move a picture, and the record beside it, to the Trash. It is recoverable: the files \
             are moved into the OS's own trash folder, not destroyed, and Files can put them back. \
             Only files inside Studio's output folder can be deleted this way — this is not a way \
             to remove an arbitrary file.",
        )
        .arg(arg("path", "The picture to move to the Trash: a path, or a bare filename from the \
             gallery.")),
        do_delete,
    );

    add(
        act(
            "set_backend",
            "Choose where pictures are made from now on, and write that choice down in the \
             configuration file. Graded `sensitive` because it decides where every later prompt \
             goes: naming a hosted service means the sentences typed into this app will leave this \
             machine and may cost money. `generate` and `variations` are regraded the moment this \
             lands, so a caller cannot point Studio at a service and generate in the same breath \
             under the old, local grade. No key is taken here — only the NAME of an environment \
             variable that holds one, which is read at call time and never stored, logged or \
             shown.",
        )
        .risk("sensitive")
        .arg(arg(
            "kind",
            "Which backend: `comfyui` for a ComfyUI server (your own GPU), `openai-images` for any \
             OpenAI-compatible /v1/images/generations endpoint (a hosted service), or `fake` for \
             the placeholder that draws from the prompt's hash on this machine.",
        ))
        .arg(
            arg("base_url", "Where the backend is. Left out, a kind takes its own default: \
             http://127.0.0.1:8188 for ComfyUI, https://api.openai.com/v1 for a hosted service.")
                .optional(),
        )
        .arg(arg("model", "Which model to ask for: a ComfyUI checkpoint filename, or a hosted \
             service's image model. Required for `openai-images`, because a guess here spends \
             money on the wrong thing.").optional())
        .arg(arg("api_key_env", "The NAME of the environment variable holding the API key for a \
             hosted service — for example OPENAI_API_KEY. Only the name is written down; the value \
             is read from the environment each time and never stored.").optional())
        .arg(arg("workflow", "A path to a ComfyUI workflow in API format, to use instead of the \
             built-in SDXL text-to-image graph. A file saved from ComfyUI's editor rather than its \
             'Save (API Format)' is refused with an explanation, not silently misread.").optional())
        // The paragraph above can only state the condition ("naming a hosted service means the
        // sentences typed into this app will leave this machine"); whether THIS call meets it
        // turns on the `kind` it carries, which `describe` never sees (#137). So the card gets
        // the unconditional half from here: what the arguments establish about where prompts go
        // after this call, and nothing guessed beyond that. A kind this closure does not know
        // gets an honest nothing rather than a guess — the card then reads as it did before.
        .explain(|args| {
            match args.get("kind").and_then(Value::as_str).unwrap_or_default().trim() {
                "fake" => "After this, prompts stay on this machine.".to_string(),
                "openai-images" => {
                    let url = given(args, "base_url");
                    let url = url.trim();
                    if url.is_empty() {
                        "After this, prompts go to api.openai.com and may cost money.".to_string()
                    } else {
                        // Name the PARSED HOST, never the raw argument: `base_url` is
                        // caller-supplied text, and "https://api.openai.com@evil.example/v1"
                        // would read as naming api.openai.com while every prompt and the key
                        // go to evil.example. A URL that parses to no host gets an honest
                        // nothing rather than a guess, like a kind this closure does not know.
                        match url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_owned))
                        {
                            Some(host) => {
                                format!("After this, prompts go to {host} and may cost money.")
                            }
                            None => String::new(),
                        }
                    }
                }
                _ => String::new(),
            }
        }),
        do_set_backend,
    );

    add(
        act(
            "cancel",
            "Stop one job. A picture already saved stays saved, and there is no half-written file \
             to clean up: Studio writes nothing until a whole picture has arrived. To stop \
             everything in the queue, ask for `cancel_all`.",
        )
        .arg(num("job", "The job id from `generate`, `variations` or `upscale`, as `describe` lists \
             under `queue`: a whole number of 1 or more.")),
        do_cancel,
    );

    add(
        act(
            "cancel_all",
            "Stop every job in the queue, which is what the window's Cancel button asks for. A \
             picture already saved stays saved, as with `cancel`: Studio writes nothing until a \
             whole picture has arrived.",
        ),
        do_cancel_all,
    );

    add(
        act(
            "refresh",
            "Read the output folder again. Pictures this app made are noticed on their own within a \
             few seconds; this is for one copied in by hand, or for a folder that changed while \
             Studio was not looking. Read-only: it changes nothing and sends nothing.",
        )
        .risk("safe"),
        do_refresh,
    );

    out
}

fn do_generate(engine: &Engine, args: &Value) -> Result<Value, String> {
    let mut plan = Plan::new(text(args, "prompt"));
    // Present and empty means "no negative prompt", which is a choice; absent means "yours",
    // which is the default. Telling those apart is the whole reason this is not `given`.
    if let Some(negative) = args.get("negative").and_then(|v| v.as_str()) {
        plan.negative = negative.to_string();
    }
    plan.width = number(args, "width");
    plan.height = number(args, "height");
    let steps = number(args, "steps");
    plan.steps = if steps == 0 { engine::DEFAULT_STEPS } else { steps };
    let count = number(args, "count");
    plan.count = if count == 0 { 1 } else { count };
    // Read separately from the other numbers, because `number` caps at u32 — which is right for a
    // size and wrong for a seed. Truncating 4294967296 to 0 would still make a picture, and the
    // record beside it would then name a seed that does not reproduce it.
    plan.seed = wide(args, "seed");

    let plan = plan.checked().map_err(|problem| refuse(engine, problem))?;
    let job = engine.generate(plan.clone()).map_err(|problem| refuse(engine, problem))?;
    let snapshot = engine.snapshot();
    Ok(json!({
        "job": job,
        "queued": true,
        "count": plan.count,
        "backend": snapshot.facts.kind,
        "where": snapshot.facts.place,
        "prompt_leaves_this_machine": snapshot.facts.prompt_leaves,
        "output_folder": engine::display(&snapshot.places.gallery),
        "read_back": "the job appears in describe's queue.running; when the pictures land they \
                      appear in gallery.newest with their path, seed and seconds",
    }))
}

fn do_variations(engine: &Engine, args: &Value) -> Result<Value, String> {
    let of = needed(engine, args, "variations", "of")?;
    let count = number(args, "count");
    let count = if count == 0 { 1 } else { count };
    let job = engine.variations(&of, count).map_err(|problem| refuse(engine, problem))?;
    Ok(json!({ "job": job, "queued": true, "of": of, "count": count }))
}

fn do_upscale(engine: &Engine, args: &Value) -> Result<Value, String> {
    let path = needed(engine, args, "upscale", "path")?;
    let factor = number(args, "factor");
    let factor = if factor == 0 { 2 } else { factor };
    let job = engine.upscale(&path, factor).map_err(|problem| refuse(engine, problem))?;
    Ok(json!({ "job": job, "queued": true, "of": path, "factor": factor, "done_on": "this machine" }))
}

fn do_open(engine: &Engine, args: &Value) -> Result<Value, String> {
    let named = needed(engine, args, "open", "path")?;
    let path = engine.resolve(&named, false).map_err(|problem| refuse(engine, problem))?;
    engine::open(&path).map_err(|problem| refuse(engine, problem))?;
    Ok(json!({ "opened": engine::display(&path) }))
}

fn do_delete(engine: &Engine, args: &Value) -> Result<Value, String> {
    let named = needed(engine, args, "delete", "path")?;
    let moved = engine.delete(&named).map_err(|problem| refuse(engine, problem))?;
    let snapshot = engine.snapshot();
    Ok(json!({
        "moved_to_trash": moved.iter().map(|each| json!({
            "name": each.name,
            "was": engine::display(&each.original),
            "now": engine::display(&each.stored),
        })).collect::<Vec<_>>(),
        "recoverable": true,
        "gallery_count": snapshot.gallery.len(),
    }))
}

fn do_set_backend(engine: &Engine, args: &Value) -> Result<Value, String> {
    let kind = needed(engine, args, "set_backend", "kind")?;
    let snapshot = engine
        .set_backend(
            &kind,
            &given(args, "base_url"),
            &given(args, "model"),
            &given(args, "api_key_env"),
            &given(args, "workflow"),
        )
        .map_err(|problem| refuse(engine, problem))?;
    // Never the key, and never the variable's contents. The name is here because a person has to be
    // told which variable to export.
    Ok(json!({
        "backend": snapshot.facts.kind,
        "where": snapshot.facts.place,
        "configured": snapshot.facts.configured,
        "api_key_env": snapshot.config.backend.api_key_env,
        "generate_is_now_graded": snapshot.facts.grade,
        "prompt_leaves_this_machine": snapshot.facts.prompt_leaves,
        "note": snapshot.facts.note,
        "output_folder": engine::display(&snapshot.places.gallery),
    }))
}

fn do_cancel(engine: &Engine, args: &Value) -> Result<Value, String> {
    // An id nobody was ever issued is a mistake about which job was meant, and is refused as that:
    // `-1` once read as 0 through a saturating float cast, 0 meant "no job was named", and the
    // whole queue stopped. Stopping everything is a real ask with its own action, `cancel_all`.
    let Some(id) = job_id(args) else {
        return Err(refuse(
            engine,
            match args.get("job") {
                None => "`cancel` needs `job`, an id from `describe`'s `queue`. To stop every job, \
                         ask for `cancel_all`.",
                Some(_) => "`cancel` was given a `job` that is not an id: an id is a whole number \
                            of 1 or more. To stop every job, ask for `cancel_all`.",
            },
        ));
    };
    let line = engine
        .cancel(Some(id))
        .map_err(|problem| refuse(engine, problem))?;
    Ok(json!({ "cancelled": line }))
}

fn do_cancel_all(engine: &Engine, _args: &Value) -> Result<Value, String> {
    let line = engine.cancel(None).map_err(|problem| refuse(engine, problem))?;
    Ok(json!({ "cancelled": line }))
}

fn do_refresh(engine: &Engine, _args: &Value) -> Result<Value, String> {
    engine.clear_notice();
    engine.spawn_relist();
    let snapshot = engine.snapshot();
    Ok(json!({ "refreshed": true, "gallery_count": snapshot.gallery.len() }))
}

/// A refusal the person at the window sees too.
///
/// Failure is said twice: once to whoever asked, once in the notice bar, which is also
/// `describe`'s notice. A mind that could not generate and a person who cannot see why are the same
/// bug counted twice.
fn refuse(engine: &Engine, message: impl Into<String>) -> String {
    let message = message.into();
    engine.say(message.clone());
    message
}

/// One action, with the sentence a reader who cannot see the screen needs.
///
/// The guard is the point: a description that is only the action's own name stops the app before
/// `serve()`, and stops the tests, which build this same list. `Action::new` takes any `&str` and
/// cannot be made to care, and the runtime is shared by every app, so the check lives here.
fn act(name: &'static str, sentence: &'static str) -> Action {
    assert!(
        sentence.len() >= 40 && !sentence.starts_with("Studio:"),
        "studio action `{name}` was given a placeholder description: {sentence:?}"
    );
    Action::new(name, sentence)
}

/// One text argument. Required is the default, which is the opposite of what a loop that marked
/// everything optional would do: a call naming nothing must not be accepted. An argument that may
/// be left out says `.optional()` at its call site, where a reader of the list can see it.
fn arg(name: &'static str, sentence: &'static str) -> Param {
    checked_argument(Param::text(name), name, sentence)
}

/// One optional number argument.
fn num(name: &'static str, sentence: &'static str) -> Param {
    checked_argument(Param::number(name), name, sentence)
}

fn checked_argument(param: Param, name: &'static str, sentence: &'static str) -> Param {
    assert!(
        sentence.len() >= 15,
        "studio argument `{name}` was given no usable description: {sentence:?}"
    );
    param.describe(sentence)
}

/// A required text argument that has to say something, or a refusal naming it.
///
/// The runtime refuses a *missing* required argument before the handler runs. This is the other
/// half: one that is present and blank. `generate prompt=""` must not report success.
fn needed(engine: &Engine, args: &Value, action: &str, name: &str) -> Result<String, String> {
    match args.get(name).and_then(|v| v.as_str()) {
        Some(value) if !value.trim().is_empty() => Ok(value.trim().to_string()),
        Some(_) => Err(refuse(engine, format!("`{action}` was given an empty `{name}`."))),
        None => Err(refuse(engine, format!("`{action}` needs `{name}`."))),
    }
}

/// An optional text argument. Absent and empty mean the same thing here: a blank `base_url` asks
/// for the kind's own default, which is what `set_backend` does with it.
fn given(args: &Value, name: &str) -> String {
    args.get(name).and_then(|v| v.as_str()).unwrap_or_default().to_string()
}

fn text(args: &Value, name: &str) -> String {
    args.get(name)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

/// A number a caller might send either way.
///
/// The runtime checks that a required argument is present, not what type it arrived as, and a mind
/// writes `width: "1024"` about as often as `width: 1024`. Refusing the string form would be a
/// refusal about JSON rather than about the picture. Zero means "not given", and each action turns
/// that into its own default.
fn number(args: &Value, name: &str) -> u32 {
    wide(args, name).unwrap_or(0).min(u32::MAX as u64) as u32
}

/// The same reading, without the cap and without a zero standing in for "absent".
fn wide(args: &Value, name: &str) -> Option<u64> {
    let value = args.get(name)?;
    value
        .as_u64()
        .or_else(|| value.as_f64().map(|number| number as u64))
        .or_else(|| value.as_str().and_then(|text| text.trim().parse::<u64>().ok()))
}

/// A job id the way `engine.cancel` takes it: a whole number of 1 or more, whether it arrived as a
/// number or as its digits. Anything else is `None`, and the caller is told about `cancel_all`.
///
/// Deliberately not `wide()`, which casts through a float in which `-1` saturates to `0`: for a
/// width that is a rounding, but for a job id it turned "cancel the job before the first one" into
/// "cancel everything".
fn job_id(args: &Value) -> Option<i32> {
    let value = args.get("job")?;
    let whole = value
        .as_i64()
        .or_else(|| {
            value
                .as_f64()
                .filter(|number| number.is_finite() && number.fract() == 0.0)
                .map(|number| number as i64)
        })
        .or_else(|| value.as_str().and_then(|text| text.trim().parse::<i64>().ok()))?;
    i32::try_from(whole).ok().filter(|id| *id >= 1)
}
