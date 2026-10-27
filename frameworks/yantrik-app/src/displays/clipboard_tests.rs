//! H1 (#972 review): a Mind's window never reaches the person's clipboard or primary selection,
//! in either direction. A real streamed window with a text field, on this crate's platform over a
//! recording one, is given what a helper would send: Ctrl+A, Ctrl+C, Ctrl+X, Ctrl+V, a drag
//! selection and a middle click. The same input dispatched as the person's (not through the
//! helper) reaches the recording clipboard, so the path is a live one.

use std::rc::Rc;
use std::sync::{Arc, Mutex};

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Clipboard, Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::ComponentHandle as _;

use crate::displays::platform::Displays;
use crate::displays::streamed;
use crate::displays::wire::{FromHelper, PointerKind};

slint::slint! {
    export component ClipWin inherits Window {
        width: 320px;
        height: 80px;
        in-out property <string> typed: "typed in the Mind's window";
        field := TextInput {
            width: 100%;
            height: 100%;
            text <=> root.typed;
        }
        init => { field.focus(); }
    }
}

/// What the person's seat was asked: each read and write of its clipboard.
type Asked = Arc<Mutex<Vec<String>>>;

struct Recording {
    asked: Asked,
}

impl Platform for Recording {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }

    fn set_clipboard_text(&self, text: &str, clipboard: Clipboard) {
        self.asked.lock().unwrap().push(format!("write {clipboard:?}: {text}"));
    }

    fn clipboard_text(&self, clipboard: Clipboard) -> Option<String> {
        self.asked.lock().unwrap().push(format!("read {clipboard:?}"));
        Some("the person's password".into())
    }
}

fn key(text: impl Into<slint::SharedString>) -> String {
    text.into().to_string()
}

/// What a helper sends for Ctrl+<letter>.
fn chord(letter: &str) -> Vec<FromHelper> {
    let ctrl = key(slint::platform::Key::Control);
    vec![
        FromHelper::Key { pressed: true, text: ctrl.clone() },
        FromHelper::Key { pressed: true, text: letter.into() },
        FromHelper::Key { pressed: false, text: letter.into() },
        FromHelper::Key { pressed: false, text: ctrl },
    ]
}

/// The input a Mind might give its window to reach the person's clipboard.
fn reaching_for_the_clipboard() -> Vec<FromHelper> {
    let mut all = vec![FromHelper::Focus(true)];
    for letter in ["a", "c", "x", "v"] {
        all.extend(chord(letter));
    }
    let at = |kind, button, x| FromHelper::Pointer { kind, button, x, y: 20.0 };
    // A drag selection, which sets the primary selection; then a middle click, which pastes it.
    all.extend([at(PointerKind::Pressed, 0, 5.0), at(PointerKind::Moved, 0, 150.0), at(PointerKind::Released, 0, 150.0)]);
    all.extend([at(PointerKind::Pressed, 2, 40.0), at(PointerKind::Released, 2, 40.0)]);
    all
}

/// The same input as the person's own window system would dispatch it.
fn as_the_persons(window: &slint::Window, heard: &FromHelper) {
    let at = |x: f32, y: f32| slint::LogicalPosition::new(x, y);
    let button = |b: u8| if b == 2 { PointerEventButton::Middle } else { PointerEventButton::Left };
    window.dispatch_event(match heard {
        FromHelper::Key { pressed: true, text } => WindowEvent::KeyPressed { text: text.as_str().into() },
        FromHelper::Key { pressed: false, text } => WindowEvent::KeyReleased { text: text.as_str().into() },
        FromHelper::Focus(on) => WindowEvent::WindowActiveChanged(*on),
        FromHelper::Pointer { kind: PointerKind::Pressed, button: b, x, y } => WindowEvent::PointerPressed { position: at(*x, *y), button: button(*b) },
        FromHelper::Pointer { kind: PointerKind::Released, button: b, x, y } => WindowEvent::PointerReleased { position: at(*x, *y), button: button(*b) },
        FromHelper::Pointer { x, y, .. } => WindowEvent::PointerMoved { position: at(*x, *y) },
        _ => return,
    });
}

#[test]
fn a_minds_window_neither_reads_nor_writes_the_persons_clipboard_or_selection() {
    let asked: Asked = Arc::default();
    let seen = asked.clone();
    let (typed, by_the_mind) = std::thread::spawn(move || {
        let log = seen.clone();
        slint::platform::set_platform(Box::new(Displays::over(Box::new(Recording { asked: seen }))))
            .expect("a platform of this thread's own");
        // The Mind's window: made inside `streamed::make`, so the platform streams it.
        let (made, adapter) = streamed::make(ClipWin::new);
        let ui = made.expect("a streamed window");
        let adapter = adapter.expect("this crate's platform makes a streamed window");
        ui.show().unwrap();
        adapter.deliver(FromHelper::Size { width: 320, height: 80, scale: 1.0 });
        for heard in reaching_for_the_clipboard() {
            adapter.deliver(heard);
        }
        let by_the_mind = log.lock().unwrap().clone();
        // The person's own window, given the same: the clipboard is there to reach.
        let persons = ClipWin::new().unwrap();
        persons.show().unwrap();
        for heard in reaching_for_the_clipboard() {
            as_the_persons(persons.window(), &heard);
        }
        (ui.get_typed().to_string(), by_the_mind)
    })
    .join()
    .expect("the test thread");
    assert!(by_the_mind.is_empty(), "the Mind's window reached the person's clipboard: {by_the_mind:?}");
    let asked = asked.lock().unwrap().clone();
    assert!(asked.iter().any(|a| a.starts_with("read")), "the person's window pastes, so the check is live: {asked:?}");
    assert!(asked.iter().any(|a| a.starts_with("write")), "the person's window copies, so the check is live: {asked:?}");
    assert!(!typed.contains("password"), "nothing of the person's clipboard reached the Mind's window: {typed}");
}
