//! Keys: which are the Terminal's own shortcuts, and which bytes the rest become for the shell.
//! Pure functions of the key and a few facts about the window, so the window's callbacks can
//! answer accept or reject at once and hand everything else to a command.

use slint::platform::Key;
use slint::private_unstable_api::re_exports::KeyEvent;
use slint::SharedString;

use crate::command::{Command, Do};

/// What a key did, as the window's callback needs to say it.
#[derive(Debug, PartialEq)]
pub enum Pressed {
    /// Not the Terminal's: let it go on.
    Ignored,
    /// A command, and the key is used up.
    Command(Stroke),
    /// Find opens or closes; the window owns whether it is open.
    Find(bool),
    CopyScreen,
    PasteClipboard,
}

/// A command a key can make. Kept apart from [`Command`] so this module's results compare.
#[derive(Debug, PartialEq)]
pub enum Stroke {
    Do(Do),
    Scroll(i32),
    Input(Vec<u8>),
}

impl From<Stroke> for Command {
    fn from(c: Stroke) -> Command {
        match c {
            Stroke::Do(d) => Command::Do(d),
            Stroke::Scroll(n) => Command::Scroll(n),
            Stroke::Input(b) => Command::Input(b),
        }
    }
}

/// What the window knows that decides a key.
pub struct Window {
    /// The close prompt is up.
    pub asking: bool,
    /// The find bar is open.
    pub finding: bool,
    /// The active shell has asked for application cursor keys.
    pub application: bool,
    pub rows: u16,
    /// A shell is there to type into.
    pub has_shell: bool,
}

/// The Terminal's shortcuts. `None` when the key is not one of them.
pub fn shortcut(event: &KeyEvent, window: &Window) -> Option<Pressed> {
    if is_modifier_key(&event.text) {
        return None;
    }
    let text = &event.text;
    if *text == SharedString::from(Key::Escape) && window.asking {
        return Some(Pressed::Command(Stroke::Do(Do::CancelClose)));
    }
    if *text == SharedString::from(Key::Escape) && window.finding {
        return Some(Pressed::Find(false));
    }
    if event.modifiers.control && event.modifiers.shift {
        return Some(match text.to_ascii_lowercase().as_str() {
            "t" | "\x14" => Pressed::Command(Stroke::Do(Do::New)),
            "w" | "\x17" => Pressed::Command(Stroke::Do(Do::Close)),
            "f" | "\x06" => Pressed::Find(!window.finding),
            "c" | "\x03" => Pressed::CopyScreen,
            "v" | "\x16" => Pressed::PasteClipboard,
            "+" | "=" => Pressed::Command(Stroke::Do(Do::ZoomIn)),
            _ => return None,
        });
    }
    if event.modifiers.control {
        if *text == SharedString::from(Key::PageDown) {
            return Some(Pressed::Command(Stroke::Do(Do::NextTab)));
        }
        if *text == SharedString::from(Key::PageUp) {
            return Some(Pressed::Command(Stroke::Do(Do::PreviousTab)));
        }
        return Some(Pressed::Command(Stroke::Do(match text.as_str() {
            "+" | "=" => Do::ZoomIn,
            "-" => Do::ZoomOut,
            "0" => Do::ZoomReset,
            _ => return None,
        })));
    }
    None
}

/// A key typed at the terminal: a shortcut, a scroll, or bytes for the shell.
pub fn pressed(event: &KeyEvent, window: &Window) -> Pressed {
    if let Some(p) = shortcut(event, window) {
        return p;
    }
    if !window.has_shell {
        return Pressed::Ignored;
    }
    let page = window.rows as i32 - 1;
    if event.modifiers.shift && event.text == SharedString::from(Key::PageUp) {
        return Pressed::Command(Stroke::Scroll(page));
    }
    if event.modifiers.shift && event.text == SharedString::from(Key::PageDown) {
        return Pressed::Command(Stroke::Scroll(-page));
    }
    match encode_key(event, window.application) {
        Some(bytes) => Pressed::Command(Stroke::Input(bytes)),
        None => Pressed::Ignored,
    }
}

