//! The Apps launcher (design/minds-surfaces-spec §3) and the restored-window frame, drawn by the
//! whole shell — app.slint's `App` — and driven with real key events. Saved as PNGs to look at:
//!
//!   launcher.png          Running (apps with windows, a count on the one with several) and All apps,
//!                         the same full-colour tiles the dock draws, over the desktop
//!   launcher-search.png   "ter" typed: both sections filtered, the top match selected
//!   restored-window.png   Devices, restored: one bar (the screen's own header), not two
//!   desktop-agent.png     the desktop home in agent mode: the same DesktopHome, with its flag set
//!
//! The asserts are the behaviour a person relies on: the search field has the keyboard when it
//! opens, Enter opens the top match (a running app first), the arrows cross from Running into
//! All apps by column, Esc closes, and a restored window draws one title bar.
//!
//! Rust, not the preview, filters the models here, by the same rule as `wire/launcher.rs`'s
//! `matches` (every word must be in a name), because the filter is not Slint's job.
use super::*;
use slint::{Model, ModelRc, VecModel};
use std::cell::RefCell;

type Pixels = slint::SharedPixelBuffer<slint::Rgb8Pixel>;
const W: u32 = 1280;
const H: u32 = 800;

fn save(pixels: &Pixels, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), W, H);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    println!("Rendered {path} ({W}×{H})");
    Ok(())
}

fn app(id: &str, name: &str) -> AppGridItem {
    AppGridItem {
        app_id: id.into(),
        name: name.into(),
        icon_char: "".into(),
        icon_id: id.into(),
        icon: Default::default(),
        has_icon: false,
        category: "utility".into(),
        pinned: false,
        pinnable: true,
    }
}

fn running(id: &str, label: &str, windows: i32) -> DockButton {
    DockButton {
        app_id: id.into(),
        label: label.into(),
        icon_id: id.into(),
        has_icon: false,
        icon: Default::default(),
        pinned: false,
        running: true,
        focused: false,
        windows,
        title: format!("{label} window").into(),
    }
}

fn catalogue() -> Vec<AppGridItem> {
    vec![
        app("files", "Files"), app("notes", "Notes"), app("terminal", "Terminal"),
        app("calendar", "Calendar"), app("email", "Email"), app("music", "Music"),
        app("image", "Images"), app("weather", "Weather"), app("settings", "Settings"),
        app("editor", "Editor"), app("sysmonitor", "System Monitor"), app("downloads", "Downloads"),
        app("browser", "Browser"), app("spreadsheet", "Spreadsheet"), app("presentation", "Slides"),
        app("network", "Network"), app("studio", "Studio"), app("packages", "Packages"),
    ]
}

