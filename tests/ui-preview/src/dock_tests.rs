//! The grounded dock (design/minds-surfaces-spec §3), drawn by the whole shell — app.slint's `App`
//! — and pressed with real pointer events. Four scenes, each saved as a PNG for a person to look at:
//!
//!   dock-five.png       5 apps, one in front, one with 3 windows; grounded, centred, as wide as its buttons
//!   dock-paged.png      20 apps: icons keep their size, the dock stops at 880px and pages with "› +4"
//!   dock-list.png       the window list over the app with 3 windows, and a click on a row activates it
//!   dock-needs-you.png  a mind is waiting: the one amber dot in the dock
//!
//! Then the proof that it costs nothing at rest: no redraw is requested over 1.2s, with the window
//! list left open.
use super::*;
use slint::platform::WindowEvent;
use slint::{ModelRc, VecModel};
use std::cell::RefCell;

type Pixels = slint::SharedPixelBuffer<slint::Rgb8Pixel>;
const W: u32 = 1280;
const H: u32 = 800;
// Theme.dock-bar and Theme.needs-you, dark mode.
const BAR: (u8, u8, u8) = (0x10, 0x14, 0x17);
const AMBER: (u8, u8, u8) = (0xE7, 0xB5, 0x67);

fn save(pixels: &Pixels, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), W, H);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    println!("Rendered {path} ({W}×{H})");
    Ok(())
}

fn px(p: &Pixels, x: u32, y: u32) -> (u8, u8, u8) {
    let c = p.as_slice()[(y * W + x) as usize];
    (c.r, c.g, c.b)
}

fn count(p: &Pixels, region: (u32, u32, u32, u32), want: (u8, u8, u8)) -> usize {
    count_within(p, region, want, 0)
}

fn count_within(p: &Pixels, region: (u32, u32, u32, u32), want: (u8, u8, u8), slack: i16) -> usize {
    let (x0, x1, y0, y1) = region;
    (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| {
        let c = px(p, x, y);
        let near = |a: u8, b: u8| (a as i16 - b as i16).abs() <= slack;
        near(c.0, want.0) && near(c.1, want.1) && near(c.2, want.2)
    }).count()
}

fn button(app: &str, label: &str, running: bool, focused: bool, windows: i32) -> DockButton {
    DockButton {
        app_id: app.into(),
        label: label.into(),
        icon_id: app.into(),
        has_icon: false,
        icon: Default::default(),
        pinned: true,
        running,
        focused,
        windows,
        title: format!("{label} window").into(),
    }
}

