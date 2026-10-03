//! Quick Settings v2 and the power popover (UI overhaul PR 2.2 and 6.3), drawn by the whole shell
//! with real pointer and key events.
//!
//!   * the VM shape: a wired cable, no battery, no backlight, no power-profiles-daemon. Those
//!     controls are not drawn and the rest closes up;
//!   * the laptop shape: Wi-Fi, battery, backlight, power mode, everything;
//!   * the panel is 360px wide and hangs from the right edge at 800, 1280 and 1920;
//!   * the toggle and its chevron are separate targets; the arrows move, Space presses, Esc closes;
//!   * the power popover with 0 and with 2 minds working, and its ten second confirmation, which
//!     expires into "never mind" and never into the act.
//!
//! Callbacks are recorded here and never acted on; the Rust that acts on them is `wire/power.rs`
//! and `control_power.rs`, whose tests read their source.
use super::*;
use slint::platform::Key;
use slint::{ModelRc, VecModel};
use std::cell::RefCell;

type Pixels = slint::SharedPixelBuffer<slint::Rgb8Pixel>;

// Every check is recorded and the scene carries on, so one run reports every failure and leaves
// every picture behind (a render of this screen takes minutes to build, and a first failure that
// hid the rest would cost one build per mistake). `finish` fails the scene if any check did.
thread_local! { static FAILED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) }; }
fn fail(text: String) {
    eprintln!("FAIL: {text}");
    FAILED.with(|f| f.borrow_mut().push(text));
}
macro_rules! assert {
    ($c:expr $(,)?) => { if !$c { fail(format!("{} ({}:{})", stringify!($c), file!(), line!())); } };
    ($c:expr, $($a:tt)+) => { if !$c { fail(format!("{} ({}:{})", format!($($a)+), file!(), line!())); } };
}
macro_rules! assert_eq {
    ($l:expr, $r:expr $(,)?) => { { let (l, r) = (&$l, &$r); if *l != *r { fail(format!("{:?} != {:?} ({}:{})", l, r, file!(), line!())); } } };
    ($l:expr, $r:expr, $($a:tt)+) => { { let (l, r) = (&$l, &$r); if *l != *r { fail(format!("{}: {:?} != {:?} ({}:{})", format!($($a)+), l, r, file!(), line!())); } } };
}
macro_rules! assert_ne {
    ($l:expr, $r:expr, $($a:tt)+) => { { let (l, r) = (&$l, &$r); if *l == *r { fail(format!("{}: both {:?} ({}:{})", format!($($a)+), l, file!(), line!())); } } };
}
fn finish() -> Result<(), Box<dyn std::error::Error>> {
    let failed = FAILED.with(|f| std::mem::take(&mut *f.borrow_mut()));
    if failed.is_empty() { println!("PASS: Quick Settings v2 and the power popover"); Ok(()) } else { Err(format!("{} check(s) failed:\n  {}", failed.len(), failed.join("\n  ")).into()) }
}

fn save(pixels: &Pixels, path: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    println!("Rendered {path} ({width}x{height})");
    Ok(())
}

fn at(p: &Pixels, width: u32, x: u32, y: u32) -> (u8, u8, u8) {
    let px = p.as_slice()[(y * width + x) as usize];
    (px.r, px.g, px.b)
}

/// How many pixels in the rectangle differ between two frames.
fn changed(a: &Pixels, b: &Pixels, width: u32, x: (u32, u32), y: (u32, u32)) -> usize {
    (y.0..y.1)
        .flat_map(|y| (x.0..x.1).map(move |x| (y * width + x) as usize))
        .filter(|&i| a.as_slice()[i] != b.as_slice()[i])
        .count()
}

/// The wired VM: a cable, no Wi-Fi device.
fn wired(g: &NetworkState) {
    g.set_mark("wired".into());
    g.set_tooltip("Wired, connected, 192.168.4.44".into());
    g.set_kind("wired".into());
    g.set_online(true);
    g.set_ssid("".into());
    g.set_ip("192.168.4.44".into());
    g.set_wifi_present(false);
    g.set_radio_on(false);
    g.set_networks(ModelRc::new(VecModel::from(Vec::<NetworkRow>::new())));
}

