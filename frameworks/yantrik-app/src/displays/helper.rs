//! The view helper: a Mind's launch of the app, kept alive on the Mind's display to show the
//! window the instance draws for it there.
//!
//! The shell starts a Mind's `open_app` on Mind View's display. That launch finds the app's one
//! instance (or starts it on the person's display, [`super::spawn`]), listens on a socket of its
//! own, and hands its launch over with the socket's path. The instance checks it and connects; from
//! then on this process shows the frames it is sent in a plain window and sends back what the
//! pointer and keys do there. It opens no store and runs no model: the app's data has one writer,
//! the instance.
//!
//! It ends when the instance closes the window or goes away (the socket closes), and when its
//! own window is closed: it asks the instance, which closes the Mind's window.

use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Rgb8Pixel, SharedPixelBuffer};

use super::wire::{self, FromHelper, PointerKind, ToHelper};
use super::VIEW_PREFIX;

/// How long the helper waits, after its hand-over was answered, for the instance to connect.
pub const CONNECT_WAIT: Duration = Duration::from_secs(5);

slint::slint! {
    export component ViewWindow inherits Window {
        in property <image> frame;
        in property <string> window-title;
        in property <length> least-width;
        in property <length> least-height;
        callback pointer(int, int, length, length);
        callback scroll(length, length, length, length);
        callback key(bool, string);
        callback resized();
        title: root.window-title;
        no-frame: true;
        background: #1e1e1e;
        min-width: root.least-width;
        min-height: root.least-height;
        forward-focus: keys;
        changed width => { root.resized(); }
        changed height => { root.resized(); }
        keys := FocusScope {
            key-pressed(e) => { root.key(true, e.text); accept }
            key-released(e) => { root.key(false, e.text); accept }
            Image {
                width: 100%;
                height: 100%;
                source: root.frame;
                image-fit: fill;
            }
            area := TouchArea {
                changed mouse-x => { root.pointer(0, 0, self.mouse-x, self.mouse-y); }
                changed mouse-y => { root.pointer(0, 0, self.mouse-x, self.mouse-y); }
                changed has-hover => {
                    if (!self.has-hover) { root.pointer(3, 0, self.mouse-x, self.mouse-y); }
                }
                pointer-event(e) => {
                    if (e.kind == PointerEventKind.down || e.kind == PointerEventKind.up) {
                        root.pointer(
                            e.kind == PointerEventKind.down ? 1 : 2,
                            e.button == PointerEventButton.right ? 1 : e.button == PointerEventButton.middle ? 2 : 0,
                            self.mouse-x, self.mouse-y);
                    }
                }
                scroll-event(e) => {
                    root.scroll(self.mouse-x, self.mouse-y, e.delta-x, e.delta-y);
                    accept
                }
            }
        }
    }
}

/// The socket a helper listens on, removed when it is dropped.
pub struct Listening {
    listener: UnixListener,
    path: PathBuf,
}

