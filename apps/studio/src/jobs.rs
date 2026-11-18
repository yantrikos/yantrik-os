//! The work that touches the disk or the network, written as plain functions.
//!
//! Each runs on a worker (`Effects::spawn`), never in `update` and never in `render`: a name is
//! turned into a file, a picture is made and filed, a file is moved to the Trash, a backend choice
//! is written down. What they return is what the command that carries it back needs, so the update
//! that lands it can answer the call that asked from that result alone.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Local;

use crate::agent_rule;
use crate::backend::{Backend, Request, Shot, Stop};
use crate::command::{BackendAsk, BackendSet, Found, Saved, Trashed};
use crate::config::{Backend as BackendConfig, Config, Kind, KINDS};
use crate::gallery::{self, Sidecar};
use crate::model::{display, file_name};
use crate::plan::Plan;
use crate::said::Said;
use crate::trash;

/// What a name has to be turned into a file: who asked, what was named, and where to look.
#[derive(Clone)]
pub struct Resolve {
    /// The action the name was given to; the rule for agents' files is per action.
    pub action: &'static str,
    pub agent: bool,
    pub named: String,
    /// The gallery's row of that name, when the model had one.
    pub listed: Option<PathBuf>,
    pub root: PathBuf,
    /// Only pictures inside Studio's own gallery (`delete`).
    pub inside: bool,
    /// Read the record beside the picture too.
    pub record: bool,
}

/// Turn what a caller named into a file.
///
/// A bare filename is looked up in the gallery, because "the lighthouse one" is how a person
/// refers to a picture and a path is how a machine does; both have to work or the action is only
/// half usable.
pub fn resolve(ask: &Resolve) -> Result<Found, Said> {
    agent_rule::admit(ask.agent, ask.action, &ask.named).map_err(Said::both)?;
    let named = ask.named.trim();
    if named.is_empty() {
        return Err(Said::both("no file was named"));
    }
    let candidate = if named.contains('/') || named.starts_with('~') {
        gallery::expand_home(named)
    } else {
        // A name with no folder in it is matched against the gallery, newest first, so the
        // picture a person is looking at is the one that is meant.
        match &ask.listed {
            Some(path) => path.clone(),
            None => {
                let guess = gallery::day_folder(&ask.root, Local::now()).join(named);
                if !guess.exists() {
                    return Err(Said::new(
                        format!(
                            "there is no {named} in the gallery. `describe` lists what is there, with the full path of each."
                        ),
                        format!("there is no {named} in the gallery."),
                    ));
                }
                guess
            }
        }
    };
    let Ok(path) = std::fs::canonicalize(&candidate) else {
        return Err(Said::both(format!("{} is not there. Nothing was changed.", candidate.display())));
    };
    if !path.is_file() {
        return Err(Said::both(format!("{} is a folder, not a picture", path.display())));
    }
    let name = file_name(&path);
    if !yantrik_image_core::is_image(&name) {
        // The same answer the file browser gives, so this app cannot be talked into treating as a
        // picture a file that Files would not show as one.
        return Err(Said::both(format!(
            "{name} is not a picture this OS recognises (it looks for {})",
            yantrik_image_core::IMAGE_EXTENSIONS.join(", ")
        )));
    }
    if ask.inside && !gallery::is_inside(&path, &ask.root) {
        return Err(Said::both(format!(
            "{} is not in Studio's gallery at {}, and this app only deletes its own pictures. Files can delete anything.",
            path.display(),
            display(&ask.root)
        )));
    }
    let sidecar = if ask.record { Sidecar::read(&path) } else { None };
    Ok(Found { path, sidecar })
}

/// Move a picture and the record beside it to the Trash.
pub fn trash_picture(ask: &Resolve, trash_root: &Path) -> Result<Trashed, Said> {
    let found = resolve(ask)?;
    let moved = trash::picture(&found.path, trash_root)?;
    Ok(Trashed { path: found.path, moved, stamp: gallery::cheap_stamp(&ask.root) })
}

