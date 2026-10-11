//! Stills of the app tiles where a person meets them, for the icon set's review: the whole shell
//! (app.slint's `App`) at 1280x800, dark, and again in the light theme beside it.
//!
//!   review-still icons-desktop    the desktop: its workspace row of pinned apps at 48px and the
//!                                 dock at 32px; also writes the dock alone, 4x, pixel for pixel
//!   review-still icons-launcher   the Apps launcher open: Running, then All apps at 48px
//!
//! Each fixture carries the three cases AppTile decides between: one of ours (its art), a
//! third-party app with an icon from the theme (here the brand mark stands in for one), and a
//! third-party app with neither (its initial). Fixture data only; nothing is launched.
use super::*;
use slint::{ModelRc, VecModel};

/// A picture standing in for an icon resolved from the icon theme.
fn theme_icon() -> slint::Image {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../brand/yantrik-icon-128.png");
    slint::Image::load_from_path(std::path::Path::new(path)).expect("the brand mark")
}

/// (icon id, label, has a theme icon). The last two are not ours.
const DOCK: &[(&str, &str, bool)] = &[
    ("files", "Files", false), ("notes", "Notes", false), ("terminal", "Terminal", false),
    ("browser", "Browser", false), ("email", "Email", false), ("calendar", "Calendar", false),
    ("music", "Music", false), ("settings", "Settings", false),
    ("org.example.Atlas", "Atlas", true), ("gimp", "GIMP", false),
];

const LAUNCHER: &[(&str, &str)] = &[
    ("agents", "Agents"), ("arcade", "Arcade"), ("blender", "Blender"), ("browser", "Browser"),
    ("calendar", "Calendar"), ("containers", "Containers"), ("documents", "Documents"),
    ("downloads", "Downloads"), ("editor", "Editor"), ("email", "Email"), ("files", "Files"),
    ("image", "Images"), ("media", "Media"), ("memory", "Memory"), ("music", "Music"),
    ("network", "Network"), ("notes", "Notes"), ("packages", "Packages"),
    ("presentation", "Slides"), ("recipes", "Recipes"), ("settings", "Settings"),
    ("snippets", "Snippets"), ("spreadsheet", "Spreadsheet"), ("studio", "Studio"),
    ("sysmonitor", "System Monitor"), ("terminal", "Terminal"), ("weather", "Weather"),
    ("org.example.Atlas", "Atlas"), ("inkscape", "Inkscape"),
];

fn dock_button(id: &str, label: &str, icon: bool, i: usize) -> DockButton {
    DockButton {
        app_id: id.into(),
        label: label.into(),
        icon_id: id.into(),
        has_icon: icon,
        icon: if icon { theme_icon() } else { Default::default() },
        pinned: i < 8,
        running: i == 1 || i == 2 || i >= 8,
        focused: i == 1,
        windows: if i == 2 { 3 } else { 1 },
        title: format!("{label} window").into(),
    }
}

fn grid_item(id: &str, name: &str) -> AppGridItem {
    let icon = id.contains('.');
    AppGridItem {
        app_id: id.into(),
        name: name.into(),
        icon_char: "".into(),
        icon_id: id.into(),
        icon: if icon { theme_icon() } else { Default::default() },
        has_icon: icon,
        category: "utility".into(),
        pinned: false,
        pinnable: true,
    }
}

fn pin(id: &str, label: &str, icon: bool, running: bool) -> DockItem {
    DockItem {
        app_id: id.into(),
        label: label.into(),
        icon_char: "".into(),
        icon_id: id.into(),
        icon: if icon { theme_icon() } else { Default::default() },
        has_icon: icon,
        is_running: running,
    }
}

fn shell() -> Result<App, Box<dyn std::error::Error>> {
    let ui = App::new()?;
    ui.global::<AppInitial>().on_of(|id| yantrik_ui_kit::app_tile::initial(&id).into());
    ui.set_current_screen(1);
    ui.set_clock_text("10:24".into());
    ui.set_date_text("Sun 4 Oct".into());
    ui.set_wallpaper_path("lake".into());
    let buttons: Vec<DockButton> = DOCK.iter().enumerate().map(|(i, (id, l, icon))| dock_button(id, l, *icon, i)).collect();
    ui.set_dock_buttons(ModelRc::new(VecModel::from(buttons)));
    let pins = vec![
        pin("files", "Files", false, false), pin("notes", "Notes", false, true),
        pin("terminal", "Terminal", false, true), pin("calendar", "Calendar", false, false),
        pin("email", "Email", false, false), pin("music", "Music", false, false),
        pin("browser", "Browser", false, false), pin("settings", "Settings", false, false),
        pin("org.example.Atlas", "Atlas", true, false), pin("gimp", "GIMP", false, false),
    ];
    ui.set_dock_items(ModelRc::new(VecModel::from(pins)));
    Ok(ui)
}

/// A region of a frame, each pixel drawn as a `k`x`k` block: the 32px tiles as they really are.
fn zoom(p: &review_stills::Pixels, (x0, y0, w, h): (u32, u32, u32, u32), k: u32, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut out = review_stills::Pixels::new(w * k, h * k);
    let src = p.as_slice();
    for (i, px) in out.make_mut_slice().iter_mut().enumerate() {
        let (x, y) = (i as u32 % (w * k) / k, i as u32 / (w * k) / k);
        *px = src[((y0 + y) * 1280 + x0 + x) as usize];
    }
    review_stills::save(&out, path, w * k, h * k)
}

pub fn run(w: &MinimalSoftwareWindow, output: &str, which: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = shell()?;
    if which == "icons-launcher" {
        let apps: Vec<AppGridItem> = LAUNCHER.iter().map(|(id, n)| grid_item(id, n)).collect();
        ui.set_grid_apps(ModelRc::new(VecModel::from(apps)));
        let running: Vec<DockButton> = DOCK.iter().enumerate().filter(|(i, _)| *i == 1 || *i == 2 || *i == 9)
            .map(|(i, (id, l, icon))| dock_button(id, l, *icon, i)).collect();
        ui.set_grid_running(ModelRc::new(VecModel::from(running)));
        ui.set_app_grid_open(true);
    }
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(1280, 800));
    // Away from every tile, so none is drawn hovered.
    w.dispatch_event(slint::platform::WindowEvent::PointerMoved { position: slint::LogicalPosition::new(640.0, 300.0) });
    std::thread::sleep(std::time::Duration::from_millis(400));
    let dark = review_stills::settle(w, 1280, 800);
    review_stills::save(&dark, output, 1280, 800)?;
    if which == "icons-desktop" {
        zoom(&dark, (300, 744, 680, 56), 4, &output.replace(".png", "-dock-zoom.png"))?;
    }
    ui.global::<ThemeMode>().set_dark(false);
    std::thread::sleep(std::time::Duration::from_millis(400));
    review_stills::save(&review_stills::settle(w, 1280, 800), &output.replace(".png", "-light.png"), 1280, 800)
}
