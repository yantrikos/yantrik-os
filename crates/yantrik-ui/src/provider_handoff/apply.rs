//! Doing what a plan says (apply) and putting back what the person had (revert): the files, the
//! marker, the gateway token and the restart. Split from `mod.rs`, which keeps the plans and cards.

use std::path::{Path, PathBuf};

use super::{
    adapter_for, Handoff, companion, hermes, marker, marker_path, mind, own_models_path, Marker, Plan, Touched, GATEWAY_ID,
};
use crate::private_file::{self, Publish};

/// One apply or revert at a time, whichever path started it.
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Do what the plan says: keep each file the person had, write the new one at 600, leave the
/// marker, restart the harness if it is running.
pub(crate) fn apply(home: &Path, plan: &Plan) -> Result<Marker, String> {
    let _one_at_a_time = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let earlier = marker(home, &plan.harness);
    let mut files = Vec::new();
    for w in &plan.writes {
        let backup = keep_original(&w.path, earlier.as_ref())?;
        private_file::write(&w.path, w.content.as_bytes(), Publish::Replace, || Ok(()))?;
        files.push(Touched { path: w.path.clone(), backup });
    }
    let m = Marker {
        provider_id: plan.provider_id.clone(),
        provider_name: plan.provider_name.clone(),
        model: plan.model.clone(),
        at: chrono::Utc::now().to_rfc3339(),
        files,
    };
    // The token is accepted before the harness is restarted onto it, and the Mind is told only
    // once the gateway will take what it is given.
    // A harness moved off the gateway (a saved provider in its place) keeps no token there.
    match &plan.token {
        Some(token) => {
            let grant = yantrik_gateway::Grant { harness: plan.harness.clone(), private_context: plan.private_context };
            crate::gateway::tokens().register(&token.0, grant)?;
        }
        None if earlier.as_ref().is_some_and(|m| m.provider_id == GATEWAY_ID) => {
            crate::gateway::tokens().revoke(&plan.harness)?;
        }
        None => {}
    }
    // A Mind that refuses the offer keeps its own provider, and so does a moved Hermes: the gateway
    // keeps no token for either, since the token went nowhere that holds it.
    let posted = match (&plan.mind_post, &plan.moved_post) {
        (Some(body), _) => Some(mind::post(&body.0)),
        (_, Some(body)) => Some(hermes::post_model("apply", Some(&body.0))),
        _ => None,
    };
    if let Some(Err(e)) = posted {
        if plan.token.is_some() {
            let _ = crate::gateway::tokens().revoke(&plan.harness);
        }
        return Err(e);
    }
    let text = serde_json::to_string_pretty(&m).map_err(|e| e.to_string())?;
    private_file::write(&marker_path(home, &plan.harness), text.as_bytes(), Publish::Replace, || Ok(()))?;
    // Given Yantrik models (the button, or a later install): "Use its own models" is undone.
    if plan.provider_id == GATEWAY_ID {
        let _ = std::fs::remove_file(own_models_path(home, &plan.harness));
    }
    if let Some(unit) = &plan.restart {
        try_restart(unit);
    }
    if plan.harness == companion::ID {
        companion::reload(home);
    }
    tracing::info!(harness = %plan.harness, provider = %plan.provider_name, model = %plan.model, "gave a harness a provider");
    Ok(m)
}

/// Put back what the person had: each kept file over the one written, or the written file
/// removed when there was none before. Then restart the harness if it is running.
pub(crate) fn revert(home: &Path, harness: &str) -> Result<(), String> {
    let _one_at_a_time = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let adapter = adapter_for(harness).ok_or_else(|| format!("{harness} cannot be given a provider here"))?;
    let m = marker(home, harness).ok_or_else(|| format!("{harness} was not given a provider here"))?;
    let restore = files_to_restore(&m, adapter, home, harness == hermes::ID && hermes::moved())?;
    for t in restore {
        private_file::check_target(&t.path)?;
        match &t.backup {
            Some(b) => {
                private_file::check_target(b)?;
                std::fs::rename(b, &t.path).map_err(|e| format!("could not put back {}: {e}", t.path.display()))?;
            }
            None => {
                let _ = std::fs::remove_file(&t.path);
            }
        }
    }
    std::fs::remove_file(marker_path(home, harness)).map_err(|e| e.to_string())?;
    if m.provider_id == GATEWAY_ID {
        crate::gateway::tokens().revoke(harness)?;
        if harness == mind::ID {
            mind::post(&mind::revert_body())?;
        }
        if harness == hermes::ID && hermes::moved() {
            hermes::post_model("revert", None)?;
        }
    }
    if let Some(unit) = adapter.unit() {
        try_restart(unit);
    }
    if harness == companion::ID {
        companion::reload(home);
    }
    tracing::info!(harness = %harness, "put a harness's own settings back");
    Ok(())
}

/// The files a marker says were written, checked before anything moves: a marker naming any other
/// file, or a backup anywhere but beside the file it keeps, was not written by this code.
///
/// `moved`: Hermes has moved to the mind account since "Use Yantrik models" was applied. The marker
/// then names the config it had in the person's home, now `.hermes.yantrik-moved`, kept whole and
/// not this code's to write: there is no file to put back, and that entry is left out (the token
/// and the moved config's `model` are what Revert undoes).
pub(super) fn files_to_restore<'a>(
    m: &'a Marker,
    adapter: &dyn Handoff,
    home: &Path,
    moved: bool,
) -> Result<Vec<&'a Touched>, String> {
    let own = adapter.files(home);
    let mut out = Vec::new();
    for t in &m.files {
        if moved && hermes::was_before_the_move(home, &t.path) {
            continue;
        }
        if !own.contains(&t.path) || t.backup.as_ref().is_some_and(|b| *b != backup_path(&t.path)) {
            return Err(format!(
                "the record of what was changed names {}, which {} never writes; nothing was undone",
                t.path.display(),
                adapter.name()
            ));
        }
        out.push(t);
    }
    Ok(out)
}

/// The person's own file, kept once beside it. A copy already there is theirs — from an earlier
/// assignment, or one that failed after writing — and is never replaced, so Revert can always
/// return what they had before this code first touched it.
fn keep_original(path: &Path, earlier: Option<&Marker>) -> Result<Option<PathBuf>, String> {
    if let Some(t) = earlier.and_then(|m| m.files.iter().find(|t| t.path == path)) {
        return Ok(t.backup.clone());
    }
    let b = backup_path(path);
    if std::fs::symlink_metadata(&b).is_ok() {
        private_file::check_target(&b)?;
        return Ok(Some(b));
    }
    if std::fs::symlink_metadata(path).is_err() {
        return Ok(None);
    }
    private_file::check_target(path)?;
    let original = std::fs::read(path).map_err(|e| format!("could not keep a copy of {}: {e}", path.display()))?;
    private_file::write(&b, &original, Publish::CreateOnly, || Ok(()))?;
    Ok(Some(b))
}

pub(crate) fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".before-yantrik");
    path.with_file_name(name)
}

fn try_restart(unit: &str) {
    let _ = std::process::Command::new("systemctl").args(["--user", "try-restart", "--", unit]).status();
}
