//! The "give a harness a provider" card, on Settings → Harnesses where it is opened. It lived
//! inside the AI page's block, so on VM 520 pressing a provider on the Harnesses page drew
//! nothing at all and the person had no way to Apply.

use super::*;

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = HandoffCardProbe::new()?;
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(1280, 800));
    let draw = || {
        let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(1280, 800);
        for _ in 0..3 {
            slint::platform::update_timers_and_animations();
            w.request_redraw();
            w.draw_if_needed(|r| { r.render(p.make_mut_slice(), 1280); });
        }
        p
    };
    ui.set_open(false);
    let closed = draw();
    ui.set_open(true);
    let open = draw();
    // The card is 420px wide in the middle of the page: that region must change when it opens.
    let (x0, x1, y0, y1) = (640 - 150, 640 + 150, 360, 440);
    let changed = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| y * 1280 + x))
        .filter(|&i| closed.as_slice()[i] != open.as_slice()[i])
        .count();
    let f = BufWriter::new(File::create(output)?);
    let mut e = png::Encoder::new(f, 1280, 800);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(open.as_bytes())?;
    assert!(changed > 20_000, "the card did not draw on the Harnesses page ({changed} pixels changed)");
    println!("PASS provider card: drawn on the Harnesses page it is opened from ({changed} pixels)");
    Ok(())
}
