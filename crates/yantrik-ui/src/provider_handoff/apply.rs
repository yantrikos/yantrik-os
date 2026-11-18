//! Doing what a plan says (apply) and putting back what the person had (revert): the files, the
//! marker, the gateway token and the restart. Split from `mod.rs`, which keeps the plans and cards.

use std::path::{Path, PathBuf};

use super::{
    adapter_for, Handoff, companion, hermes, marker, marker_path, mind, own_models_path, Marker, Plan, Touched, GATEWAY_ID,
};
use super::keep::keep_original;
use super::progress::{self, sha, FileProgress, Progress};
use super::replaced;
use crate::private_file::{self, Publish};

/// One apply or revert at a time, whichever path started it.
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Do what the plan says: keep each file the person had, write the new one at 600, leave the
/// marker, restart the harness if it is running.
pub(crate) fn apply(home: &Path, plan: &Plan) -> Result<Marker, String> {
    let _one_at_a_time = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let earlier = marker(home, &plan.harness);
    let (mut known, earlier_attempts) = record_intent(home, plan)?;
    let mut files = Vec::new();
    for w in &plan.writes {
        let mut here = known.file(&w.path);
        // Judged by what EARLIER attempts wrote only: bytes equal to this plan's are not yet this code's.
        let mut judged = here.clone();
        judged.own = earlier_attempts.file(&w.path).own;
        let kept = keep_original(&w.path, &judged, earlier.as_ref())?;
        // The copies are on disk now: say so before the live write, so a retry names them.
        here.replaced = kept.replaced.clone();
        known.set(here);
        progress::save(home, &plan.harness, &known)?;
        private_file::write(&w.path, w.content.as_bytes(), Publish::Replace, || Ok(()))?;
        files.push(Touched {
            path: w.path.clone(),
            backup: kept.backup,
            adopted: kept.adopted,
            replaced: kept.replaced,
            written: Some(sha(w.content.as_bytes())),
            restored: false,
        });
    }
    let mut m = Marker {
        provider_id: plan.provider_id.clone(),
        provider_name: plan.provider_name.clone(),
        model: plan.model.clone(),
        at: chrono::Utc::now().to_rfc3339(),
        files,
        note: None,
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
        (Some(body), _) => Some(mind::post(&body.0).map(|()| None)),
        (_, Some(body)) => Some(hermes::post_model("apply", Some(&body.0))),
        _ => None,
    };
    match posted {
        Some(Err(e)) => {
            if plan.token.is_some() {
                let _ = crate::gateway::tokens().revoke(&plan.harness);
            }
            return Err(e);
        }
        Some(Ok(note)) => m.note = note,
        None => {}
    }
    let text = serde_json::to_string_pretty(&m).map_err(|e| e.to_string())?;
    private_file::write(&marker_path(home, &plan.harness), text.as_bytes(), Publish::Replace, || Ok(()))?;
    progress::clear(home, &plan.harness);
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
///
/// Returns a sentence naming any copy of the person's newer config that Apply kept, if there is one.
pub(crate) fn revert(home: &Path, harness: &str) -> Result<Option<String>, String> {
    let _one_at_a_time = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let adapter = adapter_for(harness).ok_or_else(|| format!("{harness} cannot be given a provider here"))?;
    let m = marker(home, harness).ok_or_else(|| format!("{harness} was not given a provider here"))?;
    let unfinished = progress::load(home, harness);
    let restore: Vec<Touched> = files_to_restore(&m, adapter, home, harness == hermes::ID && hermes::moved())?.into_iter().cloned().collect();
    let mut marker_now = m.clone();
    let mut said = Vec::new();
    for t in restore.iter().filter(|t| !t.restored) {
        // A failed re-apply left Yantrik's own file live, and copies it made that no marker names yet.
        let unfinished = unfinished.file(&t.path);
        let (found, mut copies) = restore_one(t, &unfinished)?;
        copies.splice(0..0, t.replaced.iter().cloned().chain(unfinished.not_in(&t.replaced)));
        said.extend(replaced::revert_sentence(&t.path, found.as_deref(), &copies));
        // Each file is marked done as it goes, so a Revert that stops here is finished, not repeated.
        if let Some(done) = marker_now.files.iter_mut().find(|f| f.path == t.path) {
            done.restored = true;
        }
        let text = serde_json::to_string_pretty(&marker_now).map_err(|e| e.to_string())?;
        private_file::write(&marker_path(home, harness), text.as_bytes(), Publish::Replace, || Ok(()))?;
    }
    std::fs::remove_file(marker_path(home, harness)).map_err(|e| e.to_string())?;
    if m.provider_id == GATEWAY_ID {
        crate::gateway::tokens().revoke(harness)?;
        if harness == mind::ID {
            mind::post(&mind::revert_body())?;
        }
        if harness == hermes::ID && hermes::moved() {
            said.extend(hermes::post_model("revert", None)?);
        }
    }
    if let Some(unit) = adapter.unit() {
        try_restart(unit);
    }
    if harness == companion::ID {
        companion::reload(home);
    }
    progress::clear(home, harness);
    let note = (!said.is_empty()).then(|| said.join(" "));
    tracing::info!(harness = %harness, kept_a_copy = note.is_some(), "put a harness's own settings back");
    Ok(note)
}

/// Before the first live write: what each file is about to be written with, added to what an
/// unfinished apply already recorded (progress.rs says why).
///
/// Returns the record now saved and the one found (what earlier attempts wrote).
fn record_intent(home: &Path, plan: &Plan) -> Result<(Progress, Progress), String> {
    let old = progress::load(home, &plan.harness);
    let mut next = Progress::default();
    for w in &plan.writes {
        let mut f = old.file(&w.path);
        let h = sha(w.content.as_bytes());
        if !f.own.contains(&h) {
            f.own.push(h);
        }
        // About to make the backup itself: a retry that finds it has found its own. Set from the disk
        // each time: only a live file that is not this code's own is an original worth a backup.
        let b = backup_path(&w.path);
        let original = std::fs::read(&w.path).ok().is_some_and(|l| !old.file(&w.path).is_own(&l));
        f.backup_ours = (!super::keep::exists(&b) && original) || (f.backup_ours && super::keep::exists(&b));
        next.files.push(f);
    }
    progress::save(home, &plan.harness, &next)?;
    Ok((next, old))
}

/// Put one file back. Returns where an adopted backup was set aside, and the copies made now of a
/// live file edited since it was written: it is copied first, never written over.
fn restore_one(t: &Touched, unfinished: &FileProgress) -> Result<(Option<PathBuf>, Vec<PathBuf>), String> {
    private_file::check_target(&t.path)?;
    if let Some(b) = &t.backup {
        if !super::keep::exists(b) {
            return Ok((None, Vec::new())); // an earlier Revert put it back and stopped before saying so
        }
        private_file::check_target(b)?;
    }
    let live = if super::keep::exists(&t.path) { Some(std::fs::read(&t.path).map_err(|e| e.to_string())?) } else { None };
    let backup_bytes = match &t.backup {
        Some(b) => Some(std::fs::read(b).map_err(|e| format!("could not read {}: {e}", b.display()))?),
        None => None,
    };
    let mut copies = Vec::new();
    if let (Some(l), Some(h)) = (&live, &t.written) {
        // Not the person's: what was written, or what a failed re-apply (still recorded) wrote after.
        if sha(l) != *h && !unfinished.is_own(l) && backup_bytes.as_ref() != Some(l) {
            copies.push(replaced::keep_live(&t.path, l)?);
        }
    }
    let mut found = None;
    match &t.backup {
        Some(b) if t.adopted => {
            replaced::restore_keeping_backup(b, &t.path)?;
            found = Some(replaced::set_aside(b)?);
        }
        Some(b) => std::fs::rename(b, &t.path).map_err(|e| format!("could not put back {}: {e}", t.path.display()))?,
        None => {
            let _ = std::fs::remove_file(&t.path);
        }
    }
    Ok((found, copies))
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

pub(crate) fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".before-yantrik");
    path.with_file_name(name)
}

fn try_restart(unit: &str) {
    let _ = std::process::Command::new("systemctl").args(["--user", "try-restart", "--", unit]).status();
}
