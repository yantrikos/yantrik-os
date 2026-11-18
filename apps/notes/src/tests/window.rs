//! The real window, its keys, callbacks and drawing, on Slint's software renderer with no display.

use std::path::PathBuf;

use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle as _, Model as _, SharedString};
use yantrik_app::app::WindowId;
use yantrik_testing::TestApp;

use super::{autosave, settle, started, vault};
use crate::{Notes, NotesApp, ThemeMode};

/// Directory the UI screenshots are written to: beside the test binary, which lives in
/// `<target>/<profile>/deps/`, so the real build directory wherever cargo put it.
fn shot_dir() -> PathBuf {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("ui-screenshots")))
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A window's pixels: its width, height and RGBA bytes.
fn snapshot(ui: &NotesApp) -> (u32, u32, Vec<u8>) {
    let shot = ui.window().take_snapshot().expect("a snapshot of the software window");
    (shot.width(), shot.height(), shot.as_bytes().to_vec())
}

fn write_png(name: &str, width: u32, height: u32, pixels: &[u8]) {
    let mut png = png::Encoder::new(std::fs::File::create(shot_dir().join(name)).unwrap(), width, height);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header().unwrap().write_image_data(pixels).unwrap();
}

pub(super) fn screenshot(app: &TestApp<Notes>, name: &str) {
    let (width, height, pixels) = app.ui(snapshot);
    write_png(name, width, height, &pixels);
}

/// The first window and `second` side by side in one picture, as they stand on the desktop.
pub(super) fn screenshot_two(app: &TestApp<Notes>, second: WindowId, name: &str) {
    const GAP: u32 = 24;
    let shots = [app.ui(snapshot), app.window(second, snapshot).expect("the second window is open")];
    let width = shots.iter().map(|s| s.0).sum::<u32>() + GAP;
    let height = shots.iter().map(|s| s.1).max().unwrap();
    // A dark desktop behind them, opaque.
    let mut pixels: Vec<u8> = [0x20, 0x22, 0x26, 0xff].repeat((width * height) as usize);
    let mut left = 0;
    for (w, h, shot) in &shots {
        for row in 0..*h {
            let from = (row * w * 4) as usize..((row + 1) * w * 4) as usize;
            let to = ((row * width + left) * 4) as usize;
            pixels[to..to + (w * 4) as usize].copy_from_slice(&shot[from]);
        }
        left += w + GAP;
    }
    write_png(name, width, height, &pixels);
}

fn tap(ui: &NotesApp, text: &str) {
    let text: SharedString = text.into();
    ui.window().dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window().dispatch_event(WindowEvent::KeyReleased { text });
}

/// Presses and releases `text` in the window, holding Control when `control`.
fn key(app: &TestApp<Notes>, control: bool, text: &str) {
    let text = text.to_string();
    app.ui(move |ui| {
        if control {
            held(ui, Key::Control, |ui| tap(ui, &text));
        } else {
            tap(ui, &text);
        }
    });
}

/// `inside` with `modifier` held down. Called within one step on the UI thread, so the keys of
/// another test's window, on the same thread, never land in the middle of the chord.
fn held<R>(ui: &NotesApp, modifier: Key, inside: impl FnOnce(&NotesApp) -> R) -> R {
    ui.window().dispatch_event(WindowEvent::KeyPressed { text: modifier.into() });
    let r = inside(ui);
    ui.window().dispatch_event(WindowEvent::KeyReleased { text: modifier.into() });
    r
}

fn ui<R: Send + 'static>(app: &TestApp<Notes>, f: impl FnOnce(&NotesApp) -> R + Send + 'static) -> R {
    app.ui(f)
}

fn current_id(app: &TestApp<Notes>) -> String {
    app.model(|m| m.current().unwrap().id().to_string())
}

