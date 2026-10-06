//! yantrik-panel-spike: the status bar as a real Wayland panel (wlr-layer-shell), saga task #4.
//!
//! A spike, not shipped. It answers one question: can the bar and a popover be separate Wayland
//! surfaces drawn by Slint's software renderer, in one process, on one event loop, with no
//! busy loop? Read `design/layer-shell-spike-2026-10-01.md` for what was and was not observed.
//!
//!   bar      layer Top,     anchored top+left+right, 32px, exclusive zone 32, no keyboard focus
//!   popover  layer Overlay, anchored top+right, 360x200, keyboard on demand, made on click
//!
//! Flags: `--trace` (log events), `--exit-after <secs>` (print counters and CPU, then exit),
//!        `--self-click <secs>` (synthesise one click on the bar's button, to exercise the
//!        toggle without an input device; this does NOT test compositor input routing).

mod panel;
mod platform;
#[cfg(test)]
mod tests;

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use panel::Panel;
use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, SharedString};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{
            timer::{TimeoutAction, Timer},
            EventLoop,
        },
        calloop_wayland_source::WaylandSource,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keymap, Keysym, Modifiers, RawModifiers, RepeatInfo},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
        WaylandSurface,
    },
    shm::{Shm, ShmHandler},
};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface},
    Connection, QueueHandle,
};

slint::include_modules!();

/// Theme.status-bar-height. The real shell's taskbar is 40; one number is enough for a spike.
const BAR_HEIGHT: u32 = 32;
const POPOVER_SIZE: (u32, u32) = (360, 200);
const BTN_LEFT: u32 = 0x110;

struct Bar {
    output: wl_output::WlOutput,
    panel: Panel,
    view: BarView,
}

struct Popover {
    window: Rc<MinimalSoftwareWindow>,
    view: PopoverView,
    /// Present only while open: closing destroys the surface, and the compositor stops
    /// composing it. A hidden popover costs nothing.
    panel: Option<Panel>,
}

struct App {
    qh: QueueHandle<App>,
    compositor: CompositorState,
    outputs: OutputState,
    registry: RegistryState,
    seats: SeatState,
    shm: Shm,
    layer_shell: LayerShell,
    bars: Vec<Bar>,
    popover: Popover,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    /// Set by a bar's callback (which runs inside `dispatch_event`, where `App` is borrowed) and
    /// acted on by the loop afterwards: the index of the output whose button was pressed.
    toggle_requested: Rc<Cell<Option<usize>>>,
    /// Whether the keyboard is on the popover (the compositor told us with `enter`/`leave`).
    keyboard_on_popover: bool,
    trace: bool,
    wakeups: u64,
}

/// Cost of the software renderer with no compositor: full repaints of each surface and the
/// Argb8888 conversion, in microseconds per frame. `cargo run --release -p yantrik-panel-spike -- --bench`.
fn bench() {
    use slint::platform::software_renderer::PremultipliedRgbaColor as Px;
    fn run(name: &str, window: &MinimalSoftwareWindow, w: u32, h: u32) {
        window.set_size(slint::PhysicalSize::new(w, h));
        let mut px = vec![Px::default(); (w * h) as usize];
        let mut canvas = vec![0u8; (w * h * 4) as usize];
        let n = 300;
        let (mut render, mut convert) = (Duration::ZERO, Duration::ZERO);
        for _ in 0..n {
            window.request_redraw();
            let t = std::time::Instant::now();
            let mut drew = false;
            window.draw_if_needed(|r| {
                // NewBuffer repaints everything: `request_redraw` alone repaints only what changed,
                // which is nothing here, and the first version of this bench measured that.
                r.set_repaint_buffer_type(slint::platform::software_renderer::RepaintBufferType::NewBuffer);
                r.render(&mut px, w as usize);
                drew = true;
            });
            assert!(drew, "the bench painted nothing");
            render += t.elapsed();
            let t = std::time::Instant::now();
            for (p, out) in px.iter().zip(canvas.chunks_exact_mut(4)) {
                out.copy_from_slice(&[p.blue, p.green, p.red, p.alpha]);
            }
            std::hint::black_box(&canvas);
            convert += t.elapsed();
        }
        println!(
            "{name:<26} {w}x{h}: render {:>7.1} us/frame, convert {:>7.1} us/frame",
            render.as_secs_f64() * 1e6 / n as f64,
            convert.as_secs_f64() * 1e6 / n as f64
        );
    }
    let (bar_w, bar) = platform::with_window(|| BarView::new().expect("bar view"));
    bar.show().ok();
    let (pop_w, pop) = platform::with_window(|| PopoverView::new().expect("popover view"));
    pop.show().ok();
    run("bar, full repaint", &bar_w, 1280, 32);
    run("bar, full repaint", &bar_w, 1920, 32);
    run("bar, full repaint", &bar_w, 3840, 32);
    run("popover, full repaint", &pop_w, 360, 200);
    // The same two small components stretched to a screen: a floor for what a full-screen
    // surface would cost in this renderer, not a measurement of the shell's real screens.
    run("bar component, stretched", &bar_w, 1920, 1080);
}

