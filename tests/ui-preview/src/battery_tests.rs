//! The bar's battery and its popover (story 1.4), drawn on the real shell.
//!
//! The laptop shapes: discharging at 64% with a time, charging with a bolt, 8% critical, and
//! "plugged in, not charging". The popover with three profiles and with Performance missing, and
//! with no daemon at all. And the VM shape: no battery, so no indicator, and the right cluster
//! of the bar still spaced like a bar and not like one with a hole in it.
//!
//! The indicator is found by pressing the bar from the right until its popover opens, as the
//! bar-overlays scene finds Quick Settings, so a change to the bar's layout does not break this.
//! Every PNG is a crop of the top right, magnified, because that is where the change is.
use super::*;
use std::cell::RefCell;

const W: u32 = 1280;
const H: u32 = 800;
const ZOOM: u32 = 3;

fn draw(w: &MinimalSoftwareWindow) -> slint::SharedPixelBuffer<slint::Rgb8Pixel> {
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(W, H);
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), W as usize); });
    }
    pixels
}

/// Pixels that differ in the rectangle.
fn changed(a: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, b: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, x: (usize, usize), y: (usize, usize)) -> usize {
    (y.0..y.1)
        .flat_map(|y| (x.0..x.1).map(move |x| y * W as usize + x))
        .filter(|&i| a.as_slice()[i] != b.as_slice()[i])
        .count()
}

/// Save a rectangle, magnified so a 16px glyph can be read.
fn save_crop(pixels: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: &str, x: (u32, u32), y: (u32, u32), zoom: u32) -> Result<(), Box<dyn std::error::Error>> {
    let (cw, ch) = ((x.1 - x.0) * zoom, (y.1 - y.0) * zoom);
    let mut out = Vec::with_capacity((cw * ch * 3) as usize);
    for oy in 0..ch {
        for ox in 0..cw {
            let p = pixels.as_slice()[((y.0 + oy / zoom) * W + x.0 + ox / zoom) as usize];
            out.extend_from_slice(&[p.r, p.g, p.b]);
        }
    }
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), cw, ch);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&out)?;
    println!("Rendered {path} ({cw}x{ch})");
    Ok(())
}

struct Look<'a> {
    ui: &'a App,
    w: &'a MinimalSoftwareWindow,
}

