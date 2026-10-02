//! The Alt+Tab switcher (`alt_tab.slint`), drawn by the whole shell with real key and pointer
//! events. Two scenes — 8 windows, one page, and 20 windows on page 2 of 3 — and the claims that
//! matter: the card is opaque with no dimming layer, Tab, Shift+Tab, Return and Escape reach the
//! shell's callbacks, a pointer that has not moved does not select, a pointer move does, and a
//! click on a cell activates that cell.
//!
//! The selection rules themselves (order, paging, wrap) are `alt_tab.rs`'s tests; this stands in
//! for it by handing the card a page, which is all the card ever sees.
use super::*;
use slint::platform::{Key, WindowEvent};
use slint::{ModelRc, VecModel};
use std::cell::RefCell;

const NAMES: [(&str, &str, &str); 12] = [
    ("Terminal", "terminal", "Terminal"),
    ("Budget 2027 — LibreOffice Calc", "libreoffice-calc", "LibreOffice Calc"),
    ("Notes: Handover", "notes", "Notes"),
    ("Blender: (Unsaved) - Blender 4.3.2", "blender", "Blender"),
    ("Inbox - Chromium", "chromium", "Chromium"),
    ("Weather", "weather", "Weather"),
    ("Download Manager", "downloads", "Downloads"),
    ("Mind View", "mind-view", "Mind View"),
    ("System Monitor", "sysmonitor", "System Monitor"),
    ("Yantrik OS", "desktop", "Desktop"),
    ("Music Player", "music", "Music"),
    ("Editor: README.md", "editor", "Editor"),
];

fn save(pixels: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    println!("Rendered {path} ({width}×{height})");
    Ok(())
}

/// What `control_switcher::publish` hands the card for page `page` of `total` windows.
fn show(ui: &App, total: usize, selected: usize) {
    let page = selected / 8;
    let pages = total.div_ceil(8).max(1);
    let cells: Vec<SwitcherCell> = (page * 8..((page + 1) * 8).min(total))
        .map(|n| {
            let (title, app_id, name) = NAMES[n % NAMES.len()];
            let title = if n >= NAMES.len() { format!("{title} ({})", n / NAMES.len() + 1) } else { title.to_string() };
            SwitcherCell { title: title.into(), app_id: app_id.into(), app_name: name.into() }
        })
        .collect();
    let plate = cells[selected % 8].title.clone();
    ui.set_alt_tab_cells(ModelRc::new(VecModel::from(cells)));
    ui.set_alt_tab_selected((selected % 8) as i32);
    ui.set_alt_tab_plate(plate);
    ui.set_alt_tab_page(page as i32);
    ui.set_alt_tab_pages(pages as i32);
    ui.set_alt_tab_status(if pages == 1 {
        format!("{total} windows").into()
    } else {
        format!("{}–{} of {total} windows · Page {} of {pages}", page * 8 + 1, ((page + 1) * 8).min(total), page + 1).into()
    });
}

