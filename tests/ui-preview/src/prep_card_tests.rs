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
        file1_label: "".into(),
        file2_label: "".into(),
        more_files: "".into(),
        draft_label: "".into(),
    }
}

/// A card with every opening: two files (one with a long name, elided as the shell elides it), "and
/// 3 more", and the draft.
fn full_card(text: &str) -> PrepCardData {
    PrepCardData {
        file1_label: "Open Q4-deck\u{2026}final-v3.ydeck".into(),
        file2_label: "Open budget-2026.csv".into(),
        more_files: "and 3 more".into(),
        draft_label: "Open the draft".into(),
        ..card("Got something ready", text)
    }
}

const LEFT: usize = 20;
const TOP: usize = 20;
const CARD_W: usize = 300;
const COLUMN_Y: usize = 400;
const COLUMN_H: usize = 200;

/// Draw the probe at `scale` (1 or 2) and check it; the image is written to `output`.
fn check(w: &MinimalSoftwareWindow, ui: &PrepCardProbe, scale: usize, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (400 * scale, 700 * scale);
    ui.window().dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: scale as f32 });
    w.set_size(slint::PhysicalSize::new(width as u32, height as u32));
    let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width as u32, height as u32);
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(p.make_mut_slice(), width); });
    }
    let px = |x: usize, y: usize| { let c = p.as_slice()[y * width + x]; (c.r, c.g, c.b) };
    // Anything drawn in the logical rectangle x0..x1 by y0..y1 other than the ground.
    let drawn = |x0: usize, x1: usize, y0: usize, y1: usize| {
        (y0 * scale..y1 * scale).flat_map(|y| (x0 * scale..x1 * scale).map(move |x| (x, y))).filter(|&(x, y)| px(x, y) != GROUND).count()
    };

    // The card is as tall as its content: its last button row ends inside it, by its padding.
    let card_h = ui.get_card_height().round() as usize;
    let buttons_bottom = ui.get_buttons_bottom().round() as usize;
    assert!(buttons_bottom < card_h, "{scale}x: the button row ends at {buttons_bottom}, the card at {card_h}");
    // Nothing is drawn below the card's bottom edge, nor to either side of it, down to the column:
    // every open button, Got it and Stop stay inside the card.
    let bottom = TOP + card_h;
    let below = drawn(0, 400, bottom + 1, COLUMN_Y - 1);
    assert_eq!(below, 0, "{scale}x: {below} pixels below the card's bottom edge at y={bottom}");
    let side = drawn(0, LEFT - 1, TOP, bottom) + drawn(LEFT + CARD_W + 1, 400, TOP, bottom);
    assert_eq!(side, 0, "{scale}x: {side} pixels beside the card");
    assert!(drawn(LEFT + 8, LEFT + CARD_W - 8, bottom - 14, bottom - 2) > 0, "{scale}x: the last buttons are inside the card, just above its bottom edge");
    // The full card is taller than a plain one by the open rows (a file per row, "and N more", draft).
    assert!(card_h > 200, "{scale}x: the card with two files and a draft is {card_h}px tall, so the rows are laid out");

    // The column: it is 200px tall at y=400, shows something, and draws nothing past its room.
    assert!(drawn(LEFT, LEFT + CARD_W, COLUMN_Y, COLUMN_Y + COLUMN_H) > 0, "{scale}x: the column draws its cards");
    let past = drawn(0, 400, COLUMN_Y + COLUMN_H + 1, 700);
    assert_eq!(past, 0, "{scale}x: the column draws {past} pixels below the room it was given");

    let f = BufWriter::new(File::create(output)?);
    let mut e = png::Encoder::new(f, width as u32, height as u32);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(p.as_bytes())?;
    println!("PASS prep cards at {scale}x: every button is inside its card ({card_h}px), and the column stops where it is given");
    Ok(())
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = PrepCardProbe::new()?;
    ui.show()?;
    let text = "For your promise to Maya on Friday: 2 files found. The draft is in Email with the gate8 alpha report attached, but Email can't send attachments yet, so it can't be sent from there. Nothing sent or opened for you.";
    ui.set_data(full_card(text));
    let three: Vec<PrepCardData> = (0..3).map(|i| if i == 1 { card("Got something ready", text) } else { full_card(text) }).collect();
    ui.global::<PrepState>().set_cards(ModelRc::new(VecModel::from(three)));
    check(w, &ui, 1, output)?;
    check(w, &ui, 2, &output.replace(".png", "-2x.png"))
}