fn arg_value(args: &[String], flag: &str) -> Option<f64> {
    args.iter().skip_while(|a| *a != flag).nth(1).and_then(|v| v.parse().ok())
}

fn clock_text() -> SharedString {
    chrono::Local::now().format("%H:%M").to_string().into()
}

/// Time until the next wall-clock minute: the bar's only scheduled wake-up.
fn until_next_minute() -> Duration {
    use chrono::Timelike;
    let now = chrono::Local::now();
    let into_minute = now.second() as u64 * 1000 + (now.nanosecond() as u64 / 1_000_000) % 1000;
    Duration::from_millis(60_000 - into_minute).max(Duration::from_millis(500))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let trace = args.iter().any(|a| a == "--trace");
    let exit_after = arg_value(&args, "--exit-after");
    let self_click = arg_value(&args, "--self-click");

    platform::install();
    if args.iter().any(|a| a == "--bench") {
        bench();
        return;
    }
    let (pop_window, pop_view) = platform::with_window(|| PopoverView::new().expect("popover view"));
    // Slint treats a new window as active, so the popover's focused text field starts its
    // cursor-blink timer at creation, shown or not, and woke an idle process twice a second.
    // Saying it is inactive stops it; `enter` on the compositor's keyboard focus says otherwise.
    pop_window.dispatch_event(WindowEvent::WindowActiveChanged(false));
    let toggle_requested = Rc::new(Cell::new(None));

    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("yantrik-panel-spike: no Wayland display: {e}");
            std::process::exit(2);
        }
    };
    let (globals, mut queue) = registry_queue_init::<App>(&conn).expect("registry");
    let qh = queue.handle();
    let layer_shell = match LayerShell::bind(&globals, &qh) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("yantrik-panel-spike: this compositor has no zwlr_layer_shell_v1: {e}");
            std::process::exit(3);
        }
    };
    let mut app = App {
        qh: qh.clone(),
        compositor: CompositorState::bind(&globals, &qh).expect("wl_compositor"),
        outputs: OutputState::new(&globals, &qh),
        registry: RegistryState::new(&globals),
        seats: SeatState::new(&globals, &qh),
        shm: Shm::bind(&globals, &qh).expect("wl_shm"),
        layer_shell,
        bars: Vec::new(),
        popover: Popover { window: pop_window, view: pop_view, panel: None },
        pointer: None,
        keyboard: None,
        toggle_requested,
        keyboard_on_popover: false,
        trace,
        wakeups: 0,
    };
    // Outputs and seats are announced on the first roundtrips; the handlers create the bars.
    queue.roundtrip(&mut app).ok();
    queue.roundtrip(&mut app).ok();

    let mut event_loop: EventLoop<App> = EventLoop::try_new().expect("an event loop");
    WaylandSource::new(conn.clone(), queue).insert(event_loop.handle()).expect("the Wayland source");

    // The clock: one wake-up per minute, aligned to the minute. Not a Slint timer, not a poll.
    event_loop
        .handle()
        .insert_source(Timer::from_duration(until_next_minute()), |_, _, app| {
            let t = clock_text();
            for bar in &app.bars {
                bar.view.set_time(t.clone());
            }
            TimeoutAction::ToDuration(until_next_minute())
        })
        .expect("the clock");
    if let Some(secs) = self_click {
        event_loop
            .handle()
            .insert_source(Timer::from_duration(Duration::from_secs_f64(secs)), |_, _, app| {
                // Synthetic: straight into Slint, bypassing the compositor's input path.
                if let Some(bar) = app.bars.first() {
                    let (w, _) = bar.panel.size;
                    let position = LogicalPosition::new(w as f32 - 40.0, 16.0);
                    let button = slint::platform::PointerEventButton::Left;
                    bar.panel.pointer(WindowEvent::PointerMoved { position });
                    bar.panel.pointer(WindowEvent::PointerPressed { position, button });
                    bar.panel.pointer(WindowEvent::PointerReleased { position, button });
                }
                TimeoutAction::Drop
            })
            .expect("self click");
    }
    let stop = Rc::new(Cell::new(false));
    if let Some(secs) = exit_after {
        let stop = stop.clone();
        event_loop
            .handle()
            .insert_source(Timer::from_duration(Duration::from_secs_f64(secs)), move |_, _, _| {
                stop.set(true);
                TimeoutAction::Drop
            })
            .expect("exit timer");
    }

    while !stop.get() {
        // Sleep until Wayland has something, or Slint has a timer due, or an animation is
        // running (then ~60 Hz). With none of the three, block forever: that is idle.
        let animating = app.windows().any(|w| w.has_active_animations());
        let timeout = if animating { Some(Duration::from_millis(16)) } else { slint::platform::duration_until_next_timer_update() };
        let before = std::time::Instant::now();
        if event_loop.dispatch(timeout, &mut app).is_err() {
            break;
        }
        app.wakeups += 1;
        if trace && app.wakeups > 12 {
            eprintln!("wake: asked for {timeout:?}, slept {:?}, animating {animating}", before.elapsed());
        }
        slint::platform::update_timers_and_animations();
        app.apply_toggle();
        app.draw_all();
    }
    app.report();
}

