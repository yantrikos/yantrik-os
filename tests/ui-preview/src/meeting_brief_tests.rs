//! The meeting brief card (components/meeting_brief_card.slint), drawn from fixture data with a
//! long title, long rows and all five sections, at text scale 1.0 and 1.5. A card is as tall as
//! its content; nothing a row says may run out of it (it is cut with an ellipsis, not clipped or
//! overflowing); the stack the desktop draws stops where it is given; and no key opens anything:
//! the Open buttons are pointer-only, a click on each one still opens.

use super::*;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, VecModel};
use std::cell::Cell;
use std::rc::Rc;

const GROUND: (u8, u8, u8) = (0x20, 0x40, 0x60);
const LEFT: usize = 20;
const TOP: usize = 20;
const CARD_W: usize = 340;
const W: usize = 420;
const H: usize = 900;
const STACK_Y: usize = 640;
const STACK_H: usize = 240;
/// The card's padding, `Theme.sp-3`.
const PAD: usize = 12;

fn line(heading: &str, text: &str, row: i32) -> BriefLineData {
    BriefLineData { heading: heading.into(), text: text.into(), row }
}

fn long_card(id: &str, title: &str) -> MeetingBriefData {
    let lines = vec![
        line("LAST EMAILS WITH THEM", "Maya \u{b7} Slides for Friday, the final version with the regional numbers and every appendix \u{b7} 2 d", 0),
        line("", "You \u{b7} Re: Slides for Friday \u{b7} 3 d", 1),
        line("", "Ravi \u{b7} Budget \u{b7} 10 d", 2),
        line("YOUR NOTES", "Orion launch checklist, owners, dates, risks and the open questions for the steering group", 3),
        line("", "Call with Maya Chen", 4),
        line("PROMISES", "You told Ravi: send the Q4 numbers to the whole finance team before the end of the quarter", -1),
        line("", "Maya said: share the deck", 5),
        line("FILES", "orion-launch-plan-final-v3-reviewed-by-legal.pdf \u{b7} Documents \u{b7} 3 d", 6),
        line("ABOUT", "Agenda: the launch date, the owners, and what blocks us.", -1),
        line("", "Bring the Q4 numbers.", -1),
    ];
    MeetingBriefData {
        id: id.into(),
        title: title.into(),
        when: "10:00\u{2013}10:30, in 9 min".into(),
        place: "Building 4, 12th floor, the big room with the long name and the window".into(),
        people: "With Maya Chen, Ravi Rao, Priyanka Venkataraghavan, Jean-Baptiste Montgolfier".into(),
        lines: ModelRc::new(VecModel::from(lines)),
        note: "".into(),
    }
}

const LONG_TITLE: &str = "Quarterly planning and roadmap alignment for the Orion launch with every team lead across all three regions";

fn render(w: &MinimalSoftwareWindow) -> slint::SharedPixelBuffer<slint::Rgb8Pixel> {
    let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(W as u32, H as u32);
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(p.make_mut_slice(), W); });
    }
    p
}

fn key(w: &MinimalSoftwareWindow, text: slint::SharedString) {
    w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.dispatch_event(WindowEvent::KeyReleased { text });
}

fn click(w: &MinimalSoftwareWindow, x: usize, y: usize) {
    let at = LogicalPosition::new(x as f32, y as f32);
    w.dispatch_event(WindowEvent::PointerMoved { position: at });
    w.dispatch_event(WindowEvent::PointerPressed { position: at, button: PointerEventButton::Left });
    w.dispatch_event(WindowEvent::PointerReleased { position: at, button: PointerEventButton::Left });
    slint::platform::update_timers_and_animations();
}

