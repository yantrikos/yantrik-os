//! The capture bar (Print), drawn by the production component, and the status bar's red dot.
//!
//! In each theme, at 1280x800 and at a narrow 700x800 (where the shell stacks it in two rows),
//! in Screenshot and Record: the bar fits the screen with room either side and is opaque (the
//! probe's magenta backdrop never shows inside it). Then the keyboard: Esc closes it from the
//! first focus, Tab reaches the choices and Space takes one, Enter on the button captures. Then
//! the red dot: drawn while a recording runs, gone when it stops. Renders are
//! `capture-bar-<theme>-<width>-<mode>.png` and `capture-dot.png`, next to the output path given.
use super::*;
use slint::platform::{Key, WindowEvent};
use std::time::Duration;

fn save(pixels: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    println!("Rendered {path} ({width}x{height})");
    Ok(())
}

fn key(w: &MinimalSoftwareWindow, k: impl Into<slint::SharedString> + Clone) {
    w.dispatch_event(WindowEvent::KeyPressed { text: k.clone().into() });
    w.dispatch_event(WindowEvent::KeyReleased { text: k.into() });
}

const MAGENTA: slint::Rgb8Pixel = slint::Rgb8Pixel { r: 0xff, g: 0x00, b: 0xff };

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::path::Path::new(output);
    let named = |name: &str| dir.with_file_name(name).to_string_lossy().into_owned();
    let ui = CaptureBarProbe::new()?;
    ui.show()?;

    let draw = |width: u32, height: u32| {
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
        for _ in 0..3 {
            slint::platform::update_timers_and_animations();
            w.request_redraw();
            w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); });
            std::thread::sleep(Duration::from_millis(20));
        }
        pixels
    };

    let mut one_row_height = 0.0f32;
    for light in [false, true] {
        for (width, height) in [(1280u32, 800u32), (700, 800)] {
            for kind in [0, 1] {
                w.set_size(slint::PhysicalSize::new(width, height));
                ui.set_light(light);
                ui.set_compact(width < 980);
                ui.set_kind(kind);
                let pixels = draw(width, height);
                let theme = if light { "light" } else { "dark" };
                let mode = if kind == 0 { "screenshot" } else { "record" };
                save(&pixels, &named(&format!("capture-bar-{theme}-{width}-{mode}.png")), width, height)?;

                let (x, y, bw, bh) = (ui.get_bar_x(), ui.get_bar_y(), ui.get_bar_w(), ui.get_bar_h());
                assert!(bw > 200.0 && bh > 30.0, "{theme} {width} {mode}: the bar has a size ({bw}x{bh})");
                assert!(x >= 16.0 && x + bw <= width as f32 - 16.0, "{theme} {width} {mode}: the bar fits with a 16px gutter (x {x}, w {bw})");
                if width == 1280 && kind == 0 {
                    one_row_height = bh;
                }
                if width == 700 {
                    assert!(bh > one_row_height * 1.5, "narrow: two rows ({bh} vs one row's {one_row_height})");
                }
                // Opaque: no backdrop inside the bar, away from the rounded corners.
                let at = |px: f32, py: f32| pixels.as_slice()[py as usize * width as usize + px as usize];
                let inset = 14.0;
                let mut magenta = 0;
                let mut seen = 0;
                let mut py = y + inset;
                while py < y + bh - inset {
                    let mut px = x + inset;
                    while px < x + bw - inset {
                        seen += 1;
                        if at(px, py) == MAGENTA {
                            magenta += 1;
                        }
                        px += 7.0;
                    }
                    py += 5.0;
                }
                assert!(seen > 100, "sampled the bar ({seen})");
                assert_eq!(magenta, 0, "{theme} {width} {mode}: the backdrop shows through the bar at {magenta} of {seen} points");
            }
        }
    }

    // The keyboard, at 1280x800 in the dark theme.
    w.set_size(slint::PhysicalSize::new(1280, 800));
    ui.set_light(false);
    ui.set_compact(false);
    ui.set_kind(0);
    draw(1280, 800);
    ui.invoke_focus_bar();
    key(w, Key::Escape);
    assert_eq!(ui.get_closes(), 1, "Esc closes the bar from where the keyboard starts");

    // Tab to the first choice (Screenshot), Tab again to Record, Space takes it.
    ui.invoke_focus_bar();
    key(w, Key::Tab);
    key(w, Key::Tab);
    key(w, " ");
    draw(1280, 800);
    assert_eq!(ui.get_kind(), 1, "Tab reaches Record and Space chooses it");
    // Esc still closes from inside a choice: it bubbles up to the bar.
    key(w, Key::Escape);
    assert_eq!(ui.get_closes(), 2, "Esc closes the bar from a focused choice too");

    // Every control is reachable: Tab through the whole bar lands on the button, and Enter on it
    // starts the recording.
    ui.set_kind(0);
    draw(1280, 800);
    ui.invoke_focus_bar();
    let mut pressed = false;
    for _ in 0..20 {
        key(w, Key::Tab);
        key(w, Key::Return);
        draw(1280, 800);
        if ui.get_goes() > 0 {
            pressed = true;
            break;
        }
    }
    assert!(pressed, "the Capture button is reachable with Tab and pressed with Enter");

    // The red dot: drawn while a recording runs, gone when it stops.
    let before = draw(1280, 800);
    ui.global::<CaptureState>().set_elapsed("0:42".into());
    ui.global::<CaptureState>().set_recording(true);
    let recording = draw(1280, 800);
    save(&recording, &named("capture-dot.png"), 1280, 800)?;
    let bar_rows = |p: &slint::SharedPixelBuffer<slint::Rgb8Pixel>| -> Vec<slint::Rgb8Pixel> {
        (0..36usize).flat_map(|y| p.as_slice()[y * 1280..y * 1280 + 1280].to_vec()).collect()
    };
    let reddish = bar_rows(&recording).iter().filter(|p| p.r > 0xb0 && p.g < 0x90 && p.b < 0x90).count();
    let reddish_before = bar_rows(&before).iter().filter(|p| p.r > 0xb0 && p.g < 0x90 && p.b < 0x90).count();
    assert!(reddish > reddish_before + 20, "the red dot is on the status bar while recording ({reddish} vs {reddish_before})");
    ui.global::<CaptureState>().set_recording(false);
    let after = draw(1280, 800);
    assert_eq!(bar_rows(&after), bar_rows(&before), "and gone when the recording stops");

    ui.hide()?;
    println!("PASS: the capture bar fits and is opaque in both themes at 1280 and 700 wide, Esc closes it, Tab and Space choose, Enter captures, and the red dot shows only while recording");
    Ok(())
}
