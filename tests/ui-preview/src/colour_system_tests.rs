//! The shell's colour roles, read off real pixels (ColourSystemProbe in colour_system_probe.slint):
//! a primary button is the neutral primary fill with a dark label, a secondary is a plate (a lit
//! fill with an edge and a lit top pixel), a destructive is the one red, an off tile is the opaque
//! tile fill with its own border and an on tile is the accent. Also that the approval card's two buttons cannot be pressed from the
//! keyboard: Tab never lands on them and Enter does nothing, however the card was opened.
use super::*;
use slint::platform::Key;

const W: u32 = 720;
const H: u32 = 360;

fn draw(w: &MinimalSoftwareWindow) -> slint::SharedPixelBuffer<slint::Rgb8Pixel> {
    let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(W, H);
    for _ in 0..4 {
        slint::platform::update_timers_and_animations();
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(p.make_mut_slice(), W as usize); });
    }
    p
}

fn px(p: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, x: u32, y: u32) -> (u8, u8, u8) {
    let c = p.as_slice()[(y * W + x) as usize];
    (c.r, c.g, c.b)
}

/// The darkest pixel in a rectangle: on an accent fill, the core of its label's strokes. A white
/// label would leave the fill itself as the darkest thing there.
fn darkest(p: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, x: u32, y: u32, w: u32, h: u32) -> (u8, u8, u8) {
    let luma = |c: (u8, u8, u8)| c.0 as u32 * 2126 + c.1 as u32 * 7152 + c.2 as u32 * 722;
    (y..y + h).flat_map(|y| (x..x + w).map(move |x| (x, y))).map(|(x, y)| px(p, x, y)).min_by_key(|&c| luma(c)).unwrap()
}

/// WCAG 2 contrast of two drawn colours.
fn contrast(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    let lin = |v: u8| { let v = v as f64 / 255.0; if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) } };
    let lum = |c: (u8, u8, u8)| 0.2126 * lin(c.0) + 0.7152 * lin(c.1) + 0.0722 * lin(c.2);
    let (a, b) = (lum(a), lum(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn near(got: (u8, u8, u8), want: (u8, u8, u8), what: &str) {
    let d = |a: u8, b: u8| (a as i32 - b as i32).abs();
    assert!(d(got.0, want.0) <= 3 && d(got.1, want.1) <= 3 && d(got.2, want.2) <= 3, "{what}: drew {got:?}, wanted {want:?}");
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = ColourSystemProbe::new()?;
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(W, H));
    let p = draw(w);
    {
        let mut e = png::Encoder::new(BufWriter::new(File::create(output)?), W, H);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()?.write_image_data(p.as_bytes())?;
    }
    // Panel origin is (20, 20); buttons are 32px tall, so a point 4px in from the left edge and
    // level with the middle is clear of the label.
    near(px(&p, 20 + 20 + 6, 20 + 20 + 16), (0xE4, 0xEB, 0xF4), "primary button fill is the neutral primary-fill, not the accent");
    near(px(&p, 20 + 360 + 6, 20 + 20 + 16), (0xE5, 0x48, 0x4D), "destructive button fill is red");
    // The panel is bg-card (#172333). A secondary button is a plate on it: control-fill (11% white)
    // over the card, control-edge (14% white) over that along the top, and the lit pixel under it.
    let panel = px(&p, 20 + 600, 20 + 60);
    let plate = px(&p, 20 + 190 + 6, 20 + 20 + 16);
    near(plate, (0x30, 0x3B, 0x49), "secondary button is a plate: control-fill over the card");
    assert_ne!(plate, panel, "the plate does not vanish into the panel");
    near(px(&p, 20 + 190 + 40, 20 + 20), (0x4D, 0x56, 0x62), "the plate's top edge is control-edge over its fill");
    near(px(&p, 20 + 190 + 40, 20 + 20 + 1), (0x51, 0x5A, 0x66), "and the pixel under the edge is lit");
    near(px(&p, 20 + 190 + 40, 20 + 20 + 2), (0x30, 0x3B, 0x49), "one pixel of light, then the fill");
    near(px(&p, 20 + 20 + 6, 20 + 140 + 28), (0x1C, 0x29, 0x3B), "an off tile is the tile fill");
    near(px(&p, 20 + 240 + 6, 20 + 140 + 28), (0x74, 0xAA, 0xF5), "an on tile is the accent");
    assert_ne!(px(&p, 20 + 20 + 6, 20 + 140 + 28), panel, "an off tile does not vanish into the panel");

    // The ink on a light fill is dark: on the accent it is the theme's deepest ground, not white
    // (2.4:1 on this blue), and on the neutral primary its own dark ink. The primary's label, the
    // approval card's primary, and the on tile's title. Each box is inset past the rounded
    // corners, where the darker panel shows through.
    let (primary, accent) = ((0xE4, 0xEB, 0xF4), (0x74, 0xAA, 0xF5));
    for (x, y, w, h, fill, what) in [(52, 46, 126, 20, primary, "Save"), (222, 102, 126, 20, primary, "Allow once"), (272, 170, 150, 36, accent, "the on tile")] {
        let ink = darkest(&p, x, y, w, h);
        let ratio = contrast(ink, fill);
        assert!(ratio >= 4.5, "{what}'s label on its fill is {ratio:.2}:1 (darkest {ink:?})");
    }

    // Pointer: each kind answers a click.
    click(w, 20.0 + 20.0 + 75.0, 20.0 + 20.0 + 16.0);
    click(w, 20.0 + 190.0 + 75.0, 20.0 + 20.0 + 16.0);
    click(w, 20.0 + 360.0 + 75.0, 20.0 + 20.0 + 16.0);
    assert_eq!((ui.get_primary_clicks(), ui.get_secondary_clicks(), ui.get_destructive_clicks()), (1, 1, 1));

    // The approval pair: Tab then Enter and Space press nothing, and a click still does.
    let before = ui.get_pointer_only_clicks();
    for _ in 0..4 {
        key(w, slint::SharedString::from(Key::Tab));
        key(w, "\n".into());
        key(w, " ".into());
    }
    draw(w);
    assert_eq!(ui.get_pointer_only_clicks(), before, "Enter and Space never press a pointer-only button");
    click(w, 20.0 + 190.0 + 75.0, 20.0 + 76.0 + 16.0);
    assert_eq!(ui.get_pointer_only_clicks(), before + 1, "a click does");
    println!("PASS colour system: primary, secondary, destructive, on/off tiles; approval pair is pointer-only");
    Ok(())
}
