//! What is done with one picture: reading its record for the right-hand pane, more pictures like
//! it, a bigger copy, opening it, and moving it to the Trash.
//!
//! Every one starts from a name. A name is a path or a bare filename of a picture in the gallery,
//! and turning it into a file reads the disk, so it is a worker's (`jobs::resolve`). The command a
//! worker lands carries that name's own file, and the call that asked is answered from it: from
//! this file and its record, never from whichever picture the person has chosen meanwhile.

use serde_json::json;
use yantrik_app::desktop::{DesktopOutcome, DesktopRequest};
use yantrik_app::effects::Effects;

use super::{by_agent, needed, not_yet, refuse, say};
use super::make::{enqueue, start};
use crate::answers;
use crate::command::{Command, Found, Next, Trashed};
use crate::gallery;
use crate::jobs::{self, Resolve};
use crate::model::{
    display, file_name, Chosen, Model, Work, PART_DETAIL, PART_FORM, PART_GALLERY, PART_NOTICE, PART_STATUS,
};
use crate::plan::{self, Plan};
use crate::said::Said;
use crate::words;
use crate::Studio;

type Fx = Effects<Studio>;

/// The gallery's own row of this name, when the name is a bare filename: matched newest first, so
/// the picture a person is looking at is the one that is meant.
fn listed(m: &Model, named: &str) -> Option<std::path::PathBuf> {
    if named.contains('/') || named.starts_with('~') {
        return None;
    }
    m.gallery.iter().find(|row| row.name == named).map(|row| row.path.clone())
}

fn ask(m: &Model, fx: &Fx, action: &'static str, named: &str, inside: bool, record: bool) -> Resolve {
    Resolve {
        action,
        agent: by_agent(fx),
        named: named.to_string(),
        listed: listed(m, named),
        root: m.places.gallery.clone(),
        inside,
        record,
    }
}

/// Resolve `named` on a worker, and land what it found as `next`.
fn resolve(m: &Model, fx: &mut Fx, action: &'static str, named: &str, next: Next, record: bool) {
    let ask = ask(m, fx, action, named, false, record);
    fx.spawn_or(Command::Lost(action), move |_| Command::Resolved { next, result: jobs::resolve(&ask) });
}

/// A row was clicked. Choosing the same one again puts the pane back to its invitation, because a
/// second click on a selected row is what a person does to undo it.
pub fn choose(m: &mut Model, fx: &mut Fx, index: i32) {
    let Some(row) = usize::try_from(index).ok().and_then(|at| m.gallery.get(at)) else { return };
    let path = row.path.display().to_string();
    let ticket = m.chosen.ticket + 1;
    fx.changed(PART_GALLERY);
    fx.changed(PART_DETAIL);
    if m.chosen.path == path {
        m.chosen = Chosen { ticket, ..Chosen::default() };
        return;
    }
    // Read by a worker, and drawn when it lands: a click pays for one small read, and a redraw for
    // none. A Mind's window reads nothing: the pane would show the mask in its place.
    m.chosen = Chosen { path: path.clone(), detail: String::new(), ticket };
    if m.for_mind {
        m.chosen.detail = yantrik_app::for_mind::HIDDEN.to_string();
        return;
    }
    resolve(m, fx, "choose", &path, Next::Choose { ticket }, true);
}

/// `variations`: more pictures of one that exists, from the record beside it.
pub fn variations(m: &mut Model, fx: &mut Fx, of: String, count: u32, fill: bool) {
    let Some(of) = needed(m, fx, "variations", "of", &of) else { return };
    if let Some(why) = not_yet(m) {
        return refuse(m, fx, why);
    }
    let next = Next::Variations { of: of.clone(), count, fill, generation: m.backend_generation };
    resolve(m, fx, "variations", &of, next, true);
}

/// `upscale`: a bigger copy of one picture, by resampling it here.
pub fn upscale(m: &mut Model, fx: &mut Fx, of: String, factor: u32) {
    let Some(of) = needed(m, fx, "upscale", "path", &of) else { return };
    resolve(m, fx, "upscale", &of, Next::Upscale { of: of.clone(), factor }, false);
}

/// `open`: a picture, or a folder if a folder is named, in whatever the machine opens it with.
pub fn open(m: &mut Model, fx: &mut Fx, of: String) {
    let Some(of) = needed(m, fx, "open", "path", &of) else { return };
    resolve(m, fx, "open", &of, Next::Open, false);
}

/// The output folder, in whatever the machine opens folders with.
pub fn open_folder(m: &mut Model, fx: &mut Fx) {
    let folder = m.places.gallery.clone();
    let shown = display(&folder);
    fx.desktop(DesktopRequest::OpenPath(folder), move |outcome| Command::Opened { shown, outcome });
}

/// `delete`: the picture and its record into the Trash, where Files can put them back.
pub fn delete(m: &mut Model, fx: &mut Fx, of: String) {
    let Some(of) = needed(m, fx, "delete", "path", &of) else { return };
    let ask = ask(m, fx, "delete", &of, true, false);
    let trash = m.places.trash.clone();
    fx.spawn_or(Command::Lost("move to the Trash"), move |_| Command::Trashed(jobs::trash_picture(&ask, &trash)));
}

