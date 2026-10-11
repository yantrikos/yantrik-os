//! The top bar and Today (UI overhaul story 3.1), drawn on the real shell.
//!
//! The bar is render 01's: a 36px opaque #101417 strip with a 1px edge, the Yantrik mark and ONE
//! Minds chip on the left, the clock in the middle of the SCREEN, and the machine's indicators on
//! the right. This draws it with nothing needing the person and with two minds needing them, checks
//! the clock is centred at 1280 and at 1920 (the two widths where "centred on the leftover space"
//! and "centred on the screen" part company the most), then opens Today with a month, events and
//! five notifications and turns Do Not Disturb on. Every PNG is for looking at; the asserts are the
//! parts a pixel check can hold: the amber is there only when something needs the person, the clock's
//! middle is the screen's middle, the bar is opaque, Today is centred and fits the screen, and a
//! settled shell with Today open repaints nothing.
//!
//! Callbacks are recorded, never acted on: a click on the chip, the clock, the switch and a
//! notification's button is proven to reach its callback.
use super::*;
use slint::{ModelRc, VecModel};
use std::cell::{Cell, RefCell};

type Pixels = slint::SharedPixelBuffer<slint::Rgb8Pixel>;

fn draw(w: &MinimalSoftwareWindow, width: u32, height: u32) -> Pixels {
    let mut pixels = Pixels::new(width, height);
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); });
    }
    pixels
}

/// A moment for the 120ms pop and the 100ms washes to finish, then a frame.
fn settled(w: &MinimalSoftwareWindow, width: u32, height: u32) -> Pixels {
    std::thread::sleep(std::time::Duration::from_millis(350));
    draw(w, width, height)
}

fn save(pixels: &Pixels, path: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    println!("Rendered {path} ({width}x{height})");
    Ok(())
}

