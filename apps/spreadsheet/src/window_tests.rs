//! The real window, driven by pointer and keyboard events through Slint's software renderer.
//! Only the window system's event queue and the desktop's file picker are stood in for.
//!
//! One test, because a process can install Slint's platform once.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{EventLoopProxy, Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model};

use crate::picker::Choice;
use crate::session::Session;
use crate::{files, render, wire, SheetDialog, SpreadsheetApp};

type Queue = Arc<Mutex<VecDeque<Box<dyn FnOnce() + Send>>>>;

struct Proxy(Queue);
impl EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), slint::EventLoopError> {
        Ok(())
    }
    fn invoke_from_event_loop(&self, event: Box<dyn FnOnce() + Send>) -> Result<(), slint::EventLoopError> {
        self.0.lock().unwrap().push_back(event);
        Ok(())
    }
}

struct Headless {
    window: Rc<MinimalSoftwareWindow>,
    queue: Queue,
}
impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Proxy(self.queue.clone())))
    }
}

struct Driver {
    window: Rc<MinimalSoftwareWindow>,
    queue: Queue,
}

impl Driver {
    fn tick(&self) {
        let events: Vec<_> = self.queue.lock().unwrap().drain(..).collect();
        for event in events {
            event();
        }
        slint::platform::update_timers_and_animations();
        let size = self.window.size();
        let mut buffer = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(size.width, size.height);
        self.window.draw_if_needed(|r| {
            r.render(buffer.make_mut_slice(), size.width as usize);
        });
    }

    fn wait(&self, what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(6);
        while !done() {
            self.tick();
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
        self.tick();
    }

    fn click(&self, x: f32, y: f32) {
        let position = LogicalPosition::new(x, y);
        self.window.dispatch_event(WindowEvent::PointerMoved { position });
        self.window.dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left });
        self.window.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left });
        self.tick();
    }

    /// The middle of a cell: the bar (48) + formula bar (40) + rule (1) + column header (28),
    /// then 28 a row; the row numbers (48), then 100 a column.
    fn click_cell(&self, row: usize, col: usize) {
        self.click(48.0 + 100.0 * col as f32 + 50.0, 117.0 + 28.0 * row as f32 + 14.0);
    }

    fn key(&self, text: impl Into<slint::SharedString>) {
        let text = text.into();
        self.window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        self.window.dispatch_event(WindowEvent::KeyReleased { text });
        self.tick();
    }

    fn typed(&self, text: &str) {
        for ch in text.chars() {
            match ch {
                '\n' => self.key(slint::platform::Key::Return),
                c => self.key(c.to_string()),
            }
        }
    }

    fn ctrl(&self, letter: &str) {
        let control: slint::SharedString = slint::platform::Key::Control.into();
        self.window.dispatch_event(WindowEvent::KeyPressed { text: control.clone() });
        self.key(letter);
        self.window.dispatch_event(WindowEvent::KeyReleased { text: control });
        self.tick();
    }
}

fn shown(ui: &SpreadsheetApp, row: usize, col: usize) -> String {
    ui.get_rows().row_data(row).unwrap().cells.row_data(col).unwrap().text.to_string()
}

fn no_portal(_saving: bool, _current: Option<PathBuf>) -> Choice {
    Choice::Unavailable("no session bus in this test".into())
}

