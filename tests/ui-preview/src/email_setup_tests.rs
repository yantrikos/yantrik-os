//! The Mail app's Add Account form (#1052), at the two laptop sizes a first run lands on: 1280x720
//! and 1366x768. The form grows as it is filled in (the advanced servers, a hint that wraps) and
//! must never put Sign In out of reach: at each size the button is inside the window or the wheel
//! brings it inside, and a field reached by Tab ends inside the window. Found the way a person
//! finds the button, by its colour: a block of the accent fill as wide as the button and as tall.

use super::*;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};
use std::{fs::File, io::BufWriter};

/// The Sign In button is 44px tall and far wider than any other accent-filled thing on the form.
const BUTTON_H: usize = 44;
const BUTTON_MIN_W: usize = 150;

fn render(w: &MinimalSoftwareWindow, width: u32, height: u32) -> slint::SharedPixelBuffer<slint::Rgb8Pixel> {
    let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(p.make_mut_slice(), width as usize); });
    }
    p
}

/// The rows, top to bottom, that hold a run of the accent fill at least as wide as the button.
fn accent_rows(p: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, width: usize, height: usize, accent: (u8, u8, u8)) -> Vec<usize> {
    let px = p.as_slice();
    (0..height)
        .filter(|&y| {
            let (mut run, mut best) = (0usize, 0usize);
            for x in 0..width {
                let c = px[y * width + x];
                if (c.r, c.g, c.b) == accent { run += 1; best = best.max(run); } else { run = 0; }
            }
            best >= BUTTON_MIN_W
        })
        .collect()
}

/// True when the rows hold the whole button: a block of `BUTTON_H` rows on top of one another,
/// none of them the window's last row (a button cut by the edge has its rows run out there).
fn button_whole(rows: &[usize], height: usize) -> bool {
    rows.len() >= BUTTON_H - 2 && rows.windows(2).all(|r| r[1] == r[0] + 1) && rows.last().map_or(false, |&y| y + 1 < height)
}

fn wheel(w: &MinimalSoftwareWindow, x: f32, y: f32, dy: f32) {
    let at = LogicalPosition::new(x, y);
    w.dispatch_event(WindowEvent::PointerMoved { position: at });
    w.dispatch_event(WindowEvent::PointerScrolled { position: at, delta_x: 0.0, delta_y: dy });
    slint::platform::update_timers_and_animations();
}

fn key(w: &MinimalSoftwareWindow, text: slint::SharedString) {
    w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.dispatch_event(WindowEvent::KeyReleased { text });
}

fn save(p: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, output: &str, name: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let path = output.replace(".png", &format!("-{name}.png"));
    let mut e = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(p.as_bytes())?;
    Ok(())
}

pub fn run(window: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    for (width, height) in [(1280u32, 720u32), (1366, 768)] {
        for provider in ["advanced", "gmail"] {
            let ui = EmailSetupProbe::new()?;
            ui.set_provider(provider.into());
            ui.show()?;
            window.set_size(slint::PhysicalSize::new(width, height));
            let a = ui.get_accent();
            let accent = (a.red(), a.green(), a.blue());
            let (w, h) = (width as usize, height as usize);

            // Sign In: inside the window now, or brought inside by the wheel.
            let mut p = render(window, width, height);
            let mut scrolled = 0;
            while !button_whole(&accent_rows(&p, w, h, accent), h) && scrolled < 40 {
                wheel(window, 12.0, height as f32 / 2.0, -120.0);
                scrolled += 1;
                p = render(window, width, height);
            }
            assert!(
                button_whole(&accent_rows(&p, w, h, accent), h),
                "{width}x{height} ({provider}): Sign In is not inside the window, and {scrolled} wheel steps did not bring it in"
            );
            save(&p, output, &format!("{width}x{height}-{provider}"), width, height)?;

            // Tab through the form: every field that takes the keyboard ends inside the window.
            let mut seen = 0;
            for _ in 0..40 {
                key(window, slint::SharedString::from(Key::Tab));
                let _ = render(window, width, height);
                let bottom = ui.get_focused_bottom();
                if bottom > 0.0 {
                    seen += 1;
                    assert!(bottom <= height as f32, "{width}x{height} ({provider}): a field reached by Tab ends at y={bottom}, below the window's {height}");
                }
            }
            assert!(seen >= 3, "{width}x{height} ({provider}): Tab reached only {seen} fields");
            ui.hide()?;
            println!("PASS email Add Account at {width}x{height} ({provider}): Sign In reachable ({scrolled} wheel steps), {seen} Tab stops inside the window");
        }
    }
    Ok(())
}
