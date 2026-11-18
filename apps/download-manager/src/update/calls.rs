//! The calls to the engine: each runs on the desk, off the UI thread, and answers from what its
//! own call found.

use std::path::Path;

use serde_json::{json, Value};
use yantrik_app::effects::Effects;

use crate::agent_rule;
use crate::command::{Call, Command, Landed};
use crate::engine::{self, Download, Engine, Status};
use crate::model::Model;
use crate::Downloads;

type Fx = Effects<Downloads>;

/// Hand `run` to the desk. It lands as the call's own `Landed`, or as `WorkFailed` if it
/// panicked. `run` is told whether an agent asked.
pub fn start(
    m: &mut Model,
    fx: &mut Fx,
    call: Call,
    run: impl FnOnce(&Engine, bool) -> Result<Value, String> + Send + 'static,
) {
    let agent = crate::answers::agent(fx);
    let reply = m.desk.ask(move |engine| run(engine, agent));
    m.work += 1;
    fx.spawn_or(Command::WorkFailed(call), move |_| match reply.wait() {
        Some(answer) => Command::Landed(Box::new(Landed { call, answer })),
        None => Command::WorkFailed(call),
    });
}

/// One download as `describe` lists it, with the words the surface already publishes.
pub fn row_json(download: &Download) -> Value {
    let mut row = json!({
        "id": download.id,
        "filename": download.filename,
        "status": download.status.as_str(),
        "size": download.size_text(),
        "path": download.path().to_string_lossy(),
        "url": download.url,
    });
    let map = row.as_object_mut().expect("row is an object");
    if download.total.is_some() {
        map.insert("percent".into(), ((download.progress() * 100.0).round() as i64).into());
    }
    if download.status == Status::Downloading {
        map.insert("speed".into(), format!("{}/s", engine::format_bytes(download.speed_bps as u64)).into());
        if let Some(eta) = download.eta_secs {
            map.insert("eta".into(), engine::format_duration(eta).into());
        }
    }
    // Only said when true, so a caller reading a row can take their presence as the claim. A mind
    // coming back to a machine it left needs to tell "paused because I paused it" from "paused
    // because the process died under it", and only the second one is news.
    if download.interrupted {
        map.insert("interrupted".into(), true.into());
    }
    if download.restored {
        map.insert("restored".into(), true.into());
    }
    if !download.file_hash.is_empty() {
        map.insert("sha256".into(), download.file_hash.clone().into());
    }
    if download.checksum_status != "none" {
        map.insert("checksum".into(), download.checksum_status.clone().into());
    }
    if !download.error.is_empty() {
        map.insert("error".into(), download.error.clone().into());
    }
    row
}

/// The download `id` as the engine holds it right after the call, for the answer.
fn row(engine: &Engine, id: i32) -> Result<Value, String> {
    engine.get(id).map(|d| row_json(&d)).ok_or_else(|| format!("no download {id}"))
}

/// Queue a URL. An agent saves only where it may write a file at all (`agent_rule.rs`): the rule
/// reads the disk, so it is asked here, before the engine makes anything.
pub fn add(engine: &Engine, agent: bool, url: &str, sha256: &str, save_dir: &str) -> Result<Value, String> {
    let url = url.trim();
    let dir = if save_dir.trim().is_empty() {
        engine.default_dir().to_path_buf()
    } else {
        engine::expand_home(save_dir.trim())
    };
    // The person's own default folder is made first, so "no folder given" never fails as "not there".
    if dir == engine.default_dir() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot use {}: {e}", dir.display()))?;
    }
    agent_rule::may_save(agent, &dir, &engine::filename_from_url(url))?;
    // An address written out in the URL needs no lookup to refuse. A name that resolves to a
    // private one is refused when the fetch connects (`engine/fetch.rs`).
    if agent {
        if let Some(refusal) = engine::literal_refusal(url) {
            return Err(refusal);
        }
    }
    let id = engine.add_as(url, sha256, Some(save_dir), agent)?;
    tracing::info!(id, "Download queued");
    let download = engine.get(id).ok_or("the download vanished as it was queued")?;
    Ok(json!({
        "id": id,
        "filename": download.filename,
        "path": download.path().to_string_lossy(),
        "status": download.status.as_str(),
    }))
}

/// Run one command on a download and answer with the row as the engine holds it just after.
pub fn on_row(
    engine: &Engine,
    id: i32,
    command: impl FnOnce(&Engine, i32) -> Result<(), String>,
) -> Result<Value, String> {
    command(engine, id)?;
    row(engine, id)
}

/// Cancel a download and answer with its row as it was, or with `removed` where the row went.
///
/// A finished download has no partial file to discard: the person's cancel takes its row out of
/// the list and keeps the file, and an agent's is refused (`Engine::cancel`).
pub fn cancel(engine: &Engine, agent: bool, id: i32) -> Result<Value, String> {
    let before = engine.get(id).ok_or_else(|| format!("no download {id}"))?;
    if agent && before.status.is_finished() {
        return Err(format!("download {id} finished; an agent may not remove it, and its file is never deleted"));
    }
    engine.cancel(id)?;
    match engine.get(id) {
        Some(after) => Ok(row_json(&after)),
        None => {
            let mut gone = row_json(&before);
            gone["status"] = "removed".into();
            Ok(gone)
        }
    }
}

/// An agent retries only what did not finish: `retry` deletes the file first, and on a finished
/// download that is a file the person already has.
pub fn retry(engine: &Engine, agent: bool, id: i32) -> Result<Value, String> {
    if agent && engine.get(id).is_some_and(|d| d.status == Status::Completed) {
        return Err(format!("download {id} finished; an agent may not delete it to fetch it again"));
    }
    on_row(engine, id, |e, id| e.retry(id))
}

pub fn verify(engine: &Engine, id: i32, expected: Option<&str>) -> Result<Value, String> {
    on_row(engine, id, |e, id| e.verify(id, expected))
}

/// Show the folder a download went to in the file manager.
pub fn open_folder(engine: &Engine, id: i32) -> Result<(), String> {
    let download = engine.get(id).ok_or_else(|| format!("no download {id}"))?;
    open_dir(&download.save_dir)
}

fn open_dir(dir: &Path) -> Result<(), String> {
    let opener = if cfg!(target_os = "windows") { "explorer" } else { "xdg-open" };
    std::process::Command::new(opener)
        .arg(dir)
        .spawn()
        .map(drop)
        .map_err(|e| format!("cannot open {}: {e}", dir.display()))
}
