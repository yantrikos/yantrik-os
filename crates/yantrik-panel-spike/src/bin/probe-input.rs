//! Virtual pointer and keyboard for the spike's headless checks (a headless compositor has no
//! input devices). Talks `zwlr_virtual_pointer_v1` and `zwp_virtual_keyboard_v1`, which labwc
//! offers to any client of the session; that is a security-relevant global, see the design doc.
//!
//!   probe-input OP...   with OP one of
//!     move:X:Y:W:H   absolute pointer position within a W x H layout extent
//!     click          left button press and release
//!     type:TEXT      lowercase letters, one key each
//!     key:CODE       one evdev key code (Escape is 1)
//!     sleep:MS

use std::io::Write;
use std::os::fd::AsFd;
use std::time::Duration;

use wayland_client::{
    delegate_noop,
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_registry, wl_seat},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1, zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1, zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

struct S;
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for S {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}
delegate_noop!(S: ignore wl_seat::WlSeat);
delegate_noop!(S: ZwlrVirtualPointerManagerV1);
delegate_noop!(S: ZwlrVirtualPointerV1);
delegate_noop!(S: ZwpVirtualKeyboardManagerV1);
delegate_noop!(S: ZwpVirtualKeyboardV1);

/// evdev codes for a-z on a US layout.
fn evdev(c: char) -> Option<u32> {
    const ROWS: [(&str, u32); 3] = [("qwertyuiop", 16), ("asdfghjkl", 30), ("zxcvbnm", 44)];
    ROWS.iter().find_map(|(row, base)| row.find(c).map(|i| base + i as u32))
}

fn main() {
    let conn = Connection::connect_to_env().expect("wayland");
    let (globals, mut queue) = registry_queue_init::<S>(&conn).expect("registry");
    let qh = queue.handle();
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ()).expect("seat");
    let pointer = globals
        .bind::<ZwlrVirtualPointerManagerV1, _, _>(&qh, 1..=2, ())
        .ok()
        .map(|m| m.create_virtual_pointer(Some(&seat), &qh, ()));
    let keyboard = globals
        .bind::<ZwpVirtualKeyboardManagerV1, _, _>(&qh, 1..=1, ())
        .ok()
        .map(|m| m.create_virtual_keyboard(&seat, &qh, ()));
    println!("probe-input: virtual pointer {}, virtual keyboard {}", pointer.is_some(), keyboard.is_some());
    if let Some(k) = &keyboard {
        // A keymap is required before any key. The client-side xkbcommon renders the US one.
        let ctx = xkbcommon::xkb::Context::new(xkbcommon::xkb::CONTEXT_NO_FLAGS);
        let map = xkbcommon::xkb::Keymap::new_from_names(&ctx, "", "", "us", "", None, xkbcommon::xkb::KEYMAP_COMPILE_NO_FLAGS)
            .expect("us keymap");
        let text = map.get_as_string(xkbcommon::xkb::KEYMAP_FORMAT_TEXT_V1);
        let path = format!("{}/probe-keymap", std::env::var("XDG_RUNTIME_DIR").unwrap());
        let mut file = std::fs::File::options().read(true).write(true).create(true).truncate(true).open(path).unwrap();
        file.write_all(text.as_bytes()).unwrap();
        file.write_all(&[0]).unwrap();
        k.keymap(1, file.as_fd(), text.len() as u32 + 1);
    }
    queue.roundtrip(&mut S).ok();
    let mut t: u32 = 1000;
    for op in std::env::args().skip(1) {
        let parts: Vec<&str> = op.split(':').collect();
        match parts[0] {
            "move" => {
                let n: Vec<u32> = parts[1..].iter().map(|v| v.parse().unwrap()).collect();
                if let Some(p) = &pointer {
                    p.motion_absolute(t, n[0], n[1], n[2], n[3]);
                    p.frame();
                }
            }
            "click" => {
                if let Some(p) = &pointer {
                    use wayland_client::protocol::wl_pointer::ButtonState;
                    p.button(t, 0x110, ButtonState::Pressed);
                    p.frame();
                    p.button(t + 10, 0x110, ButtonState::Released);
                    p.frame();
                }
            }
            "type" => {
                for code in parts[1].chars().filter_map(evdev) {
                    if let Some(k) = &keyboard {
                        k.key(t, code, 1);
                        k.key(t + 10, code, 0);
                    }
                    t += 20;
                    conn.flush().ok();
                    queue.roundtrip(&mut S).ok();
                }
            }
            "key" => {
                if let Some(k) = &keyboard {
                    let code: u32 = parts[1].parse().unwrap();
                    k.key(t, code, 1);
                    k.key(t + 10, code, 0);
                }
            }
            "sleep" => {
                queue.roundtrip(&mut S).ok();
                std::thread::sleep(Duration::from_millis(parts[1].parse().unwrap()));
            }
            other => eprintln!("probe-input: unknown op {other}"),
        }
        t += 50;
        conn.flush().ok();
        queue.roundtrip(&mut S).ok();
    }
}
