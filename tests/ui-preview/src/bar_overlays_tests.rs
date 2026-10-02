//! The bar's panels over screens that are not the desktop (story 0.1). Quick Settings, the power
//! menu and the clipboard panel stood inside DesktopScreen, so from Files, Settings or Agents the
//! bar's buttons set a flag that drew nothing. They are the shell's now (`ShellOverlays` in
//! app.slint), and this draws the whole shell on each of those three screens and checks that
//! each panel appears and that closing it brings back exactly the frame that was there before.
//!
//! Quick Settings and the power menu are opened by pressing the bar's own buttons, found by
//! sweeping the bar, so a change to the bar's layout does not break this; the clipboard has no
//! button on the bar, its door is Super+V, which sets the flag, so the flag is set here.
use super::*;
use std::cell::Cell;

/// The screens the bug was seen on: Settings, Files and the Agents workspace.
const SCREENS: [(&str, i32); 3] = [("settings", 7), ("files", 8), ("agents", 34)];

fn save(pixels: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    println!("Rendered {path} ({width}x{height})");
    Ok(())
}

/// How many pixels in the rectangle differ between two frames.
fn changed(a: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, b: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, width: u32, x: (usize, usize), y: (usize, usize)) -> usize {
    (y.0..y.1)
        .flat_map(|y| (x.0..x.1).map(move |x| y * width as usize + x))
        .filter(|&i| a.as_slice()[i] != b.as_slice()[i])
        .count()
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280u32, 800u32);
    let ui = App::new()?;
    ui.set_current_screen(1);
    ui.set_battery_available(true);
    ui.set_battery_level(76);
    // The shell reports an app window in front by raising itself on these; here it is a count.
    let opened: Rc<Cell<u32>> = Rc::default();
    {
        let n = opened.clone();
        ui.on_shell_overlay_opened(move |_| n.set(n.get() + 1));
    }
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    let draw = || {
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
        for _ in 0..3 {
            slint::platform::update_timers_and_animations();
            w.request_redraw();
            w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); });
        }
        pixels
    };
    draw();

    // The pointer rests in the middle of the screen between steps. Left over a bar button it
    // lights the button, and a frame that differs by that highlight is not the one before.
    let park = || w.dispatch_event(slint::platform::WindowEvent::PointerMoved { position: slint::LogicalPosition::new(640.0, 420.0) });
    // The bar's indicators fade their hover wash out over `dur-fast`; a frame taken before it has
    // finished is not the frame that was there. Waited out where a frame is compared to an earlier one.
    let settle = || std::thread::sleep(std::time::Duration::from_millis(250));

    let shut = |ui: &App| {
        ui.set_quick_settings_open(false);
        ui.set_power_menu_open(false);
        ui.set_clip_panel_open(false);
    };
    // Press the bar along its length from the right until `open` says a panel came up. A press
    // that lands on something else (the bell goes to Notifications) is put back, so the sweep
    // does not leave the screen it was started on.
    let press_bar_for = |ui: &App, screen: i32, open: &dyn Fn(&App) -> bool| -> f32 {
        let mut x = width as f32 - 6.0;
        while x > 500.0 {
            click(w, x, 16.0);
            draw();
            if open(ui) {
                return x;
            }
            if ui.get_current_screen() != screen {
                ui.set_current_screen(screen);
                draw();
            }
            shut(ui);
            x -= 4.0;
        }
        panic!("no press along the bar on screen {screen} opened the panel");
    };

    for (name, screen) in SCREENS {
        ui.set_current_screen(screen);
        shut(&ui);
        park();
        let before = draw();
        let path = |what: &str| output.replace(".png", &format!("-{what}.png"));

        // Quick Settings, by the bar's own button. Its panel hangs 380px wide from the bar,
        // centred.
        let x_qs = press_bar_for(&ui, screen, &|ui| ui.get_quick_settings_open());
        let qs = draw();
        let panel = changed(&before, &qs, width, (460, 820), (40, 300));
        assert!(panel > 20_000, "{name}: Quick Settings is drawn over the screen: only {panel} pixels changed");
        assert_eq!(ui.get_current_screen(), screen, "{name}: opening it does not leave the screen");
        save(&qs, &path(name), width, height)?;
        // The bar's button again, and the backdrop, are both ways out; take the button's.
        click(w, x_qs, 16.0);
        park();
        settle();
        let back = draw();
        assert!(!ui.get_quick_settings_open(), "{name}: the same button closes it");
        assert_eq!(changed(&before, &back, width, (0, width as usize), (0, height as usize)), 0, "{name}: closed, the frame is the one that was there");

        // The power menu, the same way, further along the bar.
        let x_pm = press_bar_for(&ui, screen, &|ui| ui.get_power_menu_open());
        let pm = draw();
        let card = changed(&before, &pm, width, (440, 840), (200, 600));
        assert!(card > 20_000, "{name}: the power menu is drawn over the screen: only {card} pixels changed");
        assert_ne!(x_pm, x_qs, "{name}: the two buttons are two buttons");
        save(&pm, &path(&format!("{name}-power")), width, height)?;
        click(w, x_pm, 16.0);
        park();
        settle();
        let back = draw();
        assert!(!ui.get_power_menu_open(), "{name}: the power button closes the menu");
        assert_eq!(changed(&before, &back, width, (0, width as usize), (0, height as usize)), 0, "{name}: closed, the frame is the one that was there");

        // The clipboard, from its keybind: the flag. Escape-less; the backdrop closes it.
        ui.set_clip_panel_open(true);
        park();
        let cb = draw();
        let panel = changed(&before, &cb, width, (300, 980), (100, 700));
        assert!(panel > 20_000, "{name}: the clipboard panel is drawn over the screen: only {panel} pixels changed");
        save(&cb, &path(&format!("{name}-clipboard")), width, height)?;
        ui.set_clip_panel_open(false);
        let back = draw();
        assert_eq!(changed(&before, &back, width, (0, width as usize), (0, height as usize)), 0, "{name}: closed, the frame is the one that was there");
    }

    // Every open, by button or flag, went through the hook that raises the shell.
    assert!(opened.get() >= 9, "each of the nine opens fired shell-overlay-opened: {}", opened.get());

    // The bar is not drawn on lock, so neither is a panel that was asked for there.
    ui.set_current_screen(3);
    let locked_before = draw();
    ui.set_power_menu_open(true);
    let locked_after = draw();
    // The lock screen has its own moving parts, so "no panel" is "nowhere near a panel's worth":
    // a panel is over a hundred thousand pixels here, the lock's own drift a few hundred.
    let drift = changed(&locked_before, &locked_after, width, (0, width as usize), (0, height as usize));
    assert!(drift < 5_000, "the lock screen shows no panel: {drift} pixels changed");
    ui.set_power_menu_open(false);

    // And once everything is shut and settled, the shell is still: nothing here is a timer.
    ui.set_current_screen(8);
    draw();
    std::thread::sleep(std::time::Duration::from_millis(400));
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
    for _ in 0..5 {
        slint::platform::update_timers_and_animations();
        w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); });
    }
    let mut redraws = 0;
    for _ in 0..10 {
        slint::platform::update_timers_and_animations();
        if w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); }) {
            redraws += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(redraws, 0, "a settled shell with every panel shut repaints nothing");
    println!("settled shell on Files: {redraws} redraws over one second");
    save(&pixels, output, width, height)?;
    println!("PASS: Quick Settings, the power menu and the clipboard draw over Settings, Files and Agents, and closing restores the frame");
    Ok(())
}