impl Listening {
    /// A fresh socket for a helper of `instance`, in the session's private socket directory.
    pub fn bind(instance: &str) -> io::Result<Listening> {
        let dir = yantrik_ipc_transport::server::socket_dir();
        let tidy: String = instance.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
        let path = dir.join(format!("{VIEW_PREFIX}{tidy}-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Listening { listener, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The instance's connection, once it comes, within `wait`: of this account and, when its pid
    /// file names it, the instance itself. Anything else that connects is turned away.
    fn accept(&self, wait: Duration, instance: Option<u32>) -> io::Result<UnixStream> {
        self.listener.set_nonblocking(true)?;
        let deadline = Instant::now() + wait;
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    let ours = match super::link::peer(&stream) {
                        Some((pid, same_account)) => same_account && instance.is_none_or(|i| i as i32 == pid),
                        None => !cfg!(target_os = "linux"),
                    };
                    if !ours {
                        tracing::warn!("a connection to the view helper that is not the app's instance; refused");
                        continue;
                    }
                    stream.set_nonblocking(false)?;
                    return Ok(stream);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for Listening {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Shows the Mind's window the instance `instance` draws, until it closes. `Err` says why it
/// could not.
pub fn show(listening: Listening, instance: &str) -> Result<(), String> {
    let stream = listening
        .accept(CONNECT_WAIT, crate::instance::holder(instance))
        .map_err(|e| format!("the app's instance did not connect to draw the Mind's window ({e})"))?;
    // Nothing else may connect now.
    drop(listening);
    let reading = stream.try_clone().map_err(|e| e.to_string())?;
    let writing = Arc::new(Mutex::new(stream));
    let ui = ViewWindow::new().map_err(|e| format!("no window on this display: {e}"))?;
    wire_up(&ui, &writing);
    let weak = ui.as_weak();
    std::thread::Builder::new()
        .name("yantrik-view-in".into())
        .spawn(move || read_loop(reading, weak))
        .map_err(|e| e.to_string())?;
    ui.show().map_err(|e| format!("the Mind's window could not be shown: {e}"))?;
    say(&writing, &size_of(&ui));
    slint::run_event_loop_until_quit().map_err(|e| e.to_string())?;
    let _ = ui.hide();
    Ok(())
}

type Out = Arc<Mutex<UnixStream>>;

fn say(out: &Out, message: &FromHelper) {
    let mut stream = out.lock().unwrap_or_else(|p| p.into_inner());
    if wire::send(&mut *stream, &message.encode()).is_err() {
        // The instance is gone; the reading side ends the window.
        let _ = stream.shutdown(std::net::Shutdown::Both);
    }
}

fn size_of(ui: &ViewWindow) -> FromHelper {
    let size = ui.window().size();
    FromHelper::Size { width: size.width.max(1), height: size.height.max(1), scale: ui.window().scale_factor() }
}

fn wire_up(ui: &ViewWindow, out: &Out) {
    let o = out.clone();
    ui.on_pointer(move |kind, button, x, y| {
        let kind = match kind {
            1 => PointerKind::Pressed,
            2 => PointerKind::Released,
            3 => PointerKind::Exited,
            _ => PointerKind::Moved,
        };
        say(&o, &FromHelper::Pointer { kind, button: button.clamp(0, 2) as u8, x, y });
    });
    let o = out.clone();
    ui.on_scroll(move |x, y, dx, dy| say(&o, &FromHelper::Scroll { x, y, dx, dy }));
    let o = out.clone();
    ui.on_key(move |pressed, text| say(&o, &FromHelper::Key { pressed, text: text.to_string() }));
    let (o, weak) = (out.clone(), ui.as_weak());
    ui.on_resized(move || {
        if let Some(ui) = weak.upgrade() {
            say(&o, &size_of(&ui));
        }
    });
    let o = out.clone();
    ui.window().on_close_requested(move || {
        // The instance closes the Mind's window, and then this one ends.
        say(&o, &FromHelper::CloseRequested);
        slint::CloseRequestResponse::KeepWindowShown
    });
}

fn read_loop(mut stream: UnixStream, ui: slint::Weak<ViewWindow>) {
    let mut frame: Option<SharedPixelBuffer<Rgb8Pixel>> = None;
    let mut first_props = true;
    loop {
        let message = match wire::receive(&mut stream) {
            Ok(Some(body)) => ToHelper::decode(&body),
            Ok(None) => Ok(ToHelper::Close),
            Err(e) => Err(e),
        };
        let message = match message {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(error = %e, "the instance sent what it never sends; the window closes");
                ToHelper::Close
            }
        };
        if let ToHelper::Frame { width, height, x, y, w, h, rgb } = &message {
            let buffer = frame
                .take()
                .filter(|b| b.width() == *width && b.height() == *height)
                .unwrap_or_else(|| SharedPixelBuffer::new(*width, *height));
            let mut buffer = buffer;
            patch(buffer.make_mut_slice(), *width as usize, (*x, *y, *w, *h), rgb);
            let shown = buffer.clone();
            frame = Some(buffer);
            let _ = ui.upgrade_in_event_loop(move |ui| ui.set_frame(slint::Image::from_rgb8(shown)));
            continue;
        }
        let closing = message == ToHelper::Close;
        let first = first_props;
        if matches!(message, ToHelper::Props { .. }) {
            first_props = false;
        }
        let _ = ui.upgrade_in_event_loop(move |ui| apply(&ui, message, first));
        if closing {
            let _ = slint::invoke_from_event_loop(|| {
                let _ = slint::quit_event_loop();
            });
            return;
        }
    }
}

/// One patch of rows into the frame.
fn patch(into: &mut [Rgb8Pixel], width: usize, (x, y, w, h): (u32, u32, u32, u32), rgb: &[u8]) {
    let (x, y, w) = (x as usize, y as usize, w as usize);
    for row in 0..h as usize {
        let line = &rgb[row * w * 3..(row + 1) * w * 3];
        let start = (y + row) * width + x;
        for (p, c) in into[start..start + w].iter_mut().zip(line.chunks_exact(3)) {
            *p = Rgb8Pixel { r: c[0], g: c[1], b: c[2] };
        }
    }
}

fn apply(ui: &ViewWindow, message: ToHelper, first: bool) {
    use slint::winit_030::WinitWindowAccessor;
    match message {
        ToHelper::Props { title, preferred, min, minimized, maximized } => {
            ui.set_window_title(title.into());
            ui.set_least_width(min.0 as f32);
            ui.set_least_height(min.1 as f32);
            if first && preferred.0 > 0 && preferred.1 > 0 {
                ui.window().set_size(slint::LogicalSize::new(preferred.0 as f32, preferred.1 as f32));
            }
            ui.window().set_minimized(minimized);
            ui.window().set_maximized(maximized);
        }
        ToHelper::Raise => {
            ui.window().set_minimized(false);
            let _ = ui.window().with_winit_window(|w| w.focus_window());
        }
        ToHelper::Drag => {
            let _ = ui.window().with_winit_window(|w| w.drag_window());
        }
        ToHelper::Close | ToHelper::Frame { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_patch_lands_in_its_rows_and_nowhere_else() {
        let mut frame = vec![Rgb8Pixel::default(); 4 * 3];
        patch(&mut frame, 4, (1, 1, 2, 2), &[1, 1, 1, 2, 2, 2, 3, 3, 3, 4, 4, 4]);
        let at = |x: usize, y: usize| frame[y * 4 + x].r;
        assert_eq!((at(1, 1), at(2, 1), at(1, 2), at(2, 2)), (1, 2, 3, 4));
        assert_eq!(frame.iter().filter(|p| p.r != 0).count(), 4, "nothing outside the patch");
    }

    #[test]
    fn a_helper_listens_where_only_this_account_reaches() {
        let _env = crate::env_lock();
        let runtime = std::env::temp_dir().join(format!("yantrik-helper-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&runtime);
        std::fs::create_dir_all(&runtime).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
        let before = std::env::var_os("XDG_RUNTIME_DIR");
        std::env::set_var("XDG_RUNTIME_DIR", &runtime);
        let listening = Listening::bind("notes").unwrap();
        let path = listening.path().to_path_buf();
        let dir = yantrik_ipc_transport::server::socket_dir();
        assert!(super::super::link::acceptable(&path, &dir).is_ok(), "{}", path.display());
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        drop(listening);
        assert!(!path.exists(), "removed with the helper");
        match before {
            Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v),
            None => std::env::remove_var("XDG_RUNTIME_DIR"),
        }
        let _ = std::fs::remove_dir_all(&runtime);
    }
}