impl App {
    fn windows(&self) -> impl Iterator<Item = &Rc<MinimalSoftwareWindow>> {
        self.bars.iter().map(|b| &b.panel.window).chain(self.popover.panel.as_ref().map(|p| &p.window))
    }

    fn draw_all(&mut self) {
        for bar in &mut self.bars {
            if bar.panel.draw_if_needed() && self.trace {
                eprintln!("bar: painted ({:?})", bar.panel.stats);
            }
        }
        if let Some(p) = &mut self.popover.panel {
            if p.draw_if_needed() && self.trace {
                eprintln!("popover: painted ({:?})", p.stats);
            }
        }
    }

    fn add_bar(&mut self, output: wl_output::WlOutput) {
        let surface = self.compositor.create_surface(&self.qh);
        let layer = self.layer_shell.create_layer_surface(&self.qh, surface, Layer::Top, Some("yantrik-bar"), Some(&output));
        layer.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT);
        // Width 0 = "the whole anchored edge"; the compositor answers with the output width.
        layer.set_size(0, BAR_HEIGHT);
        // The line the compositor keeps windows out of. This replaces rc.xml's <margin>.
        layer.set_exclusive_zone(BAR_HEIGHT as i32);
        // A bar must never take typing from the window you are in.
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();

        let (window, view) = platform::with_window(|| BarView::new().expect("bar view"));
        view.set_time(clock_text());
        view.show().ok();
        let index = self.bars.len();
        let flag = self.toggle_requested.clone();
        view.on_toggle_popover(move || flag.set(Some(index)));
        let panel = Panel::new(window, layer, &self.shm);
        self.bars.push(Bar { output, panel, view });
    }

    fn apply_toggle(&mut self) {
        let Some(index) = self.toggle_requested.take() else { return };
        if self.popover.panel.is_some() {
            self.close_popover();
        } else if let Some(bar) = self.bars.get(index) {
            let output = bar.output.clone();
            self.open_popover(&output);
        }
    }

    fn open_popover(&mut self, output: &wl_output::WlOutput) {
        let surface = self.compositor.create_surface(&self.qh);
        let layer = self.layer_shell.create_layer_surface(&self.qh, surface, Layer::Overlay, Some("yantrik-popover"), Some(output));
        layer.set_anchor(Anchor::TOP | Anchor::RIGHT);
        layer.set_size(POPOVER_SIZE.0, POPOVER_SIZE.1);
        // 0, not -1: sit below other surfaces' exclusive zones (under the bar), not over them.
        layer.set_exclusive_zone(0);
        layer.set_margin(4, 8, 0, 0);
        layer.set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
        layer.commit();
        self.popover.panel = Some(Panel::new(self.popover.window.clone(), layer, &self.shm));
        self.popover.view.show().ok();
        self.set_open_label(true);
    }

    fn close_popover(&mut self) {
        // Dropping the LayerSurface destroys it.
        self.popover.panel = None;
        self.popover.window.dispatch_event(WindowEvent::WindowActiveChanged(false));
        self.popover.view.hide().ok();
        self.keyboard_on_popover = false;
        self.set_open_label(false);
    }

    fn set_open_label(&self, open: bool) {
        for bar in &self.bars {
            bar.view.set_popover_open(open);
        }
    }

    fn report(&self) {
        let (mut renders, mut painted, mut commits) = (0, 0, 0);
        for p in self.bars.iter().map(|b| &b.panel).chain(self.popover.panel.as_ref()) {
            renders += p.stats.renders;
            painted += p.stats.pixels_painted;
            commits += p.stats.commits;
        }
        let ticks = std::fs::read_to_string("/proc/self/stat")
            .ok()
            .and_then(|s| {
                let rest = s.rsplit_once(')')?.1.to_string();
                let f: Vec<&str> = rest.split_whitespace().collect();
                Some((f.get(11)?.parse::<u64>().ok()?, f.get(12)?.parse::<u64>().ok()?))
            })
            .unwrap_or((0, 0));
        eprintln!(
            "report: bars={} wakeups={} renders={} commits={} pixels_painted={} cpu_ticks(user,sys)={:?} (1 tick = 10 ms)",
            self.bars.len(),
            self.wakeups,
            renders,
            commits,
            painted,
            ticks
        );
    }

    /// The panel that owns this wl_surface.
    fn panel_for(&self, surface: &wl_surface::WlSurface) -> Option<&Panel> {
        self.bars.iter().map(|b| &b.panel).chain(self.popover.panel.as_ref()).find(|p| p.layer.wl_surface() == surface)
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        // The compositor took the surface away (the output went, or it refused it).
        if self.trace {
            eprintln!("layer surface closed by the compositor");
        }
        if self.popover.panel.as_ref().is_some_and(|p| p.layer.wl_surface() == layer.wl_surface()) {
            self.close_popover();
        }
        self.bars.retain(|b| b.panel.layer.wl_surface() != layer.wl_surface());
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface, configure: LayerSurfaceConfigure, _: u32) {
        let (w, h) = configure.new_size;
        if self.trace {
            eprintln!("configure: {w}x{h}");
        }
        let target = self
            .bars
            .iter_mut()
            .map(|b| &mut b.panel)
            .chain(self.popover.panel.as_mut())
            .find(|p| p.layer.wl_surface() == layer.wl_surface());
        if let Some(p) = target {
            p.resize(w, h);
            p.draw_if_needed();
        }
    }
}