/// A laptop on Wi-Fi.
fn wifi(g: &NetworkState) {
    g.set_mark("wifi".into());
    g.set_bars(3);
    g.set_tooltip("Wi-Fi Home, connected, 192.168.1.20".into());
    g.set_kind("wifi".into());
    g.set_online(true);
    g.set_ssid("Home".into());
    g.set_ip("192.168.1.20".into());
    g.set_wifi_present(true);
    g.set_radio_on(true);
    g.set_networks(ModelRc::new(VecModel::from(Vec::<NetworkRow>::new())));
}

fn vm_shape(ui: &App) {
    ui.set_battery_available(false);
    ui.set_brightness_available(false);
    ui.set_volume_available(true);
    ui.set_volume_level(45);
    ui.set_power_profile("".into());
    wired(&ui.global::<NetworkState>());
}

fn laptop_shape(ui: &App) {
    ui.set_battery_available(true);
    ui.set_battery_level(76);
    ui.set_brightness_available(true);
    ui.set_brightness_level(70);
    ui.set_volume_available(true);
    ui.set_volume_level(62);
    ui.set_power_profile("balanced".into());
    ui.set_power_performance_offered(false);
    wifi(&ui.global::<NetworkState>());
}

const TILE_OFF: (u8, u8, u8) = (0x1e, 0x25, 0x2b);
const ACCENT: (u8, u8, u8) = (0x8f, 0xb4, 0xe3);

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = App::new()?;
    ui.set_current_screen(1);
    ui.set_mind_mode("ask".into());
    ui.set_mind_mode_label("Ask".into());
    ui.set_settings_dark_mode(true);
    ui.set_dnd_mode(false);
    ui.show()?;

    let size = std::cell::Cell::new((1280u32, 800u32));
    let resize = |width: u32, height: u32| {
        size.set((width, height));
        w.set_size(slint::PhysicalSize::new(width, height));
    };
    let draw = || {
        let (width, height) = size.get();
        let mut pixels = Pixels::new(width, height);
        for _ in 0..3 {
            slint::platform::update_timers_and_animations();
            w.request_redraw();
            w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); });
        }
        pixels
    };
    let settle = || std::thread::sleep(std::time::Duration::from_millis(250));
    let park = || w.dispatch_event(slint::platform::WindowEvent::PointerMoved { position: slint::LogicalPosition::new(300.0, 500.0) });
    let named = |name: &str| std::path::Path::new(output).with_file_name(name).to_string_lossy().into_owned();
    let tap = |text: slint::SharedString| key(w, text);

    // What the panel asked for, recorded.
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        let l = log.clone(); ui.on_mind_mode_chosen(move |m| l.borrow_mut().push(format!("mode {m}")));
        let l = log.clone(); ui.on_mind_private_chosen(move |on| l.borrow_mut().push(format!("private {on}")));
        let l = log.clone(); ui.on_mind_view_chosen(move |on| l.borrow_mut().push(format!("view {on}")));
        let l = log.clone(); ui.on_toggle_dnd_mode(move || l.borrow_mut().push("dnd".into()));
        let l = log.clone(); ui.on_toggle_dark_mode(move || l.borrow_mut().push("dark".into()));
        let l = log.clone(); ui.on_set_power_profile(move |p| l.borrow_mut().push(format!("profile {p}")));
        let l = log.clone(); ui.on_take_screenshot(move || l.borrow_mut().push("screenshot".into()));
        let l = log.clone(); ui.on_lock_screen(move || l.borrow_mut().push("lock".into()));
        let l = log.clone(); ui.on_power_action(move |a| l.borrow_mut().push(format!("power {a}")));
        let l = log.clone(); ui.on_volume_mute_toggled(move || l.borrow_mut().push("mute".into()));
        let l = log.clone(); ui.on_brightness_changed(move |v| l.borrow_mut().push(format!("brightness {v}")));
    }
    let take = || std::mem::take(&mut *log.borrow_mut());

    // ── The VM shape and the laptop shape, at 1280×800 ──
    resize(1280, 800);
    park();
    let closed = draw();
    vm_shape(&ui);
    ui.set_quick_settings_open(true);
    let vm = draw();
    settle();
    let vm = { let _ = vm; draw() };
    save(&vm, &named("qs-vm-shape.png"), 1280, 800)?;
    laptop_shape(&ui);
    let laptop = draw();
    settle();
    let laptop = { let _ = laptop; draw() };
    save(&laptop, &named("qs-laptop-shape.png"), 1280, 800)?;
    assert!(changed(&closed, &laptop, 1280, (900, 1280), (36, 520)) > 30_000, "the panel is drawn at the right");
    // The panel hangs against the right margin: nothing of it is left of x=912 (1280 - 8 - 360).
    assert_eq!(changed(&closed, &laptop, 1280, (0, 900), (60, 700)), 0, "nothing is drawn left of the panel");

    // The laptop's panel is taller than the VM's: it has the network switch's twin (power mode),
    // the brightness row and the battery text. Compare the bottom edge of the panel in the
    // column at x=1000, the first row from the bottom where the panel colour ends.
    let bottom = |p: &Pixels| -> u32 {
        let panel = at(p, 1280, 930, 48);
        (60..800u32).rev().find(|&y| at(p, 1280, 930, y) == panel).unwrap_or(0)
    };
    assert!(bottom(&laptop) > bottom(&vm) + 30, "the laptop panel ({}) is taller than the VM's ({})", bottom(&laptop), bottom(&vm));

    // The state colours are the colour system's: an off tile is #1E252B, a lit tile is the accent.
    // Laptop, 1280: row 2 of the machine flow is Dark style (lit) at x 928..1088, DND (off) right.
    assert_eq!(at(&laptop, 1280, 1240, 266), TILE_OFF, "Do Not Disturb is off: the tile fill");
    assert_eq!(at(&laptop, 1280, 1070, 330), ACCENT, "Dark style is on: the accent");
    println!("ran: VM and laptop shapes, state colours");

    // ── 360px at 800, 1280 and 1920 ──
    for (width, height) in [(800u32, 600u32), (1280, 800), (1920, 1080)] {
        ui.set_quick_settings_open(false);
        resize(width, height);
        park();
        let before = draw();
        ui.set_quick_settings_open(true);
        draw();
        settle();
        let p = draw();
        let left = width - 8 - 360;
        // Inside the 16px top padding, clear of the rounded corners.
        let inside = at(&p, width, left + 20, 48);
        assert_ne!(inside, at(&before, width, left + 20, 48), "{width}: the panel covers its left edge");
        assert_eq!(inside, at(&p, width, width - 8 - 20, 48), "{width}: same panel fill at both ends of the top padding");
        assert_eq!(inside, (0x15, 0x1a, 0x1e), "{width}: the charcoal panel, #151A1E");
        assert_ne!(at(&p, width, left - 6, 100), inside, "{width}: the panel does not start further left than 360px");
        assert_eq!(at(&p, width, left - 6, 100), at(&before, width, left - 6, 100), "{width}: outside the panel is untouched");
        save(&p, &named(&format!("qs-w{width}.png")), width, height)?;
    }
    println!("ran: 360px at 800, 1280, 1920");
    ui.set_quick_settings_open(false);
    resize(1280, 800);
    park();

    // ── Pointer: the toggle and its chevron are separate targets ──
    take();
    ui.set_quick_settings_open(true);
    draw();
    settle();
    draw();
    // Mind mode is the first tile: x 928..1112, y 84..140; its chevron is the right 44px.
    click(w, 950.0, 112.0);
    draw();
    assert_eq!(take(), ["mode plan"], "the body presses the tile, from Ask to Plan");
    assert!(ui.get_quick_settings_open() && !ui.get_mind_menu_open(), "a body press leaves the panel up and opens no menu");
    click(w, 1090.0, 112.0);
    draw();
    assert!(take().is_empty(), "the chevron is not the toggle");
    assert!(ui.get_mind_menu_open(), "the chevron opens the mode choice");
    assert!(!ui.get_quick_settings_open(), "and puts Quick Settings away");
    ui.set_mind_menu_open(false);
    println!("ran: toggle and chevron are separate targets");

    // ── Keyboard on the VM shape: arrows move, Space presses, Esc closes ──
    vm_shape(&ui);
    ui.set_quick_settings_open(true);
    draw();
    settle();
    draw();
    take();
    let down = |n: usize| for _ in 0..n { tap(Key::DownArrow.into()); draw(); };
    // 1 Mind mode, 2 Private, 3 Mind View, 4 Wired, 5 Do Not Disturb, 6 Dark style,
    // 7 Volume (there is no Power mode tile and no Bluetooth on this machine), 8 Screenshot.
    down(1);
    tap(" ".into());
    assert_eq!(take(), ["mode plan"], "Down then Space presses Mind mode");
    tap(Key::DownArrow.into());
    tap("\n".into());
    assert_eq!(take(), ["private true"], "Enter presses Private");
    tap(Key::RightArrow.into());
    tap(" ".into());
    assert_eq!(take(), ["view false"], "Right goes to the next control, and Space presses it");
    tap(Key::LeftArrow.into());
    tap(" ".into());
    assert_eq!(take(), ["private true"], "Left goes back");
    down(3); // Mind View, Wired, Do Not Disturb... from Private: view, wired, dnd
    tap(" ".into());
    assert_eq!(take(), ["dnd"], "the cable tile is a stop, and Do Not Disturb is the one after it");
    tap(Key::DownArrow.into());
    tap(" ".into());
    assert_eq!(take(), ["dark"]);
    tap(Key::DownArrow.into()); // Volume: Space does nothing to a slider
    tap(" ".into());
    assert!(take().is_empty(), "Space on a slider presses nothing");
    // The arrows are the slider's own (Down lowers the level), so Tab leaves it. The next stop is
    // the footer: the absent Power mode, Bluetooth and brightness are not stops.
    tap("\t".into());
    tap(" ".into());
    assert_eq!(take(), ["screenshot"], "after Volume the next stop is the footer");
    tap(Key::Escape.into());
    draw();
    assert!(!ui.get_quick_settings_open(), "Esc closes");
    println!("ran: keyboard");

    // The laptop has Power mode between Dark style and Volume.
    laptop_shape(&ui);
    ui.set_quick_settings_open(true);
    draw();
    settle();
    draw();
    take();
    // Mind mode, Private, Mind View, Wi-Fi, DND, Dark, Power mode: the seventh stop.
    for _ in 0..7 { tap(Key::DownArrow.into()); }
    tap(" ".into());
    assert_eq!(take(), ["profile power-saver"], "the seventh stop on a laptop is Power mode; Balanced steps to Power saver");

    // The footer: Lock and Power put Quick Settings away and do their own thing; Power opens the
    // power popover. (Laptop shape: the footer's four icons are at y=465, Power the last.)
    ui.set_quick_settings_open(false);
    ui.set_quick_settings_open(true);
    draw();
    settle();
    draw();
    take();
    click(w, 1204.0, 465.0);
    draw();
    assert_eq!(take(), ["lock"], "the footer's Lock locks");
    assert!(!ui.get_quick_settings_open(), "and puts Quick Settings away");
    ui.set_quick_settings_open(true);
    draw();
    settle();
    draw();
    click(w, 1240.0, 465.0);
    draw();
    assert!(!ui.get_quick_settings_open() && ui.get_power_menu_open(), "the footer's Power swaps Quick Settings for the power popover");
    ui.set_power_menu_open(false);

    // The speaker icon at the left of the Volume row mutes, as the media key does; a muted sink
    // draws the crossed speaker, and the level is still the machine's.
    ui.set_quick_settings_open(false);
    ui.set_quick_settings_open(true);
    draw();
    settle();
    let unmuted = draw();
    click(w, 936.0, 364.0);
    draw();
    assert_eq!(take(), ["mute"], "pressing the speaker asks to mute");
    ui.set_volume_muted(true);
    let muted = draw();
    save(&muted, &named("qs-laptop-muted.png"), 1280, 800)?;
    assert!(changed(&unmuted, &muted, 1280, (920, 960), (348, 380)) > 10, "a muted sink draws a different speaker");
    ui.set_volume_muted(false);
    ui.set_quick_settings_open(false);

    // ── Idle: a settled panel repaints nothing ──
    ui.set_quick_settings_open(true);
    draw();
    settle();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let (width, height) = size.get();
    let mut pixels = Pixels::new(width, height);
    for _ in 0..5 {
        slint::platform::update_timers_and_animations();
        w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); });
    }
    let mut redraws = 0;
    for _ in 0..10 {
        slint::platform::update_timers_and_animations();
        if w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); }) { redraws += 1; }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(redraws, 0, "a settled Quick Settings repaints nothing");
    ui.set_quick_settings_open(false);
    println!("ran: Quick Settings idle: {redraws} redraws over one second");

    // ── The power popover ──
    laptop_shape(&ui);
    ui.set_power_can_hibernate(false);
    ui.set_power_minds_working(0);
    park();
    let closed = draw();
    ui.set_power_menu_open(true);
    draw();
    settle();
    let list = draw();
    save(&list, &named("power-list.png"), 1280, 800)?;
    assert!(changed(&closed, &list, 1280, (900, 1280), (36, 330)) > 8_000, "the popover is drawn at the right, under the bar");

    // Hibernate is a row only where logind offers it.
    ui.set_power_can_hibernate(true);
    let with_hibernate = draw();
    assert!(changed(&list, &with_hibernate, 1280, (900, 1280), (150, 330)) > 1_000, "Hibernate appears when logind offers it");
    ui.set_power_can_hibernate(false);

    // Lock acts at once: Tab to the first row, Space.
    take();
    tap("\t".into());
    tap(" ".into());
    assert_eq!(take(), ["power lock"], "Lock is the first row, and acts without asking");
    assert!(!ui.get_power_menu_open(), "and puts the popover away");

    // Restart with nobody working: Cancel, Restart (red), and the countdown.
    ui.set_power_menu_open(true);
    draw();
    for _ in 0..4 { tap("\t".into()); } // Lock, Log out, Suspend, Restart
    tap(" ".into());
    draw();
    settle();
    let zero = draw();
    save(&zero, &named("power-confirm-0-minds.png"), 1280, 800)?;
    assert!(take().is_empty(), "Restart asks first: nothing was performed by choosing the row");
    assert!(changed(&list, &zero, 1280, (900, 1280), (50, 300)) > 5_000, "the confirmation replaces the list");

    // Enter on arrival is Cancel: focus starts on the safe button.
    tap("\n".into());
    draw();
    assert!(take().is_empty(), "Enter on arrival cancels, it does not restart");
    assert!(ui.get_power_menu_open(), "and returns to the list");
    let back = draw();
    assert_eq!(changed(&list, &back, 1280, (900, 1280), (50, 330)), 0, "the list is as it was");

    // Again, then Tab to the red button and Enter: performed once.
    for _ in 0..4 { tap("\t".into()); }
    tap(" ".into());
    draw();
    tap("\t".into());
    tap("\n".into());
    draw();
    assert_eq!(take(), ["power restart"], "Tab then Enter on the red button performs the act, once");
    assert!(!ui.get_power_menu_open());

    // Two minds working: the sentence and the two choices.
    ui.set_power_minds_working(2);
    ui.set_power_menu_open(true);
    draw();
    for _ in 0..5 { tap("\t".into()); } // ... Restart is 4th; Shut down 5th
    tap(" ".into());
    draw();
    settle();
    let two = draw();
    save(&two, &named("power-confirm-2-minds.png"), 1280, 800)?;
    assert!(take().is_empty());
    assert!(changed(&zero, &two, 1280, (900, 1280), (50, 330)) > 3_000, "with two minds working the confirmation says so");

    // The countdown ticks once a second and runs out into the list, not into the act.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let ticked = draw();
    assert!(changed(&two, &ticked, 1280, (900, 1280), (50, 330)) > 0, "the seconds count down");
    for _ in 0..11 {
        std::thread::sleep(std::time::Duration::from_millis(1000));
        draw();
    }
    let expired = draw();
    assert!(take().is_empty(), "an expired confirmation performs nothing");
    assert!(ui.get_power_menu_open(), "and leaves the popover up on its list");
    assert_eq!(changed(&list, &expired, 1280, (900, 1280), (50, 330)), 0, "it is back on the list");
    println!("ran: power popover, 0 and 2 minds working, confirmation expires into never mind");

    // Esc puts it away.
    tap(Key::Escape.into());
    draw();
    assert!(!ui.get_power_menu_open(), "Esc closes the popover");
    finish()
}