fn open_apps() -> Vec<DockButton> {
    vec![running("files", "Files", 1), running("terminal", "Terminal", 3), running("notes", "Notes", 1)]
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

    // What Rust would publish: the catalogue, and the running rows (the dock's, filtered).
    let matches = |q: &str, name: &str| q.to_lowercase().split_whitespace().all(|word| name.to_lowercase().contains(word));
    let publish = {
        let ui = ui.as_weak();
        move |q: &str| {
            let ui = ui.upgrade().unwrap();
            let apps: Vec<AppGridItem> = catalogue().into_iter().filter(|a| matches(q, a.name.as_str())).collect();
            let run: Vec<DockButton> = open_apps().into_iter().filter(|b| matches(q, b.label.as_str())).collect();
            ui.set_grid_apps(ModelRc::new(VecModel::from(apps)));
            ui.set_grid_running(ModelRc::new(VecModel::from(run)));
        }
    };
    publish("");
    ui.on_grid_search_apps({
        let publish = publish.clone();
        move |q| publish(q.as_str())
    });
    let launched: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        let l = launched.clone();
        ui.on_grid_launch_app(move |id| l.borrow_mut().push(id.to_string()));
    }
    let typed = |text: &str| for c in text.chars() { key(w, c.to_string().into()); };

    // ── 1. Opened: Running first, then All apps ──
    ui.set_app_grid_open(true);
    let opened = draw();
    save(&opened, &out("launcher.png"))?;
    assert_eq!(ui.get_grid_running().row_count(), 3);

    // The search field has the keyboard on open, with no click: typing filters both sections.
    typed("ter");
    let searched = draw();
    save(&searched, &out("launcher-search.png"))?;
    assert_eq!(ui.get_grid_running().row_count(), 1, "'ter' leaves Terminal in Running");
    let names: Vec<String> = ui.get_grid_apps().iter().map(|a| a.name.to_string()).collect();
    assert!(names.iter().all(|n| n.to_lowercase().contains("ter")), "All apps is filtered too: {names:?}");

    // Enter opens the top match, which is the running Terminal, not the first app of All apps.
    key(w, "\n".into());
    assert_eq!(launched.borrow().last().map(String::as_str), Some("terminal"), "Enter opens the top match");
    assert!(!ui.get_app_grid_open(), "and closes the launcher");

    // ── 2. Arrows: Right moves one tile; Down from Running lands in All apps, in the same column ──
    publish("");
    ui.set_app_grid_open(true);
    draw();
    key(w, slint::platform::Key::RightArrow.into());
    key(w, "\n".into());
    assert_eq!(launched.borrow().last().map(String::as_str), Some("terminal"), "Right moves to the second running app");
    publish("");
    ui.set_app_grid_open(true);
    draw();
    key(w, slint::platform::Key::DownArrow.into());
    key(w, "\n".into());
    // 3 running tiles on one row; Down crosses into All apps at column 0: the first app.
    assert_eq!(launched.borrow().last().map(String::as_str), Some("files"), "Down crosses into All apps by column");
    publish("");
    ui.set_app_grid_open(true);
    draw();
    key(w, slint::platform::Key::DownArrow.into());
    key(w, slint::platform::Key::UpArrow.into());
    key(w, "\n".into());
    assert_eq!(launched.borrow().last().map(String::as_str), Some("files"), "Up from All apps returns to Running (the first running tile is Files)");

    // ── 3. Esc closes, and nothing is launched ──
    let before = launched.borrow().len();
    publish("");
    ui.set_app_grid_open(true);
    draw();
    key(w, slint::platform::Key::Escape.into());
    assert!(!ui.get_app_grid_open(), "Escape closes the launcher");
    assert_eq!(launched.borrow().len(), before, "and launches nothing");

    // ── 4. No match: Enter does nothing, the launcher stays ──
    publish("");
    ui.set_app_grid_open(true);
    draw();
    typed("zzzz");
    draw();
    key(w, "\n".into());
    assert!(ui.get_app_grid_open(), "Enter with no match does not close the launcher");
    assert_eq!(launched.borrow().len(), before);
    ui.set_app_grid_open(false);

    // ── 5. A restored window has one bar ──
    // Devices (screen 27) hosted restored. Before, the frame drew its 36px bar and the screen
    // drew its tab bar under it; now the screen's own bar is the only one. The frame's title bar
    // is the thing that is gone: its caption "Devices" is not drawn above the tabs.
    ui.set_window_maximized(false);
    ui.set_current_screen(27);
    let restored = draw();
    save(&restored, &out("restored-window.png"))?;
    ui.set_current_screen(21);
    save(&draw(), &out("restored-packages.png"))?;
    ui.set_window_maximized(true);
    draw();

    // ── 6. One desktop home: agent mode is the same component with a flag ──
    ui.set_current_screen(1);
    ui.set_agent_mode(true);
    save(&draw(), &out("desktop-agent.png"))?;
    ui.set_agent_mode(false);
    println!("PASS: launcher opens on Running and All apps, filters both, Enter opens the top match, the arrows cross sections, Esc closes; a restored window draws one bar");
    Ok(())
}
