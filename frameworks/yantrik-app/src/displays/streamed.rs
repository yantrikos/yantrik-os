//! A Mind's window as this process draws it: a Slint window adapter of this crate's, rendered by
//! Slint's software renderer into a frame whose changed parts go to the window's view helper.
//!
//! The adapter is what [`super::platform`] hands Slint for a window made inside [`make`]; every
//! other window of the process is winit's, on the person's display. What the helper sends back
//! (its size, the pointer, keys, a close) is dispatched to the window here, on the UI thread, as
//! winit would dispatch its own events.
//!
//! Drawing is pulled, not pushed, and costs the person's UI thread nothing it need not (#972 M3):
//! - one timer draws every visible streamed window that asked for a redraw, at most [`FRAME`]
//!   apart, and stops itself once none asks;
//! - nothing is rendered or copied while the link still holds the last frame ([`Sink::ready`]):
//!   the window stays marked to be drawn, and one frame is in flight at most;
//! - only the region the renderer reports as changed is sent, written straight into the message.
//!
//! What the helper sends back reaches the window inside [`delivering`], and nothing done there
//! reaches the person's clipboard or primary selection ([`super::platform`], #972 H1). The middle
//! button, which pastes the selection, is not passed on at all.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use slint::platform::software_renderer::{RepaintBufferType, SoftwareRenderer};
use slint::platform::{Renderer, WindowAdapter, WindowEvent, WindowProperties};
use slint::{LogicalPosition, LogicalSize, PhysicalSize, Rgb8Pixel};

use super::wire::{FromHelper, PointerKind, ToHelper, MAX_SIDE};

/// The longest a changed window waits to be drawn and sent.
pub const FRAME: Duration = Duration::from_millis(33);

/// Where a streamed window's messages go: its view helper's link, or nothing yet.
pub trait Sink {
    /// Sends a request (title, raise, move, close). `false` when the link has ended.
    fn send(&self, message: ToHelper) -> bool;
    /// Whether a frame would be taken now: the last one has gone and the link is up.
    fn ready(&self) -> bool;
    /// Sends one frame's encoded body. `false` when it was not taken.
    fn send_frame(&self, body: Vec<u8>) -> bool;
}

/// One streamed window.
pub struct StreamedWindow {
    window: slint::Window,
    renderer: SoftwareRenderer,
    needs_redraw: Cell<bool>,
    size: Cell<PhysicalSize>,
    visible: Cell<bool>,
    /// The frame as last drawn, one pixel per physical pixel.
    frame: RefCell<Vec<Rgb8Pixel>>,
    /// Send the whole frame next, not the changed region: a patch was dropped, or the size changed.
    whole_next: Cell<bool>,
    sink: RefCell<Option<Rc<dyn Sink>>>,
    /// The properties last sent, so an unchanged one is not sent again.
    sent: RefCell<Option<ToHelper>>,
}

thread_local! {
    /// Set while [`make`] makes a window, so the platform makes this kind.
    static MAKING: Cell<bool> = const { Cell::new(false) };
    /// The window the platform made inside [`make`], for it to hand back.
    static MADE: RefCell<Option<Rc<StreamedWindow>>> = const { RefCell::new(None) };
    /// Every streamed window alive, for the draw timer and for a title bar's requests.
    static LIVE: RefCell<Vec<Weak<StreamedWindow>>> = const { RefCell::new(Vec::new()) };
    static PUMP: RefCell<Option<slint::Timer>> = const { RefCell::new(None) };
    /// Set while what a helper said is dispatched into a streamed window.
    static DELIVERING: Cell<bool> = const { Cell::new(false) };
}

/// Runs `f` as input from a Mind's window: whatever Slint does inside it on that input is a Mind's
/// window's doing ([`in_a_minds_window`]).
pub fn delivering<R>(f: impl FnOnce() -> R) -> R {
    let was = DELIVERING.with(|d| d.replace(true));
    let out = f();
    DELIVERING.with(|d| d.set(was));
    out
}

/// Whether input from a Mind's window is being dispatched now: the platform then keeps the
/// person's clipboard and primary selection out of reach, both ways.
pub fn in_a_minds_window() -> bool {
    DELIVERING.with(|d| d.get())
}

/// Makes a window with `make`, as a streamed window when the platform is this crate's
/// ([`super::platform::installed`]). The window is forced into being inside, so the adapter is
/// chosen now. `None` beside it when the platform made an ordinary window (a test's headless one).
pub fn make<U: slint::ComponentHandle>(
    make: impl FnOnce() -> Result<U, slint::PlatformError>,
) -> (Result<U, slint::PlatformError>, Option<Rc<StreamedWindow>>) {
    MAKING.with(|m| m.set(true));
    let made = make().inspect(|ui| {
        let _ = ui.window();
    });
    MAKING.with(|m| m.set(false));
    (made, MADE.with(|m| m.borrow_mut().take()))
}

/// For the platform: whether the window being made now is to be streamed.
pub(crate) fn making() -> bool {
    MAKING.with(|m| m.get())
}

