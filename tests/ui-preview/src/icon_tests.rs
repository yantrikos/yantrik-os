//! Icon crispness probe (icon_probe.slint): renders rows of app tiles at today's sizes and at
//! whole-pixel sizes, for a person to compare.
use super::*;

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    const W: u32 = 720;
    const H: u32 = 400;
    let ui = IconProbe::new()?;
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(W, H));
    let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(W, H);
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(p.make_mut_slice(), W as usize); });
    }
    let mut e = png::Encoder::new(BufWriter::new(File::create(output)?), W, H);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(p.as_bytes())?;
    println!("Rendered {output}");
    Ok(())
}
