//! The lock screen, the theme cards and the lake desktop, drawn by the real components and saved
//! as PNGs for a person to look at (design/renders/03-lock-screen.jpg and
//! 01-desktop-colour-system-grounded-dock.jpg are what they are compared with):
//!
//!   verify-lock          the shell's own lock screen (app.slint's App on screen 3) and the
//!                        compositor's `LockView` (crates/yantrik-lock), at the size asked for,
//!                        over the lake wallpaper blurred by the production code; pressed with
//!                        real pointer and key events
//!   verify-themes        Settings > Appearance with the theme cards
//!   verify-lake-desktop  the desktop on its new default wallpaper
//!
//! Then the proof that a lock screen at rest costs nothing: no redraw is requested over 1.2s.
use super::*;
use slint::platform::{Key, WindowEvent};
use slint::{ModelRc, VecModel};
use std::cell::RefCell;

type Pixels = slint::SharedPixelBuffer<slint::Rgb8Pixel>;

fn save(pixels: &Pixels, path: &str, w: u32, h: u32) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), w, h);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    println!("Rendered {path} ({w}×{h})");
    Ok(())
}

fn drawer<'a>(w: &'a MinimalSoftwareWindow, width: u32, height: u32) -> impl Fn() -> Pixels + 'a {
    move || {
        let mut pixels = Pixels::new(width, height);
        for _ in 0..2 {
            slint::platform::update_timers_and_animations();
            w.request_redraw();
            w.draw_if_needed(|r| {
                r.render(pixels.make_mut_slice(), width as usize);
            });
        }
        pixels
    }
}

fn px(p: &Pixels, w: u32, x: u32, y: u32) -> (u8, u8, u8) {
    let c = p.as_slice()[(y * w + x) as usize];
    (c.r, c.g, c.b)
}

/// How many pixels in the box are brighter than `floor` on every channel: text and the filled
/// button, which are the only light things on a darkened picture.
fn bright(p: &Pixels, w: u32, (x0, x1, y0, y1): (u32, u32, u32, u32), floor: u8) -> usize {
    (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| {
        let c = px(p, w, x, y);
        c.0 > floor && c.1 > floor && c.2 > floor
    }).count()
}

fn out_dir(output: &str) -> std::path::PathBuf {
    std::path::Path::new(output).parent().map(|p| p.to_path_buf()).unwrap_or_default()
}

fn mv(w: &MinimalSoftwareWindow, x: f32, y: f32) {
    w.dispatch_event(WindowEvent::PointerMoved { position: slint::LogicalPosition::new(x, y) });
}

