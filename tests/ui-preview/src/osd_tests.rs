//! The on-screen display, drawn in its own window (osd_window.slint's `OsdWindow`, which labwc
//! keeps above app windows) for the volume and the brightness keys (story 2.1), then left alone
//! to hide.
//!
//! Checks what a person would see: a 240x64 charcoal pill (the whole window), with
//! the bar filled to the level it was given; a second key press inside the hold keeps it up (one
//! timer, restarted); and once it has hidden the window asks for no redraws at all, which is the
//! proof that nothing is left running behind it. Renders are `osd-volume.png`, `osd-brightness.png`,
//! `osd-muted.png`, `osd-caps-lock.png` and `osd-hidden.png`, next to the output path given.
use super::*;
use std::time::{Duration, Instant};

fn save(pixels: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    println!("Rendered {path} ({width}×{height})");
    Ok(())
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (240u32, 64u32);
    let ui = OsdWindow::new()?;
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    let draw = || {
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
        slint::platform::update_timers_and_animations();
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); });
        pixels
    };
    // Let a fade finish: a few frames spread over more than the 120ms it runs for.
    let settle = || {
        let mut last = draw();
        for _ in 0..4 {
            std::thread::sleep(Duration::from_millis(60));
            last = draw();
        }
        last
    };
    let dir = std::path::Path::new(output);
    let named = |name: &str| dir.with_file_name(name).to_string_lossy().into_owned();
    let at = |p: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, x: usize, y: usize| p.as_slice()[y * width as usize + x];

    // The pill is the whole window.
    let (pill_x, pill_y) = (0usize, 0usize);
    let charcoal = slint::Rgb8Pixel { r: 0x15, g: 0x1a, b: 0x1e };

    let before = settle();

    ui.invoke_show_osd("volume".into(), true, 45, "45%".into(), false);
    let volume = settle();
    save(&volume, &named("osd-volume.png"), width, height)?;
    assert_eq!(at(&volume, pill_x + 120, pill_y + 4), charcoal, "an opaque charcoal pill");
    assert_ne!(at(&before, pill_x + 120, pill_y + 4), charcoal, "and nothing there before it");

    // The bar's fill ends where the level says: bright to the left of the 45% mark, track after.
    // The track runs between the icon and the number; find its fill's right edge on the row.
    let bar_y = pill_y + 32;
    let bright = |p: slint::Rgb8Pixel| p.r > 0xc0 && p.g > 0xc0 && p.b > 0xc0;
    let row_fill = |p: &slint::SharedPixelBuffer<slint::Rgb8Pixel>| -> usize {
        (pill_x + 50..pill_x + 190).filter(|x| bright(at(p, *x, bar_y))).count()
    };
    let at_45 = row_fill(&volume);
    assert!(at_45 > 20, "the bar is filled for 45% ({at_45}px of fill)");
    // A 4px track (Theme.osd-track-h), centred: rows 30 to 33 of the 64px pill, nothing above or below.
    let column: Vec<bool> = (28..36).map(|y| bright(at(&volume, pill_x + 60, pill_y + y))).collect();
    assert_eq!(column, [false, false, true, true, true, true, false, false], "the level bar is 4px tall, centred");
    // And the fill grows from the left end of the track, not out from its middle.
    assert!(bright(at(&volume, pill_x + 52, bar_y)), "45% is filled from the track's left end");

    // A second key press inside the hold: brightness replaces the volume and the hold restarts.
    std::thread::sleep(Duration::from_millis(700));
    ui.invoke_show_osd("brightness".into(), true, 80, "80%".into(), false);
    let brightness = settle();
    save(&brightness, &named("osd-brightness.png"), width, height)?;
    let at_80 = row_fill(&brightness);
    assert!(at_80 > at_45, "80% fills more of the bar than 45% did ({at_80} vs {at_45})");
    std::thread::sleep(Duration::from_millis(600));
    let still = draw();
    assert_eq!(at(&still, pill_x + 120, pill_y + 4), charcoal, "1.6s after the first press it is still up: the second one restarted the hold");

    // A muted speaker and Caps Lock, drawn once each for the eye.
    ui.invoke_show_osd("volume-off".into(), true, 45, "Muted".into(), true);
    save(&settle(), &named("osd-muted.png"), width, height)?;
    ui.invoke_show_osd("caps-lock".into(), false, 0, "Caps Lock on".into(), false);
    save(&settle(), &named("osd-caps-lock.png"), width, height)?;

    // Now leave it alone. It must hide itself, and the window must then ask for no redraws.
    let start = Instant::now();
    let mut gone = false;
    while start.elapsed() < Duration::from_millis(3000) {
        std::thread::sleep(Duration::from_millis(100));
        let p = draw();
        if at(&p, pill_x + 120, pill_y + 4) != charcoal {
            gone = true;
            break;
        }
    }
    assert!(gone, "the pill hides itself after the hold");
    assert!(start.elapsed() > Duration::from_millis(900), "and not before the hold ({:?})", start.elapsed());
    let hidden = settle();
    save(&hidden, &named("osd-hidden.png"), width, height)?;
    assert_eq!(hidden.as_slice(), before.as_slice(), "hidden is drawn as if it had never been shown");

    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
    let mut redraws = 0;
    for _ in 0..15 {
        slint::platform::update_timers_and_animations();
        if w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); }) {
            redraws += 1;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(redraws, 0, "a hidden OSD must not keep the window drawing");

    ui.hide()?;
    println!("PASS: the OSD draws for volume and brightness, a second press restarts the hold, it hides after it, and then {redraws} redraws over 1.5 seconds");
    Ok(())
}
