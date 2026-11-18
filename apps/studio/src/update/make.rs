//! The queue: asking for pictures, starting them, filing what comes back, and stopping them.
//!
//! A render takes seconds to minutes, so a job is never waited for. Asking enqueues it and answers
//! with its id; each picture of a job is one worker, and the command it lands is what moves the
//! job on to its next picture or out of the queue. At most `RENDERING_AT_ONCE` jobs are being made
//! together; the rest are `pending` in `describe`, and start as the others finish.
//!
//! Stopping is a flag the picture in flight polls, and for a ComfyUI server a word to the server
//! as well. A job stopped before its next picture starts is removed without starting it. Nothing
//! is ever written half: a picture is filed whole with its record, or not at all.

use serde_json::json;
use yantrik_app::effects::Effects;

use super::{not_yet, refuse};
use crate::answers;
use crate::backend::Stop;
use crate::command::{Command, Saved, Which};
use crate::gallery::{self, content_stamp};
use crate::jobs;
use crate::model::{
    display, short_prompt, Entry, Job, Model, Work, PART_GALLERY, PART_JOBS, PART_NOTICE, PART_STATUS,
    RENDERING_AT_ONCE,
};
use crate::plan::Plan;
use crate::said::Said;
use crate::Studio;

type Fx = Effects<Studio>;

/// `generate`: the ask is checked, queued and answered with the job's id.
///
/// `None` is the window's: the ask is made from its fields.
pub fn generate(m: &mut Model, fx: &mut Fx, plan: Option<Plan>) {
    let plan = plan.unwrap_or_else(|| m.form.plan());
    if let Some(why) = not_yet(m) {
        return refuse(m, fx, why);
    }
    let plan = match plan.checked() {
        Ok(plan) => plan,
        Err(said) => return refuse(m, fx, said),
    };
    let label = match plan.count {
        1 => short_prompt(&plan.prompt),
        n => format!("{n} pictures of {}", short_prompt(&plan.prompt)),
    };
    // Sent: the box no longer holds an unsent sentence.
    m.sent_prompt = plan.prompt.clone();
    super::keep_prompt(m, fx, false);
    let id = enqueue(m, fx, "generate", &label, plan.count, Work::Plan { plan: plan.clone(), made_from: String::new() });
    start(m, fx);
    let facts = m.facts();
    answers::settle(
        m,
        fx,
        json!({
            "job": id,
            "queued": true,
            "count": plan.count,
            "backend": facts.kind,
            "where": facts.place,
            "prompt_leaves_this_machine": facts.prompt_leaves,
            "output_folder": display(&m.places.gallery),
            "read_back": "the job appears in describe's queue.running; when the pictures land they \
                          appear in gallery.newest with their path, seed and seconds",
        }),
    );
}

/// Put a job at the back of the queue. Returns its identifier, which is what `cancel` takes.
pub fn enqueue(m: &mut Model, fx: &mut Fx, kind: &str, label: &str, wanted: u32, work: Work) -> i32 {
    let id = m.next_job;
    m.next_job += 1;
    m.queue.push(Entry {
        job: Job {
            id,
            kind: kind.to_string(),
            label: label.to_string(),
            status: "pending".to_string(),
            wanted,
            made: 0,
            seconds: 0.0,
        },
        work,
        backend: m.backend.clone(),
        stop: Stop::new(),
        flying: false,
    });
    // A fresh ask supersedes the last thing that went wrong, which is otherwise still leading the
    // summary while a new picture is being made.
    m.notice = String::new();
    fx.changed(PART_NOTICE);
    queue_moved(fx);
    id
}

fn queue_moved(fx: &mut Fx) {
    fx.changed(PART_JOBS);
    fx.changed(PART_STATUS);
}

/// How many jobs hold a place among those being made.
fn making(m: &Model) -> usize {
    m.queue.iter().filter(|entry| entry.flying || entry.job.status == "running").count()
}

/// Start whatever may be started: a job stopped before it began goes at once, and pictures go to
/// workers while there is room.
pub fn start(m: &mut Model, fx: &mut Fx) {
    while let Some(at) = m.queue.iter().position(|entry| !entry.flying && entry.stop.is_set()) {
        let (id, made) = (m.queue[at].job.id, m.queue[at].job.made);
        abandon(m, fx, id, made);
    }
    let mut at = 0;
    while at < m.queue.len() {
        let entry = &m.queue[at];
        let waiting = !entry.flying && entry.job.status != "running";
        let continuing = !entry.flying && entry.job.status == "running";
        if (waiting && making(m) < RENDERING_AT_ONCE) || continuing {
            send(m, fx, at);
        }
        at += 1;
    }
    queue_moved(fx);
}