/// For the platform: a new streamed window, kept for [`make`] to hand back.
pub(crate) fn new_adapter() -> Rc<StreamedWindow> {
    let adapter = Rc::new_cyclic(|me: &Weak<StreamedWindow>| StreamedWindow {
        window: slint::Window::new(me.clone() as Weak<dyn WindowAdapter>),
        renderer: SoftwareRenderer::new_with_repaint_buffer_type(RepaintBufferType::ReusedBuffer),
        needs_redraw: Cell::new(true),
        size: Cell::new(PhysicalSize::default()),
        visible: Cell::new(false),
        frame: RefCell::new(Vec::new()),
        whole_next: Cell::new(true),
        sink: RefCell::new(None),
        sent: RefCell::new(None),
    });
    LIVE.with(|live| {
        let mut live = live.borrow_mut();
        live.retain(|w| w.strong_count() > 0);
        live.push(Rc::downgrade(&adapter));
    });
    MADE.with(|m| *m.borrow_mut() = Some(adapter.clone()));
    adapter
}

impl StreamedWindow {
    /// Where its messages go from now on; the helper is sent the window whole.
    pub fn attach(&self, sink: Rc<dyn Sink>) {
        *self.sink.borrow_mut() = Some(sink);
        self.whole_next.set(true);
        self.needs_redraw.set(true);
        // What Slint said of the window before the helper was there, sent now.
        let last = self.sent.borrow().clone();
        if let Some(last) = last {
            self.send(last);
        }
    }

    /// Sends one message, if a helper is attached.
    pub fn send(&self, message: ToHelper) -> bool {
        let sink = self.sink.borrow().clone();
        sink.is_some_and(|s| s.send(message))
    }

    /// What the helper said, applied to the window as its window system would, inside
    /// [`delivering`]. The middle button is dropped: in a text field it pastes the selection.
    pub fn deliver(&self, heard: FromHelper) {
        delivering(|| self.dispatch(heard));
    }

    fn dispatch(&self, heard: FromHelper) {
        if matches!(heard, FromHelper::Pointer { button: 2, kind: PointerKind::Pressed | PointerKind::Released, .. }) {
            return;
        }
        let w = &self.window;
        let at = |x: f32, y: f32| LogicalPosition::new(x, y);
        let button = |b: u8| match b {
            1 => slint::platform::PointerEventButton::Right,
            2 => slint::platform::PointerEventButton::Middle,
            _ => slint::platform::PointerEventButton::Left,
        };
        let _ = match heard {
            FromHelper::Size { width, height, scale } => {
                self.resize(PhysicalSize::new(width.max(1), height.max(1)), scale);
                Ok(())
            }
            FromHelper::Pointer { kind, button: b, x, y } => w.try_dispatch_event(match kind {
                PointerKind::Moved => WindowEvent::PointerMoved { position: at(x, y) },
                PointerKind::Pressed => WindowEvent::PointerPressed { position: at(x, y), button: button(b) },
                PointerKind::Released => WindowEvent::PointerReleased { position: at(x, y), button: button(b) },
                PointerKind::Exited => WindowEvent::PointerExited,
            }),
            FromHelper::Scroll { x, y, dx, dy } => {
                w.try_dispatch_event(WindowEvent::PointerScrolled { position: at(x, y), delta_x: dx, delta_y: dy })
            }
            FromHelper::Key { pressed: true, text } => w.try_dispatch_event(WindowEvent::KeyPressed { text: text.into() }),
            FromHelper::Key { pressed: false, text } => w.try_dispatch_event(WindowEvent::KeyReleased { text: text.into() }),
            FromHelper::Focus(on) => w.try_dispatch_event(WindowEvent::WindowActiveChanged(on)),
            FromHelper::CloseRequested => w.try_dispatch_event(WindowEvent::CloseRequested),
        };
    }

    /// Draws the window whole next: Private mode ended while it was blanked.
    pub fn redraw_whole(&self) {
        self.whole_next.set(true);
        self.request_redraw();
    }