/// A rectangle of the frame, magnified.
fn save_crop(pixels: &Pixels, full_w: u32, path: &str, x: (u32, u32), y: (u32, u32), zoom: u32) -> Result<(), Box<dyn std::error::Error>> {
    let (cw, ch) = ((x.1 - x.0) * zoom, (y.1 - y.0) * zoom);
    let mut out = Vec::with_capacity((cw * ch * 3) as usize);
    for oy in 0..ch {
        for ox in 0..cw {
            let p = pixels.as_slice()[((y.0 + oy / zoom) * full_w + x.0 + ox / zoom) as usize];
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

fn px(p: &Pixels, width: u32, x: u32, y: u32) -> (u8, u8, u8) {
    let c = p.as_slice()[(y * width + x) as usize];
    (c.r, c.g, c.b)
}

/// Pixels near a colour inside a rectangle.
fn near(p: &Pixels, width: u32, x: (u32, u32), y: (u32, u32), want: (u8, u8, u8), tol: i32) -> usize {
    let mut n = 0;
    for yy in y.0..y.1 {
        for xx in x.0..x.1 {
            let (r, g, b) = px(p, width, xx, yy);
            if (r as i32 - want.0 as i32).abs() <= tol && (g as i32 - want.1 as i32).abs() <= tol && (b as i32 - want.2 as i32).abs() <= tol {
                n += 1;
            }
        }
    }
    n
}

/// The box around the bright (text-coloured) pixels in a rectangle: `(left, right)`.
fn bright_extent(p: &Pixels, width: u32, x: (u32, u32), y: (u32, u32)) -> Option<(u32, u32)> {
    let mut found: Option<(u32, u32)> = None;
    for yy in y.0..y.1 {
        for xx in x.0..x.1 {
            let (r, g, b) = px(p, width, xx, yy);
            if r > 170 && g > 170 && b > 170 {
                found = Some(match found { None => (xx, xx), Some((l, rt)) => (l.min(xx), rt.max(xx)) });
            }
        }
    }
    found
}

/// The first and last row of a rectangle where the frame differs from another.
fn changed_rows(a: &Pixels, b: &Pixels, width: u32, x: (u32, u32), y: (u32, u32)) -> Option<(u32, u32)> {
    let mut rows: Option<(u32, u32)> = None;
    for yy in y.0..y.1 {
        for xx in x.0..x.1 {
            if a.as_slice()[(yy * width + xx) as usize] != b.as_slice()[(yy * width + xx) as usize] {
                rows = Some(match rows { None => (yy, yy), Some((f, l)) => (f.min(yy), l.max(yy)) });
                break;
            }
        }
    }
    rows
}

fn changed_cols(a: &Pixels, b: &Pixels, width: u32, x: (u32, u32), y: (u32, u32)) -> Option<(u32, u32)> {
    let mut cols: Option<(u32, u32)> = None;
    for xx in x.0..x.1 {
        for yy in y.0..y.1 {
            if a.as_slice()[(yy * width + xx) as usize] != b.as_slice()[(yy * width + xx) as usize] {
                cols = Some(match cols { None => (xx, xx), Some((f, l)) => (f.min(xx), l.max(xx)) });
                break;
            }
        }
    }
    cols
}

fn changed(a: &Pixels, b: &Pixels, width: u32, x: (u32, u32), y: (u32, u32)) -> usize {
    (y.0..y.1)
        .flat_map(|y| (x.0..x.1).map(move |x| (y * width + x) as usize))
        .filter(|&i| a.as_slice()[i] != b.as_slice()[i])
        .count()
}

/// October 2026: the 1st is a Thursday, so four blanks, thirty-one days, today the 2nd.
fn october() -> ModelRc<CalendarDay> {
    let mut cells = Vec::new();
    for i in 0..42 {
        let day = i as i32 - 4 + 1;
        let real = (1..=31).contains(&day);
        cells.push(CalendarDay {
            day_number: if real { day } else { 0 },
            is_today: day == 2,
            is_selected: false,
            is_current_month: real,
            has_events: matches!(day, 2 | 8 | 15 | 21),
            event_count: if matches!(day, 2 | 8 | 15 | 21) { 1 } else { 0 },
        });
    }
    ModelRc::new(VecModel::from(cells))
}

fn note(id: &str, app: &str, summary: &str, body: &str, ago: &str, read: bool, actions: &[(&str, &str)]) -> NotificationData {
    NotificationData {
        id: id.into(),
        app_name: app.into(),
        summary: summary.into(),
        body: body.into(),
        urgency: 1,
        time_ago: ago.into(),
        is_read: read,
        is_group_header: false,
        group_name: app.into(),
        group_icon: "".into(),
        group_count: 0,
        group_unread: 0,
        is_turn_group: false,
        group_key: "".into(),
        expanded: false,
        in_group: false,
        actions: ModelRc::new(VecModel::from(
            actions.iter().map(|(i, l)| NotifActionData { id: (*i).into(), label: (*l).into() }).collect::<Vec<_>>(),
        )),
        source: "yantrik".into(),
        sender_line: "".into(),
        sender_short: "".into(),
    }
}

fn fill_today(ui: &App) {
    let g = ui.global::<TodayState>();
    g.set_weekday("Friday".into());
    g.set_date_line("2 October 2026".into());
    g.set_month_title("October 2026".into());
    g.set_days(october());
    g.set_events_state("ok".into());
    g.set_events(ModelRc::new(VecModel::from(vec![
        TodayEvent { title: "Stand-up".into(), time_text: "09:30 – 09:45".into() },
        TodayEvent { title: "Dentist".into(), time_text: "16:00 – 17:00".into() },
    ])));
    g.set_notifications(ModelRc::new(VecModel::from(vec![
        note("1", "Calendar", "Stand-up in 20 minutes", "Room 4 and online", "14:12", false, &[("snooze", "Snooze")]),
        note("2", "Updates", "System update ready", "Installs when you restart", "13:40", false, &[("restart", "Restart now"), ("later", "Later")]),
        note("3", "Downloads", "starfall.zip finished", "Saved in Downloads", "12:05", true, &[]),
        note("4", "Files", "Copied 12 items", "To Documents", "11:30", true, &[]),
        note("5", "Notes", "Reminder: call the bank", "", "09:02", true, &[]),
    ])));
}

/// The bar as a person sees it, at one width: full strip plus its two ends magnified.
fn look_at_bar(
    w: &MinimalSoftwareWindow,
    ui: &App,
    width: u32,
    height: u32,
    name: &str,
    output: &str,
) -> Result<Pixels, Box<dyn std::error::Error>> {
    w.set_size(slint::PhysicalSize::new(width, height));
    w.dispatch_event(slint::platform::WindowEvent::PointerMoved { position: slint::LogicalPosition::new(width as f32 / 2.0, height as f32 / 2.0) });
    let frame = settled(w, width, height);
    let path = |what: &str| output.replace(".png", &format!("-{name}-{what}.png"));
    save_crop(&frame, width, &path("bar"), (0, width), (0, 40), 1)?;
    save_crop(&frame, width, &path("left"), (0, 480), (0, 40), 3)?;
    save_crop(&frame, width, &path("right"), (width - 480, width), (0, 40), 3)?;
    let _ = ui;
    Ok(frame)
}

const BAR_BG: (u8, u8, u8) = (0x10, 0x14, 0x17);
const BAR_EDGE: (u8, u8, u8) = (0x34, 0x3B, 0x41);
const AMBER: (u8, u8, u8) = (0xE7, 0xB5, 0x67);
const TEAL: (u8, u8, u8) = (0x69, 0xC8, 0xBC);

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (mut width, mut height) = (1280u32, 800u32);
    let ui = App::new()?;
    ui.set_current_screen(1);
    ui.set_clock_text("14:32".into());
    ui.set_date_text("Fri 2 Oct".into());
    ui.set_battery_available(true);
    ui.set_battery_level(64);
    ui.set_battery_state("discharging".into());
    ui.set_volume_available(true);
    ui.set_volume_level(62);
    ui.set_mind_mode_label("Ask first".into());
    {
        let n = ui.global::<NetworkState>();
        n.set_mark("wifi".into());
        n.set_bars(4);
        n.set_kind("wifi".into());
        n.set_online(true);
        n.set_tooltip("Wi-Fi Harbor, connected".into());
        n.set_wifi_present(true);
        n.set_radio_on(true);
    }
    fill_today(&ui);

    // What the clicks reached.
    let section: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        let s = section.clone();
        ui.global::<AgentsState>().on_show_section(move |id| s.borrow_mut().push(id.to_string()));
    }
    let dnd_toggles: Rc<Cell<u32>> = Rc::default();
    {
        let n = dnd_toggles.clone();
        ui.on_toggle_dnd_mode(move || n.set(n.get() + 1));
    }
    let actions: Rc<RefCell<Vec<(String, String)>>> = Rc::default();
    {
        let a = actions.clone();
        ui.on_notification_action(move |id, action| a.borrow_mut().push((id.to_string(), action.to_string())));
    }
    let screens: Rc<RefCell<Vec<i32>>> = Rc::default();
    {
        let s = screens.clone();
        ui.on_navigate(move |screen| s.borrow_mut().push(screen));
    }
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    let path = |what: &str| output.replace(".png", &format!("-{what}.png"));
    let agents = ui.global::<AgentsState>();

    // ── Nothing needs the person ──
    agents.set_request_minds(0);
    agents.set_needs_count(0);
    let calm = look_at_bar(w, &ui, width, height, "calm-1280", output)?;
    save(&calm, &path("desktop-calm"), width, height)?;
    assert_eq!(px(&calm, width, 800, 10), BAR_BG, "the bar is the opaque charcoal, not glass over the wallpaper");
    assert_eq!(px(&calm, width, 800, 35), BAR_EDGE, "with a 1px neutral edge on its bottom row");
    assert_ne!(px(&calm, width, 800, 36), BAR_EDGE, "and nothing of the bar below it");
    assert_eq!(near(&calm, width, (0, 360), (0, 36), AMBER, 12), 0, "no amber on a bar where nothing needs the person");
    assert!(near(&calm, width, (0, 360), (0, 36), TEAL, 12) >= 8, "the Minds dot is teal");

    // ── Two minds need the person ──
    agents.set_request_minds(2);
    agents.set_needs_count(3);
    let busy = look_at_bar(w, &ui, width, height, "needs-you-1280", output)?;
    save(&busy, &path("desktop-needs-you"), width, height)?;
    let amber = near(&busy, width, (0, 360), (0, 36), AMBER, 12);
    assert!(amber >= 15, "'· 2 need you' is drawn in amber: {amber} amber pixels");
    // It grows the chip and nothing else on the left moves: the mark stays where it was.
    assert_eq!(changed(&calm, &busy, width, (0, 40), (0, 36)), 0, "the mark does not move when the count appears");

    // ── An approval card waits, from a caller that is no agent: the agents' counts are zero,
    // and the chip still says "1 approval" with its dot and words in amber (sign-off item 7) ──
    agents.set_request_minds(0);
    agents.set_needs_count(0);
    ui.set_cards_pending(1);
    let approval = look_at_bar(w, &ui, width, height, "approval-1280", output)?;
    save(&approval, &path("desktop-approval"), width, height)?;
    let amber = near(&approval, width, (0, 360), (0, 36), AMBER, 12);
    assert!(amber >= 15, "'1 approval' is drawn in amber with nothing in the agents' counts: {amber} amber pixels");
    assert!(changed(&calm, &approval, width, (40, 360), (0, 36)) > 0, "the chip reads differently from the calm one");
    ui.set_cards_pending(0);
    agents.set_request_minds(2);
    agents.set_needs_count(3);

    // ── The clock is under the screen's middle, at both widths ──
    for (sw, sh) in [(1280u32, 800u32), (1920, 1080)] {
        let frame = look_at_bar(w, &ui, sw, sh, &format!("clock-{sw}"), output)?;
        let (l, r) = bright_extent(&frame, sw, (sw / 2 - 150, sw / 2 + 150), (4, 32)).expect("the clock is drawn mid-bar");
        let centre = (l + r) as f32 / 2.0;
        println!("clock at {sw}: {l}..{r}, centre {centre} (screen centre {})", sw as f32 / 2.0);
        assert!((centre - sw as f32 / 2.0).abs() <= 3.0, "the clock is centred on the screen at {sw}, not on what the sides leave: its middle is {centre}");
        assert!(l > sw / 2 - 140 && r < sw / 2 + 140, "the clock sits well inside its box, so the box was not clipping it");
    }
    width = 1280;
    height = 800;
    w.set_size(slint::PhysicalSize::new(width, height));

    // ── The chip, the clock, the mark: each reaches its callback ──
    ui.set_current_screen(1);
    draw(w, width, height);
    // Press the chip: it is the first thing after the mark, around x=100.
    section.borrow_mut().clear();
    screens.borrow_mut().clear();
    click(w, 100.0, 18.0);
    draw(w, width, height);
    assert_eq!(section.borrow().last().map(String::as_str), Some("needs_you"), "with something waiting, the chip opens Agents on Needs you");
    assert_eq!(ui.get_current_screen(), 34, "and Agents is the screen");
    ui.set_current_screen(1);
    agents.set_request_minds(0);
    agents.set_needs_count(0);
    draw(w, width, height);
    click(w, 100.0, 18.0);
    assert_eq!(section.borrow().last().map(String::as_str), Some("workroom"), "with nothing waiting it opens the workroom, not an empty Needs you");
    ui.set_current_screen(1);
    agents.set_request_minds(1);
    agents.set_needs_count(3);

    // ── Today ──
    ui.set_current_screen(1);
    let before = settled(w, width, height);
    click(w, 640.0, 18.0);
    let open = settled(w, width, height);
    assert!(ui.get_today_open(), "a click on the clock opens Today");
    save(&open, &path("today"), width, height)?;
    let rows = changed_rows(&before, &open, width, (300, 980), (37, 800)).expect("Today is drawn");
    let cols = changed_cols(&before, &open, width, (300, 980), (37, 800)).expect("Today is drawn");
    println!("Today covers x {}..{}, y {}..{}", cols.0, cols.1, rows.0, rows.1);
    // 380px wide and centred on 640: its left edge is at 450 (the right edge's frame also shows
    // the desktop's own content under it changing, so the left edge is the clean measure).
    assert!((cols.0 as i32 - 450).abs() <= 2, "Today hangs from the clock, centred on the screen: its left edge is {}", cols.0);
    assert!(rows.0 >= 36 && rows.0 <= 48, "it opens just under the bar: top at {}", rows.0);
    assert!(rows.1 <= height - 48 - 8, "and stops above the 48px dock on an 800px screen: bottom at {}", rows.1);

    // Clicking the clock again puts it away; so does a click on the desktop.
    click(w, 640.0, 18.0);
    assert!(!ui.get_today_open(), "the clock closes Today it opened");
    click(w, 640.0, 18.0);
    assert!(ui.get_today_open());
    click(w, 100.0, 700.0);
    assert!(!ui.get_today_open(), "a click outside closes it");
    ui.set_today_open(true);
    settled(w, width, height);

    // The switch, found by pressing down the panel's middle until it fires.
    let mut fired_at = None;
    for y in (90..260).step_by(6) {
        click(w, 640.0, y as f32);
        if dnd_toggles.get() > 0 { fired_at = Some(y); break; }
    }
    assert!(fired_at.is_some(), "the Do Not Disturb switch reaches `toggle-dnd-mode`");
    // The owner flips the real thing; here the test is the owner.
    ui.set_dnd_mode(true);
    let dnd_on = settled(w, width, height);
    save(&dnd_on, &path("today-dnd-on"), width, height)?;
    save_crop(&dnd_on, width, &path("today-dnd-on-bar"), (800, 1280), (0, 40), 3)?;
    assert!(changed(&open, &dnd_on, width, (1000, 1280), (0, 36)) > 100, "do-not-disturb adds a moon to the bar");
    // The tile is the accent when on: accent pixels (not the dark tile) fill a good part of it.
    let tile = near(&dnd_on, width, (470, 810), (100, 180), (0x8B, 0xB4, 0xF0), 90);
    assert!(tile > 1500, "the switch is drawn in the accent when on: {tile} accent-ish pixels");

    // A notification's own button reaches the action callback, with the notification's id.
    let mut hit = false;
    'sweep: for y in (300..790).step_by(4) {
        for x in (480..800).step_by(10) {
            click(w, x as f32, y as f32);
            if actions.borrow().iter().any(|(id, a)| id == "1" && a == "snooze") { hit = true; break 'sweep; }
            if !ui.get_today_open() { ui.set_today_open(true); draw(w, width, height); }
        }
    }
    assert!(hit, "pressing Snooze on the first notification sends (1, snooze); the rest are a scroll away");

    // All notifications: the footer goes to the notification centre.
    ui.set_today_open(true);
    settled(w, width, height);
    screens.borrow_mut().clear();
    let mut went = false;
    'foot: for y in (500..795).rev().step_by(4) {
        for x in [520.0f32, 640.0, 760.0] {
            click(w, x, y as f32);
            if ui.get_current_screen() == 9 { went = true; break 'foot; }
            if !ui.get_today_open() { ui.set_today_open(true); draw(w, width, height); }
        }
    }
    assert!(went, "'All notifications' opens the notification centre (screen 9)");
    assert!(!ui.get_today_open(), "and puts Today away");

    // ── A short screen: Today scrolls its body instead of running off ──
    ui.set_current_screen(1);
    ui.set_dnd_mode(false);
    let (sw, sh) = (1024u32, 600u32);
    w.set_size(slint::PhysicalSize::new(sw, sh));
    let short_before = settled(w, sw, sh);
    ui.set_today_open(true);
    let short = settled(w, sw, sh);
    save(&short, &path("today-1024x600"), sw, sh)?;
    let rows = changed_rows(&short_before, &short, sw, (300, 720), (37, sh)).expect("Today is drawn on a short screen");
    assert!(rows.1 <= sh - 48 - 6, "Today fits a 600px screen above the dock: bottom at {}", rows.1);
    ui.set_today_open(false);
    w.set_size(slint::PhysicalSize::new(width, height));

    // ── Empty and unavailable calendar: said, not drawn as a free day ──
    ui.set_current_screen(1);
    let g = ui.global::<TodayState>();
    g.set_events_state("unavailable".into());
    g.set_events(ModelRc::default());
    g.set_notifications(ModelRc::default());
    ui.set_today_open(true);
    let down = settled(w, width, height);
    save(&down, &path("today-calendar-down"), width, height)?;
    g.set_events_state("ok".into());
    let empty = settled(w, width, height);
    assert!(changed(&down, &empty, width, (450, 830), (330, 460)) > 200, "an empty day and an unreachable calendar do not read the same");
    ui.set_today_open(false);
    fill_today(&ui);

    // ── Idle: Today open, the pointer parked, nothing animating -> 0 redraws ──
    ui.set_current_screen(8);
    ui.set_today_open(true);
    draw(w, width, height);
    w.dispatch_event(slint::platform::WindowEvent::PointerMoved { position: slint::LogicalPosition::new(200.0, 600.0) });
    std::thread::sleep(std::time::Duration::from_millis(500));
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
    assert_eq!(redraws, 0, "a settled shell with Today open repaints nothing");
    println!("settled shell with Today open: {redraws} redraws over one second");
    save(&pixels, output, width, height)?;
    println!("PASS: the bar is opaque and calm, the clock is under the screen's middle at 1280 and 1920, the Minds chip counts and routes, and Today opens, fits and settles");
    Ok(())
}