/// One picture of a job: asked of the backend that job was queued against, then filed.
///
/// `Err(None)` is a cancellation: nothing was written, and the job says so rather than failing.
pub fn make_picture(
    backend: &Arc<dyn Backend>,
    plan: &Plan,
    index: u32,
    made_from: &str,
    root: &Path,
    stop: &Stop,
) -> Result<Saved, Option<String>> {
    if stop.is_set() {
        return Err(None);
    }
    let request = plan.request(index);
    let attempt = std::time::Instant::now();
    match backend.generate(&request, stop) {
        Ok(shot) => {
            let seconds = attempt.elapsed().as_secs_f64();
            let saved = save_as(root, &shot, &request, seconds, made_from, backend.kind()).map_err(Some)?;
            tracing::info!("{} made in {seconds:.1}s by {}", saved.record.path.display(), backend.kind());
            Ok(saved)
        }
        Err(problem) if problem == "cancelled" => Err(None),
        // Which backend was asked is part of the answer: "could not be reached" means something
        // different when it is a server on your own LAN and when it is a service that would have
        // kept your prompt. Named by the backend that was actually asked rather than by the
        // configuration beside it, so the record and the refusal cannot disagree about who made
        // what.
        Err(problem) => Err(Some(format!("{} could not make the picture: {problem}", backend.kind()))),
    }
}

/// A resample of one picture, filed beside the rest with a record that says where it came from.
pub fn upscale_picture(source: &Path, factor: u32, root: &Path, stop: &Stop) -> Result<Saved, Option<String>> {
    if stop.is_set() {
        return Err(None);
    }
    let started = std::time::Instant::now();
    let bytes = gallery::resample(source, factor, gallery::LONGEST_EDGE).map_err(Some)?;
    let (width, height) = gallery::png_size(&bytes);
    let shot = Shot {
        bytes,
        width: width.unwrap_or(0),
        height: height.unwrap_or(0),
        model: String::new(),
        sent: format!("{}x{}", width.unwrap_or(0), height.unwrap_or(0)),
    };
    // The prompt travels with the picture. An upscaled lighthouse is still a picture of a
    // lighthouse made from that sentence, and losing the sentence would be losing the part of the
    // record that matters.
    let original = Sidecar::read(source);
    let request = Request {
        prompt: original.as_ref().map(|each| each.prompt.clone()).unwrap_or_default(),
        negative: String::new(),
        width: shot.width,
        height: shot.height,
        steps: 0,
        cfg: 0.0,
        seed: original.as_ref().map(|each| each.seed).unwrap_or(0),
    };
    let made_from = format!("upscale of {}", file_name(source));
    // Nothing was sent to a model, and the record does not claim otherwise: `studio-resample`
    // goes in the field that names a backend everywhere else.
    save_as(root, &shot, &request, started.elapsed().as_secs_f64(), &made_from, "studio-resample")
        .map_err(Some)
}