pub fn is_modifier_key(text: &SharedString) -> bool {
    // Slint uses U+0010..U+0018 for physical modifier keys. These overlap
    // shell control bytes (Shift is Ctrl+P), so never forward them to a PTY.
    text.len() == 1 && matches!(text.as_bytes()[0], 0x10..=0x18)
}

pub fn encode_key(event: &KeyEvent, application: bool) -> Option<Vec<u8>> {
    let text = &event.text;
    if is_modifier_key(text) {
        return None;
    }
    let modifier = 1
        + u8::from(event.modifiers.shift)
        + 2 * u8::from(event.modifiers.alt)
        + 4 * u8::from(event.modifiers.control);
    for (key, letter) in [
        (Key::UpArrow, 'A'),
        (Key::DownArrow, 'B'),
        (Key::RightArrow, 'C'),
        (Key::LeftArrow, 'D'),
        (Key::Home, 'H'),
        (Key::End, 'F'),
    ] {
        if text == &SharedString::from(key) {
            return Some(
                if modifier > 1 {
                    format!("\x1b[1;{modifier}{letter}")
                } else {
                    format!("\x1b{}{letter}", if application { "O" } else { "[" })
                }
                .into_bytes(),
            );
        }
    }
    for (key, number) in [
        (Key::Insert, 2),
        (Key::Delete, 3),
        (Key::PageUp, 5),
        (Key::PageDown, 6),
        (Key::F5, 15),
        (Key::F6, 17),
        (Key::F7, 18),
        (Key::F8, 19),
        (Key::F9, 20),
        (Key::F10, 21),
        (Key::F11, 23),
        (Key::F12, 24),
    ] {
        if text == &SharedString::from(key) {
            return Some(
                if modifier > 1 { format!("\x1b[{number};{modifier}~") } else { format!("\x1b[{number}~") }.into_bytes(),
            );
        }
    }
    for (key, letter) in [(Key::F1, 'P'), (Key::F2, 'Q'), (Key::F3, 'R'), (Key::F4, 'S')] {
        if text == &SharedString::from(key) {
            return Some(format!("\x1bO{letter}").into_bytes());
        }
    }
    let mut result = if text == &SharedString::from(Key::Return) || text == "\n" || text == "\r" {
        vec![b'\r']
    } else if text == &SharedString::from(Key::Backspace) {
        vec![127]
    } else if text == &SharedString::from(Key::Escape) {
        vec![27]
    } else if text == &SharedString::from(Key::Tab) {
        if event.modifiers.shift {
            b"\x1b[Z".to_vec()
        } else {
            vec![9]
        }
    } else {
        if text.is_empty() || text.chars().any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c)) {
            return None;
        }
        let bytes = text.as_bytes();
        if event.modifiers.control && bytes.len() == 1 && (b'@'..=b'_').contains(&bytes[0].to_ascii_uppercase()) {
            vec![bytes[0].to_ascii_uppercase() & 31]
        } else if event.modifiers.control && text == " " {
            vec![0]
        } else {
            bytes.to_vec()
        }
    };
    if event.modifiers.alt {
        result.insert(0, 27);
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: impl Into<SharedString>) -> KeyEvent {
        let mut e = KeyEvent::default();
        e.text = text.into();
        e
    }

    fn window() -> Window {
        Window { asking: false, finding: false, application: false, rows: 24, has_shell: true }
    }

    #[test]
    fn keyboard_encoding_preserves_shell_controls_and_application_mode() {
        let mut e = key("c");
        e.modifiers.control = true;
        assert_eq!(encode_key(&e, false).unwrap(), b"\x03");
        e.modifiers.control = false;
        e.text = Key::UpArrow.into();
        assert_eq!(encode_key(&e, false).unwrap(), b"\x1b[A");
        assert_eq!(encode_key(&e, true).unwrap(), b"\x1bOA");
        e.modifiers.control = true;
        assert_eq!(encode_key(&e, false).unwrap(), b"\x1b[1;5A");
        e.modifiers.control = false;
        e.text = Key::Tab.into();
        assert_eq!(encode_key(&e, false).unwrap(), b"\t");
        e.text = "日本語".into();
        assert_eq!(encode_key(&e, false).unwrap(), "日本語".as_bytes());
        e.text = Key::Backspace.into();
        assert_eq!(encode_key(&e, false).unwrap(), b"\x7f");
    }

    #[test]
    fn physical_modifiers_never_become_shell_control_bytes() {
        let mut event = KeyEvent::default();
        for key in [
            Key::Shift,
            Key::ShiftR,
            Key::Control,
            Key::ControlR,
            Key::Alt,
            Key::AltGr,
            Key::Meta,
            Key::MetaR,
            Key::CapsLock,
        ] {
            event.text = key.into();
            assert!(encode_key(&event, false).is_none(), "Modifier was forwarded: {key:?}");
            assert_eq!(pressed(&event, &window()), Pressed::Ignored, "{key:?}");
        }
        event.modifiers.control = true;
        event.text = "p".into();
        assert_eq!(encode_key(&event, false).unwrap(), b"\x10");
        event.text = "q".into();
        assert_eq!(encode_key(&event, false).unwrap(), b"\x11");
    }

    #[test]
    fn the_shortcuts_are_the_ones_they_were() {
        let mut e = key("t");
        e.modifiers.control = true;
        e.modifiers.shift = true;
        assert_eq!(shortcut(&e, &window()), Some(Pressed::Command(Stroke::Do(Do::New))));
        e.text = "w".into();
        assert_eq!(shortcut(&e, &window()), Some(Pressed::Command(Stroke::Do(Do::Close))));
        e.text = "f".into();
        assert_eq!(shortcut(&e, &window()), Some(Pressed::Find(true)));
        let finding = Window { finding: true, ..window() };
        assert_eq!(shortcut(&e, &finding), Some(Pressed::Find(false)));
        e.modifiers.shift = false;
        e.text = Key::PageDown.into();
        assert_eq!(shortcut(&e, &window()), Some(Pressed::Command(Stroke::Do(Do::NextTab))));
        e.text = "-".into();
        assert_eq!(shortcut(&e, &window()), Some(Pressed::Command(Stroke::Do(Do::ZoomOut))));
        e.text = "x".into();
        assert_eq!(shortcut(&e, &window()), None, "Ctrl+X goes to the shell");
        assert_eq!(pressed(&e, &window()), Pressed::Command(Stroke::Input(vec![0x18])));
    }

    #[test]
    fn escape_answers_the_prompt_first_then_the_find_bar_then_the_shell() {
        let esc = key(Key::Escape);
        let both = Window { asking: true, finding: true, ..window() };
        assert_eq!(pressed(&esc, &both), Pressed::Command(Stroke::Do(Do::CancelClose)));
        let finding = Window { finding: true, ..window() };
        assert_eq!(pressed(&esc, &finding), Pressed::Find(false));
        assert_eq!(pressed(&esc, &window()), Pressed::Command(Stroke::Input(vec![27])));
    }

    #[test]
    fn shift_page_keys_scroll_the_history_a_page() {
        let mut e = key(Key::PageUp);
        e.modifiers.shift = true;
        assert_eq!(pressed(&e, &window()), Pressed::Command(Stroke::Scroll(23)));
        e.text = Key::PageDown.into();
        assert_eq!(pressed(&e, &window()), Pressed::Command(Stroke::Scroll(-23)));
    }

    #[test]
    fn with_no_shell_a_key_is_not_taken() {
        let none = Window { has_shell: false, ..window() };
        assert_eq!(pressed(&key("a"), &none), Pressed::Ignored);
    }
}