#[test]
fn real_notes_autosave_navigation_conflict_trash_preview_and_idle() {
    let app = started();
    let dir = vault(&app);
    ui(&app, |ui| {
        ui.show().unwrap();
        ui.window().set_size(slint::PhysicalSize::new(1120, 760));
    });
    key(&app, true, "n");
    settle(&app);
    assert!(ui(&app, |ui| ui.get_opened()), "Ctrl+N works in the empty window");
    let (on, off) = app.ui(|ui| {
        held(ui, Key::Control, |ui| {
            tap(ui, "f");
            tap(ui, "e");
            let on = ui.get_focus_mode();
            tap(ui, "e");
            (on, ui.get_focus_mode())
        })
    });
    assert!(on && !off, "Ctrl+E toggles focus mode, from the search field too");

    ui(&app, |ui| ui.invoke_action("new:Field notes".into()));
    settle(&app);
    let first = current_id(&app);
    ui(&app, |ui| ui.invoke_focus_editor());
    key(&app, false, "A thoughtful morning 🦀\n");
    assert!(ui(&app, |ui| ui.get_modified()));
    let before = ui(&app, |ui| ui.get_content().to_string());
    key(&app, true, "z");
    assert_ne!(ui(&app, |ui| ui.get_content().to_string()), before, "Native undo must work");
    app.ui(|ui| held(ui, Key::Control, |ui| held(ui, Key::Shift, |ui| tap(ui, "z"))));
    assert_eq!(ui(&app, |ui| ui.get_content().to_string()), before);

    // Switching immediately must await the save rather than dropping the edit.
    ui(&app, |ui| ui.invoke_action("new:Second page".into()));
    settle(&app);
    assert_eq!(ui(&app, |ui| ui.get_note_title().to_string()), "Second page");
    assert!(std::fs::read_to_string(dir.join(&first)).unwrap().contains("thoughtful"));
    let second = ui(&app, |ui| ui.get_content().to_string());
    key(&app, true, "z");
    assert_eq!(ui(&app, |ui| ui.get_content().to_string()), second, "Undo must not cross notes");

    let chosen = first.clone();
    ui(&app, move |ui| {
        ui.invoke_choose(chosen.into());
        ui.set_notebook("Journal".into());
        ui.invoke_metadata();
    });
    autosave(&app);
    assert!(!ui(&app, |ui| ui.get_modified()));
    assert!(std::fs::read_to_string(dir.join(&first).with_extension("meta")).unwrap().contains("Journal"));
    ui(&app, |ui| {
        ui.set_query("THOUGHTFUL".into());
        ui.invoke_filter();
    });
    assert_eq!(ui(&app, |ui| ui.get_notes().row_count()), 1);
    ui(&app, |ui| {
        ui.set_query("".into());
        ui.invoke_filter();
    });

    let text = "# Field notes\n\nA quiet place for ideas.\n\n## Today's intentions\n\n- [x] Make room to think\n- [ ] Follow the interesting question\n\n> Small steps, taken with care.\n\n```rust\nlet idea = \"keep going\";\n```\n";
    ui(&app, move |ui| {
        ui.set_content(text.into());
        ui.invoke_edited(text.into());
    });
    autosave(&app);
    assert!(!ui(&app, |ui| ui.get_modified()));
    let previews = app.ui(|ui| {
        held(ui, Key::Control, |ui| {
            (0..3)
                .map(|_| {
                    tap(ui, "p");
                    ui.get_preview()
                })
                .collect::<Vec<_>>()
        })
    });
    assert_eq!(previews, [true, false, true], "Ctrl+P toggles the preview");
    assert!(ui(&app, |ui| ui.get_blocks().row_count()) > 4);
    screenshot(&app, "notes-dark.png");
    ui(&app, |ui| {
        ui.global::<ThemeMode>().set_dark(false);
        ui.window().set_size(slint::PhysicalSize::new(800, 600));
    });
    screenshot(&app, "notes-light.png");

    // An outside edit under a draft: the save is refused, the draft kept, the way out a copy.
    ui(&app, |ui| ui.invoke_action("preview".into()));
    std::fs::write(dir.join(&first), "external edit").unwrap();
    ui(&app, |ui| {
        ui.set_content("# Draft kept\n".into());
        ui.invoke_edited("# Draft kept\n".into());
        ui.invoke_action("save".into());
    });
    settle(&app);
    assert!(ui(&app, |ui| ui.get_modified()));
    assert!(super::notice(&app).contains("another program changed this note"), "the refusal is the notice");
    assert_eq!(ui(&app, |ui| ui.get_outside()), 1, "the choices are offered");
    // The bar says it, so the window's notice line does not say it again.
    assert_eq!(ui(&app, |ui| ui.get_notice().to_string()), "");
    ui(&app, |ui| ui.invoke_action("new".into()));
    settle(&app);
    assert_eq!(ui(&app, |ui| ui.get_note_title().to_string()), "Draft kept");
    ui(&app, |ui| ui.invoke_action("copy".into()));
    settle(&app);
    assert!(!ui(&app, |ui| ui.get_modified()));
    assert_eq!(std::fs::read_to_string(dir.join(&first)).unwrap(), "external edit");
    let copy = current_id(&app);
    ui(&app, |ui| ui.invoke_action("trash".into()));
    settle(&app);
    assert_eq!(ui(&app, |ui| ui.get_trash_count()), 1);
    ui(&app, move |ui| {
        ui.invoke_choose(format!("trash:{copy}").into());
        ui.invoke_action("restore".into());
    });
    settle(&app);
    assert_eq!(ui(&app, |ui| ui.get_trash_count()), 0);

    // Idle: once the window is not the active one, nothing keeps it drawing, and the app holds
    // no timer that would wake it.
    autosave(&app);
    app.ui(|ui| ui.window().dispatch_event(WindowEvent::WindowActiveChanged(false)));
    app.ui(|_| slint::platform::update_timers_and_animations());
    assert!(!app.ui(|ui| ui.window().has_active_animations()));
    assert_eq!(app.timers_armed(), 0);
}

