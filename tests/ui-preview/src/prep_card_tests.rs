//! The cards for promised things (commitment_prep.slint). On VM 520 the button row hung 12px out of
//! the bottom of the card, the next card covered it, and at 2x the second card's buttons were under
//! the dock. A card is as tall as its content, and the column of cards stops where it is given.

use super::*;
use slint::{ComponentHandle, ModelRc, VecModel};

const GROUND: (u8, u8, u8) = (0x20, 0x40, 0x60);

fn card(title: &str, text: &str) -> PrepCardData {
    PrepCardData {
        item: "commitment-prep-1".into(),
        title: title.into(),
        text: text.into(),
        yes_label: "Got it".into(),
        yes_id: "commitment_prep:seen:commitment-prep-1".into(),
        no_label: "Stop getting things ready".into(),
        no_id: "commitment_prep:off".into(),
    }
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = PrepCardProbe::new()?;
    ui.show()?;
    let (width, height) = (400usize, 700usize);
    w.set_size(slint::PhysicalSize::new(width as u32, height as u32));
    let text = "For your promise to Maya on Friday: no files opened. The draft is in Email with the gate8 alpha report attached, but Email can't send attachments yet, so it can't be sent from there. Nothing sent.";
    ui.set_data(card("Got something ready", text));
    let three: Vec<PrepCardData> = (0..3).map(|_| card("Got something ready", text)).collect();
    ui.global::<PrepState>().set_cards(ModelRc::new(VecModel::from(three)));

    let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width as u32, height as u32);
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(p.make_mut_slice(), width); });
    }
    let px = |x: usize, y: usize| { let c = p.as_slice()[y * width + x]; (c.r, c.g, c.b) };
    // Anything drawn in the rows `from..to` between x 24 and 316, other than the ground.
    let drawn = |from: usize, to: usize| (from..to).flat_map(|y| (24..316).map(move |x| (x, y))).filter(|&(x, y)| px(x, y) != GROUND).count();

    // One card: nothing under its bottom edge (a pixel-wide border at card-height - 1).
    let card_bottom = 20 + ui.get_card_height() as usize;
    let under = drawn(card_bottom + 1, card_bottom + 24);
    assert_eq!(under, 0, "the card's button row hangs out of it: {under} pixels below its bottom edge at y={card_bottom}");
    assert!(drawn(card_bottom - 14, card_bottom - 2) > 0, "the buttons are inside the card, just above its bottom edge");

    // The column: it is 200px tall at y=300, shows something, and draws nothing past y=500.
    assert!(drawn(300, 500) > 0, "the column draws its cards");
    let past = drawn(501, 600);
    assert_eq!(past, 0, "the column draws {past} pixels below the room it was given");

    let f = BufWriter::new(File::create(output)?);
    let mut e = png::Encoder::new(f, width as u32, height as u32);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(p.as_bytes())?;
    println!("PASS prep cards: the button row is inside its card ({card_bottom}px), and the column stops where it is given");
    Ok(())
}