/// Hand the next picture of the job at `at` to a worker.
fn send(m: &mut Model, fx: &mut Fx, at: usize) {
    let root = m.places.gallery.clone();
    let entry = &mut m.queue[at];
    entry.job.status = "running".to_string();
    entry.flying = true;
    let (id, index) = (entry.job.id, entry.job.made);
    let (work, backend, stop) = (entry.work.clone(), entry.backend.clone(), entry.stop.clone());
    fx.spawn_or(Command::MadeLost(id), move |host| {
        // The framework's own flag too: a close or a logout stops a render in flight.
        let stop = stop.linked(host);
        let outcome = match &work {
            Work::Plan { plan, made_from } => jobs::make_picture(&backend, plan, index, made_from, &root, &stop),
            Work::Upscale { source, factor } => jobs::upscale_picture(source, *factor, &root, &stop),
        };
        Command::Made { id, outcome }
    });
}

/// A picture of a job landed: filed, stopped or failed.
pub fn made(m: &mut Model, fx: &mut Fx, id: i32, outcome: Result<Saved, Option<String>>) {
    let Some(entry) = m.entry_mut(id) else { return };
    entry.flying = false;
    match outcome {
        Ok(saved) => {
            entry.job.made += 1;
            entry.job.seconds += saved.seconds;
            let done = entry.job.made >= entry.job.wanted;
            file(m, fx, saved);
            if done {
                finish(m, id);
            }
        }
        Err(None) => {
            let made = entry.job.made;
            abandon(m, fx, id, made);
        }
        Err(Some(problem)) => {
            // The notice is the only place a failure can go that a caller will read: the job is
            // out of the queue by then, and the gallery never held it. Set before the job leaves,
            // so a reader who sees an empty queue sees the reason in the same state.
            m.notice = problem;
            fx.changed(PART_NOTICE);
            finish(m, id);
        }
    }
    start(m, fx);
}

/// Put one new picture at the front of the gallery, where it belongs.
fn file(m: &mut Model, fx: &mut Fx, saved: Saved) {
    super::gallery::drop_reads(m, fx);
    let path = saved.record.path.clone();
    m.gallery.retain(|each| each.path != path);
    m.gallery.insert(0, saved.record);
    m.gallery.truncate(gallery::LISTING_CAP);
    m.stamp = saved.stamp;
    m.content = content_stamp(&m.gallery);
    fx.changed(PART_GALLERY);
    fx.changed(PART_STATUS);
}

fn finish(m: &mut Model, id: i32) {
    m.queue.retain(|entry| entry.job.id != id);
}

/// A job that was stopped leaves the queue, and says what it left behind.
fn abandon(m: &mut Model, fx: &mut Fx, id: i32, made: u32) {
    // The notice is set before the job leaves the queue, and that order is the point: a reader who
    // sees an empty queue has to see the reason in the same state. The other order has a window,
    // small and exactly where a watcher polling the queue lands, in which the job is gone and the
    // notice is not there yet, so a failure reads as a success.
    m.notice = match made {
        0 => "Cancelled before anything was made.".to_string(),
        1 => "Cancelled. 1 picture had already been written to the gallery and has been left there; a generation cannot be resumed, so ask again if you want the rest.".to_string(),
        n => format!(
            "Cancelled. {n} pictures had already been written to the gallery and have been left there; a generation cannot be resumed, so ask again if you want the rest."
        ),
    };
    fx.changed(PART_NOTICE);
    finish(m, id);
}

/// Stop one job, or every job. `All` is what a person pressing Cancel in the window means and what
/// a Mind asking for `cancel_all` means.
pub fn cancel(m: &mut Model, fx: &mut Fx, which: Which) {
    // An id nobody was ever issued is a mistake about which job was meant, and is refused as that:
    // `-1` once read as 0, 0 meant "no job was named", and the whole queue stopped. Stopping
    // everything is a real ask with its own action, `cancel_all`.
    let targets: Vec<i32> = match which {
        Which::Unusable { given } => {
            let agent = if given {
                "`cancel` was given a `job` that is not an id: an id is a whole number of 1 or more. To stop every job, ask for `cancel_all`."
            } else {
                "`cancel` needs `job`, an id from `describe`'s `queue`. To stop every job, ask for `cancel_all`."
            };
            return refuse(m, fx, Said::new(agent, "A job could not be named, so nothing was stopped."));
        }
        Which::One(id) if m.entry(id).is_some() => vec![id],
        Which::One(id) => {
            return refuse(
                m,
                fx,
                Said::new(
                    format!("there is no job {id} in the queue. `describe` lists the ones there are."),
                    format!("there is no job {id} in the queue."),
                ),
            );
        }
        Which::All => m.jobs().map(|job| job.id).collect(),
    };
    if targets.is_empty() {
        return refuse(m, fx, Said::both("nothing was queued or running"));
    }
    for id in &targets {
        if let Some(entry) = m.entry_mut(*id) {
            entry.stop.set();
            // "cancelling" rather than gone, because the worker is still in the middle of
            // something; a queue that said nothing while a server was still rendering would be
            // describing a busy machine as idle.
            entry.job.status = "cancelling".to_string();
        }
    }
    let line = match targets.len() {
        1 => format!("asked job {} to stop", targets[0]),
        n => format!("asked {n} jobs to stop"),
    };
    // A job that had not started goes at once.
    start(m, fx);
    answers::settle(m, fx, json!({ "cancelled": line }));
}
