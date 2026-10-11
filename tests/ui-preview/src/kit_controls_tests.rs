//! The kit's four shared controls — YSlider, YPopover, YToggleTile, YIndicator — drawn and driven
//! with real pointer and key events (KitControlsProbe in kit_controls_probe.slint, through the
//! shell's shims). Every later indicator, popover and Quick Settings screen is built from these,
//! so what is asserted here is what they will all inherit:
//!
//!  * the slider's thumb centre is exactly where its value says, a press jumps there and a drag
//!    follows (and clamps past either end), the keys and the wheel move it by the right amounts,
//!    and `changed` fires once per real change and never for a no-op;
//!  * the popover is clamped inside the screen whatever its anchor, takes focus when it opens
//!    (so Esc works at once), asks to close on Esc and on the backdrop but not on its own surface,
//!    and costs no redraws once it has settled;
//!  * the tile toggles by click and by Space/Enter, the chevron half raises its own callback
//!    without toggling, and Right does the same from the keyboard;
//!  * the indicator tells a left click from a middle click, counts wheel steps, answers Space,
//!    and draws a tooltip after resting on it.
use super::*;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::LogicalPosition;
use std::time::Duration;

const W: u32 = 880;
const H: u32 = 460;

fn save(pixels: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), W, H);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    println!("Rendered {path} ({W}×{H})");
    Ok(())
}

fn at(x: f32, y: f32) -> LogicalPosition {
    LogicalPosition::new(x, y)
}