/// A name was turned into a file (or was not).
pub fn resolved(m: &mut Model, fx: &mut Fx, next: Next, result: Result<Found, Said>) {
    let found = match (result, &next) {
        (Ok(found), _) => found,
        // A row whose picture is gone from under the window has nothing to read: the pane stays
        // as it was, as it always did.
        (Err(_), Next::Choose { .. }) => return,
        (Err(said), _) => return refuse(m, fx, said),
    };
    match next {
        Next::Open => {
            let shown = display(&found.path);
            fx.desktop(DesktopRequest::OpenPath(found.path), move |outcome| Command::Opened { shown, outcome });
        }
        Next::Variations { of, count, fill, generation } => variations_found(m, fx, found, of, count, fill, generation),
        Next::Upscale { of, factor } => {
            let factor = if factor == 0 { 2 } else { factor.clamp(2, plan::MAX_UPSCALE) };
            let name = file_name(&found.path);
            let label = format!("a {factor}x resample of {name}");
            let work = Work::Upscale { source: found.path, factor };
            let id = enqueue(m, fx, "upscale", &label, 1, work);
            start(m, fx);
            answers::settle(
                m,
                fx,
                json!({ "job": id, "queued": true, "of": of, "factor": factor, "done_on": "this machine" }),
            );
        }
        Next::Choose { ticket } => {
            // A row the person has since left, or chosen again, is not the one this is the record of.
            if m.chosen.ticket == ticket {
                m.chosen.detail = words::detail(&found.path, found.sidecar.as_ref());
                fx.changed(PART_DETAIL);
            }
        }
    }
}

fn variations_found(m: &mut Model, fx: &mut Fx, found: Found, of: String, count: u32, fill: bool, generation: u64) {
    // The sentence was approved against the backend of the moment it was asked. If the backend was
    // changed while its record was read, it is asked for again, not sent to the new one.
    if generation != m.backend_generation {
        return refuse(m, fx, "The backend was changed while this was being set up, so nothing was sent. Ask again.");
    }
    let Some(sidecar) = found.sidecar else {
        return refuse(
            m,
            fx,
            Said::new(
                format!(
                    "there is no record beside {}, so there is nothing to say what prompt made it. Generate from a sentence instead: `generate prompt=\"…\"`.",
                    found.path.display()
                ),
                format!(
                    "there is no record beside {}, so there is nothing to say what prompt made it. Type a sentence in the prompt box instead.",
                    found.path.display()
                ),
            ),
        );
    };
    // Fill the prompt box with the sentence the chosen picture was made from, so the person who
    // pressed this can see what is about to be asked for again, and change it.
    if fill {
        m.form.prompt = sidecar.prompt.clone();
        fx.changed(PART_FORM);
    }
    if sidecar.prompt.trim().is_empty() {
        return refuse(
            m,
            fx,
            format!("{} has a record but no prompt in it, so there is nothing to vary", found.path.display()),
        );
    }
    let count = count.clamp(1, plan::MAX_COUNT);
    let plan = Plan {
        prompt: sidecar.prompt.clone(),
        negative: sidecar.negative,
        width: sidecar.width,
        height: sidecar.height,
        steps: sidecar.steps.unwrap_or(plan::DEFAULT_STEPS),
        cfg: sidecar.cfg.unwrap_or(plan::DEFAULT_CFG),
        seed: None,
        count,
    };
    let plan = match plan.checked() {
        Ok(plan) => plan,
        Err(said) => return refuse(m, fx, said),
    };
    let name = file_name(&found.path);
    let label = match count {
        1 => format!("a variation of {name}"),
        n => format!("{n} variations of {name}"),
    };
    let work = Work::Plan { plan, made_from: format!("variation of {name}") };
    let id = enqueue(m, fx, "variations", &label, count, work);
    start(m, fx);
    answers::settle(m, fx, json!({ "job": id, "queued": true, "of": of, "count": count }));
}

/// A picture and its record went to the Trash, or did not.
pub fn trashed(m: &mut Model, fx: &mut Fx, result: Result<Trashed, Said>) {
    let trashed = match result {
        Ok(trashed) => trashed,
        Err(said) => return refuse(m, fx, said),
    };
    super::gallery::drop_reads(m, fx);
    m.gallery.retain(|row| row.path != trashed.path);
    m.stamp = trashed.stamp;
    m.content = gallery::content_stamp(&m.gallery);
    // A chosen picture that is no longer in the gallery was deleted or moved out from under us.
    if m.chosen.path == trashed.path.display().to_string() {
        m.chosen = Chosen { ticket: m.chosen.ticket + 1, ..Chosen::default() };
    }
    let names: Vec<String> = trashed.moved.iter().map(|each| each.name.clone()).collect();
    say(
        m,
        fx,
        format!(
            "Moved {} to the Trash. Files can put {} back until the Trash is emptied.",
            names.join(" and "),
            if names.len() == 1 { "it" } else { "them" }
        ),
    );
    fx.changed(PART_GALLERY);
    fx.changed(PART_STATUS);
    fx.changed(PART_DETAIL);
    fx.changed(PART_NOTICE);
    answers::settle(
        m,
        fx,
        json!({
            "moved_to_trash": trashed.moved.iter().map(|each| json!({
                "name": each.name,
                "was": display(&each.original),
                "now": display(&each.stored),
            })).collect::<Vec<_>>(),
            "recoverable": true,
            "gallery_count": m.gallery.len(),
        }),
    );
}

/// The desktop was asked to open a file or the folder, and said how it went.
pub fn opened(m: &mut Model, fx: &mut Fx, shown: String, outcome: DesktopOutcome) {
    match outcome {
        DesktopOutcome::Done => answers::settle(m, fx, json!({ "opened": shown })),
        DesktopOutcome::Failed(sentence) => refuse(m, fx, sentence),
    }
}