impl PointerHandler for App {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        use slint::platform::PointerEventButton as B;
        for event in events {
            let Some(panel) = self.panel_for(&event.surface) else { continue };
            let position = LogicalPosition::new(event.position.0 as f32, event.position.1 as f32);
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => panel.pointer(WindowEvent::PointerMoved { position }),
                PointerEventKind::Leave { .. } => panel.pointer(WindowEvent::PointerExited),
                PointerEventKind::Press { button, .. } if button == BTN_LEFT => {
                    if self.trace {
                        eprintln!("pointer press at {:?}", event.position);
                    }
                    panel.pointer(WindowEvent::PointerPressed { position, button: B::Left })
                }
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                    panel.pointer(WindowEvent::PointerReleased { position, button: B::Left })
                }
                _ => {}
            }
        }
    }
}

impl KeyboardHandler for App {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, surface: &wl_surface::WlSurface, _: u32, _: &[u32], _: &[Keysym]) {
        self.keyboard_on_popover = self.popover.panel.as_ref().is_some_and(|p| p.layer.wl_surface() == surface);
        if self.trace {
            eprintln!("keyboard enter (popover: {})", self.keyboard_on_popover);
        }
        if self.keyboard_on_popover {
            self.popover.window.dispatch_event(WindowEvent::WindowActiveChanged(true));
        }
    }
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32) {
        if self.keyboard_on_popover {
            self.popover.window.dispatch_event(WindowEvent::WindowActiveChanged(false));
        }
        self.keyboard_on_popover = false;
    }
    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        if self.trace {
            eprintln!("key {:?} (popover has focus: {})", event.keysym, self.keyboard_on_popover);
        }
        if !self.keyboard_on_popover {
            return;
        }
        if event.keysym == Keysym::Escape {
            self.close_popover();
            return;
        }
        let text: SharedString = match event.keysym {
            Keysym::BackSpace => Key::Backspace.into(),
            Keysym::Return | Keysym::KP_Enter => Key::Return.into(),
            Keysym::Left => Key::LeftArrow.into(),
            Keysym::Right => Key::RightArrow.into(),
            _ => match event.utf8 {
                Some(t) if !t.is_empty() => t.into(),
                _ => return,
            },
        };
        self.popover.window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        self.popover.window.dispatch_event(WindowEvent::KeyReleased { text });
    }
    fn repeat_key(&mut self, c: &Connection, q: &QueueHandle<Self>, k: &wl_keyboard::WlKeyboard, s: u32, event: KeyEvent) {
        self.press_key(c, q, k, s, event);
    }
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}
    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: Modifiers, _: RawModifiers, _: u32) {}
    fn update_repeat_info(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: RepeatInfo) {}
    fn update_keymap(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: Keymap<'_>) {}
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seats
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        match capability {
            Capability::Keyboard if self.keyboard.is_none() => self.keyboard = self.seats.get_keyboard(qh, &seat, None).ok(),
            Capability::Pointer if self.pointer.is_none() => self.pointer = self.seats.get_pointer(qh, &seat).ok(),
            _ => {}
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        match capability {
            Capability::Keyboard => {
                if let Some(k) = self.keyboard.take() {
                    k.release();
                }
            }
            Capability::Pointer => {
                if let Some(p) = self.pointer.take() {
                    p.release();
                }
            }
            _ => {}
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl CompositorHandler for App {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        // One bar per output, made when the output appears and gone when it does.
        self.add_bar(output);
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.bars.retain(|b| b.output != output);
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(App);
delegate_output!(App);
delegate_shm!(App);
delegate_seat!(App);
delegate_keyboard!(App);
delegate_pointer!(App);
delegate_layer!(App);
delegate_registry!(App);