impl Look<'_> {
    fn battery(&self, level: i32, state: &str, status: &str, time: &str) {
        self.ui.set_battery_available(true);
        self.ui.set_battery_level(level);
        self.ui.set_battery_charging(state == "charging");
        self.ui.set_battery_state(state.into());
        self.ui.set_battery_status_text(status.into());
        self.ui.set_battery_time_text(time.into());
    }

    fn park(&self) {
        self.w.dispatch_event(slint::platform::WindowEvent::PointerMoved { position: slint::LogicalPosition::new(640.0, 420.0) });
    }

    /// Press the bar from the right until the battery's popover is up; where it was.
    fn open_popover(&self) -> f32 {
        let mut x = W as f32 - 6.0;
        while x > 700.0 {
            click(self.w, x, 16.0);
            draw(self.w);
            if self.ui.get_battery_popover_open() {
                return x;
            }
            self.ui.set_quick_settings_open(false);
            self.ui.set_power_menu_open(false);
            self.ui.set_clip_panel_open(false);
            if self.ui.get_current_screen() != 8 {
                self.ui.set_current_screen(8);
            }
            x -= 3.0;
        }
        panic!("no press along the bar opened the battery popover");
    }
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = App::new()?;
    // Files, not the desktop: the desktop has its own moving parts and this scene's idle check
    // wants a shell that is still.
    ui.set_current_screen(8);
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(W, H));
    let look = Look { ui: &ui, w };
    let path = |what: &str| output.replace(".png", &format!("-{what}.png"));
    let bar = ((W - 360), W);
    let strip = (0, 40);

    // ── The VM shape: no battery. ──
    ui.set_battery_available(false);
    look.park();
    let vm = draw(w);
    save_crop(&vm, &path("vm"), bar, strip, ZOOM)?;
    // Pressing everywhere along the bar must never open a battery popover that has no battery.
    let mut x = W as f32 - 6.0;
    while x > 700.0 {
        click(w, x, 16.0);
        draw(w);
        assert!(!ui.get_battery_popover_open(), "no battery, so no popover (pressed at x={x})");
        ui.set_quick_settings_open(false);
        ui.set_power_menu_open(false);
        ui.set_clip_panel_open(false);
        if ui.get_current_screen() != 8 { ui.set_current_screen(8); }
        x -= 3.0;
    }
    look.park();
    draw(w);

    // ── The laptop shapes. ──
    look.battery(64, "discharging", "Discharging", "2 h 15 min left");
    look.park();
    let discharging = draw(w);
    let drawn = changed(&vm, &discharging, (W as usize - 360, W as usize), (0, 32));
    assert!(drawn > 200, "a battery on the bar changes the right cluster: {drawn} pixels");
    save_crop(&discharging, &path("discharging-64"), bar, strip, ZOOM)?;

    look.battery(52, "charging", "Charging", "40 min to full");
    let charging = draw(w);
    assert!(changed(&discharging, &charging, (W as usize - 360, W as usize), (0, 32)) > 50, "charging looks different from discharging (bolt, colour)");
    save_crop(&charging, &path("charging-52"), bar, strip, ZOOM)?;

    look.battery(8, "discharging", "Discharging", "18 min left");
    let critical = draw(w);
    save_crop(&critical, &path("critical-8"), bar, strip, ZOOM)?;

    look.battery(80, "plugged-not-charging", "Plugged in, not charging", "");
    let limit = draw(w);
    assert!(changed(&limit, &charging, (W as usize - 360, W as usize), (0, 32)) > 50, "plugged in and not charging is not drawn as charging");
    save_crop(&limit, &path("plugged-not-charging-80"), bar, strip, ZOOM)?;

    // The tooltip: the pointer rests on the indicator for the tooltip's delay.
    look.battery(64, "discharging", "Discharging", "2 h 15 min left");
    ui.set_battery_popover_open(false);
    let x_ind = look.open_popover();
    ui.set_battery_popover_open(false);
    draw(w);
    w.dispatch_event(slint::platform::WindowEvent::PointerMoved { position: slint::LogicalPosition::new(x_ind, 16.0) });
    draw(w);
    std::thread::sleep(std::time::Duration::from_millis(700));
    let tip = draw(w);
    save_crop(&tip, &path("tooltip"), (W - 560, W), (0, 90), 2)?;
    assert!(changed(&discharging, &tip, (W as usize - 460, W as usize), (34, 80)) > 300, "the tooltip is drawn under the indicator after the delay");
    look.park();
    draw(w);

    // ── The popover. ──
    ui.set_power_profile("balanced".into());
    ui.set_power_performance_offered(true);
    let chosen: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        let c = chosen.clone();
        ui.on_set_power_profile(move |p| c.borrow_mut().push(p.to_string()));
    }
    let before = draw(w);
    let x_open = look.open_popover();
    look.park();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let three = draw(w);
    assert!(ui.get_battery_anchor_x() > 1000.0, "the popover hangs from the battery at the right of the bar: {}", ui.get_battery_anchor_x());
    assert!(changed(&before, &three, (900, 1280), (36, 240)) > 5_000, "the popover is drawn under the bar");
    save_crop(&three, &path("popover-three-profiles"), (W - 460, W), (0, 220), 2)?;

    // Each profile button reaches its callback. Find the buttons' row by pressing down the
    // first button's column, then press each one.
    let mut row = None;
    for y in (80..200).step_by(4) {
        click(w, 980.0, y as f32);
        if !chosen.borrow().is_empty() { row = Some(y as f32); break; }
    }
    let row = row.expect("a press on Power Saver reached `set-power-profile`");
    assert_eq!(chosen.borrow().last().map(String::as_str), Some("power-saver"));
    click(w, 1090.0, row);
    assert_eq!(chosen.borrow().last().map(String::as_str), Some("balanced"));
    click(w, 1200.0, row);
    assert_eq!(chosen.borrow().last().map(String::as_str), Some("performance"));
    assert!(ui.get_battery_popover_open(), "choosing a profile does not close the popover");

    // Without Performance: two buttons, and where the third was there is nothing to press.
    ui.set_power_profile("power-saver".into());
    ui.set_power_performance_offered(false);
    look.park();
    // Let the buttons' colour change settle: a frame taken mid-fade shows two half-lit buttons.
    std::thread::sleep(std::time::Duration::from_millis(300));
    let two = draw(w);
    save_crop(&two, &path("popover-no-performance"), (W - 460, W), (0, 220), 2)?;
    assert!(changed(&three, &two, (900, 1280), (100, 200)) > 1_000, "Performance is gone, not greyed");
    // The two that are left take the whole row between them: a press where Performance was
    // reaches Balanced, and Performance cannot be asked for at all.
    click(w, 1240.0, row);
    assert_eq!(chosen.borrow().last().map(String::as_str), Some("balanced"), "a press where Performance was reaches Balanced");
    assert_eq!(chosen.borrow().iter().filter(|p| *p == "performance").count(), 1, "Performance was asked for once, while it was offered, and not since");

    // A backlight puts the shared brightness slider in the popover, and a VM (none) draws none.
    ui.set_power_profile("balanced".into());
    ui.set_power_performance_offered(true);
    ui.set_brightness_available(true);
    ui.set_brightness_level(70);
    std::thread::sleep(std::time::Duration::from_millis(300));
    let lit = draw(w);
    save_crop(&lit, &path("popover-with-brightness"), (W - 460, W), (0, 260), 2)?;
    assert!(changed(&three, &lit, (900, 1280), (100, 260)) > 1_000, "a backlight adds the brightness slider");
    let levels: Rc<RefCell<Vec<i32>>> = Rc::default();
    {
        let l = levels.clone();
        ui.on_brightness_changed(move |v| l.borrow_mut().push(v));
    }
    // The slider is the popover's first row below the level; press its middle.
    let mut slid = false;
    for y in (110..200).step_by(4) {
        click(w, 1090.0, y as f32);
        if !levels.borrow().is_empty() { slid = true; break; }
    }
    assert!(slid, "pressing the slider reaches the shell's one brightness callback");
    assert!(ui.get_brightness_level() >= 30 && ui.get_brightness_level() <= 70, "the shared level follows the slider: {}", ui.get_brightness_level());
    ui.set_brightness_available(false);

    // No daemon: no choice at all, and the popover is shorter.
    ui.set_power_profile("".into());
    std::thread::sleep(std::time::Duration::from_millis(300));
    let none = draw(w);
    save_crop(&none, &path("popover-no-daemon"), (W - 460, W), (0, 220), 2)?;
    assert!(changed(&two, &none, (900, 1280), (100, 200)) > 1_000, "no daemon, no profile choice");

    // The bar's button again closes it, and Escape too.
    ui.set_power_profile("balanced".into());
    ui.set_power_performance_offered(true);
    click(w, x_open, 16.0);
    look.park();
    draw(w);
    assert!(!ui.get_battery_popover_open(), "the battery's button closes its popover");
    ui.set_battery_popover_open(true);
    draw(w);
    click(w, 300.0, 500.0);
    draw(w);
    assert!(!ui.get_battery_popover_open(), "a press outside closes it");

    // One panel at a time: Quick Settings and the battery's popover do not stack.
    ui.set_quick_settings_open(true);
    draw(w);
    click(w, x_open, 16.0);
    draw(w);
    assert!(ui.get_battery_popover_open() && !ui.get_quick_settings_open(), "opening the battery's popover puts Quick Settings away");
    ui.set_battery_popover_open(false);

    // Settled, with the popover shut and then open, the shell repaints nothing.
    for open in [false, true] {
        ui.set_battery_popover_open(open);
        look.park();
        std::thread::sleep(std::time::Duration::from_millis(400));
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(W, H);
        for _ in 0..5 {
            slint::platform::update_timers_and_animations();
            w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), W as usize); });
        }
        let mut redraws = 0;
        for _ in 0..10 {
            slint::platform::update_timers_and_animations();
            if w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), W as usize); }) { redraws += 1; }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert_eq!(redraws, 0, "a settled shell repaints nothing (popover open: {open})");
        println!("settled shell, popover open={open}: {redraws} redraws over one second");
    }

    println!("PASS: battery states, popover profiles, VM shape, one panel at a time, idle");
    Ok(())
}