#[test]
fn a_person_fills_the_grid_types_sums_undoes_saves_and_reopens() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let queue = Queue::default();
    slint::platform::set_platform(Box::new(Headless { window: window.clone(), queue: queue.clone() })).unwrap();
    let d = Driver { window: window.clone(), queue };

    let ui = SpreadsheetApp::new().unwrap();
    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(1200, 760));
    files::ASK.with(|a| a.set(no_portal));
    let shared = Rc::new(std::cell::RefCell::new(Session::new()));
    files::SESSION.with(|c| *c.borrow_mut() = Some(shared.clone()));
    wire::wire(&ui, &shared);
    files::redraw(&ui, &shared.borrow());
    d.tick();

    // (1) The grid is filled at start: 50 rows of 26 cells, and 26 column names.
    assert_eq!(ui.get_rows().row_count(), 50);
    assert_eq!(ui.get_rows().row_data(49).unwrap().cells.row_count(), 26);
    assert_eq!(ui.get_col_names().row_count(), 26);
    assert_eq!(ui.get_col_names().row_data(25).unwrap(), "Z");

    // Click A1 and type: numbers, a blank, and a SUM over the blank.
    d.click_cell(0, 0);
    assert_eq!(ui.get_active_ref(), "A1", "the click selected A1");
    d.typed("2\n3\n\n=SUM(A1:A3)\n");
    assert_eq!(shown(&ui, 0, 0), "2");
    assert_eq!(shown(&ui, 1, 0), "3");
    assert_eq!(shown(&ui, 2, 0), "");
    assert_eq!(shown(&ui, 3, 0), "5", "SUM over a blank works it out as 0");
    assert_eq!(ui.get_active_ref(), "A5", "Enter moves down");
    assert!(ui.get_modified());

    // Recalculation: change A1 and the sum follows.
    d.click_cell(0, 0);
    assert_eq!(ui.get_cell_data(), "2", "the formula bar shows what A1 holds");
    d.typed("10\n");
    assert_eq!(shown(&ui, 3, 0), "13");

    // An error keeps its code in the cell, and the status line says what went wrong.
    d.click_cell(0, 1);
    d.typed("hello\n=B1+1\n");
    assert_eq!(shown(&ui, 1, 1), "#VALUE!");
    assert!(ui.get_status_is_problem());
    assert_eq!(ui.get_status_text(), "B2 shows #VALUE!: B1 holds text (\"hello\"), so it can't be added.");
    assert_eq!(ui.get_active_ref(), "B2", "a cell that went wrong stays selected");

    // Undo (Ctrl+Z) takes B2 back; redo puts it again; the buttons follow.
    d.ctrl("z");
    assert_eq!(shown(&ui, 1, 1), "");
    assert!(ui.get_can_redo());
    d.ctrl("y");
    assert_eq!(shown(&ui, 1, 1), "#VALUE!");
    d.ctrl("z");
    d.ctrl("z");
    assert_eq!(shown(&ui, 0, 1), "", "B1's hello is undone too");
    d.click_cell(0, 0);
    d.ctrl("z");
    assert_eq!(shown(&ui, 0, 0), "2", "A1 is back to 2");
    assert_eq!(shown(&ui, 3, 0), "5");

    // (2) Save with no file: the picker cannot be asked here, so a path is typed instead.
    let dir = std::env::temp_dir().join(format!("ysheets-window-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("driven.csv");
    d.ctrl("s");
    d.wait("the save dialog", || ui.get_dialog() == SheetDialog::SavePath);
    assert!(ui.get_dialog_message().contains("file picker could not be asked"));
    ui.set_dialog_path(path.display().to_string().into());
    ui.invoke_dialog_confirm();
    d.tick();
    assert_eq!(ui.get_dialog(), SheetDialog::None);
    assert!(!ui.get_modified(), "saved");
    assert_eq!(ui.get_file_name(), "driven.csv");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "2\r\n3\r\n\r\n=SUM(A1:A3)\r\n");

    // Change it, then Open: the unsaved question comes first; Discard, then type the path.
    d.click_cell(5, 3);
    d.typed("scratch\n");
    assert!(ui.get_modified());
    ui.invoke_command("open".into());
    d.tick();
    assert_eq!(ui.get_dialog(), SheetDialog::Unsaved);
    ui.invoke_dialog_discard();
    d.wait("the open dialog", || ui.get_dialog() == SheetDialog::OpenPath);
    ui.set_dialog_path(path.display().to_string().into());
    ui.invoke_dialog_confirm();
    d.tick();
    assert_eq!(shown(&ui, 3, 0), "5", "reopened, the formula is worked out again");
    assert_eq!(shown(&ui, 5, 3), "", "and the discarded edit is gone");
    assert_eq!(ui.get_rows().row_count(), 50, "a four-row file still opens a full grid");
    assert!(!ui.get_modified());

    // A save through the picker, when it answers: no dialog, straight to the file.
    let picked = dir.join("picked.csv");
    std::fs::write(dir.join(".picked"), picked.display().to_string()).unwrap();
    fn chose(_saving: bool, _current: Option<PathBuf>) -> Choice {
        let dir = std::env::temp_dir().join(format!("ysheets-window-{}", std::process::id()));
        Choice::Chosen(PathBuf::from(std::fs::read_to_string(dir.join(".picked")).unwrap()))
    }
    files::ASK.with(|a| a.set(chose));
    ui.invoke_command("save-as".into());
    d.wait("the picked save", || picked.exists());
    assert_eq!(ui.get_file_name(), "picked.csv");

    // The close button asks about unsaved work, and Cancel keeps the window.
    d.click_cell(0, 2);
    d.typed("x\n");
    window.dispatch_event(WindowEvent::CloseRequested);
    d.tick();
    assert_eq!(ui.get_dialog(), SheetDialog::Unsaved);
    ui.invoke_dialog_cancel();
    d.tick();
    assert_eq!(ui.get_dialog(), SheetDialog::None);
    assert!(ui.window().is_visible(), "Cancel keeps the window open");

    render::render(&ui, &shared.borrow());
    screenshot(&window, "ysheets-driven.png");
    let _ = std::fs::remove_dir_all(&dir);
}

fn screenshot(window: &MinimalSoftwareWindow, name: &str) {
    let size = window.size();
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(size.width, size.height);
    window.request_redraw();
    window.draw_if_needed(|r| {
        r.render(pixels.make_mut_slice(), size.width as usize);
    });
    let dir = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("ui-screenshots")))
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut png = png::Encoder::new(std::fs::File::create(dir.join(name)).unwrap(), size.width, size.height);
    png.set_color(png::ColorType::Rgb);
    png.set_depth(png::BitDepth::Eight);
    png.write_header().unwrap().write_image_data(pixels.as_bytes()).unwrap();
}