pub fn run(w: &MinimalSoftwareWindow, output: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let ui = App::new()?;
    ui.set_current_screen(1);
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        let l = log.clone();
        ui.on_alt_tab_navigate(move |dir, cols| l.borrow_mut().push(format!("navigate:{dir}:{cols}")));
        let l = log.clone();
        ui.on_alt_tab_hover(move |i| l.borrow_mut().push(format!("hover:{i}")));
        let l = log.clone();
        ui.on_alt_tab_activate(move |i| l.borrow_mut().push(format!("activate:{i}")));
        let l = log.clone();
        ui.on_alt_tab_cancel(move || l.borrow_mut().push("cancel".to_string()));
    }
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
    let behind = draw();

    // ── 8 windows: one page, the previous window (index 1) selected ───────────────────────────
    show(&ui, 8, 1);
    ui.set_alt_tab_open(true);
    let one_page = draw();
    save(&one_page, output, width, height)?;
    // Opaque and undimmed: outside the card the desktop is exactly what it was.
    assert_eq!(behind.as_slice()[5 * width as usize + 5], one_page.as_slice()[5 * width as usize + 5], "no dimming layer");
    assert!(log.borrow().is_empty(), "opening alone selects nothing: {:?}", log.borrow());

    // The keys reach the shell's callbacks, with the column count the card is drawn with.
    key(w, Key::Tab.into());
    key(w, Key::Backtab.into());
    key(w, Key::RightArrow.into());
    key(w, Key::DownArrow.into());
    key(w, Key::PageDown.into());
    key(w, Key::Return.into());
    key(w, Key::Escape.into());
    draw();
    let cols = if width < 560 { 2 } else if width < 760 { 3 } else { 4 };
    let expect: Vec<String> = [
        format!("navigate:next:{cols}"),
        format!("navigate:previous:{cols}"),
        format!("navigate:right:{cols}"),
        format!("navigate:down:{cols}"),
        format!("navigate:page-next:{cols}"),
        "activate:-1".to_string(),
        "cancel".to_string(),
    ]
    .to_vec();
    assert_eq!(*log.borrow(), expect, "Tab, Shift+Tab, arrows, PageDown, Return and Escape");
    log.borrow_mut().clear();

    // ── 20 windows, page 2 of 3, third cell selected ──────────────────────────────────────────
    show(&ui, 20, 10);
    draw();
    save(&draw(), &output.replace(".png", "-20.png"), width, height)?;

    // The pointer's position when the card opened must not select: nothing has moved yet. Then a
    // real move over a cell does, and a click on it activates it.
    let point = |x: f32, y: f32| WindowEvent::PointerMoved { position: slint::LogicalPosition::new(x, y) };
    let card_x = (width as f32 - (width as f32 - 64.0).min(1040.0)) / 2.0;
    let card_w = (width as f32 - 64.0).min(1040.0);
    let cell_w = 224f32.min((card_w - 32.0 - (cols as f32 - 1.0) * 12.0) / cols as f32);
    let cell_h = 8.0 + (cell_w - 16.0) * 9.0 / 16.0 + 41.0;
    let grid_w = cols as f32 * cell_w + (cols as f32 - 1.0) * 12.0;
    let x0 = card_x + (card_w - grid_w) / 2.0;
    let rows = (8 + cols - 1) / cols;
    // The pager row is a compact icon button's height (Theme.h-compact, 28).
    let card_h = 16.0 + rows as f32 * cell_h + (rows as f32 - 1.0) * 12.0 + 12.0 + 28.0 + 16.0;
    let y0 = (height as f32 - (card_h + 48.0)) / 2.0 + 16.0;
    let (cx, cy) = (x0 + cell_w / 2.0, y0 + cell_h / 2.0); // the centre of cell 0
    draw();
    assert!(log.borrow().is_empty(), "a pointer that has not moved selects nothing: {:?}", log.borrow());
    w.dispatch_event(point(cx - 4.0, cy));
    w.dispatch_event(point(cx, cy));
    draw();
    assert!(log.borrow().iter().any(|e| e == "hover:0"), "a pointer move over a cell selects it: {:?}", log.borrow());
    click(w, cx, cy);
    draw();
    assert!(log.borrow().iter().any(|e| e == "activate:0"), "a click on a cell activates it: {:?}", log.borrow());
    
    // The page arrows are the kit's icon buttons, which take the keyboard focus when clicked. The
    // card's keys must still work afterwards, or a click on "next page" would leave Escape dead.
    log.borrow_mut().clear();
    let (px, py) = (card_x + card_w - 16.0 - 14.0, y0 - 16.0 + card_h - 16.0 - 14.0); // the "next page" button's centre
    click(w, px, py);
    draw();
    assert!(log.borrow().iter().any(|e| e == &format!("navigate:page-next:{cols}")), "the next-page arrow pages: {:?}", log.borrow());
    log.borrow_mut().clear();
    key(w, Key::Escape.into());
    draw();
    assert_eq!(*log.borrow(), ["cancel"], "Escape still reaches the card after a click on a page arrow");
    Ok(())
}