fn mv(w: &MinimalSoftwareWindow, x: f32, y: f32) {
    w.dispatch_event(WindowEvent::PointerMoved { position: slint::LogicalPosition::new(x, y) });
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::path::Path::new(output).parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let out = |name: &str| dir.join(name).to_string_lossy().to_string();

    let ui = App::new()?;
    ui.set_current_screen(1);
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(W, H));
    let draw = || {
        let mut pixels = Pixels::new(W, H);
        for _ in 0..2 {
            slint::platform::update_timers_and_animations();
            w.request_redraw();
            w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), W as usize); });
        }
        pixels
    };
    // Real time passes for the hover delays (500ms label, 250ms list): the timers read the clock.
    let wait = |ms: u64| {
        std::thread::sleep(std::time::Duration::from_millis(ms));
        slint::platform::update_timers_and_animations();
    };
    let set_buttons = |b: Vec<DockButton>| ui.set_dock_buttons(ModelRc::new(VecModel::from(b)));
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        let l = log.clone();
        ui.on_taskbar_window_clicked(move |t| l.borrow_mut().push(format!("activate:{t}")));
        let l = log.clone();
        ui.on_launch_app(move |a| l.borrow_mut().push(format!("launch:{a}")));
    }
    let row_y = H - 24;

    // ── 1. Five apps: Files, Notes (in front), Terminal (3 windows), Browser, Calendar ──
    set_buttons(vec![
        button("files", "Files", false, false, 0),
        button("notes", "Notes", true, true, 1),
        button("terminal", "Terminal", true, false, 3),
        button("browser", "Browser", false, false, 0),
        button("calendar", "Calendar", false, false, 0),
    ]);
    ui.set_dock_windows(ModelRc::new(VecModel::from(vec![
        DockWindow { app_id: "notes".into(), title: "Notes window".into(), workspace: "".into(), current: true },
        DockWindow { app_id: "terminal".into(), title: "Terminal: build".into(), workspace: "".into(), current: false },
        DockWindow { app_id: "terminal".into(), title: "Terminal: logs".into(), workspace: "".into(), current: false },
        DockWindow { app_id: "terminal".into(), title: "Terminal: ssh".into(), workspace: "".into(), current: false },
    ])));
    let five = draw();
    // Where the bar is: the span of its own colour along a row well below the rounded corners.
    let span = |p: &Pixels, y: u32| -> (u32, u32) {
        let xs: Vec<u32> = (0..W).filter(|&x| px(p, x, y) == BAR).collect();
        (*xs.first().expect("the bar is drawn"), *xs.last().unwrap())
    };
    assert_ne!(px(&five, 40, row_y), BAR, "the desktop is not the bar's colour, so the span below means something");
    let (l, r) = span(&five, row_y);
    let width = r - l + 3; // plus the 1px border each side, and the pixel itself
    // 5 apps: padding 8 + Apps, five, divider, mind (8 children, 7 gaps of 4) = 8 + 281 + 28.
    assert!((width as i32 - 317).abs() <= 3, "the dock is exactly as wide as its buttons: {width}px, expected 317");
    let centre = (l + r) / 2;
    assert!((centre as i32 - 640).abs() <= 2, "and centred: its middle is at x={centre}");
    assert_eq!(px(&five, centre, H - 1), BAR, "its bottom edge is the screen's bottom edge, with no border under it");
    assert_eq!(px(&five, l, H - 1), BAR, "the bottom corners are square");
    assert_ne!(px(&five, l - 1, 752), BAR, "and the top corners are round: nothing is drawn in the corner's cut");
    assert_ne!(px(&five, centre, 751), BAR, "the dock is 48px tall: nothing above y=752");
    assert_eq!(px(&five, centre, 752), (0x34, 0x3B, 0x41), "a 1px neutral border runs along its top");
    // Notes (second button) is in front: a raised fill under it that Files lacks.
    let btn = |i: u32| l + 1 + 4 + 44 * (i + 1) + 20; // Apps is slot 0
    assert_ne!(px(&five, btn(1) - 17, row_y), px(&five, btn(0) - 17, row_y), "the button in front has the raised neutral fill");
    save(&five, &out("dock-five.png"))?;

    // A click on an app that is not open launches it; on a single window activates that window;
    // on several, opens the list instead of choosing for the person.
    click(w, btn(0) as f32, row_y as f32);
    draw();
    click(w, btn(1) as f32, row_y as f32);
    draw();
    assert_eq!(*log.borrow(), ["launch:files", "activate:Notes window"], "a closed app launches, a single window activates");
    log.borrow_mut().clear();
    mv(w, 5.0, 300.0);
    draw();

    // ── 2. Twenty apps: paging, never shrinking ──
    let names = ["files", "notes", "terminal", "browser", "calendar", "email", "music", "image", "weather", "settings", "editor", "sysmonitor", "downloads", "network", "containers", "documents", "spreadsheet", "presentation", "snippets", "studio"];
    set_buttons(names.iter().enumerate().map(|(i, n)| button(n, n, i < 3, i == 1, if i == 2 { 3 } else { i32::from(i < 3) })).collect());
    let paged = draw();
    assert_eq!(ui.get_dock_capacity(), 16, "880px holds sixteen 40px buttons beside the page buttons");
    let (l, r) = span(&paged, row_y);
    let width = r - l + 3;
    assert!((860..=880).contains(&width), "the dock stops growing at 880px: {width}px");
    save(&paged, &out("dock-paged.png"))?;
    let next_x = l + 1 + 4 + 44 + 36 + 16 * 44 + 16 + 1; // Apps, the back page button, sixteen apps
    click(w, next_x as f32, row_y as f32);
    let page_two = draw();
    assert_eq!(ui.get_dock_first(), 4, "the last page is full: it starts at 4, not 16");
    save(&page_two, &out("dock-paged-2.png"))?;
    // Back: the wheel pages over the WHOLE bar, not at one spot (#585 S3). Scroll up at the Apps
    // button, on a middle app, on the page buttons, in the gap beside the divider and on the mind
    // button: every one must go back a page. The sweep that used to stand here only logged where
    // the wheel landed and asserted nothing.
    let mut soft: Vec<String> = Vec::new();
    let (pl, pr) = span(&page_two, row_y);
    let spots: [(&str, u32); 5] = [
        ("the Apps button", pl + 1 + 4 + 20),
        ("an app in the middle", pl + 1 + 4 + 44 * 3 + 20),
        ("the back page button", pl + 1 + 4 + 44 + 16),
        ("the page-on button", pl + 1 + 4 + 44 + 36 + 16 * 44 + 16),
        ("the mind button", pr - 4 - 20),
    ];
    for (what, wx) in spots {
        ui.set_dock_first(4);
        draw();
        mv(w, wx as f32, row_y as f32);
        draw();
        w.dispatch_event(WindowEvent::PointerScrolled { position: slint::LogicalPosition::new(wx as f32, row_y as f32), delta_x: 0.0, delta_y: 60.0 });
        draw();
        if ui.get_dock_first() >= 4 { soft.push(format!("the wheel over {what} (x={wx}) did not page back: first={}", ui.get_dock_first())); }
    }
    ui.set_dock_first(0);
    // Alt+Tab to an app on another page reveals that page.
    ui.set_dock_reveal_index(18);
    draw();
    if ui.get_dock_first() != 4 { soft.push(format!("an app brought to the front on another page reveals that page (first={})", ui.get_dock_first())); }
    ui.set_dock_reveal_index(-1);
    ui.set_dock_first(0);

    // ── 3. The window list ──
    set_buttons(vec![
        button("files", "Files", false, false, 0),
        button("notes", "Notes", true, true, 1),
        button("terminal", "Terminal", true, false, 3),
        button("browser", "Browser", false, false, 0),
        button("calendar", "Calendar", false, false, 0),
    ]);
    let base = draw();
    let (l, _) = span(&base, row_y);
    let term = (l + 1 + 4 + 44 * 3 + 20) as f32;
    // Hover on a single-window app: after 500ms its name appears, 8px above the dock.
    mv(w, (l + 1 + 4 + 44 * 2 + 20) as f32, row_y as f32);
    draw();
    wait(560);
    let label = draw();
    assert!(count(&label, (500, 780, 700, 750), (0x15, 0x1A, 0x1E)) > 300, "the name label (opaque panel) is up after 500ms");
    mv(w, term, row_y as f32);
    draw();
    draw(); // (not asserted: a debug-build frame can outlast the 250ms before the list opens)
    // Hover on the app with 3 windows: after 250ms the list opens instead of the label.
    wait(300);
    let listed = draw();
    // 40px header + 3 × 56px rows, 8px above the dock's top edge at y=752: the panel spans 536..744.
    assert!(count(&listed, (term as u32 - 150, term as u32 + 150, 540, 740), (0x15, 0x1A, 0x1E)) > 20_000, "the window list is 320px wide, over the dock");
    assert_eq!(px(&listed, term as u32, 745), px(&base, term as u32, 745), "and ends 8px above the dock");
    save(&listed, &out("dock-list.png"))?;
    // A click on the second row activates that window.
    mv(w, term, 536.0 + 40.0 + 56.0 + 28.0);
    draw();
    click(w, term, 536.0 + 40.0 + 56.0 + 28.0);
    draw();
    assert_eq!(*log.borrow(), ["activate:Terminal: logs"], "a click on a row activates that window: {:?}", log.borrow());
    log.borrow_mut().clear();
    // Click on the multi-window app opens the list and stays; Escape puts it away.
    mv(w, 5.0, 300.0);
    wait(300);
    draw();
    click(w, term, row_y as f32);
    let held = draw();
    assert!(count(&held, (term as u32 - 150, term as u32 + 150, 540, 740), (0x15, 0x1A, 0x1E)) > 20_000, "a click on an app with several windows opens the list");
    assert!(log.borrow().is_empty(), "and does not choose a window for the person");

    // At rest with the list open: nothing asks for a frame.
    let mut redraws = 0;
    for _ in 0..12 {
        slint::platform::update_timers_and_animations();
        if w.draw_if_needed(|r| { r.render(Pixels::new(W, H).make_mut_slice(), W as usize); }) {
            redraws += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(redraws, 0, "the dock at rest, list open, requests no redraws");
    key(w, slint::platform::Key::Escape.into());
    let closed = draw();
    assert_eq!(count(&closed, (term as u32 - 150, term as u32 + 150, 540, 740), (0x15, 0x1A, 0x1E)), 0, "Escape puts the list away");

    // ── 4. A mind needs the person ──
    mv(w, 5.0, 300.0);
    let calm = draw();
    let mind = (l + 1 + 4 + 44 * 6 + 5 + 20) as u32; // centre of the mind button
    let zone = (mind - 30, mind + 30, 752, 800);
    assert_eq!(count_within(&calm, zone, AMBER, 24), 0, "no amber in the dock while no mind is waiting");
    ui.set_cards_pending(2);
    let needs = draw();
    save(&needs, &out("dock-needs-you.png"))?;
    let dot = count_within(&needs, zone, AMBER, 24);
    assert!((12..=40).contains(&dot), "one 6px amber dot on the mind button: {dot} pixels");
    // (The Notes tile is itself a yellow-orange, so "nowhere else" means nothing new elsewhere.)
    let elsewhere = |p: &Pixels| count_within(p, (0, mind - 30, 752, 800), AMBER, 24) + count_within(p, (mind + 30, W, 752, 800), AMBER, 24);
    assert_eq!(elsewhere(&needs), elsewhere(&calm), "and nothing else in the dock turned amber");
    save(&needs, &out("dock-needs-you.png"))?;
    ui.set_cards_pending(0);
    assert_eq!(count_within(&draw(), zone, AMBER, 24), 0, "answered: the dot goes");

    // And at rest again, nothing hovered, no list: still no frames.
    let mut redraws = 0;
    for _ in 0..12 {
        slint::platform::update_timers_and_animations();
        if w.draw_if_needed(|r| { r.render(Pixels::new(W, H).make_mut_slice(), W as usize); }) {
            redraws += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(redraws, 0, "the idle dock requests no redraws once settled");

    ui.hide()?;
    assert!(soft.is_empty(), "failed: {soft:?}");
    println!("PASS: the dock is grounded, centred and as wide as its buttons (317px for 5 apps, capped at 880px with 16 of 20 and a page button); focus is a raised fill; a closed app launches, one window activates, several open the list; the hover label and window list open after 500ms and 250ms and a click on a row activates; the wheel and reveal turn pages; one amber dot on the mind button, and 0 redraws at rest");
    Ok(())
}
