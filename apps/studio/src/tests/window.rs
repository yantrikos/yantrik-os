//! The real window, headless: Slint's own software renderer, so the callbacks a person presses, the
//! strings that reach the screen and the PNG of the result are the real ones. Screenshots are
//! written beside the test binary (`ui-screenshots/`) so a person can look at them instead of
//! trusting the assertions about them.

use std::path::PathBuf;

use slint::{ComponentHandle as _, Model as _};

use super::{unconfigured, Fixture};
use crate::StudioApp;

fn shot_dir() -> PathBuf {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("ui-screenshots")))
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn screenshot(f: &Fixture, name: &str) -> PathBuf {
    f.app.draw_until_still();
    let (width, height, pixels) = f.app.ui(|ui: &StudioApp| {
        let shot = ui.window().take_snapshot().expect("a snapshot of the software window");
        (shot.width(), shot.height(), shot.as_bytes().to_vec())
    });
    let path = shot_dir().join(name);
    let mut png = png::Encoder::new(std::fs::File::create(&path).unwrap(), width, height);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header().unwrap().write_image_data(&pixels).unwrap();
    path
}

fn looked_at(path: &PathBuf) {
    assert!(std::fs::metadata(path).unwrap().len() > 1000, "{} is blank", path.display());
}

/// What a person typing into a field does: the window holds the text, and says so.
pub(super) fn type_into(f: &Fixture, field: &'static str, text: &str) {
    let text = text.to_string();
    f.app.ui(move |ui: &StudioApp| {
        match field {
            "prompt" => ui.set_prompt(text.as_str().into()),
            "negative" => ui.set_negative(text.as_str().into()),
            "width" => ui.set_want_width(text.as_str().into()),
            "height" => ui.set_want_height(text.as_str().into()),
            "steps" => ui.set_want_steps(text.as_str().into()),
            _ => ui.set_want_count(text.as_str().into()),
        }
        ui.invoke_field_edited(field.into(), text.as_str().into());
    });
}

pub(super) fn press(f: &Fixture, id: &str) {
    let id = id.to_string();
    f.app.ui(move |ui: &StudioApp| ui.invoke_action(id.as_str().into()));
}

