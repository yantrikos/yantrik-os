//! The Lens in its own window (lens_window.slint's `LensWindow`, #733), at the size the shell
//! gives it on a 1280x800 display: the 440px panel plus its 12px gap, from under the 36px bar to
//! above the 48px dock (452x716).
//!
//! Checks what a person would see and do: closed, the window draws nothing; shown, the panel
//! fills its strip; the field has the keyboard the moment it opens, and what is typed reaches
//! the shell (`input-edited`, the App's `lens-input-text`); Escape asks the shell to close it;
//! in a conversation the composer has the keyboard and Return sends. Renders are
//! `lens-window-search.png` and `lens-window-chat.png`, next to the output path given.
use super::*;
use slint::platform::{Key, WindowEvent};
use slint::{Model, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::time::Duration;

const WIDTH: u32 = 452;
const HEIGHT: u32 = 716;

fn save(pixels: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), WIDTH, HEIGHT);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    println!("Rendered {path} ({WIDTH}×{HEIGHT})");
    Ok(())
}

fn key(w: &MinimalSoftwareWindow, text: impl Into<SharedString>) {
    let text = text.into();
    w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.dispatch_event(WindowEvent::KeyReleased { text });
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = LensWindow::new()?;
    let edited: Rc<RefCell<Vec<String>>> = Rc::default();
    let sent: Rc<RefCell<Vec<String>>> = Rc::default();
    let closed = Rc::new(std::cell::Cell::new(0));
    {
        let edited = edited.clone();
        ui.on_input_edited(move |t| edited.borrow_mut().push(t.to_string()));
        let sent = sent.clone();
        ui.on_send_message(move |t| sent.borrow_mut().push(t.to_string()));
        let closed = closed.clone();
        ui.on_close_lens(move || closed.set(closed.get() + 1));
    }
    let mut view = ui.get_view();
    view.screen_height = 800.0;
    view.mind_name = "Yantrik Mind".into();
    view.companion_online = true;
    ui.set_view(view.clone());
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
    w.dispatch_event(WindowEvent::WindowActiveChanged(true));

    let draw = || {
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(WIDTH, HEIGHT);
        slint::platform::update_timers_and_animations();
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), WIDTH as usize); });
        pixels
    };
    // The panel slides in over 200ms; a few frames spread past it.
    let settle = || {
        let mut last = draw();
        for _ in 0..6 {
            std::thread::sleep(Duration::from_millis(60));
            last = draw();
        }
        last
    };
    let at = |p: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, x: u32, y: u32| p.as_slice()[(y * WIDTH + x) as usize];

    let closed_frame = settle();
    // The panel's own strip: 12px down from the top, 440px across from the left, 12px up from the
    // foot. Each corner of its inside is drawn once it is open, and nothing was there before.
    let inside = [(8, 40), (432, 40), (220, 24), (220, HEIGHT - 24), (8, HEIGHT - 40), (432, HEIGHT - 40)];

    // ── Opened as the ask bar ──
    view.shown = true;
    ui.set_view(view.clone());
    let search = settle();
    save(&search, &w_named(output, "lens-window-search.png"))?;
    for (x, y) in inside {
        assert_ne!(at(&search, x, y), at(&closed_frame, x, y), "the panel is drawn at ({x}, {y}) once it is shown");
    }
    // The field has the keyboard as it opens: the first key typed is in it, and the shell hears it.
    key(w, "h");
    key(w, "i");
    draw();
    assert_eq!(ui.get_input_text().as_str(), "hi", "typing goes straight into the field that opened");
    assert_eq!(edited.borrow().last().map(String::as_str), Some("hi"), "and reaches the shell's lens-input-text");
    key(w, Key::Escape);
    draw();
    assert_eq!(closed.get(), 1, "Escape asks the shell to put the Lens away");

    // ── Opened in a conversation ──
    view.chat_mode = true;
    view.messages = ModelRc::new(VecModel::from(vec![MessageData {
        role: "assistant".into(),
        content: "Notes is open on the left; I can work beside it.".into(),
        is_streaming: false,
        blocks: ModelRc::new(VecModel::from(Vec::<ContentBlock>::new())),
        run: "".into(),
    }]));
    ui.set_view(view.clone());
    let chat = settle();
    save(&chat, &w_named(output, "lens-window-chat.png"))?;
    for (x, y) in inside {
        assert_ne!(at(&chat, x, y), at(&closed_frame, x, y), "the conversation fills the panel at ({x}, {y})");
    }
    assert!(ui.get_view().messages.row_count() == 1);
    // The composer has the keyboard, and Return sends what is in it.
    for c in ["o", "k"] {
        key(w, c);
    }
    key(w, Key::Return);
    draw();
    assert_eq!(sent.borrow().as_slice(), ["ok"], "the composer had the keyboard, and Return sent it");

    // ── Put away ──
    view.shown = false;
    ui.set_view(view);
    let gone = settle();
    for (x, y) in inside {
        assert_eq!(at(&gone, x, y), at(&closed_frame, x, y), "closed, the window draws nothing at ({x}, {y})");
    }
    println!("PASS: the Lens draws in its own 452x716 window, takes the keyboard as it opens, forwards typing, Escape and a sent message to the shell, and draws nothing once put away");
    Ok(())
}

fn w_named(output: &str, name: &str) -> String {
    std::path::Path::new(output).with_file_name(name).to_string_lossy().into_owned()
}
