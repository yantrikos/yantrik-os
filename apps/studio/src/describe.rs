//! What the Mind reads: one line, trouble first, and the state behind it.
//!
//! Deliberately a glance: the newest few pictures with what made them, the queue, and where the
//! files are. A caller that wants the rest can read the folder, which is the point of writing real
//! files into it.
//!
//! The sentences a person typed are in the queue's labels and in each picture's `prompt`. Drawn
//! for a Mind (`Boot::for_mind`) they are the framework's one mask, and the rest stays. In Private
//! mode the framework answers with the neutral view and none of this is read
//! (`tests/private_mode.rs`).

use serde_json::{json, Value};
use yantrik_app::control::View;
use yantrik_app::for_mind::HIDDEN;

use crate::config::shown_address;
use crate::model::{display, Model};

pub fn view(m: &Model) -> View {
    View::new(summary(m)).state(state(m))
}

/// One line, trouble first: a caller surveying every open window pays one line per app, and the
/// reason to look is what belongs at the front.
pub fn summary(m: &Model) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !m.notice.is_empty() {
        parts.push(m.notice.clone());
    }
    let running = m.jobs().filter(|job| job.status == "running").count();
    let waiting = m.queue.len() - running;
    match (running, waiting) {
        (0, 0) => {}
        (1, 0) => parts.push("1 picture being made".to_string()),
        (n, 0) => parts.push(format!("{n} pictures being made")),
        (0, n) => parts.push(format!("{n} waiting")),
        (r, p) => parts.push(format!("{r} being made, {p} waiting")),
    }
    parts.push(backend_in_short(m));
    parts.push(match m.gallery.len() {
        0 => "nothing in the gallery yet".to_string(),
        1 => format!("1 picture in {}", display(&m.places.gallery)),
        n => format!("{n} pictures in {}", display(&m.places.gallery)),
    });
    format!("Studio \u{2014} {}", parts.join("; "))
}

fn backend_in_short(m: &Model) -> String {
    let facts = m.facts();
    if !facts.configured {
        return "no backend configured, so pictures are placeholders".to_string();
    }
    match facts.kind {
        "fake" => "placeholder images, because no image model is configured".to_string(),
        "comfyui" if !facts.prompt_leaves => {
            format!("ComfyUI at {}", shown_address(&m.config.backend.base_url))
        }
        "comfyui" => format!("ComfyUI away from your network at {}", shown_address(&m.config.backend.base_url)),
        _ => format!("a hosted service at {}", shown_address(&m.config.backend.base_url)),
    }
}

/// The state object.
pub fn state(m: &Model) -> Value {
    let running: Vec<_> = m.jobs().filter(|job| job.status == "running").map(|job| job.json(m.for_mind)).collect();
    let waiting: Vec<_> = m.jobs().filter(|job| job.status != "running").map(|job| job.json(m.for_mind)).collect();
    let mut state = serde_json::Map::new();
    state.insert("backend".into(), m.config.state(m.key_is_set()));
    state.insert("output_folder".into(), json!(display(&m.places.gallery)));
    state.insert(
        "gallery".into(),
        json!({
            "count": m.gallery.len(),
            "newest": m.gallery.iter().take(6).map(|record| newest(m, record)).collect::<Vec<_>>(),
        }),
    );
    // An empty queue is still shown. A caller has to be able to tell "nothing is running" from
    // "I could not read it".
    state.insert("queue".into(), json!({ "running": running, "pending": waiting }));
    if !m.notice.is_empty() {
        state.insert("notice".into(), json!(m.notice));
    }
    json!(state)
}

/// One picture of the gallery, its sentence masked where a Mind is looking.
fn newest(m: &Model, record: &crate::gallery::Record) -> Value {
    let mut row = record.json();
    if m.for_mind && row.get("prompt").is_some() {
        row["prompt"] = json!(HIDDEN);
    }
    row
}
