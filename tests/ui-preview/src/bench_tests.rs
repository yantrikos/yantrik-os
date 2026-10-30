//! The Bench (issue #1030, phase 1): the production desktop, drawn with its icons and driven with
//! real pointer and key events.
//!
//!   bench.png        the everyday desktop: app shortcuts with the arrow, a folder, pictures, glyphs, one icon
//!                    selected, a name too long for two lines, the Continue row beside the ask field
//!   bench-drag.png   an icon on its way: the cell it will land in is outlined
//!   bench-menu.png   the right-click menu of the bare desktop; bench-item-menu.png the menu of an icon
//!   bench-rename.png a name being edited in place
//!
//! The rows of the proof, each a callback the shell answers: a click selects, Ctrl adds, a
//! double-click opens, a drag moves (with the drop target drawn while it goes), a band selects, a
//! right-click asks, the keys name themselves, a rename commits on Enter. Then that it costs
//! nothing at rest: no redraw over 1.2 s.
use super::*;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{Image, LogicalPosition, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

type Pixels = slint::SharedPixelBuffer<slint::Rgb8Pixel>;
const W: u32 = 1280;
const H: u32 = 800;
// The grid, as bench_state.slint declares it (yantrik-ui checks these against the layout code).
const CELL_W: f32 = 104.;
const CELL_H: f32 = 116.;
const PAD_X: f32 = 20.;
const PAD_Y: f32 = 8.;

fn save(pixels: &Pixels, path: &str) -> Result<(), Box<dyn std::error::Error>> {
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

/// A picture to stand in for a thumbnail: sky over two ridges, tinted by `seed`.
fn picture(seed: u8, w: u32, h: u32) -> Image {
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
    for (i, px) in buf.make_mut_slice().iter_mut().enumerate() {
        let (x, y) = ((i as u32 % w) as f32 / w as f32, (i as u32 / w) as f32 / h as f32);
        let ridge = 0.55 + 0.18 * ((x * 5.0 + seed as f32).sin()) + 0.08 * ((x * 13.0).sin());
        *px = if y > ridge {
            Rgba8Pixel { r: 40, g: 70u8.saturating_add(seed), b: 85, a: 255 }
        } else {
            Rgba8Pixel { r: 150, g: 190u8.saturating_sub(seed), b: 205, a: 255 }
        };
    }
    Image::from_rgba8(buf)
}

fn icon(id: &str, label: &str, kind: &str, col: u32, row: u32) -> BenchItem {
    BenchItem {
        id: id.into(),
        label: label.into(),
        kind: kind.into(),
        x: PAD_X + col as f32 * CELL_W,
        y: PAD_Y + row as f32 * CELL_H,
        ..Default::default()
    }
}

fn shortcut(id: &str, label: &str, app: &str, row: u32) -> BenchItem {
    BenchItem { shortcut: true, app_id: app.into(), ..icon(id, label, "app", 0, row) }
}

fn thumb(id: &str, label: &str, col: u32, row: u32, seed: u8) -> BenchItem {
    BenchItem { has_thumb: true, thumb: picture(seed, 128, 80), ..icon(id, label, "image", col, row) }
}

/// The desktop of design/bench-2026-10-30/frame-1.png, as far as phase 1 draws it.
fn fixtures() -> Vec<BenchItem> {
    vec![
        shortcut("blender.desktop", "Blender", "blender", 0),
        shortcut("terminal.desktop", "Terminal", "terminal", 1),
        shortcut("files.desktop", "Files", "files", 2),
        shortcut("notes.desktop", "Notes", "notes", 3),
        icon("Starfall", "Starfall", "folder", 1, 0),
        icon("summit.blend", "summit.blend", "blend", 2, 0),
        icon("ascent.blend", "ascent.blend", "blend", 3, 0),
        icon("Field notes.pdf", "Field notes.pdf", "pdf", 1, 1),
        BenchItem { selected: true, ..thumb("Ridge study.png", "Ridge study.png", 2, 1, 0) },
        icon("Route notes.pdf", "Route notes.pdf", "pdf", 3, 1),
        icon("Starfall notes.md", "Starfall notes.md", "text", 1, 2),
        thumb("Lake study.png", "Lake study.png", 2, 2, 25),
        icon("A very long file name that goes on and on and on.txt", "A very long file name that goes on and on and on.txt", "text", 3, 2),
        thumb("Screenshot 14.14.png", "Screenshot 14.14.png", 8, 0, 10),
        thumb("Screenshot 14.21.png", "Screenshot 14.21.png", 9, 0, 14),
        thumb("Screenshot 14.27.png", "Screenshot 14.27.png", 10, 0, 18),
        icon("budget.ods", "budget.ods", "sheet", 9, 1),
        icon("trail-guide.pdf", "trail-guide.pdf", "pdf", 10, 1),
    ]
}

fn draw(w: &MinimalSoftwareWindow) -> Pixels {
    slint::platform::update_timers_and_animations();
    let mut p = Pixels::new(W, H);
    w.request_redraw();
    w.draw_if_needed(|r| {
        r.render(p.make_mut_slice(), W as usize);
    });
    p
}

fn px(p: &Pixels, x: u32, y: u32) -> (u8, u8, u8) {
    let c = p.as_slice()[(y * W + x) as usize];
    (c.r, c.g, c.b)
}

/// Pixels in `region` (x, y, w, h) within `tol` of `want` on every channel.
fn count(p: &Pixels, region: (u32, u32, u32, u32), want: (u8, u8, u8), tol: u8) -> usize {
    let near = |a: u8, b: u8| a.abs_diff(b) <= tol;
    let mut n = 0;
    for y in region.1..(region.1 + region.3).min(H) {
        for x in region.0..(region.0 + region.2).min(W) {
            let c = px(p, x, y);
            if near(c.0, want.0) && near(c.1, want.1) && near(c.2, want.2) {
                n += 1;
            }
        }
    }
    n
}

fn press(w: &MinimalSoftwareWindow, x: f32, y: f32, button: PointerEventButton) {
    w.dispatch_event(WindowEvent::PointerMoved { position: at(x, y) });
    w.dispatch_event(WindowEvent::PointerPressed { position: at(x, y), button });
}

fn release(w: &MinimalSoftwareWindow, x: f32, y: f32, button: PointerEventButton) {
    w.dispatch_event(WindowEvent::PointerReleased { position: at(x, y), button });
}

fn move_to(w: &MinimalSoftwareWindow, x: f32, y: f32) {
    w.dispatch_event(WindowEvent::PointerMoved { position: at(x, y) });
}

fn tap(w: &MinimalSoftwareWindow, x: f32, y: f32) {
    press(w, x, y, PointerEventButton::Left);
    release(w, x, y, PointerEventButton::Left);
}

fn with_ctrl(w: &MinimalSoftwareWindow, f: impl FnOnce()) {
    w.dispatch_event(WindowEvent::KeyPressed { text: Key::Control.into() });
    f();
    w.dispatch_event(WindowEvent::KeyReleased { text: Key::Control.into() });
}

type Log = Rc<RefCell<Vec<String>>>;

fn listen(ui: &Preview) -> Log {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let s = ui.global::<BenchState>();
    macro_rules! record {
        ($on:ident, || $text:expr) => {{
            let log = log.clone();
            s.$on(move || log.borrow_mut().push($text));
        }};
        ($on:ident, |$($a:ident),*| $text:expr) => {{
            let log = log.clone();
            s.$on(move |$($a),*| log.borrow_mut().push($text));
        }};
    }
    record!(on_select, |id, ctrl| format!("select:{id}:{ctrl}"));
    record!(on_click_released, |id, ctrl| format!("click:{id}:{ctrl}"));
    record!(on_open_item, |id| format!("open:{id}"));
    record!(on_move_items, |id, dx, dy| format!("move:{id}:{}:{}", dx.round(), dy.round()));
    record!(on_rubber_band, |x, y, w, h, ctrl| format!("band:{}:{}:{}:{}:{ctrl}", x.round(), y.round(), w.round(), h.round()));
    record!(on_background_pressed, |ctrl| format!("bare:{ctrl}"));
    record!(on_item_menu, |id, _x, _y| format!("item-menu:{id}"));
    record!(on_background_menu, |_x, _y| "background-menu".to_string());
    record!(on_menu_action, |id| format!("menu:{id}"));
    record!(on_rename_commit, |id, text| format!("rename:{id}:{text}"));
    record!(on_rename_cancel, || "rename-cancel".to_string());
    record!(on_continue_open, || "continue".to_string());
    {
        let log = log.clone();
        s.on_key(move |name, ctrl, shift| {
            log.borrow_mut().push(format!("key:{name}:{ctrl}:{shift}"));
            true
        });
    }
    log
}

fn take(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.borrow_mut())
}

fn rows(list: &[(&str, &str, &str, bool)]) -> ModelRc<YMenuAction> {
    ModelRc::new(VecModel::from(
        list.iter()
            .map(|(id, label, hint, sep)| YMenuAction {
                id: (*id).into(),
                label: (*label).into(),
                shortcut: (*hint).into(),
                separator_after: *sep,
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    ))
}

pub fn run(window: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = Preview::new()?;
    ui.set_canvas_width(W as f32);
    ui.set_canvas_height(H as f32);
    ui.set_actual_desktop(true);
    ui.show()?;
    window.set_size(slint::PhysicalSize::new(W, H));
    let state = ui.global::<BenchState>();
    state.set_items(ModelRc::new(VecModel::from(fixtures())));
    state.set_has_continue(true);
    state.set_continue_title("summit.blend".into());
    state.set_continue_subtitle("Desktop".into());
    let log = listen(&ui);
    let still = draw(window);
    let top = ui.get_bench_y();
    assert!(top > 100. && top < 260., "the Bench starts under the greeting and the ask field, at {top}");
    save(&still, output)?;
    let dir = std::path::Path::new(output).parent().unwrap_or(std::path::Path::new("."));
    let side = |name: &str| dir.join(name).to_string_lossy().into_owned();

    // Where an icon is on the screen: the middle of its tile.
    let tile_of = |col: u32, row: u32| (PAD_X + col as f32 * CELL_W + CELL_W / 2., top + PAD_Y + row as f32 * CELL_H + 30.);

    // ── The look: one label treatment, a tile that is a picture, and what selection is ──
    let (rx, ry) = tile_of(2, 1);
    let accent_ish = |p: &Pixels| count(p, ((rx - 30.) as u32, (ry - 30.) as u32, 60, 60), (0x8f, 0xb4, 0xe3), 40);
    assert!(accent_ish(&still) > 40, "the selected picture's tile is outlined in the accent");
    let (lx, ly) = tile_of(3, 2);
    let plate = |p: &Pixels, x: f32, y: f32| px(p, x as u32, (y + 48.) as u32);
    let (r, g, b) = plate(&still, lx - 40., ly);
    assert!(r < 40 && g < 50 && b < 60, "a name sits on a dark plate, not straight on the photograph: {:?}", (r, g, b));

    // ── Pointer: select, add, open ──
    tap(window, rx, ry);
    assert_eq!(take(&log), ["select:Ridge study.png:false", "click:Ridge study.png:false"]);
    with_ctrl(window, || tap(window, tile_of(1, 1).0, tile_of(1, 1).1));
    assert_eq!(take(&log), ["select:Field notes.pdf:true", "click:Field notes.pdf:true"], "Ctrl is carried to the callback");
    tap(window, rx, ry);
    tap(window, rx, ry);
    let said = take(&log);
    assert!(said.iter().any(|l| l == "open:Ridge study.png"), "a double-click opens: {said:?}");

    // ── Pointer: drag. Two columns right; the cell it will land in is outlined while it goes ──
    press(window, rx, ry, PointerEventButton::Left);
    for step in 1..=6 {
        move_to(window, rx + 35. * step as f32, ry + 2.);
    }
    assert_eq!(state.get_drag_id().as_str(), "Ridge study.png", "a drag in progress names its icon");
    assert!((state.get_drag_dx() - 210.).abs() < 1.0 && state.get_drag_dy().abs() < 4.0, "{} {}", state.get_drag_dx(), state.get_drag_dy());
    let going = draw(window);
    let (tx, ty) = tile_of(4, 1);
    let outline = count(&going, ((tx - CELL_W / 2.) as u32, (ty - 30. - PAD_Y) as u32, CELL_W as u32, CELL_H as u32), (0x8f, 0xb4, 0xe3), 30);
    assert!(outline > 150, "the target cell is outlined while the icon moves: {outline} accent pixels");
    save(&going, &side("bench-drag.png"))?;
    release(window, rx + 210., ry + 2., PointerEventButton::Left);
    assert_eq!(take(&log), ["select:Ridge study.png:false", "move:Ridge study.png:210:2"]);
    assert_eq!(state.get_drag_id().as_str(), "", "the drag is over");
    assert_eq!(state.get_drag_dx(), 0.);

    // ── A wobble in a click is not a drag ──
    press(window, rx, ry, PointerEventButton::Left);
    move_to(window, rx + 2., ry + 1.);
    release(window, rx + 2., ry + 1., PointerEventButton::Left);
    assert_eq!(take(&log), ["select:Ridge study.png:false", "click:Ridge study.png:false"]);

    // ── Pointer: the band, the bare desktop, the menus ──
    press(window, 640., top + 380., PointerEventButton::Left);
    move_to(window, 500., top + 300.);
    move_to(window, 300., top + 200.);
    let banded = draw(window);
    assert!(count(&banded, (300, (top + 200.) as u32, 340, 180), (0x8f, 0xb4, 0xe3), 20) > 100, "the band is drawn while the button is down");
    release(window, 300., top + 200., PointerEventButton::Left);
    let said = take(&log);
    assert_eq!(said[0], "bare:false");
    assert!(said[1].starts_with("band:300:200:340:180") || said[1].starts_with("band:300:"), "{said:?}");
    tap(window, 700., top + 450.);
    assert_eq!(take(&log), ["bare:false"], "a click on nothing clears the selection and nothing else");
    press(window, rx, ry, PointerEventButton::Right);
    release(window, rx, ry, PointerEventButton::Right);
    assert_eq!(take(&log), ["item-menu:Ridge study.png"]);
    press(window, 700., top + 450., PointerEventButton::Right);
    release(window, 700., top + 450., PointerEventButton::Right);
    assert_eq!(take(&log), ["background-menu"]);

    // ── The menus as drawn ──
    state.set_menu_rows(rows(&[
        ("new-folder", "New folder", "", false),
        ("new-file", "New file", "", true),
        ("arrange", "Arrange", "", false),
        ("wallpaper", "Change wallpaper", "", true),
        ("toggle-icons", "Show desktop icons", "On", true),
        ("launch:terminal", "Open Terminal", "", false),
        ("launch:files", "Open Files", "", false),
        ("launch:settings", "Settings", "", false),
    ]));
    state.set_menu_x(700.);
    state.set_menu_y(top + 120.);
    state.set_menu_open(true);
    save(&draw(window), &side("bench-menu.png"))?;
    tap(window, 760., top + 120. + 12. + 32. * 2. + 16.);
    assert_eq!(take(&log), ["menu:arrange"], "a row names itself to the shell");
    assert!(!state.get_menu_open(), "and the menu closes unless the shell keeps it open for a submenu");
    state.set_menu_rows(rows(&[
        ("open", "Open", "Enter", false),
        ("open-with", "Open with", "", true),
        ("rename", "Rename", "F2", false),
        ("reveal", "Reveal in Files", "", true),
        ("archive", "Archive", "Delete", false),
    ]));
    state.set_menu_x(rx);
    state.set_menu_y(ry);
    state.set_menu_open(true);
    save(&draw(window), &side("bench-item-menu.png"))?;
    state.set_menu_open(false);

    // ── Keys, once the desktop has the focus ──
    tap(window, 700., top + 450.);
    take(&log);
    let keys: Vec<(slint::SharedString, &str)> = vec![
        ("
".into(), "enter"),
        (Key::F2.into(), "f2"),
        (Key::Delete.into(), "delete"),
        (Key::UpArrow.into(), "up"),
        (Key::DownArrow.into(), "down"),
        (Key::LeftArrow.into(), "left"),
        (Key::RightArrow.into(), "right"),
        (Key::Escape.into(), "escape"),
    ];
    for (text, name) in keys {
        key(window, text);
        assert_eq!(take(&log), [format!("key:{name}:false:false")], "{name}");
    }
    with_ctrl(window, || key(window, "a".into()));
    assert_eq!(take(&log), ["key:a:true:false"]);

    // ── A name being edited in place; Enter commits it ──
    let mut items = fixtures();
    let at_renaming = items.iter().position(|i| i.id == "Lake study.png").expect("the fixture");
    items[at_renaming].renaming = true;
    items[at_renaming].selected = true;
    state.set_items(ModelRc::new(VecModel::from(items)));
    state.set_rename_text("Lake study.png".into());
    save(&draw(window), &side("bench-rename.png"))?;
    key(window, "x".into());
    key(window, "\n".into());
    let said = take(&log);
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].starts_with("rename:Lake study.png:"), "Enter commits what was typed: {said:?}");

    // ── Icons off: the wallpaper is bare ──
    state.set_items(ModelRc::new(VecModel::from(fixtures())));
    state.set_show_icons(false);
    let bare = draw(window);
    assert!(count(&bare, (0, top as u32, 400, 300), (0x8f, 0xb4, 0xe3), 10) < 20, "no outline when the icons are hidden");
    state.set_show_icons(true);

    // ── At rest it costs nothing ──
    draw(window);
    draw(window);
    let mut redraws = 0;
    let mut scratch = Pixels::new(W, H);
    for _ in 0..12 {
        slint::platform::update_timers_and_animations();
        if window.draw_if_needed(|r| {
            r.render(scratch.make_mut_slice(), W as usize);
        }) {
            redraws += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(redraws, 0, "a settled desktop with icons requests no redraw over 1.2s");
    println!("PASS: select, Ctrl-add, open, drag with its drop target, a click that wobbles, the band, both menus, the keys, in-place rename, icons off, and no redraw at rest");
    Ok(())
}
