//! The window: its callbacks bound to commands, and the model drawn for the parts that changed.
//! The only code that writes the window's properties.
//!
//! Every string arrives written (`words.rs`); the window formats nothing. The fields the person
//! types into belong to the model (`Command::Edit`): the window is drawn from it, and a field is
//! written only where it differs from what the window already holds, so the caret stays where it
//! is while the person types and a refused edit does not stay in the box.

use slint::{ModelRc, SharedPixelBuffer, SharedString, VecModel};
use yantrik_app::app::Changed;
use yantrik_app::cx::Cx;
use yantrik_app::for_mind::shown;

use crate::command::Command;
use crate::gallery::{Record, Thumbnail};
use crate::model::{
    display, Field, Model, PART_BACKEND, PART_DETAIL, PART_FORM, PART_GALLERY, PART_JOBS, PART_NOTICE, PART_STATUS,
    PART_WIDE,
};
use crate::words;
use crate::{JobRow, ShotRow, Studio, StudioApp};

/// Every button in the window, including the four on a gallery row. A row button hands back
/// `<verb>:<path>` rather than an index, because the index would be the index into the list the
/// click is about to change.
fn pressed(id: &str) -> Command {
    let (verb, target) = match id.split_once(':') {
        Some((verb, target)) => (verb, target.to_string()),
        None => (id, String::new()),
    };
    match verb {
        "generate" => Command::Generate(None),
        "refresh" => Command::Refresh,
        "folder" => Command::OpenFolder,
        "open" => Command::Open { of: target },
        "variations" => Command::Variations { of: target, count: 1, fill: true },
        "upscale" => Command::Upscale { of: target, factor: 2 },
        "delete" => Command::Delete { of: target },
        "cancel" => Command::Cancel(crate::command::Which::All),
        "backend" => Command::ExplainBackend,
        other => Command::Unknown(other.to_string()),
    }
}

pub fn wire(cx: &Cx<Studio>) {
    let ui = cx.ui();
    ui.on_action(cx.cmd1(|id: SharedString| pressed(id.as_str())));
    ui.on_choose(cx.cmd1(Command::Choose));
    ui.on_resized(cx.cmd1(Command::Resized));
    let typing = cx.clone();
    ui.on_field_edited(move |field, text| {
        if let Some(field) = Field::of(field.as_str()) {
            typing.run(Command::Edit(field, text.to_string()));
        }
    });
    ui.invoke_focus_prompt();
}

pub fn render(ui: &StudioApp, m: &Model, changed: &Changed) {
    if changed.has(PART_BACKEND) {
        ui.set_backend_line(words::headline(m).into());
        ui.set_generate_label(words::generate_label(m).into());
        ui.set_facts(words::facts_block(m).into());
        ui.set_output_folder(display(&m.places.gallery).into());
    }
    if changed.has(PART_STATUS) {
        ui.set_status(words::status_line(m).into());
        ui.set_busy(!m.queue.is_empty());
    }
    if changed.has(PART_NOTICE) {
        ui.set_notice(m.notice.as_str().into());
    }
    if changed.has(PART_WIDE) {
        ui.set_wide(m.wide);
    }
    if changed.has(PART_FORM) {
        form(ui, m);
    }
    if changed.has(PART_GALLERY) {
        let rows: Vec<ShotRow> = m.gallery.iter().map(|record| row(record, &m.chosen.path, m.for_mind)).collect();
        ui.set_shots(ModelRc::new(VecModel::from(rows)));
    }
    if changed.has(PART_DETAIL) {
        ui.set_detail(m.chosen.detail.as_str().into());
    }
    if changed.has(PART_JOBS) {
        let rows: Vec<JobRow> = m
            .jobs()
            .map(|job| JobRow {
                label: job.shown_label(m.for_mind).into(),
                status: job.status.as_str().into(),
                progress: if job.wanted > 1 { format!("{} of {}", job.made, job.wanted).into() } else { SharedString::new() },
            })
            .collect();
        ui.set_jobs(ModelRc::new(VecModel::from(rows)));
    }
    ui.set_read_only(m.for_mind);
}

/// The fields, from the model, where they differ from what the window holds.
fn form(ui: &StudioApp, m: &Model) {
    let f = &m.form;
    if ui.get_prompt().as_str() != f.prompt {
        ui.set_prompt(f.prompt.as_str().into());
    }
    if ui.get_negative().as_str() != f.negative {
        ui.set_negative(f.negative.as_str().into());
    }
    if ui.get_want_width().as_str() != f.width {
        ui.set_want_width(f.width.as_str().into());
    }
    if ui.get_want_height().as_str() != f.height {
        ui.set_want_height(f.height.as_str().into());
    }
    if ui.get_want_steps().as_str() != f.steps {
        ui.set_want_steps(f.steps.as_str().into());
    }
    if ui.get_want_count().as_str() != f.count {
        ui.set_want_count(f.count.as_str().into());
    }
}

/// One gallery row. Drawn for a Mind, the sentence and the picture are not shown.
fn row(record: &Record, chosen: &str, for_mind: bool) -> ShotRow {
    let path = record.path.display().to_string();
    let thumbnail = record.thumbnail.as_ref().filter(|_| !for_mind);
    ShotRow {
        name: record.name.as_str().into(),
        prompt: if record.prompt.is_empty() {
            "(no prompt recorded beside this file)".into()
        } else {
            shown(for_mind, &record.prompt).as_ref().into()
        },
        meta: words::meta_line(record).into(),
        path: display(&record.path).into(),
        // What the row says: the folder in words. The path is the record pane's last line.
        place: record
            .path
            .parent()
            .map(|dir| yantrik_app::place::place_of(dir, &yantrik_app::place::home()))
            .unwrap_or_default()
            .into(),
        has_thumb: thumbnail.is_some(),
        thumb: thumbnail.map(pixels).unwrap_or_default(),
        selected: path == chosen,
    }
}

/// Decoded pixels into the one image type Slint will draw.
///
/// The bytes cross from the worker as plain RGB, because Slint's `Image` is not `Send` and a
/// thumbnail decoded on a worker cannot be handed to the UI thread as an image. Copying into the
/// buffer's own bytes keeps the row order Slint expects and needs no unsafe.
fn pixels(thumb: &Thumbnail) -> slint::Image {
    let mut buffer = SharedPixelBuffer::<slint::Rgb8Pixel>::new(thumb.width, thumb.height);
    let bytes = buffer.make_mut_bytes();
    let take = bytes.len().min(thumb.rgb.len());
    bytes[..take].copy_from_slice(&thumb.rgb[..take]);
    slint::Image::from_rgb8(buffer)
}