/// Write one shot into today's folder with its record.
///
/// `backend_name` arrives from whoever made the pixels: a generation names the backend that was
/// asked, and a resample says `studio-resample`, because nothing was asked and the record must not
/// claim otherwise.
fn save_as(
    root: &Path,
    shot: &Shot,
    request: &Request,
    seconds: f64,
    made_from: &str,
    backend_name: &str,
) -> Result<Saved, String> {
    let now = Local::now();
    let folder = gallery::day_folder(root, now);
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("{} could not be created ({e}), so the picture has nowhere to go", folder.display()))?;
    let path = gallery::unique_file(&folder, now, request.seed);

    // The picture is written before its record, and if the record cannot be written the picture
    // is removed again. A picture with no record beside it is indistinguishable in the gallery
    // from one somebody copied in, which loses the thing that makes this app worth having; a
    // record with no picture is worse, and cannot happen this way round.
    // A file nobody had (`O_EXCL`), never written through a link planted at the name.
    if let Err(e) = crate::safe_write::write_new(&path, &shot.bytes, 0o644) {
        // What was half-written is removed; a file that was already there is not ours to remove.
        if e.kind() != std::io::ErrorKind::AlreadyExists {
            let _ = std::fs::remove_file(&path);
        }
        return Err(format!("{} could not be written ({e})", path.display()));
    }
    let sidecar = Sidecar {
        prompt: request.prompt.clone(),
        negative: request.negative.clone(),
        seed: request.seed,
        backend: backend_name.to_string(),
        model: shot.model.clone(),
        seconds: (seconds * 100.0).round() / 100.0,
        width: shot.width,
        height: shot.height,
        sent: shot.sent.clone(),
        created: now.to_rfc3339_opts(chrono::SecondsFormat::Secs, false),
        made_from: made_from.to_string(),
        steps: (request.steps > 0).then_some(request.steps),
        cfg: (request.cfg > 0.0).then_some(request.cfg),
        made_by: "yantrik-studio".to_string(),
    };
    if let Err(problem) = sidecar.write(&path) {
        let _ = std::fs::remove_file(&path);
        return Err(format!("{problem}; the picture was removed rather than left without its record"));
    }
    // One row rather than a full re-read: this is the newest picture in the folder, so it belongs
    // at the front, and decoding twelve thumbnails for every shot of a batch is waste.
    let record = gallery::record(&path, true);
    Ok(Saved { record, stamp: gallery::cheap_stamp(root), seconds })
}

/// Check a backend choice with the parser the config file goes through, and write it down.
///
/// Nothing is written until the choice has been checked, so a mistake does not leave behind a
/// configuration that the next start would read and fail on. `output_folder` is the person's,
/// kept; `settings` is where this instance was started with, `None` with no home to write in.
pub fn set_backend(
    ask: &BackendAsk,
    output_folder: &str,
    settings: Option<&Path>,
) -> Result<BackendSet, String> {
    if let Some(problem) = crate::config::bad_variable_name(&ask.api_key_env) {
        return Err(problem);
    }
    let Some(parsed) = Kind::parse(&ask.kind) else {
        let named = crate::config::short_word(&ask.kind).map(|word| format!("`{word}`")).unwrap_or_else(|| "That".to_string());
        return Err(format!(
            "{named} is not a backend Studio has. It can make pictures with {}, or with `fake`, which draws a placeholder here.",
            KINDS.join(", ")
        ));
    };
    let mut backend = BackendConfig::defaults_for(parsed);
    if !ask.base_url.trim().is_empty() {
        backend.base_url = ask.base_url.trim().trim_end_matches('/').to_string();
    }
    if !ask.model.trim().is_empty() {
        backend.model = ask.model.trim().to_string();
    }
    if !ask.api_key_env.trim().is_empty() {
        backend.api_key_env = ask.api_key_env.trim().to_string();
    }
    if !ask.workflow.trim().is_empty() {
        backend.workflow = gallery::expand_home(ask.workflow.trim()).display().to_string();
    }

    let candidate = Config { backend: backend.clone(), configured: true, output_folder: String::new(), refused: None };
    let text = serde_json::to_string(&candidate.to_json()).map_err(|e| e.to_string())?;
    let mut config = Config::parse(&text)?;
    if parsed == Kind::ComfyUi && !backend.workflow.is_empty() {
        // Reading the graph now is the difference between "your workflow file is not in the API
        // format" and a 400 from a server three minutes into a queue.
        crate::workflow::graph(&backend.workflow)?;
    }
    // The folder the person already had stays theirs; `parse` starts from empty.
    config.output_folder = output_folder.to_string();
    config.configured = true;

    let written = match settings {
        Some(path) => {
            config.save_to(path)?;
            true
        }
        None => false,
    };
    Ok(BackendSet { config, written })
}