#[test]
fn escape_and_cancel_close_the_dialog_and_the_model_hears_of_it() {
    let app = started();
    ui(&app, |ui| {
        ui.show().unwrap();
        ui.window().set_size(slint::PhysicalSize::new(1120, 760));
        ui.invoke_action("import".into());
    });
    assert_eq!(ui(&app, |ui| ui.get_dialog()), 1);
    app.ui(|ui| tap(ui, &SharedString::from(Key::Escape)));
    assert_eq!(ui(&app, |ui| ui.get_dialog()), 0);
    assert_eq!(app.model(|m| m.pane.dialog), 0, "a later redraw cannot open it again");
    ui(&app, |ui| ui.invoke_action("focus".into()));
    assert!(app.model(|m| m.pane.focus_mode));
    key(&app, true, "f");
    assert!(!ui(&app, |ui| ui.get_focus_mode()));
    assert_eq!(super::state(&app)["focus"], false, "describe says what the window shows");
}


/// The outside bar and the comparison, drawn: the choices are kit buttons the person can press,
/// and Escape closes the comparison as it closes every dialog.
#[test]
fn the_outside_bar_and_the_comparison_are_drawn_and_answer_the_keys() {
    let (app, path) = super::with_note("Shared");
    ui(&app, |ui| {
        ui.show().unwrap();
        ui.window().set_size(slint::PhysicalSize::new(1120, 760));
    });
    ui(&app, |ui| {
        ui.set_content("# Shared\n\nmine, typed here".into());
        ui.invoke_edited("# Shared\n\nmine, typed here".into());
    });
    std::fs::write(&path, "# Shared\n\ntheirs, written elsewhere\n").unwrap();
    app.look_at_files();
    let said = ui(&app, |ui| (ui.get_outside(), ui.get_outside_said().to_string()));
    assert_eq!(said, (1, crate::update::said::outside(&super::outside(&app)).unwrap().to_string()));
    screenshot(&app, "notes-outside-bar.png");
    ui(&app, |ui| ui.invoke_action("compare".into()));
    settle(&app);
    assert_eq!(ui(&app, |ui| ui.get_dialog()), 3);
    screenshot(&app, "notes-compare.png");
    app.ui(|ui| tap(ui, &SharedString::from(Key::Escape)));
    assert_eq!(app.model(|m| (m.pane.dialog, m.pane.theirs.is_none())), (0, true), "closed, and forgotten");
    assert_eq!(ui(&app, |ui| ui.get_outside()), 1, "the choice is still to be made");
}

