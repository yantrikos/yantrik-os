//! App label probe (label_probe.slint): AppLabel on the lake wallpaper and on a glass card, for a
//! person to judge legibility, and two checks a picture can carry: a label on the wallpaper is
//! drawn in white, and its shadow darkens the wallpaper under it.
use super::*;

const W: u32 = 1280;
const H: u32 = 800;
/// The rows' box tops in label_probe.slint; each row's names span x 40 to 768.
const ROWS: [u32; 5] = [120, 360, 430, 560, 700];

fn luma(p: &slint::Rgb8Pixel) -> f32 {
    0.2126 * p.r as f32 + 0.7152 * p.g as f32 + 0.0722 * p.b as f32
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = LabelProbe::new()?;
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(W, H));
    let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(W, H);
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(p.make_mut_slice(), W as usize); });
    }
    let px = |x: u32, y: u32| &p.as_slice()[(y * W + x) as usize];

    // The brightest stretch of wallpaper a row sits on, measured left of the names, where nothing
    // is drawn: that is where a white label most needs its shadow.
    let ground = |top: u32| -> f32 {
        let (mut sum, mut n) = (0.0, 0);
        for y in top..top + 18 {
            for x in 0..38 {
                sum += luma(px(x, y));
                n += 1;
            }
        }
        sum / n as f32
    };
    let top = *ROWS.iter().max_by(|a, b| ground(**a).total_cmp(&ground(**b))).unwrap();
    let bright = ground(top);
    // Across the row's seven names: ink near white, and pixels darker than the unpainted
    // wallpaper 3px above the same column, which only the shadow can make.
    let (mut white, mut shaded) = (0, 0);
    for x in 40..768 {
        let above = luma(px(x, top - 3));
        for y in top..top + 18 {
            let l = luma(px(x, y));
            if l >= 230.0 { white += 1; }
            if l <= above - 20.0 { shaded += 1; }
        }
    }
    let mut e = png::Encoder::new(BufWriter::new(File::create(output)?), W, H);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(p.as_bytes())?;
    assert!(white > 100, "a label on the wallpaper is white: {white} near-white pixels over ground {bright:.0}");
    assert!(shaded > 100, "its shadow darkens the wallpaper under it: {shaded} pixels 20 below the wallpaper above");
    println!("PASS: an app label on the lake wallpaper is white ({white} px) over its shadow ({shaded} px darker than the wallpaper above) on the brightest row (ground {bright:.0}); rendered {output}");
    Ok(())
}