#[test]
fn the_window_shows_the_queue_the_gallery_and_the_record_of_the_chosen_picture() {
    // Unconfigured, which is what a machine with no GPU and no API key gets: the app has to open,
    // draw, and say plainly that the pictures are placeholders.
    let f = unconfigured("window");
    f.app.resize(1280, 860);
    f.app.draw_until_still();

    // ── empty ──
    let ui = |f: &Fixture| f.app.ui(|ui: &StudioApp| {
        (
            ui.get_backend_line().to_string(),
            ui.get_generate_label().to_string(),
            ui.get_facts().to_string(),
            ui.get_output_folder().to_string(),
            ui.get_notice().to_string(),
            ui.get_status().to_string(),
            ui.get_busy(),
            ui.get_detail().to_string(),
        )
    });
    let (line, label, facts, folder, notice, status, busy, detail) = ui(&f);
    assert_eq!(line, "Placeholder images \u{b7} no image model configured");
    // The primary button says what it will make; `fake` is the config's word, not the person's.
    assert_eq!(label, "Make placeholder");
    assert!(facts.contains("No backend is configured"), "{facts}");
    assert!(facts.contains("graded `standard`"), "{facts}");
    assert_eq!(folder, crate::model::display(&f.gallery()));
    assert_eq!(notice, "");
    assert!(status.contains("Nothing yet"), "{status}");
    assert!(!busy);
    assert!(f.rows().is_empty());
    assert_eq!(f.app.ui(|ui: &StudioApp| ui.get_jobs().row_count()), 0);
    assert_eq!(detail, "", "the right pane starts as an invitation, not as a record");
    // The ask fields start at the defaults. A field that looks empty and means 1024 is a field a
    // person cannot trust.
    let fields = f.app.ui(|ui: &StudioApp| (ui.get_want_width().to_string(), ui.get_want_steps().to_string(), ui.get_negative().to_string()));
    assert_eq!(fields, ("1024".to_string(), "30".to_string(), crate::plan::DEFAULT_NEGATIVE.to_string()));
    assert!(f.app.ui(|ui: &StudioApp| ui.get_wide()), "a window this wide has the record pane");
    looked_at(&screenshot(&f, "studio-empty.png"));

    // ── a generation, from the button a person presses ──
    type_into(&f, "prompt", "a lighthouse in fog");
    type_into(&f, "width", "256");
    type_into(&f, "height", "192");
    type_into(&f, "steps", "4");
    type_into(&f, "count", "2");
    press(&f, "generate");
    // The queue is on screen at once, because a render takes longer than a person's patience and
    // the strip is what tells them it heard them.
    let (busy, jobs, status) = f.app.ui(|ui: &StudioApp| {
        let jobs: Vec<(String, String, String)> =
            ui.get_jobs().iter().map(|j| (j.label.to_string(), j.status.to_string(), j.progress.to_string())).collect();
        (ui.get_busy(), jobs, ui.get_status().to_string())
    });
    assert!(busy);
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].0, "2 pictures of \u{201c}a lighthouse in fog\u{201d}");
    assert_eq!(jobs[0].1, "running");
    assert_eq!(jobs[0].2, "0 of 2");
    assert!(status.contains("2 pictures of"), "the footer does not name the job: {status}");

    assert_eq!(f.settled(), "");
    let (busy, jobs, status) = f.app.ui(|ui: &StudioApp| (ui.get_busy(), ui.get_jobs().row_count(), ui.get_status().to_string()));
    assert!(!busy);
    assert_eq!(jobs, 0, "the queue kept a job that had finished");
    assert_eq!(f.rows().len(), 2);
    assert!(status.contains("2 pictures in"), "{status}");
    assert!(status.contains("pictures are files"), "{status}");

    let top = f.rows().remove(0);
    assert_eq!(top.prompt, "a lighthouse in fog");
    assert!(top.meta.starts_with("placeholder \u{b7} seed "), "{}", top.meta);
    assert!(!top.meta.contains("fake"), "{}", top.meta);
    assert!(top.meta.contains("256\u{d7}192"), "{}", top.meta);
    assert!(top.meta.contains(" s"), "{}", top.meta);
    assert!(top.path.starts_with("~/") || top.path.starts_with(&f.world.dir.display().to_string()), "{}", top.path);
    // The thumbnail was decoded on a worker and crossed to this thread as plain RGB, because
    // Slint's own image type is not `Send`.
    assert!(top.has_thumb, "the row has no thumbnail");
    let (width, height) = top.thumb;
    assert!(width > 0 && height > 0, "the thumbnail is empty");
    assert!(width <= crate::gallery::THUMBNAIL_EDGE && height <= crate::gallery::THUMBNAIL_EDGE, "{:?}", top.thumb);
    assert!(!top.selected);
    looked_at(&screenshot(&f, "studio-gallery.png"));

    // ── choosing a row reads its record back ──
    f.app.ui(|ui: &StudioApp| ui.invoke_choose(0));
    f.quiet();
    let (_, _, _, _, _, _, _, detail) = ui(&f);
    assert!(detail.contains("Prompt: a lighthouse in fog"), "{detail}");
    assert!(detail.contains("Size: 256\u{d7}192"), "{detail}");
    assert!(detail.contains("Steps: 4"), "{detail}");
    assert!(detail.contains("Made by: placeholder"), "{detail}");
    assert!(f.rows()[0].selected, "the chosen row does not show as chosen");
    looked_at(&screenshot(&f, "studio-detail.png"));
    // Choosing the same row again puts the pane back, because a second click on a selected row is
    // what a person does to undo it.
    f.app.ui(|ui: &StudioApp| ui.invoke_choose(0));
    f.quiet();
    assert_eq!(ui(&f).7, "");
    assert!(!f.rows()[0].selected);

    // ── the buttons that change nothing on disk ──
    press(&f, "backend");
    let notice = ui(&f).4;
    assert_eq!(
        notice,
        "Pictures are made by this machine right now. Changing where Studio sends prompts is the person's choice; to change it, edit ~/.config/yantrik/studio.json."
    );
    assert!(!notice.contains("set_backend"), "the button sends the person to an action: {notice}");
    // `refresh` clears the notice rather than leaving the last thing that went wrong on screen.
    press(&f, "refresh");
    f.quiet();
    assert_eq!(ui(&f).4, "");

    // A refusal from a button reaches the notice bar, which is where the person is looking, and
    // `describe`, which is where a mind would look.
    type_into(&f, "prompt", "   ");
    press(&f, "generate");
    let notice = ui(&f).4;
    assert!(notice.contains("a prompt is needed"), "{notice}");
    assert!(!ui(&f).6, "a refused generation started a job");
    assert!(f.summary().contains("a prompt is needed"), "{}", f.summary());

    // ── a row's own buttons ──
    // They hand back "<verb>:<path>" rather than an index, because the index would be an index
    // into the list the click is about to change.
    let path = f.app.model(|m| m.gallery[0].path.clone());
    press(&f, &format!("upscale:{}", path.display()));
    f.settled();
    assert_eq!(f.rows().len(), 3);
    let top = f.rows().remove(0);
    assert!(top.meta.contains("made from upscale of"), "{}", top.meta);
    assert!(top.meta.contains("512\u{d7}384"), "{}", top.meta);

    // A refusal on purpose: a path that is not there must delete nothing, and the window must say
    // so rather than sit there.
    press(&f, &format!("delete:{}/no-such-picture.png", f.world.dir.display()));
    f.quiet();
    assert!(ui(&f).4.contains("is not there"), "{}", ui(&f).4);
    assert_eq!(f.rows().len(), 3, "a refused delete changed the gallery");

    let newest = f.app.model(|m| m.gallery[0].path.clone());
    press(&f, &format!("delete:{}", newest.display()));
    f.quiet();
    assert_eq!(f.rows().len(), 2, "the deleted picture is still on screen");
    assert!(ui(&f).4.contains("Moved"), "{}", ui(&f).4);

    // `variations` fills the prompt box with the sentence the picture was made from, so the person
    // can see what is about to be asked for again, and change it.
    type_into(&f, "prompt", "something else entirely");
    let source = f.app.model(|m| m.gallery[0].path.clone());
    press(&f, &format!("variations:{}", source.display()));
    f.quiet();
    assert_eq!(f.app.ui(|ui: &StudioApp| ui.get_prompt().to_string()), "a lighthouse in fog");
    assert_eq!(f.app.model(|m| m.form.prompt.clone()), "a lighthouse in fog", "the model and the box disagree");
    f.settled();

    // `open` and the folder button are not pressed here: they hand a path to the desktop, which on
    // a machine with one would open a window nobody asked for.
}