fn press(w: &MinimalSoftwareWindow, x: f32, y: f32) {
    w.dispatch_event(WindowEvent::PointerMoved { position: at(x, y) });
    w.dispatch_event(WindowEvent::PointerPressed { position: at(x, y), button: PointerEventButton::Left });
}
fn drag_to(w: &MinimalSoftwareWindow, x: f32, y: f32) {
    w.dispatch_event(WindowEvent::PointerMoved { position: at(x, y) });
}
fn release(w: &MinimalSoftwareWindow, x: f32, y: f32) {
    w.dispatch_event(WindowEvent::PointerReleased { position: at(x, y), button: PointerEventButton::Left });
}
fn middle_click(w: &MinimalSoftwareWindow, x: f32, y: f32) {
    w.dispatch_event(WindowEvent::PointerMoved { position: at(x, y) });
    w.dispatch_event(WindowEvent::PointerPressed { position: at(x, y), button: PointerEventButton::Middle });
    w.dispatch_event(WindowEvent::PointerReleased { position: at(x, y), button: PointerEventButton::Middle });
}
fn wheel(w: &MinimalSoftwareWindow, x: f32, y: f32, delta_y: f32) {
    w.dispatch_event(WindowEvent::PointerMoved { position: at(x, y) });
    w.dispatch_event(WindowEvent::PointerScrolled { position: at(x, y), delta_x: 0.0, delta_y });
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = KitControlsProbe::new()?;
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(W, H));
    // Animations and the tooltip's delay run on real time, so a settle waits for it.
    let draw = || {
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(W, H);
        for _ in 0..2 {
            slint::platform::update_timers_and_animations();
            w.request_redraw();
            w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), W as usize); });
        }
        pixels
    };
    let settle = |ms: u64| {
        std::thread::sleep(Duration::from_millis(ms));
        draw()
    };
    settle(250);

    // ── Slider geometry: the thumb is where the value says, to the pixel ──────────
    let (sx, sy) = (ui.get_slider_x(), ui.get_slider_y());
    let (tx, tw) = (ui.get_track_x(), ui.get_track_w());
    let travel = tw - 18.0; // slider-thumb
    let want = tx + 9.0 + 0.62 * travel;
    assert!(
        (ui.get_thumb_x() - want).abs() < 0.01,
        "the thumb centre of 62/100 is at {want}, drawn at {}",
        ui.get_thumb_x()
    );
    let mid_y = sy + 16.0;
    let track_px = |frac: f32| sx + tx + 9.0 + frac * travel;
    assert_eq!(ui.get_panel_w(), 360.0, "the popover is the token's 360px wide");
    assert_eq!(ui.get_panel_x(), 20.0, "centred under its anchor (200 - 180) when there is room");

    // The page as a person first sees it, with the keyboard ring on the volume thumb.
    ui.invoke_focus_volume();
    save(&settle(50), output)?;
    // The page's own controls are tested with the popover away: while it is open its backdrop
    // owns every click outside it, which is the point of it.
    ui.set_pop_open(false);
    settle(250);

    // ── Pointer: a press jumps, a drag follows, past the ends it clamps ─────────────
    let base = ui.get_changes();
    press(w, track_px(0.25), mid_y);
    draw();
    assert_eq!(ui.get_last_change(), 25, "pressing at 25% of the travel jumps to 25");
    assert_eq!(ui.get_changes(), base + 1, "and reports exactly once");
    drag_to(w, track_px(0.80), mid_y);
    draw();
    assert_eq!(ui.get_last_change(), 80, "dragging to 80% reports 80");
    assert!(
        (ui.get_thumb_x() - (tx + 9.0 + 0.80 * travel)).abs() < 0.01,
        "the thumb follows the pointer exactly while dragging"
    );
    save(&settle(200), &output.replace(".png", "-drag.png"))?;
    drag_to(w, sx + tx + tw + 80.0, mid_y + 20.0);
    draw();
    assert_eq!(ui.get_last_change(), 100, "dragging past the right end clamps to max");
    drag_to(w, sx - 40.0, mid_y);
    draw();
    assert_eq!(ui.get_last_change(), 0, "dragging past the left end clamps to 0");
    let after_drag = ui.get_changes();
    drag_to(w, sx - 60.0, mid_y);
    draw();
    assert_eq!(ui.get_changes(), after_drag, "moving without changing the level reports nothing");
    drag_to(w, track_px(0.5), mid_y);
    release(w, track_px(0.5), mid_y);
    draw();
    assert_eq!(ui.get_volume(), 50, "the drag ends at 50");
    assert!(
        (ui.get_thumb_x() - (tx + 9.0 + 0.5 * travel)).abs() < 0.01,
        "released, the thumb rests on the value the owner holds"
    );

    // ── Keyboard ───────────────────────────────────────────────────────────────
    ui.invoke_focus_volume();
    let before = ui.get_changes();
    key(w, Key::RightArrow.into());
    assert_eq!(ui.get_volume(), 51, "Right is +1 step");
    key(w, Key::LeftArrow.into());
    assert_eq!(ui.get_volume(), 50, "Left is -1 step");
    key(w, Key::PageUp.into());
    assert_eq!(ui.get_volume(), 60, "PageUp is +10 steps");
    key(w, Key::PageDown.into());
    assert_eq!(ui.get_volume(), 50, "PageDown is -10 steps");
    key(w, Key::UpArrow.into());
    assert_eq!(ui.get_volume(), 51, "Up is +1 step, like Right");
    key(w, Key::End.into());
    assert_eq!(ui.get_volume(), 100, "End is max");
    let at_max = ui.get_changes();
    key(w, Key::RightArrow.into());
    key(w, Key::PageUp.into());
    assert_eq!(ui.get_volume(), 100, "nothing goes past max");
    assert_eq!(ui.get_changes(), at_max, "and a key that changes nothing reports nothing");
    key(w, Key::Home.into());
    assert_eq!(ui.get_volume(), 0, "Home is 0");
    key(w, Key::LeftArrow.into());
    assert_eq!(ui.get_volume(), 0, "nothing goes below 0");
    assert_eq!(ui.get_changes(), at_max + 1, "keys reported {} changes", ui.get_changes() - before);
    ui.set_volume(50);

    // ── Wheel: one click is one step, a touchpad adds up, reversing drops the remainder ─────
    let (wx, wy) = (track_px(0.5), mid_y);
    wheel(w, wx, wy, 60.0);
    assert_eq!(ui.get_volume(), 51, "one wheel click up is +1");
    wheel(w, wx, wy, -120.0);
    assert_eq!(ui.get_volume(), 49, "two clicks down is -2");
    for _ in 0..3 { wheel(w, wx, wy, 20.0); }
    assert_eq!(ui.get_volume(), 50, "three small touchpad deltas add up to one step");
    wheel(w, wx, wy, 40.0);
    wheel(w, wx, wy, -40.0);
    assert_eq!(ui.get_volume(), 50, "reversing drops the carried remainder instead of paying it off");

    // ── Toggle tile ──────────────────────────────────────────────────────────
    let (fx, fy, fw) = (ui.get_wifi_x(), ui.get_wifi_y(), ui.get_wifi_w());
    let toggles = ui.get_toggles();
    click(w, fx + 40.0, fy + 28.0);
    assert_eq!((ui.get_toggles(), ui.get_wifi_on()), (toggles + 1, false), "a click on the body toggles");
    ui.invoke_focus_wifi();
    key(w, " ".into());
    assert_eq!((ui.get_toggles(), ui.get_wifi_on()), (toggles + 2, true), "Space toggles");
    key(w, "\n".into());
    assert_eq!((ui.get_toggles(), ui.get_wifi_on()), (toggles + 3, false), "Enter toggles");
    let details = ui.get_details();
    click(w, fx + fw - 22.0, fy + 28.0);
    assert_eq!(ui.get_details(), details + 1, "a click on the chevron half raises details-requested");
    assert_eq!(ui.get_toggles(), toggles + 3, "and does not toggle");
    ui.invoke_focus_wifi();
    key(w, Key::RightArrow.into());
    assert_eq!(ui.get_details(), details + 2, "Right on the body opens the details too");
    assert_eq!(ui.get_toggles(), toggles + 3, "without toggling");
    let bt = ui.get_bt_on();
    click(w, ui.get_bt_x() + 40.0, ui.get_bt_y() + 28.0);
    assert_eq!(ui.get_bt_on(), !bt, "a tile without a chevron toggles from anywhere on it");
    ui.set_wifi_on(true);

    // ── Indicator ────────────────────────────────────────────────────────────
    let (cx, iy) = (ui.get_ind_x(), ui.get_ind_y());
    let cy = iy + 16.0;
    assert_eq!(ui.get_ind_bottom() - iy, 32.0, "the indicator is h-regular tall, and reports its edges");
    assert!(cx > 790.0 && cx < 836.0, "its anchor is where it is drawn (right end of the bar), not where the layout offset counted twice puts it: {cx}");
    let clicks = ui.get_ind_clicks();
    click(w, cx, cy);
    assert_eq!(ui.get_ind_clicks(), clicks + 1, "a left click is `clicked`");
    middle_click(w, cx, cy);
    assert_eq!(ui.get_ind_middle(), 1, "a middle click is `middle-clicked`");
    assert_eq!(ui.get_ind_clicks(), clicks + 1, "and not also `clicked`");
    wheel(w, cx, cy, 60.0);
    wheel(w, cx, cy, 60.0);
    wheel(w, cx, cy, -60.0);
    assert_eq!(ui.get_ind_scroll(), 1, "scrolled reports whole steps: up, up, down is +1");
    ui.invoke_focus_indicator();
    key(w, " ".into());
    assert_eq!(ui.get_ind_clicks(), clicks + 2, "Space activates a focused indicator");
    key(w, "\n".into());
    assert_eq!(ui.get_ind_clicks(), clicks + 3, "and so does Enter");

    // Resting on it brings up the tooltip, and leaving takes it away.
    ui.set_ind_active(false);
    w.dispatch_event(WindowEvent::PointerMoved { position: at(5.0, 5.0) });
    ui.invoke_focus_wifi(); // takes the ring off the indicator for the picture
    let bare = settle(100);
    w.dispatch_event(WindowEvent::PointerMoved { position: at(cx, cy) });
    draw();
    settle(800);
    let tip = settle(50);
    let (x0, x1) = ((cx - 60.0) as usize, (cx + 60.0) as usize);
    let (y0, y1) = ((iy - 34.0) as usize, (iy - 2.0) as usize);
    let changed = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| y * W as usize + x))
        .filter(|&i| bare.as_slice()[i] != tip.as_slice()[i])
        .count();
    assert!(changed > 400, "after resting on it the tooltip is drawn above: only {changed} pixels changed");
    save(&tip, &output.replace(".png", "-tooltip.png"))?;
    w.dispatch_event(WindowEvent::PointerMoved { position: at(5.0, 5.0) });
    let gone = settle(200);
    let left = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| y * W as usize + x))
        .filter(|&i| bare.as_slice()[i] != gone.as_slice()[i])
        .count();
    assert!(left < 40, "and it goes when the pointer leaves: {left} pixels still differ");

    // ── Popover: clamped, focused on open, closes on Esc and the backdrop ─────────
    ui.set_anchor_x(870.0);
    assert_eq!(ui.get_panel_x() + ui.get_panel_w(), W as f32 - 8.0, "a trigger at the right edge: the panel stops at the margin");
    ui.set_anchor_x(0.0);
    assert_eq!(ui.get_panel_x(), 8.0, "a trigger at the left edge: the panel stops at the margin");
    ui.set_anchor_x(200.0);
    assert_eq!(ui.get_panel_y(), 32.0, "it hangs 8px below the anchor");
    assert!(ui.get_panel_y() + ui.get_panel_h() < H as f32, "and fits the screen");
    save(&settle(50), &output.replace(".png", "-panel.png"))?;

    // Opening moves focus into it: Esc is heard with nothing clicked.
    ui.set_pop_open(false);
    settle(250);
    ui.set_pop_open(true);
    settle(250);
    let asked = ui.get_close_requests();
    key(w, Key::Escape.into());
    assert_eq!(ui.get_close_requests(), asked + 1, "Esc, straight after opening, asks to close");
    // And from a focused child: the key bubbles up to the panel.
    ui.invoke_focus_popover_tile();
    key(w, Key::Escape.into());
    assert_eq!(ui.get_close_requests(), asked + 2, "Esc from a focused tile in it asks to close");
    ui.invoke_focus_popover_slider();
    key(w, Key::Escape.into());
    assert_eq!(ui.get_close_requests(), asked + 3, "and from a focused slider");
    // A click outside is the backdrop; a click on the panel's own surface is not.
    click(w, 700.0, 440.0);
    assert_eq!(ui.get_close_requests(), asked + 4, "a click outside asks to close");
    click(w, 30.0, ui.get_panel_y() + 8.0);
    assert_eq!(ui.get_close_requests(), asked + 4, "a click on the panel's padding does not");
    // The popover does not close itself: the owner decides.
    assert!(ui.get_pop_open(), "asking is not closing");

    // ── Idle: the popover, once settled, costs nothing ────────────────────────────
    ui.set_pop_open(false);
    settle(250);
    ui.set_pop_open(true);
    for _ in 0..4 { settle(100); }
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(W, H);
    let mut redraws = 0;
    for _ in 0..12 {
        slint::platform::update_timers_and_animations();
        if w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), W as usize); }) {
            redraws += 1;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(redraws, 0, "a settled popover must not keep repainting");

    // The same page in light mode: a render to look at.
    ui.set_pop_open(true);
    ui.set_light(true);
    save(&settle(300), &output.replace(".png", "-light.png"))?;
    ui.set_light(false);

    ui.hide()?;
    println!(
        "PASS: slider thumb at its value to the pixel, press/drag/clamp, keys and wheel step correctly and report only real changes; popover clamps at both edges, takes focus on open, closes on Esc and the backdrop only, 0 redraws once settled; tile toggles by click, Space and Enter, chevron and Right raise details without toggling; indicator separates left, middle, wheel and keys and shows its tooltip"
    );
    Ok(())
}
