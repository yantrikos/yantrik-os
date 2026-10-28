//! Every string the window shows, written here and nowhere else.
//!
//! The window formats nothing, so the sentence a person reads and the JSON a Mind reads (the
//! summary and the state in `describe.rs`) are built beside each other and cannot drift apart.

use std::path::Path;

use crate::gallery::{Record, Sidecar};
use crate::model::{display, Model};

/// What a person reads for the `fake` backend. `fake` stays the word in the config file and in
/// `set_backend`, because agents and configs use it; on screen it is developer vocabulary, and
/// what the backend makes is a placeholder.
const PLACEHOLDER: &str = "placeholder";

/// The backend's name as a person reads it.
pub fn backend_name(kind: &str) -> &str {
    if kind == "fake" {
        PLACEHOLDER
    } else {
        kind
    }
}

/// The primary button's words. "Generate" on the placeholder backend promises a picture of the
/// sentence; what arrives is a pattern drawn from its hash.
pub fn generate_label(m: &Model) -> &'static str {
    if m.facts().kind == "fake" {
        "Make placeholder"
    } else {
        "Generate"
    }
}

/// The line under the app's own name: which backend, and whether the prompt leaves.
pub fn headline(m: &Model) -> String {
    let facts = m.facts();
    // Unconfigured and chosen alike: either way no model draws, and the line says so.
    if facts.kind == "fake" {
        return "Placeholder images \u{b7} no image model configured".to_string();
    }
    let mut line = format!("{} \u{b7} {}", facts.kind, facts.place);
    if facts.prompt_leaves {
        line.push_str(" \u{b7} your prompt leaves this machine");
    }
    line
}

/// The right pane's account of where pictures are made, including the ways it is incomplete.
pub fn facts_block(m: &Model) -> String {
    let facts = m.facts();
    let mut lines = vec![format!("Backend: {}", backend_name(facts.kind)), format!("Where: {}", facts.place)];
    if !facts.model.is_empty() && facts.kind != "fake" {
        lines.push(format!("Model: {}", facts.model));
    }
    if facts.kind == "openai-images" {
        // The variable's name, never its contents. A person has to be able to tell which variable
        // to export; the value is the one thing this window must not show, so the only question
        // asked of it is whether it is there.
        lines.push(format!(
            "API key: {}, which is {}.",
            crate::config::variable_phrase(&m.config.backend.api_key_env),
            if m.key_is_set() { "set, so prompts can be sent" } else { "not usable, so nothing can be sent yet" }
        ));
    }
    lines.push(format!("Making a picture is graded `{}` on this configuration.", facts.grade));
    if facts.prompt_leaves {
        lines.push("Your prompt is sent off this machine, and the service may charge for it.".to_string());
    } else {
        lines.push("Your prompt does not leave a network you own.".to_string());
    }
    if !facts.window_note.is_empty() {
        lines.push(facts.window_note.clone());
    }
    lines.join("\n")
}

/// The footer: what is happening, or what is waiting to.
pub fn status_line(m: &Model) -> String {
    if let Some(job) = m.jobs().find(|job| job.status == "running") {
        let progress = if job.wanted > 1 {
            format!(" \u{2014} {} of {}", job.made, job.wanted)
        } else {
            String::new()
        };
        return format!("{}{} \u{b7} {:.0}s so far", job.shown_label(m.for_mind), progress, job.seconds);
    }
    let waiting = m.queue.len();
    if waiting > 0 {
        return format!("{waiting} waiting to start");
    }
    format!(
        "{} in {} \u{b7} pictures are files, and each has a record beside it",
        match m.gallery.len() {
            0 => "Nothing yet".to_string(),
            1 => "1 picture".to_string(),
            n => format!("{n} pictures"),
        },
        display(&m.places.gallery)
    )
}

/// The one line under a thumbnail: everything the brief says a gallery should show, in the order a
/// person reads it.
pub fn meta_line(record: &Record) -> String {
    let size = format!("{}\u{d7}{}", record.width, record.height);
    if !record.has_sidecar {
        // Not a guess presented as a record. A picture with no sidecar is a picture this app cannot
        // account for, and saying so is the difference between a gallery and a pile of files.
        return format!("{size} \u{b7} no record beside it, so nothing is known about how it was made");
    }
    // The placeholder backend records itself as both backend and model, which read
    // "fake · fake"; it has no model, so it is named once.
    let placeholder = record.backend == "fake";
    let mut parts = vec![backend_name(&record.backend).to_string()];
    if !record.model.is_empty() && !placeholder {
        parts.push(record.model.clone());
    }
    parts.push(format!("seed {}", record.seed));
    parts.push(size);
    parts.push(format!("{:.1} s", record.seconds));
    if !record.made_from.is_empty() {
        parts.push(format!("made from {}", record.made_from));
    }
    parts.join(" \u{b7} ")
}

/// The right pane, for one chosen picture. Read from the sidecar, not from the listing: the sidecar
/// is the record, and the pane is where a person checks that the record is real.
pub fn detail(path: &Path, sidecar: Option<&Sidecar>) -> String {
    let Some(sidecar) = sidecar else {
        return format!(
            "{}\n\nThere is no record beside this picture, so nothing can be said about how it was \
             made. It may have been copied into the folder by hand.",
            display(path)
        );
    };
    let mut lines = vec![display(path), String::new(), format!("Prompt: {}", sidecar.prompt)];
    if !sidecar.negative.is_empty() {
        lines.push(format!("Kept out: {}", sidecar.negative));
    }
    lines.push(format!("Seed: {}", sidecar.seed));
    lines.push(format!("Size: {}\u{d7}{}", sidecar.width, sidecar.height));
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
    lines.push(format!("Made by: {}", backend_name(&sidecar.backend)));
    if !sidecar.model.is_empty() && sidecar.backend != "fake" {
        lines.push(format!("Model: {}", sidecar.model));
    }
    lines.push(format!("Took: {:.1} seconds", sidecar.seconds));
    lines.push(format!("When: {}", sidecar.created));
    if !sidecar.made_from.is_empty() {
        lines.push(format!("Made from: {}", sidecar.made_from));
    }
    lines.join("\n")
}