/// The recovery bar and the close prompt, drawn: kit buttons the person can press, and Escape is
/// the prompt's Cancel.
#[test]
fn the_recovery_bar_and_the_close_prompt_are_drawn_and_answer() {
    let (app, path) = super::with_note("Kept for later");
    app.run(crate::command::Command::Edit("# Kept for later\n\ntyped before the crash".into()));
    app.run_writes();
    let app = super::killed_and_started_again(app);
    ui(&app, |ui| {
        ui.show().unwrap();
        ui.window().set_size(slint::PhysicalSize::new(1120, 760));
    });
    assert!(ui(&app, |ui| ui.get_recovery_said().to_string()).contains("Kept for later"));
    screenshot(&app, "notes-recovery-bar.png");
    ui(&app, |ui| ui.invoke_action("restore-draft".into()));
    settle(&app);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Kept for later\n\ntyped before the crash");
    ui(&app, |ui| {
        ui.set_content("# Kept for later\n\nand more".into());
        ui.invoke_edited("# Kept for later\n\nand more".into());
    });
    app.close_requested(yantrik_app::app::WindowId::PRIMARY);
    assert_eq!(ui(&app, |ui| ui.get_dialog()), crate::update::closing::CLOSE_DIALOG);
    screenshot(&app, "notes-close-prompt.png");
    // The prompt is modal: the window's shortcuts do not reach the app behind it.
    let notes = app.model(|m| m.library.len());
    key(&app, true, "n");
    app.ui(|ui| tap(ui, &SharedString::from(Key::F5)));
    settle(&app);
    assert_eq!((app.model(|m| m.library.len()), app.jobs_waiting()), (notes, 0), "no new note, no Refresh");
    assert_eq!(ui(&app, |ui| ui.get_dialog()), crate::update::closing::CLOSE_DIALOG, "still asking");
    app.ui(|ui| tap(ui, &SharedString::from(Key::Escape)));
    assert_eq!(ui(&app, |ui| ui.get_dialog()), 0, "Escape is Cancel");
    assert!(app.model(|m| m.closing.is_none()) && app.exit_reason().is_none());
    // Return is Save.
    app.close_requested(yantrik_app::app::WindowId::PRIMARY);
    app.ui(|ui| tap(ui, &SharedString::from(Key::Return)));
    settle(&app);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Kept for later\n\nand more");
    assert_eq!(app.exit_reason(), Some(yantrik_app::app::ExitReason::LastWindowClosed));
}

/// One note in two windows, drawn (gate row "Two document contexts"): Ctrl+Shift+N opens the note
/// in a second window, what is typed in that window's editor is drawn in the first, an outside
/// change shows the same bar in both, and the window manager handing the first window the keys
/// back makes it the one the actions act on.
#[test]
fn one_note_in_two_windows_is_drawn_in_both_and_the_window_manager_moves_the_focus() {
    let (app, path) = super::with_note("Two windows");
    let size = slint::PhysicalSize::new(900, 640);
    ui(&app, move |ui| {
        ui.show().unwrap();
        ui.window().set_size(size);
    });
    app.ui(|ui| held(ui, Key::Control, |ui| held(ui, Key::Shift, |ui| tap(ui, "N"))));
    settle(&app);
    let windows = app.windows();
    assert_eq!(windows.len(), 2, "Ctrl+Shift+N opened the note in a second window");
    let second = windows[1];
    assert_eq!(app.window(second, |ui| ui.get_note_title().to_string()), Some("Two windows".into()));
    app.window(second, move |ui| ui.window().set_size(size));
    let typed = "# Two windows\n\nTyped in the second window, drawn in the first.";
    app.window(second, move |ui| {
        ui.set_content(typed.into());
        ui.invoke_edited(typed.into());
    });
    assert_eq!(ui(&app, |ui| ui.get_content().to_string()), typed, "the first window draws it");
    assert!(ui(&app, |ui| ui.get_modified()), "and calls it unsaved too");
    screenshot_two(&app, second, "notes-two-windows.png");
    std::fs::write(&path, "# Two windows\n\nWritten by another program.\n").unwrap();
    app.look_at_files();
    let bars = (ui(&app, |ui| ui.get_outside()), app.window(second, |ui| ui.get_outside()));
    assert_eq!(bars, (1, Some(1)), "one conflict, shown in both windows");
    screenshot_two(&app, second, "notes-two-windows-conflict.png");
    // The second window is focused (it was opened last). The window manager makes the first the
    // active one again; its keys take the focus back, and the actions go to it.
    assert_eq!(super::state(&app)["focused_window"], second.0);
    ui(&app, |ui| {
        ui.invoke_focus_shortcuts();
        ui.window().dispatch_event(WindowEvent::WindowActiveChanged(false));
        ui.window().dispatch_event(WindowEvent::WindowActiveChanged(true));
    });
    assert_eq!(super::state(&app)["focused_window"], 0, "the first window is the focused one");
}
