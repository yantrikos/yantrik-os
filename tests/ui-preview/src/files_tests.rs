use super::*;
use slint::platform::{Key, WindowEvent};
fn chord(window: &MinimalSoftwareWindow, modifier: Key, text: impl Into<slint::SharedString>) {
    window.dispatch_event(WindowEvent::KeyPressed {
        text: modifier.into(),
    });
    key(window, text.into());
    window.dispatch_event(WindowEvent::KeyReleased {
        text: modifier.into(),
    });
}
pub fn run(window: &MinimalSoftwareWindow) -> Result<(), Box<dyn std::error::Error>> {
    let ui = FilesProbe::new()?;
    ui.set_entries(slint::ModelRc::new(slint::VecModel::from(
        (0..1000)
            .map(|i| FileEntry {
                name: format!("File {i:04}.txt").into(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    )));
    ui.show()?;
    window.set_size(slint::PhysicalSize::new(800, 600));
    let draw = || {
        slint::platform::update_timers_and_animations();
        let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(800, 600);
        window.request_redraw();
        window.draw_if_needed(|r| { r.render(p.make_mut_slice(), 800); });
    };
    ui.set_loading(true); draw();
    ui.set_loading(false); draw();
    chord(window, Key::Control, "f"); draw(); key(window, "after load".into());
    assert_eq!(ui.get_query(), "after load", "Loading restores keyboard focus");
    key(window, Key::Escape.into()); draw();
    click(window, 280., 210.);
    assert_eq!(ui.get_selected(), 0);
    key(window, Key::End.into());
    draw();
    assert_eq!(ui.get_selected(), 999);
    click(window, 280., 540.);
    assert!(ui.get_selected() > 990, "End scrolls selection into view");
    key(window, Key::Home.into());
    draw();
    assert_eq!(ui.get_selected(), 0);
    chord(window, Key::Shift, Key::DownArrow);
    assert_eq!(ui.get_selected(), 1);
    assert!(ui.get_shift_selected());
    chord(window, Key::Control, "c");
    assert_eq!(ui.get_action(), "copy");
    chord(window, Key::Control, "x");
    assert_eq!(ui.get_action(), "cut");
    chord(window, Key::Control, "v");
    assert_eq!(ui.get_action(), "paste");
    chord(window, Key::Control, "f");
    draw();
    key(window, "Budget".into());
    assert_eq!(ui.get_query(), "Budget");
    key(window, Key::Escape.into());
    draw();
    assert_eq!(ui.get_query(), "");
    chord(window, Key::Control, "f"); draw(); key(window, "fresh".into());
    assert_eq!(ui.get_query(), "fresh", "Clearing search also clears the actual input text");
    key(window, Key::Escape.into()); draw();
    chord(window, Key::Control, "l");
    draw();
    chord(window, Key::Control, "a");
    key(window, "/tmp".into());
    key(window, "\n".into());
    assert_eq!(ui.get_action(), "path:/tmp");
    click(window, 540., 24.);
    draw();
    key(window, "New folder test".into());
    key(window, "\n".into());
    assert_eq!(ui.get_action(), "folder:New folder test");
    ui.set_selected(-1);
    ui.set_grid(true);
    draw();
    click(window, 220., 220.);
    assert_eq!(ui.get_selected(), 0);
    click(window, 390., 220.);
    assert_eq!(ui.get_selected(), 1);
    click(window, 220., 340.);
    assert_eq!(ui.get_selected(), 4);
    key(window, Key::End.into());
    draw();
    assert_eq!(ui.get_selected(), 999);
    click(window, 720., 535.);
    assert!(
        ui.get_selected() > 980,
        "Grid End scrolls selected row into view"
    );
    for _ in 0..5 { slint::platform::update_timers_and_animations(); let mut p=slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(800,600); window.draw_if_needed(|r| {r.render(p.make_mut_slice(),800);}); std::thread::sleep(std::time::Duration::from_millis(100)); }
    let mut redraws=0;
    for _ in 0..10 {slint::platform::update_timers_and_animations(); let mut p=slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(800,600); if window.draw_if_needed(|r| {r.render(p.make_mut_slice(),800);}) {redraws+=1;} std::thread::sleep(std::time::Duration::from_millis(100));}
    assert_eq!(redraws,0,"Idle Files should not request continuous redraws");
    println!("PASS: Files idle with 1000 entries requested zero redraws in one second");
    println!("PASS: Files keyboard scrolling over 1000 entries, Shift selection, clipboard shortcuts, search, location, new-folder dialog and compact grid hit targets");

    // ── The 1280×800 screen from issue #208 ──
    //
    // The live VM runs at 1280×800, and the Files window's client area there is 1280×692:
    // 800 minus the 32px status bar, the 40px taskbar and the 36px window title.
    let draw_at = |w: u32, h: u32| {
        slint::platform::update_timers_and_animations();
        let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(w, h);
        window.request_redraw();
        window.draw_if_needed(|r| { r.render(p.make_mut_slice(), w as usize); });
    };
    let mut problems: Vec<String> = vec![];
    let entry = |name: &str, dir: bool| FileEntry {
        name: name.into(),
        is_dir: dir,
        count_known: dir,
        item_count: if dir { 3 } else { 0 },
        ..Default::default()
    };
    ui.set_entries(slint::ModelRc::new(slint::VecModel::from(vec![
        entry("Documents", true), entry("Downloads", true), entry("Music", true),
        entry("Pictures", true), entry("Projects", true), entry("Shared", true),
        entry("Videos", true), entry("backup", true), entry("budget.csv", false),
        entry("landscape.png", false), entry("notes.md", false), entry("setup.sh", false),
        entry("projects", true), entry("src", true), entry("vendor", true),
        entry("forge.py", false),
    ])));
    ui.set_recent(slint::ModelRc::new(slint::VecModel::from(vec![
        FileRecentData { name: "notes.md".into(), size_text: "4.1 KB".into(), changed_text: "8 min ago".into(), icon_char: "≡".into() },
        FileRecentData { name: "landscape.png".into(), size_text: "2.4 MB".into(), changed_text: "3 h ago".into(), icon_char: "▣".into() },
        FileRecentData { name: "budget.csv".into(), size_text: "8.7 KB".into(), changed_text: "1 d ago".into(), icon_char: "◇".into() },
    ])));
    ui.set_grid(true);
    ui.set_action("".into());
    ui.set_canvas_width(1280.);
    ui.set_canvas_height(692.);
    window.set_size(slint::PhysicalSize::new(1280, 692));
    draw_at(1280, 692);
    key(window, Key::Home.into());
    draw_at(1280, 692);
    ui.set_selected(-1);
    // The third row's first tile is "projects". The recent strip used to sit under the
    // listing in the layout and steal 104px from the scroll viewport, so at this window
    // size the third row's labels were cut in half and this point was already strip.
    click(window, 290., 560.);
    if ui.get_selected() != 12 {
        problems.push(format!(
            "third row clipped at 1280x800: clicking 'projects' selected {}, expected 12",
            ui.get_selected()
        ));
    }
    // The notice used to be a band in the layout above the grid: when it appeared the
    // whole view moved down under the pointer and the next click landed on the neighbour
    // tile. The same point must select the same tile with the banner up and down.
    ui.set_selected(-1);
    ui.set_canvas_height(800.);
    window.set_size(slint::PhysicalSize::new(1280, 800));
    ui.set_notice("".into());
    draw_at(1280, 800);
    click(window, 290., 318.);
    let without_banner = ui.get_selected();
    ui.set_selected(-1);
    ui.set_notice("Created folder \"Tour 23 Sep\"".into());
    draw_at(1280, 800);
    click(window, 290., 318.);
    if without_banner != 6 || ui.get_selected() != without_banner {
        problems.push(format!(
            "banner moved the grid: the same click selected {} without it and {} with it, expected 6 both times",
            without_banner,
            ui.get_selected()
        ));
    }
    // The banner names what was created and offers Open and Rename for it (issue #208).
    ui.set_notice_created("Tour 23 Sep".into());
    ui.set_notice_created_dir(true);
    ui.set_action("".into());
    draw_at(1280, 800);
    // In the 30px footer at 1280 wide, right to left: dismiss, Rename, Open.
    click(window, 1132., 785.);
    if ui.get_action() != "folder:Tour 23 Sep" {
        problems.push(format!(
            "Open in the banner did not open the new folder: action {}",
            ui.get_action()
        ));
    }
    ui.set_action("".into());
    click(window, 1204., 785.);
    draw_at(1280, 800);
    let dialog_height = ui.get_dialog_height();
    if !(130.0..175.0).contains(&dialog_height) {
        problems.push(format!(
            "the rename dialog is {dialog_height}px tall: it should hug its title and name field, the old fixed 200px left an empty band between them"
        ));
    }
    key(window, Key::End.into());
    key(window, " x".into());
    key(window, "\n".into());
    if ui.get_action() != "rename:Tour 23 Sep x" {
        problems.push(format!(
            "Rename in the banner did not rename the new folder: action {}",
            ui.get_action()
        ));
    }
    assert!(problems.is_empty(), "1280x800 layout problems:\n{}", problems.join("\n"));
    println!("PASS: Files at 1280x800 — banner does not move the grid, third row not clipped, Open and Rename reach the new folder, dialog hugs its contents");
    Ok(())
}