    fn resize(&self, size: PhysicalSize, scale: f32) {
        if (self.window.scale_factor() - scale).abs() > f32::EPSILON {
            let _ = self.window.try_dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: scale });
        }
        let size = PhysicalSize::new(size.width.min(MAX_SIDE), size.height.min(MAX_SIDE));
        if self.size.replace(size) != size {
            let _ = self.window.try_dispatch_event(WindowEvent::Resized { size: size.to_logical(scale) });
            self.whole_next.set(true);
        }
        self.request_redraw();
    }

    /// Whether the draw timer has anything to do for this window.
    fn wants_drawing(&self) -> bool {
        self.visible.get() && self.needs_redraw.get()
    }

    /// Draws the window if it changed and sends what changed. Called by the draw timer. Nothing is
    /// rendered or copied while the link still holds the last frame: the window stays marked.
    fn draw_and_send(&self) {
        let size = self.size.get();
        if !self.visible.get() || size.width == 0 || size.height == 0 || !self.needs_redraw.get() {
            return;
        }
        let Some(sink) = self.sink.borrow().clone() else { return };
        if !sink.ready() {
            return;
        }
        self.needs_redraw.set(false);
        let (width, height) = (size.width as usize, size.height as usize);
        let mut frame = self.frame.borrow_mut();
        if frame.len() != width * height {
            *frame = vec![Rgb8Pixel::default(); width * height];
            // A new buffer holds nothing the renderer drew: it draws all of it again.
            self.renderer.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
            self.whole_next.set(true);
        }
        let region = self.renderer.render(frame.as_mut_slice(), width);
        let (x, y, w, h) = if self.whole_next.get() {
            (0, 0, width, height)
        } else {
            let origin = region.bounding_box_origin();
            let extent = region.bounding_box_size();
            let x = (origin.x.max(0) as usize).min(width);
            let y = (origin.y.max(0) as usize).min(height);
            (x, y, (extent.width as usize).min(width - x), (extent.height as usize).min(height - y))
        };
        if self.window.has_active_animations() {
            self.needs_redraw.set(true);
        }
        if w == 0 || h == 0 {
            return;
        }
        let mut body = ToHelper::frame_header(width as u32, height as u32, x as u32, y as u32, w as u32, h as u32);
        for row in y..y + h {
            for p in &frame[row * width + x..row * width + x + w] {
                body.extend_from_slice(&[p.r, p.g, p.b]);
            }
        }
        drop(frame);
        let taken = sink.send_frame(body);
        // Ready a moment ago, and the UI thread is the only one that offers frames: taken, unless
        // the link ended, and then nothing more is drawn for it.
        self.whole_next.set(!taken);
    }

    /// The window's title and sizes, as Slint gives them, sent when they changed; kept for a
    /// helper that attaches later.
    fn send_properties(&self, p: WindowProperties<'_>) {
        let layout = p.layout_constraints();
        let logical = |s: LogicalSize| (s.width.max(0.0).round() as u32, s.height.max(0.0).round() as u32);
        if self.size.get().width == 0 {
            // Until the helper says how big its window is, the window's own wish.
            let scale = self.window.scale_factor();
            self.size.set(layout.preferred.to_physical(scale));
        }
        let message = ToHelper::Props {
            title: p.title().to_string(),
            preferred: logical(layout.preferred),
            min: layout.min.map(logical).unwrap_or((0, 0)),
            minimized: p.is_minimized(),
            maximized: p.is_maximized(),
        };
        if self.sent.borrow().as_ref() == Some(&message) {
            return;
        }
        self.send(message.clone());
        *self.sent.borrow_mut() = Some(message);
    }
}

impl WindowAdapter for StreamedWindow {
    fn window(&self) -> &slint::Window {
        &self.window
    }

    fn renderer(&self) -> &dyn Renderer {
        &self.renderer
    }

    fn size(&self) -> PhysicalSize {
        self.size.get()
    }

    fn set_size(&self, size: slint::WindowSize) {
        let scale = self.window.scale_factor();
        self.resize(size.to_physical(scale), scale);
    }

    fn set_visible(&self, visible: bool) -> Result<(), slint::PlatformError> {
        self.visible.set(visible);
        if visible {
            self.request_redraw();
        }
        Ok(())
    }

    fn request_redraw(&self) {
        self.needs_redraw.set(true);
        if self.visible.get() {
            run_pump();
        }
    }

    fn update_window_properties(&self, properties: WindowProperties<'_>) {
        self.send_properties(properties);
    }
}

/// The streamed window whose `slint::Window` this is, if it is one.
pub fn of(window: &slint::Window) -> Option<Rc<StreamedWindow>> {
    LIVE.with(|live| {
        live.borrow().iter().filter_map(Weak::upgrade).find(|w| std::ptr::eq(&w.window, window))
    })
}

/// A title bar's move, in a streamed window: the helper asks its compositor. `false` for any
/// other window.
pub fn drag(window: &slint::Window) -> bool {
    of(window).is_some_and(|w| w.send(ToHelper::Drag))
}

/// The draw timer: started by a redraw a visible streamed window asks for, and stopped once none
/// asks, so an idle or hidden Mind's window costs no wakeups.
fn run_pump() {
    PUMP.with(|pump| {
        let mut pump = pump.borrow_mut();
        if pump.as_ref().is_some_and(slint::Timer::running) {
            return;
        }
        let timer = pump.get_or_insert_with(slint::Timer::default);
        timer.start(slint::TimerMode::Repeated, FRAME, || {
            let windows: Vec<Rc<StreamedWindow>> =
                LIVE.with(|live| live.borrow().iter().filter_map(Weak::upgrade).collect());
            for window in &windows {
                window.draw_and_send();
            }
            if !windows.iter().any(|w| w.wants_drawing()) {
                PUMP.with(|p| {
                    if let Some(t) = p.borrow().as_ref() {
                        t.stop();
                    }
                });
            }
        });
    });
}