pub fn run_lock(w: &MinimalSoftwareWindow, output: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let dir = out_dir(output);
    // The wallpaper is blurred once, by the same code the shell runs when a wallpaper is chosen.
    let cache = dir.join("lock-cache");
    lock_wallpaper::build("lake", &cache)?;
    let wallpaper = slint::Image::load_from_path(&cache.join("lock-wallpaper.png")).map_err(|_| "the blurred wallpaper did not load")?;
    let (fw, fh) = (width as f32, height as f32);

    // Geometry of the field row, from the layout: avatar 88 + name + gaps below the 50% line.
    let tall = height >= 700;
    let top = fh * if tall { 0.50 } else { 0.42 };
    let field_y = top + 88. + 12. + 22. + 12. + 26.;
    let row_left = (fw - 400.) / 2.;
    let eye = (row_left + 340. - 26., field_y);
    let go = (row_left + 340. + 8. + 26., field_y);
    let clock_y = (fh * 0.11) as u32;
    let (sy, sx_shut, sx_restart, sx_suspend) = (fh - 32. - 20., fw - 32. - 30., fw - 175., fw - 300.);
    let draw = drawer(w, width, height);

    // ── The compositor's lock screen, the real LockView ──
    let view = LockView::new()?;
    view.set_time("14:32".into());
    view.set_date("Friday, 2 October".into());
    view.set_user_name("Pranab".into());
    view.set_initial("P".into());
    view.set_prompt("Enter your password to unlock".into());
    view.set_wallpaper(wallpaper.clone());
    view.set_has_wallpaper(true);
    // The kind of connection, as the shell hands it over (`network_kind`): never its name.
    view.set_network("Wired".into());
    view.set_battery_percent(64);
    view.set_notifications(3);
    let asked: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    {
        let a = asked.clone();
        view.on_submit(move || a.borrow_mut().push("submit"));
        let a = asked.clone();
        view.on_suspend(move || a.borrow_mut().push("suspend"));
    }
    view.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    view.set_digits(7);
    let client = draw();
    save(&client, &output.replace(".png", "-client.png"), width, height)?;
    assert!(bright(&client, width, (width / 2 - 200, width / 2 + 200, clock_y, clock_y + 150), 200) > 400, "the client draws the same clock");
    // Bottom left, 24px in: a 16px glyph and "Wired" at 12px, in the light lock text.
    let net_box = (24, 24 + 90, height - 24 - 16, height - 24);
    assert!(bright(&client, width, net_box, 180) > 40, "the network corner is drawn bottom left");
    // Nothing in it takes a click: one there neither unlocks nor reaches main.rs.
    click(w, 40., fh - 32.);
    click(w, go.0, go.1);
    click(w, sx_suspend, sy);
    assert_eq!(*asked.borrow(), ["submit", "suspend"], "the client's buttons reach main.rs");
    click(w, eye.0, eye.1);
    assert!(view.get_revealed(), "the eye flips the view's own flag, which main.rs reads");
    view.set_revealed_text("hunter2".into());
    save(&draw(), &output.replace(".png", "-client-revealed.png"), width, height)?;
    view.hide()?;
    w.set_size(slint::PhysicalSize::new(width, height));

    // ── The shell's own screen ──
    let ui = App::new()?;
    ui.set_current_screen(3);
    ui.set_clock_text("14:32".into());
    ui.set_lock_date_text("Friday, 2 October".into());
    ui.set_lock_user_name("Pranab".into());
    ui.set_lock_initial("P".into());
    ui.set_lock_prompt("Enter your password to unlock".into());
    ui.set_lock_wallpaper(wallpaper.clone());
    ui.set_lock_has_wallpaper(true);
    ui.set_network_online(true);
    ui.set_network_medium("ethernet".into());
    ui.set_network_label("Ethernet".into());
    ui.set_network_detail("192.168.4.44".into());
    ui.set_battery_available(true);
    ui.set_battery_level(64);
    ui.set_notification_unread_count(3);
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        let l = log.clone();
        ui.on_try_unlock(move |s| l.borrow_mut().push(format!("unlock:{s}")));
        let l = log.clone();
        ui.on_power_action(move |a| l.borrow_mut().push(format!("power:{a}")));
    }
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    let empty = draw();
    save(&empty, output, width, height)?;

    // The picture: a large light clock, the lake behind it and not a flat fill, the avatar disc.
    assert!(bright(&empty, width, (width / 2 - 200, width / 2 + 200, clock_y, clock_y + 150), 200) > 400, "a large light clock is drawn");
    let corner = px(&empty, width, width / 8, height * 3 / 4);
    assert_ne!(corner, (0x10, 0x14, 0x17), "the wallpaper shows, not the solid charcoal");
    assert!(corner.2 >= corner.0, "the lake is blue-grey, not warm: {corner:?}");
    let net_box = (24, 24 + 90, height - 24 - 16, height - 24);
    assert!(bright(&empty, width, net_box, 180) > 40, "the shell's screen draws the network corner too");
    // Offline is said, not left blank; before the first reading nothing is said.
    ui.set_network_online(false);
    ui.set_network_detail("Offline".into());
    let offline = draw();
    save(&offline, &output.replace(".png", "-offline.png"), width, height)?;
    assert_ne!(offline.as_bytes(), empty.as_bytes(), "Offline is drawn in place of Wired");
    // (With no corner the notification count drops back into that spot, so it is put away here.)
    ui.set_notification_unread_count(0);
    ui.set_network_detail("".into());
    let unread = draw();
    assert_eq!(bright(&unread, width, net_box, 120), 0, "no reading, no network corner");
    ui.set_notification_unread_count(3);
    ui.set_network_online(true);
    ui.set_network_detail("192.168.4.44".into());
    assert_eq!(draw().as_bytes(), empty.as_bytes(), "back to Wired, the same frame");

    // At rest it draws nothing: no timer, no animation, nothing between clock ticks.
    mv(w, fw / 2., fh * 0.8);
    draw();
    let mut redraws = 0;
    let mut pixels = Pixels::new(width, height);
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        slint::platform::update_timers_and_animations();
        if w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); }) {
            redraws += 1;
        }
    }
    assert_eq!(redraws, 0, "a lock screen at rest asks for no frames");
    // Typing: the first key hands the keyboard to the input (it is not a character, as it never
    // was); what follows is the entry, drawn as dots.
    key(w, "x".into());
    for c in "hunter2".chars() {
        key(w, c.to_string().into());
    }
    let dots = draw();
    save(&dots, &output.replace(".png", "-typed.png"), width, height)?;
    assert_ne!(empty.as_bytes(), dots.as_bytes(), "typing changes the field");
    // The eye shows what was typed; the same click hides it again.
    click(w, eye.0, eye.1);
    let shown = draw();
    // Compared away from the eye itself, whose hover wash changes with the pointer: the dots
    // become letters, so the left of the field must differ.
    let field = |p: &Pixels| -> Vec<(u8, u8, u8)> {
        let (x0, y0) = (row_left as u32 + 8, field_y as u32 - 8);
        (y0..y0 + 16).flat_map(|y| (x0..x0 + 120).map(move |x| (x, y))).map(|(x, y)| px(p, width, x, y)).collect()
    };
    save(&shown, &output.replace(".png", "-revealed.png"), width, height)?;
    assert_ne!(field(&dots), field(&shown), "the eye shows the entry");
    click(w, eye.0, eye.1);
    mv(w, fw / 2., fh * 0.8); // off the button, over bare wallpaper, so no hover is part of the comparison
    let hidden_again = draw();
    let differing: Vec<(u32, u32)> = (0..height).flat_map(|y| (0..width).map(move |x| (x, y))).filter(|&(x, y)| px(&dots, width, x, y) != px(&hidden_again, width, x, y)).collect();
    if !differing.is_empty() {
        println!("DIFF {} pixels differ, from {:?} to {:?}", differing.len(), differing.first(), differing.last());
        let (x, y) = differing[0];
        println!("DIFF before {:?} after {:?}; beside: {:?} {:?}", px(&dots, width, x, y), px(&hidden_again, width, x, y), px(&dots, width, x - 1, y), px(&dots, width, x + 1, y));
    }
    assert!(differing.is_empty(), "and hides it again, pixel for pixel");

    // Enter and the arrow both send what was typed, once, and empty the field.
    key(w, Key::Return.into());
    assert_eq!(log.borrow().last().map(String::as_str), Some("unlock:hunter2"), "Enter sends the entry: {:?}", log.borrow());
    for c in "abc".chars() {
        key(w, c.to_string().into());
    }
    click(w, go.0, go.1);
    assert_eq!(log.borrow().last().map(String::as_str), Some("unlock:abc"), "the arrow sends the entry: {:?}", log.borrow());
    let unlocks = log.borrow().iter().filter(|l| l.starts_with("unlock:")).count();
    assert_eq!(unlocks, 2, "one send per Enter or click");
    assert_eq!(draw().as_bytes(), empty.as_bytes(), "the field is empty after a send");

    // Suspend acts at once. Restart and Shut down ask once more: the first click only arms.
    click(w, sx_suspend, sy);
    assert_eq!(log.borrow().last().map(String::as_str), Some("power:suspend"), "{:?}", log.borrow());
    click(w, sx_restart, sy);
    assert!(!log.borrow().iter().any(|l| l == "power:restart"), "one click on Restart does not restart");
    save(&draw(), &output.replace(".png", "-confirm.png"), width, height)?;
    click(w, sx_restart, sy);
    assert_eq!(log.borrow().last().map(String::as_str), Some("power:restart"), "{:?}", log.borrow());
    click(w, sx_shut, sy);
    assert!(!log.borrow().iter().any(|l| l == "power:shutdown"), "one click on Shut down does not shut down");
    click(w, sx_shut, sy);
    assert_eq!(log.borrow().last().map(String::as_str), Some("power:shutdown"), "{:?}", log.borrow());

    // An error is said in words, in the colour for it.
    ui.set_lock_error("Wrong password. Try again in 4 s".into());
    let wrong = draw();
    save(&wrong, &output.replace(".png", "-wrong.png"), width, height)?;
    ui.set_lock_error("".into());

    // No wallpaper: the solid charcoal, still a lock screen.
    ui.set_lock_has_wallpaper(false);
    let solid = draw();
    // The scrim (#080b0e at 55%) over the charcoal (#101417), within a count of rounding.
    let corner = px(&solid, width, 8, 400);
    assert!(
        [(corner.0, 11u8), (corner.1, 15), (corner.2, 17)].iter().all(|&(got, want)| got.abs_diff(want) <= 1),
        "the scrim over the charcoal: {corner:?}"
    );
    ui.set_lock_has_wallpaper(true);
    draw();

    ui.hide()?;

    println!("PASS: lock screen at {width}x{height}: clock, wallpaper, avatar, typed dots, eye, send, power confirm, error, solid fallback, idle");
    Ok(())
}

