//! The output folder: read on a worker, noticed when it moves, and never read by the UI thread.
//!
//! One read at a time. The poll asks first with two directory timestamps (`gallery::cheap_stamp`)
//! and reads the folder only if they moved; an explicit refresh replaces a read in flight. A read
//! that lands after the app itself changed the gallery (a picture filed, one moved to the Trash) is
//! dropped, so the list never goes back to a moment the person has left.

use serde_json::json;
use yantrik_app::effects::Effects;
use yantrik_app::jobs::{Concurrency, JobKey, Scope};

use crate::answers;
use crate::command::Command;
use crate::gallery::{self, Listed};
use crate::model::{Model, PART_DETAIL, PART_GALLERY, PART_NOTICE, PART_STATUS};
use crate::Studio;

type Fx = Effects<Studio>;

fn key() -> JobKey {
    JobKey { feature: "studio.gallery", scope: Scope::App }
}

/// Look at the folder's timestamps and read it again if they moved. Free while nothing happens.
pub fn poll(m: &mut Model, fx: &mut Fx) {
    // One read at a time: a folder written to steadily would otherwise start a worker every tick,
    // each decoding the same twelve thumbnails.
    if m.reading.is_some() {
        return;
    }
    let root = m.places.gallery.clone();
    let known = m.stamp.clone();
    let id = fx.spawn_keyed_or(key(), Concurrency::RejectIfRunning, Command::Unchanged, move |_| {
        if gallery::cheap_stamp(&root) == known {
            Command::Unchanged
        } else {
            Command::Listed(Box::new(gallery::read_all(&root)))
        }
    });
    m.reading = Some(id);
}

/// The `refresh` action and button: the notice is dropped, and the folder is read again whatever
/// its timestamps say. A read already running is replaced.
pub fn refresh(m: &mut Model, fx: &mut Fx) {
    if !m.notice.is_empty() {
        m.notice = String::new();
        fx.changed(PART_NOTICE);
    }
    let root = m.places.gallery.clone();
    let id = fx.spawn_keyed_or(
        key(),
        Concurrency::ReplacePrevious,
        Command::Lost("read of the output folder"),
        move |_| Command::Listed(Box::new(gallery::read_all(&root))),
    );
    m.reading = Some(id);
}

/// A listing landed. It replaces the gallery only if it found something different, so a read that
/// found the same pictures redraws nothing. A call that asked for it is answered with what this
/// read found.
pub fn listed(m: &mut Model, fx: &mut Fx, listed: Listed) {
    m.reading = None;
    m.stamp = listed.stamp;
    if listed.content != m.content {
        m.content = listed.content;
        m.gallery = listed.rows;
        fx.changed(PART_GALLERY);
        fx.changed(PART_STATUS);
        fx.changed(PART_DETAIL);
    }
    let count = m.gallery.len();
    answers::settle(m, fx, json!({ "refreshed": true, "gallery_count": count }));
}

/// The app changed the gallery itself: a read that began earlier is of a folder that has moved on.
pub fn drop_reads(m: &mut Model, fx: &mut Fx) {
    if let Some(id) = m.reading.take() {
        fx.cancel(id);
    }
}
