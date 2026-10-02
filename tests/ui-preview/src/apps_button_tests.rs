//! The taskbar's Apps button, pressed on the screens it did nothing on (#219): the whole shell,
//! app.slint's App, with a real pointer press on the button. The launcher is drawn on the desktop
//! screen, so from Files, Notifications, Recipes or Settings the press flipped a property nothing
//! drew. Now it goes to the desktop, puts the Lens away, and the launcher is on screen.
use super::*;

fn save(pixels: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    println!("Rendered {path} ({width}×{height})");
    Ok(())
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280u32, 800u32);
    let ui = App::new()?;
    ui.set_current_screen(1);
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    let draw = || {
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
        for _ in 0..2 {
            slint::platform::update_timers_and_animations();
            w.request_redraw();
            w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); });
        }
        pixels
    };
    draw();

    // The button is the first thing in the dock, at its left end; the dock is centred. Found by pressing
    // along the dock from the left until the launcher opens, so a change of padding does not break this.
    let taskbar_y = height as f32 - 24.0;
    let mut button_x = None;
    for x in (556..720).step_by(4) {
        click(w, x as f32, taskbar_y);
        draw();
        if ui.get_app_grid_open() {
            button_x = Some(x as f32);
            break;
        }
    }
    let button_x = button_x.expect("the Apps button opens the launcher from the desktop");
    click(w, button_x, taskbar_y);
    draw();
    assert!(!ui.get_app_grid_open(), "a second press closes it again");

    // Files, Notifications, Recipes, Settings: the screens the live tour pressed it on.
    for (screen, name) in [(8, "Files"), (9, "Notifications"), (35, "Recipes"), (7, "Settings")] {
        ui.set_app_grid_open(false);
        ui.set_current_screen(screen);
        ui.set_lens_open(true);
        let before = draw();
        click(w, button_x, taskbar_y);
        let after = draw();
        assert!(ui.get_app_grid_open(), "the Apps button opens the launcher from {name}");
        assert_eq!(ui.get_current_screen(), 1, "from {name} it goes to the desktop, where the launcher is drawn");
        assert!(!ui.get_lens_open(), "and from {name} it puts the Lens away, which would cover the launcher");
        // It is on screen: the middle of the display changed.
        let (x0, x1, y0, y1) = (320usize, 960usize, 200usize, 600usize);
        let changed = (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| y * width as usize + x))
            .filter(|&i| before.as_slice()[i] != after.as_slice()[i])
            .count();
        assert!(changed > 20_000, "from {name}, the launcher is drawn: only {changed} pixels changed");
        if screen == 8 {
            save(&after, output, width, height)?;
        }
    }

    // Chat, from anywhere (#241): the mind button at the right end of the dock, found by pressing
    // leftward across the dock until the Lens opens. From Files and Settings it goes to the
    // desktop, where the Lens is drawn, and puts the launcher away.
    let shown: Rc<std::cell::Cell<u32>> = Rc::default();
    {
        let shown = shown.clone();
        ui.on_show_desktop(move || shown.set(shown.get() + 1));
    }
    ui.set_active_harness_name("Hermes".into());
    ui.set_app_grid_open(false);
    ui.set_lens_open(false);
    ui.set_current_screen(8);
    draw();
    let mut chat_x = None;
    for x in (556..720).rev().step_by(4).map(|x| x as f32) {
        click(w, x, taskbar_y);
        draw();
        if ui.get_lens_open() {
            chat_x = Some(x);
            break;
        }
    }
    let chat_x = chat_x.expect("the Chat button opens the Lens");
    assert_eq!(ui.get_current_screen(), 1, "from Files, Chat goes to the desktop, where the Lens is drawn");
    assert_eq!(shown.get(), 0, "finding Chat did not press Show desktop");
    for (screen, name) in [(7, "Settings"), (35, "Recipes")] {
        ui.set_lens_open(false);
        ui.set_app_grid_open(true);
        ui.set_current_screen(screen);
        draw();
        click(w, chat_x, taskbar_y);
        draw();
        assert!(ui.get_lens_open(), "Chat opens the Lens from {name}");
        assert_eq!(ui.get_current_screen(), 1, "from {name}, on the desktop");
        assert!(!ui.get_app_grid_open(), "and from {name} it puts the launcher away");
    }

    // Show desktop is the very corner: a pointer thrown there lands on the last pixel of both
    // edges, and that press reaches it without touching Chat.
    let lens_before = ui.get_lens_open();
    click(w, width as f32 - 1.0, height as f32 - 1.0);
    draw();
    assert_eq!(shown.get(), 1, "a press on the bottom-right pixel is Show desktop");
    assert_eq!(ui.get_lens_open(), lens_before, "and not Chat");
    click(w, width as f32 - 6.0, taskbar_y);
    draw();
    assert_eq!(shown.get(), 2, "the strip is the corner's whole height, not just its last pixel");
    save(&draw(), &output.replace(".png", "-chat.png"), width, height)?;

    println!(
        "PASS: the Apps button opens the launcher from the desktop and closes it again; from Files, Notifications, Recipes and Settings it goes to the desktop, puts the Lens away and the launcher is drawn; Chat opens the Lens on the desktop from Files, Settings and Recipes; Show desktop is the bottom-right corner"
    );
    Ok(())
}