/// The theme cards' colours, from the theme files themselves: the card must show what the theme
/// has, so the fixture reads the same files the shell embeds.
pub(crate) fn theme_card(file: &str, id: &str, wallpaper: &str) -> ThemeCardData {
    let value = |key: &str| -> slint::Color {
        // The last one: `accent` is also a top-level preset id, before the palette's own.
        let line = file.lines().filter(|l| l.trim_start().starts_with(&format!("{key} ="))).last().unwrap_or_else(|| panic!("{id}: no {key}"));
        let hex = line.split('"').nth(1).unwrap().trim_start_matches('#');
        let n = u32::from_str_radix(hex, 16).unwrap();
        slint::Color::from_rgb_u8((n >> 16) as u8, (n >> 8) as u8, n as u8)
    };
    let text = |key: &str| file.lines().find(|l| l.starts_with(&format!("{key} ="))).and_then(|l| l.split('"').nth(1)).unwrap_or_default().to_string();
    ThemeCardData {
        id: id.into(),
        name: text("name").into(),
        description: text("description").into(),
        preview: lock_wallpaper::preview_image(wallpaper).expect("a preview"),
        ground: value("bg_deep"),
        surface: value("bg_surface"),
        text: value("text_primary"),
        accent: value("accent"),
    }
}