fn check(w: &MinimalSoftwareWindow, ui: &MeetingBriefProbe, scale: f32, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    ui.set_scale(scale);
    let p = render(w);
    let px = |x: usize, y: usize| { let c = p.as_slice()[y * W + x]; (c.r, c.g, c.b) };
    let drawn = |x0: usize, x1: usize, y0: usize, y1: usize| {
        (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| px(x, y) != GROUND).count()
    };

    let card_h = ui.get_card_height().round() as usize;
    let buttons_bottom = ui.get_buttons_bottom().round() as usize;
    let bottom = TOP + card_h;
    assert!(card_h > 150, "{scale}x: the card is {card_h}px tall, so its sections are laid out");
    assert!(buttons_bottom < card_h, "{scale}x: the button row ends at {buttons_bottom}, the card at {card_h}");
    assert!(bottom < STACK_Y - 4, "{scale}x: the card ({bottom}px) runs into the stack's box");

    // Nothing beside the card, nor below it down to the stack's box: a card is as tall as its content.
    let side = drawn(0, LEFT - 1, TOP, bottom) + drawn(LEFT + CARD_W + 1, W, TOP, bottom);
    assert_eq!(side, 0, "{scale}x: {side} pixels beside the card");
    let below = drawn(0, W, bottom + 1, STACK_Y - 1);
    assert_eq!(below, 0, "{scale}x: {below} pixels below the card's bottom edge at y={bottom}");

    // Long words are cut with an ellipsis inside the card's padding, never run into it: the strip
    // between the last button's edge and the card's edge holds nothing but the card's own fill.
    let fill = px(LEFT + CARD_W - 4, TOP + 6);
    let strip: usize = (TOP + 4..bottom - 4)
        .flat_map(|y| (LEFT + CARD_W - PAD + 1..LEFT + CARD_W - 3).map(move |x| (x, y)))
        .filter(|&(x, y)| px(x, y) != fill)
        .count();
    assert_eq!(strip, 0, "{scale}x: {strip} pixels of text or buttons in the card's right padding: a long title or row overflowed instead of eliding");

    // The stack: it draws its cards, and nothing past the room it was given.
    assert!(drawn(LEFT, LEFT + CARD_W, STACK_Y, STACK_Y + STACK_H) > 0, "{scale}x: the stack draws its cards");
    let past = drawn(0, W, STACK_Y + STACK_H + 1, H);
    assert_eq!(past, 0, "{scale}x: the stack draws {past} pixels below the room it was given");

    let mut e = png::Encoder::new(BufWriter::new(File::create(output)?), W as u32, H as u32);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(p.as_bytes())?;
    println!("PASS meeting brief at text scale {scale}: the card is {card_h}px, every line is inside it, long rows elide, the stack stops at its box");
    Ok(())
}

/// No key opens anything; a click on each Open does. The counts are the global's own callbacks.
fn check_pointer_only(w: &MinimalSoftwareWindow, ui: &MeetingBriefProbe) {
    let rows: Rc<Cell<i32>> = Rc::new(Cell::new(0));
    let last_row: Rc<Cell<i32>> = Rc::new(Cell::new(-1));
    let events: Rc<Cell<i32>> = Rc::new(Cell::new(0));
    let dismissed: Rc<Cell<i32>> = Rc::new(Cell::new(0));
    let state = ui.global::<MeetingBriefState>();
    let (r, l) = (rows.clone(), last_row.clone());
    state.on_open_row(move |_, row| { r.set(r.get() + 1); l.set(row); });
    let e = events.clone();
    state.on_open_event(move |_| e.set(e.get() + 1));
    let d = dismissed.clone();
    state.on_dismiss(move |_| d.set(d.get() + 1));

    ui.set_scale(1.0);
    let _ = render(w);
    // Tab and Backtab round the whole focus chain, with every confirming key at each stop: if an
    // Open button could take focus it would be pressed here. Dismiss may take focus and answer;
    // it opens nothing.
    for _ in 0..12 {
        for k in [slint::SharedString::from(Key::Tab), "\n".into(), "\r".into(), " ".into()] {
            key(w, k);
        }
        key(w, slint::SharedString::from(Key::Backtab));
        key(w, "\n".into());
        key(w, " ".into());
        let _ = render(w);
    }
    assert_eq!(rows.get(), 0, "no key presses a row's Open");
    assert_eq!(events.get(), 0, "no key presses Open in Calendar");

    // A click does. The first row's Open is the right-hand button in its row; find it by pressing
    // down the column the buttons sit in.
    let card_h = ui.get_card_height().round() as usize;
    let button_x = LEFT + CARD_W - PAD - 30;
    for y in (TOP..TOP + card_h).step_by(2) {
        click(w, button_x, y);
        if rows.get() > 0 {
            break;
        }
    }
    assert!(rows.get() >= 1, "a click on a row's Open opens it");
    assert!((0..=6).contains(&last_row.get()), "it names a row of the card");
    let footer_y = TOP + ui.get_buttons_bottom().round() as usize - 12;
    click(w, LEFT + PAD + 20, footer_y);
    assert_eq!(events.get(), 1, "a click on Open in Calendar opens it");
    for x in (LEFT + PAD + 60..LEFT + CARD_W).step_by(6) {
        click(w, x, footer_y);
        if dismissed.get() > 0 {
            break;
        }
    }
    assert!(dismissed.get() >= 1, "a click on Dismiss dismisses");
    println!("PASS meeting brief: Tab, Backtab, Enter, Return and Space open nothing (x12); a click opens a row, opens the event, and dismisses");
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = MeetingBriefProbe::new()?;
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(W as u32, H as u32));
    ui.set_data(long_card("meeting:e1:1", LONG_TITLE));
    ui.global::<MeetingBriefState>().set_cards(ModelRc::new(VecModel::from(vec![
        long_card("meeting:e1:1", LONG_TITLE),
        long_card("meeting:e2:2", "Design review"),
    ])));
    check(w, &ui, 1.0, output)?;
    check(w, &ui, 1.5, &output.replace(".png", "-1_5x.png"))?;
    check_pointer_only(w, &ui);
    Ok(())
}