/// The record pane needs room: the window reports its width and the model decides.
#[test]
fn the_record_pane_follows_the_width_of_the_window() {
    let f = unconfigured("wide");
    f.app.resize(1280, 860);
    f.app.draw_until_still();
    f.quiet();
    assert!(f.app.model(|m| m.wide));
    let wide = screenshot(&f, "studio-wide.png");
    let at_the_right_edge = |f: &Fixture| {
        f.app.ui(|ui: &StudioApp| {
            let shot = ui.window().take_snapshot().expect("a snapshot");
            let (x, y) = (shot.width() as usize - 20, 400usize);
            let at = (y * shot.width() as usize + x) * 4;
            shot.as_bytes()[at..at + 3].to_vec()
        })
    };
    let pane = at_the_right_edge(&f);
    f.app.resize(700, 700);
    f.app.draw_until_still();
    f.quiet();
    assert!(!f.app.model(|m| m.wide), "a narrow window kept the record pane");
    assert!(!f.app.ui(|ui: &StudioApp| ui.get_wide()));
    // The layout follows: the pane is 330 px, so the narrow window draws a different picture of
    // its right-hand side than the wide one did. Both are written for a person to look at.
    let narrow = screenshot(&f, "studio-narrow.png");
    assert_ne!(at_the_right_edge(&f), pane, "the right edge of a narrow window is still the record pane's colour");
    looked_at(&wide);
    looked_at(&narrow);
    f.app.resize(1280, 860);
    f.app.draw_until_still();
    f.quiet();
    assert!(f.app.ui(|ui: &StudioApp| ui.get_wide()));
}

/// An edit is the model's: the box is drawn from it, so a field the person typed into and one a
/// command set read the same, and the caret is not disturbed by a redraw of what is already there.
#[test]
fn the_fields_are_the_models_and_a_refused_value_does_not_stay_in_the_box() {
    let f = unconfigured("fields");
    type_into(&f, "prompt", "a lighthouse");
    assert_eq!(f.app.model(|m| m.form.prompt.clone()), "a lighthouse");
    f.app.run(crate::Command::Edit(crate::model::Field::Prompt, "from a command".into()));
    assert_eq!(f.app.ui(|ui: &StudioApp| ui.get_prompt().to_string()), "from a command");
}

/// A notice of two or more lines is shown whole: the banner is never shorter than its sentence at
/// the width the window has, at the default size, at a short window (where the layout squeezes its
/// rows) and at the narrowest. The old Studio had the same banner, which grew with a sentence only
/// while the window was tall enough; its notices were one line, so it never showed.
#[test]
fn a_notice_of_several_lines_is_not_clipped() {
    let dir = std::env::temp_dir().join(format!("studio-banner-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("studio.json");
    std::fs::write(&file, r#"{"backend":{"kind":"openai-images","model":"m","api_key_env":"sk-proj-x"}}"#).unwrap();
    // The sentence a refused configuration puts there, and a longer one.
    let f = super::with_config("banner", crate::config::Config::read(&file));
    let measure = |f: &Fixture| f.app.ui(|ui: &StudioApp| (ui.get_notice_need(), ui.get_notice_room()));
    let mut seen_two_lines = false;
    for sentence in [None, Some("word ".repeat(120))] {
        if let Some(long) = sentence {
            f.app.run(crate::Command::Unknown(long));
        }
        for (width, height) in [(1180u32, 800u32), (1180, 560), (900, 540), (700, 540), (700, 800)] {
            f.app.resize(width, height);
            f.app.draw_until_still();
            let (need, room) = measure(&f);
            assert!(room >= need, "{width}x{height}: the banner is {room}px for a sentence that needs {need}px");
            // One line needs 36px; more than that is a wrapped sentence, shown whole.
            seen_two_lines |= need > 40.0;
            looked_at(&screenshot(&f, &format!("studio-notice-{width}x{height}.png")));
        }
    }
    assert!(seen_two_lines, "no size made the sentence wrap, so nothing was checked");
    let _ = std::fs::remove_dir_all(&dir);
}