pub fn run_themes(w: &MinimalSoftwareWindow, output: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let ui = SettingsProbe::new()?;
    ui.set_canvas_width(width as f32);
    ui.set_canvas_height(height as f32);
    let all: Vec<_> = ["Appearance", "AI & Intelligence", "Desktop", "Network", "Accounts", "Privacy & Security", "System", "Skills", "Harnesses"]
        .iter().enumerate().map(|(i, s)| SettingsCategoryItem { id: i as i32, label: (*s).into(), icon: "".into() }).collect();
    ui.set_categories(ModelRc::new(VecModel::from(all)));
    ui.set_themes(ModelRc::new(VecModel::from(vec![
        theme_card(include_str!("../../../crates/yantrik-design-tokens/themes/lake.toml"), "lake", "lake"),
        theme_card(include_str!("../../../crates/yantrik-design-tokens/themes/nightfall.toml"), "nightfall", "nightfall"),
    ])));
    ui.set_category(0);
    ui.set_dark(true);
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    let draw = drawer(w, width, height);
    let first = draw();
    save(&first, output, width, height)?;

    // Lake is chosen; pressing Nightfall's card chooses it (and only reports: wiring is Rust's).
    let cy = 270.;
    click(w, 460., cy);
    assert_eq!(ui.get_action(), "theme:lake", "a click on the first card chooses it");
    click(w, 840., cy);
    assert_eq!(ui.get_action(), "theme:nightfall", "a click on the second card chooses it");
    ui.set_theme("nightfall".into());
    save(&draw(), &output.replace(".png", "-nightfall.png"), width, height)?;
    println!("PASS: theme cards at {width}x{height}: both places drawn from their files, a click chooses one");
    Ok(())
}

pub fn run_desktop(w: &MinimalSoftwareWindow, output: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let ui = App::new()?;
    ui.set_current_screen(1);
    ui.set_wallpaper_path("lake".into());
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    let draw = drawer(w, width, height);
    let lake = draw();
    save(&lake, output, width, height)?;
    // The photograph shows between the bar and the dock: many distinct colours, blue-grey on
    // average, not a flat wash.
    let mut distinct = std::collections::HashSet::new();
    let (mut r, mut b, mut n) = (0u64, 0u64, 0u64);
    for y in (height / 4..height * 3 / 4).step_by(7) {
        for x in (0..width).step_by(7) {
            let c = px(&lake, width, x, y);
            distinct.insert((c.0 / 4, c.1 / 4, c.2 / 4));
            r += u64::from(c.0);
            b += u64::from(c.2);
            n += 1;
        }
    }
    assert!(distinct.len() > 150, "a photograph, not a flat fill: {} colours", distinct.len());
    assert!(b > r, "the lake is blue-grey (mean red {} blue {})", r / n, b / n);
    ui.set_wallpaper_path("serenity".into());
    assert_ne!(lake.as_bytes(), draw().as_bytes(), "the other wallpapers are still selectable and different");
    println!("PASS: the desktop's default wallpaper is the lake photograph at {width}x{height}");
    Ok(())
}
