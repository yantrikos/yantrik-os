//! yantrik-lock — the lock screen as a Wayland session lock (#313).
//!
//! The shell's lock screen was a screen inside its own window, an ordinary toplevel. An app window
//! that was in front when the desktop locked stayed in front, visible and usable, and the
//! compositor would raise any window over the "lock" on request: VM 520 showed Weather and
//! Calendar over it, working, with the shell saying `locked: true`. This takes the lock at the
//! compositor instead (`ext-session-lock-v1`, the protocol swaylock uses): while it holds it, the
//! compositor shows only these surfaces and sends input only to them.
//!
//! It does not know the secret. The shell does (`lock::check_unlock`: the login password, or the
//! PIN on an account without one, #414, named by `--ask password|pin`), and this asks it, one
//! line at a time over stdin/stdout:
//!
//! Beside the picture it takes the pointer, for the eye that shows what was typed, the arrow that
//! sends it and the Suspend, Restart and Shut down buttons. Those three run `systemctl` as the
//! person's own session (logind), and none of them unlocks anything: a restart ends the session
//! and a suspend leaves it locked.
//!
//!   → `locked`           the compositor has locked the session
//!   → `secret <text>`    the person pressed Enter
//!   ← `ok` | `no`        the shell's answer; on `ok` this unlocks and exits 0
//!
//! Exit 3: this compositor has no session lock, so the shell keeps its own screen (and says so).
//! If the shell goes away (stdin closes) this never unlocks on its own: the session stays locked
//! until a shell asks a new lock client to take over.

use std::cell::Cell;
use std::io::{BufRead, Write};
use std::rc::Rc;
use std::time::Duration;

use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::ComponentHandle;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    compositor::SurfaceData, delegate_compositor, delegate_keyboard, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_session_lock, delegate_shm,
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
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Keymap, Modifiers, RawModifiers, RepeatInfo},
        pointer::{CursorIcon, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec, ThemedPointer},
        Capability, SeatHandler, SeatState,
    },
    session_lock::{SessionLock, SessionLockHandler, SessionLockState, SessionLockSurface, SessionLockSurfaceConfigure},
    shm::{raw::RawPool, Shm, ShmHandler},
};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_buffer, wl_keyboard, wl_output, wl_seat, wl_shm, wl_surface},
    Connection, QueueHandle,
};

slint::include_modules!();

mod wallpaper;

/// The compositor has no session lock.
const EXIT_UNSUPPORTED: i32 = 3;
/// Anything else that stopped it before it could lock.
const EXIT_FAILED: i32 = 4;
/// Longer than any password a person types; stop taking keys rather than grow without bound.
const MOST_CHARS: usize = 256;

/// What the pointer asked for on the lock view. The view's callbacks run inside Slint's event
/// dispatch, where `App` is borrowed, so they leave a note here and `App` reads it afterwards.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Request {
    Submit,
    Power(Power),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Power {
    Suspend,
    Restart,
    ShutDown,
}

impl Power {
    /// The logind verb: `systemctl <verb>` from the person's own session is allowed by polkit's
    /// active-session rule, and needs no password and no privilege of ours.
    fn verb(self) -> &'static str {
        match self {
            Power::Suspend => "suspend",
            Power::Restart => "reboot",
            Power::ShutDown => "poweroff",
        }
    }

    fn failed(self) -> &'static str {
        match self {
            Power::Suspend => "Could not suspend",
            Power::Restart => "Could not restart",
            Power::ShutDown => "Could not shut down",
        }
    }
}

/// What the shell told this client about the person and the machine when it started it. Plain
/// words and numbers: no secret, and no notification's text (only how many there are).
#[derive(Default)]
struct Look {
    name: String,
    wallpaper: Option<String>,
    network: String,
    notifications: i32,
    /// False on a machine declared open at boot: there, Restart would come back to an unlocked
    /// desktop, so the button is not drawn and the request is ignored (security review of #601).
    can_restart: bool,
}

impl Look {
    fn from_args(args: &[String]) -> Look {
        let value = |flag: &str| args.iter().skip_while(|a| *a != flag).nth(1).cloned();
        Look {
            name: value("--name").unwrap_or_default(),
            wallpaper: value("--wallpaper").filter(|p| !p.is_empty()),
            network: value("--network").unwrap_or_default(),
            notifications: value("--notifications").and_then(|n| n.parse().ok()).unwrap_or(0).max(0),
            can_restart: !args.iter().any(|a| a == "--no-restart"),
        }
    }
}

use yantrik_ui_kit::lock_shared::initial_of;

/// The battery, as the kernel reports it: percent and whether it is charging. `None` on a machine
/// with no battery, so the lock draws no battery at all rather than a made-up level.
fn battery() -> Option<(i32, bool)> {
    let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in dir.flatten() {
        let path = entry.path();
        let read = |name: &str| std::fs::read_to_string(path.join(name)).ok().map(|s| s.trim().to_string());
        if read("type").as_deref() != Some("Battery") || read("scope").as_deref() == Some("Device") {
            continue;
        }
        let percent: i32 = read("capacity")?.parse().ok()?;
        let charging = matches!(read("status").as_deref(), Some("Charging"));
        return Some((percent.clamp(0, 100), charging));
    }
    None
}

/// What to ask for, from `--ask`: the login password unless the shell says the PIN.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ask {
    Password,
    Pin,
}

impl Ask {
    fn from_args(args: &[String]) -> Ask {
        match args.iter().skip_while(|a| *a != "--ask").nth(1).map(String::as_str) {
            Some("pin") => Ask::Pin,
            _ => Ask::Password,
        }
    }

    fn prompt(self) -> &'static str {
        match self {
            Ask::Password => "Enter your password to unlock",
            Ask::Pin => "Enter PIN to unlock",
        }
    }

    fn wrong(self) -> &'static str {
        match self {
            Ask::Password => "Wrong password",
            Ask::Pin => "Wrong PIN",
        }
    }

    /// Whether a typed character belongs in the entry: digits for a PIN, anything printable for a
    /// password.
    fn takes(self, c: char) -> bool {
        match self {
            Ask::Pin => c.is_ascii_digit(),
            Ask::Password => !c.is_control(),
        }
    }
}

struct Headless(Rc<MinimalSoftwareWindow>);

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

struct App {
    conn: Connection,
    qh: QueueHandle<App>,
    compositor: CompositorState,
    outputs: OutputState,
    registry: RegistryState,
    seats: SeatState,
    shm: Shm,
    lock: Option<SessionLock>,
    /// Each lock surface and the size the compositor gave it.
    surfaces: Vec<(SessionLockSurface, (u32, u32))>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<ThemedPointer>,
    /// What the view's buttons asked for since `App` last looked.
    requested: Rc<Cell<Option<Request>>>,
    /// The pre-blurred wallpaper's path, read once the session is locked.
    wallpaper: Option<String>,
    can_restart: bool,
    window: Rc<MinimalSoftwareWindow>,
    view: LockView,
    ask: Ask,
    entry: String,
    error: String,
    locked: bool,
    exit: Option<i32>,
}

fn main() {
    // Not dumpable: another process of the same user cannot read this one's memory (the password
    // being typed) or open its descriptors through /proc, and it leaves no core behind.
    // SAFETY: prctl with PR_SET_DUMPABLE takes plain integers and touches no memory of ours.
    unsafe {
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
    }
    let args: Vec<String> = std::env::args().collect();
    let greeting = args.iter().skip_while(|a| *a != "--greeting").nth(1).cloned().unwrap_or_default();
    let ask = Ask::from_args(&args);
    let look = Look::from_args(&args);

    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless(window.clone()))).expect("one platform");
    let view = LockView::new().expect("the lock view");
    // The name under the avatar; a shell that sent none (an older one) sends its greeting.
    let name = if look.name.is_empty() { greeting } else { look.name.clone() };
    view.set_initial(initial_of(&name).into());
    view.set_user_name(name.into());
    view.set_prompt(ask.prompt().into());
    view.set_network(look.network.clone().into());
    view.set_notifications(look.notifications);
    view.set_can_restart(look.can_restart);
    // The wallpaper is not read here: only after the compositor has locked (see `locked`), so no
    // file can stop the lock from being taken.
    let requested: Rc<Cell<Option<Request>>> = Rc::default();
    {
        let note = |request: Request| {
            let requested = requested.clone();
            move || requested.set(Some(request))
        };
        view.on_submit(note(Request::Submit));
        view.on_suspend(note(Request::Power(Power::Suspend)));
        view.on_restart(note(Request::Power(Power::Restart)));
        view.on_shutdown(note(Request::Power(Power::ShutDown)));
    }
    view.show().ok();

    let Ok(conn) = Connection::connect_to_env() else {
        eprintln!("yantrik-lock: no Wayland display to lock");
        std::process::exit(EXIT_FAILED);
    };
    let Ok((globals, queue)) = registry_queue_init::<App>(&conn) else {
        std::process::exit(EXIT_FAILED);
    };
    let qh = queue.handle();
    let mut event_loop: EventLoop<App> = EventLoop::try_new().expect("an event loop");

    let lock_state = SessionLockState::new(&globals, &qh);
    let mut app = App {
        conn: conn.clone(),
        qh: qh.clone(),
        compositor: CompositorState::bind(&globals, &qh).expect("wl_compositor"),
        outputs: OutputState::new(&globals, &qh),
        registry: RegistryState::new(&globals),
        seats: SeatState::new(&globals, &qh),
        shm: Shm::bind(&globals, &qh).expect("wl_shm"),
        lock: None,
        surfaces: Vec::new(),
        keyboard: None,
        pointer: None,
        requested,
        wallpaper: look.wallpaper.clone(),
        can_restart: look.can_restart,
        window,
        view,
        ask,
        entry: String::new(),
        error: String::new(),
        locked: false,
        exit: None,
    };

    // The outputs are announced on the first roundtrip; a lock surface is made for each.
    let mut queue = queue;
    let _ = queue.roundtrip(&mut app);
    let Ok(lock) = lock_state.lock(&qh) else {
        eprintln!("yantrik-lock: this compositor has no ext-session-lock-v1");
        std::process::exit(EXIT_UNSUPPORTED);
    };
    for output in app.outputs.outputs() {
        let surface = app.compositor.create_surface(&qh);
        app.surfaces.push((lock.create_lock_surface(surface, &output, &qh), (0, 0)));
    }
    app.lock = Some(lock);

    WaylandSource::new(conn, queue).insert(event_loop.handle()).expect("the Wayland source");
    // The clock on the lock screen moves.
    event_loop
        .handle()
        .insert_source(Timer::from_duration(Duration::from_secs(1)), |_, _, app| {
            if app.locked {
                app.draw();
            }
            TimeoutAction::ToDuration(Duration::from_secs(15))
        })
        .expect("the clock");

    loop {
        if event_loop.dispatch(Duration::from_millis(250), &mut app).is_err() {
            std::process::exit(EXIT_FAILED);
        }
        if let Some(code) = app.exit {
            std::process::exit(code);
        }
    }
}

impl App {
    /// Say something to the shell. A shell that is gone cannot answer, and a lock that cannot ask
    /// never opens: nothing here treats silence as yes.
    fn tell(&self, line: &str) -> bool {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{line}").and_then(|_| out.flush()).is_ok()
    }

    fn ask_shell(&mut self) {
        let entry = std::mem::take(&mut self.entry);
        if entry.is_empty() {
            return;
        }
        // The answer can take a moment (the password check, and a slow-down after wrong ones):
        // say so before waiting for it, with the field already empty.
        self.error = "Checking…".into();
        self.draw();
        if !self.tell(&format!("secret {entry}")) {
            self.error = "The desktop is not answering".into();
            return;
        }
        let mut answer = String::new();
        match std::io::stdin().lock().read_line(&mut answer) {
            Ok(n) if n > 0 && answer.trim() == "ok" => self.unlock(),
            // `no <what to say>`: the shell's words, which carry how long to wait.
            Ok(n) if n > 0 => {
                let said = answer.trim().strip_prefix("no").unwrap_or("").trim();
                self.error = if said.is_empty() { self.ask.wrong().into() } else { said.into() };
            }
            _ => self.error = "The desktop is not answering".into(),
        }
    }

    fn unlock(&mut self) {
        if let Some(lock) = self.lock.take() {
            lock.unlock();
            let _ = self.conn.roundtrip();
        }
        self.exit = Some(0);
    }

    fn draw(&mut self) {
        let now = chrono::Local::now();
        self.view.set_time(now.format("%H:%M").to_string().into());
        self.view.set_date(now.format("%A, %B %-d").to_string().into());
        self.view.set_digits(self.entry.chars().count() as i32);
        // What was typed leaves this view the moment the eye is shut (and an empty entry is
        // shown as nothing), so a hidden entry is never held in it.
        let shown = if self.view.get_revealed() { self.entry.clone() } else { String::new() };
        self.view.set_revealed_text(shown.into());
        self.view.set_error(self.error.clone().into());
        // Read again each time: a laptop on battery moves, and a stale level on a lock screen is
        // a lie. None on a machine with no battery, which draws none.
        let (percent, charging) = battery().unwrap_or((-1, false));
        self.view.set_battery_percent(percent);
        self.view.set_battery_charging(charging);

        let qh = self.qh.clone();
        for (surface, (w, h)) in &self.surfaces {
            let (w, h) = (*w, *h);
            if w == 0 || h == 0 {
                continue;
            }
            self.window.set_size(slint::PhysicalSize::new(w, h));
            slint::platform::update_timers_and_animations();
            let mut pixels = vec![PremultipliedRgbaColor::default(); (w * h) as usize];
            self.window.request_redraw();
            self.window.draw_if_needed(|renderer| {
                renderer.render(&mut pixels, w as usize);
            });
            let Ok(mut pool) = RawPool::new((w * h * 4) as usize, &self.shm) else { continue };
            // ARGB8888 is little-endian: blue, green, red, alpha in memory. A lock surface is
            // opaque — nothing behind it may show through.
            for (px, out) in pixels.iter().zip(pool.mmap().chunks_exact_mut(4)) {
                out.copy_from_slice(&[px.blue, px.green, px.red, 0xff]);
            }
            let buffer = pool.create_buffer(0, w as i32, h as i32, (w * 4) as i32, wl_shm::Format::Argb8888, (), &qh);
            let s = surface.wl_surface();
            s.attach(Some(&buffer), 0, 0);
            s.damage_buffer(0, 0, w as i32, h as i32);
            s.commit();
            buffer.destroy();
        }
    }

    /// Do what the view's buttons asked for, after the pointer event that pressed them.
    fn act_on_request(&mut self) {
        match self.requested.take() {
            Some(Request::Submit) => self.ask_shell(),
            Some(Request::Power(Power::Restart)) if !self.can_restart => {}
            Some(Request::Power(power)) => {
                // Not waited on here: `systemctl suspend` can take seconds, and the lock must keep
                // answering. A thread reaps it; a refusal is shown on the screen. Its stdin and
                // stdout are not this client's: those are the channel to the shell, and a child
                // holding them could read the shell's `ok` or write into the conversation.
                use std::process::Stdio;
                match std::process::Command::new("systemctl")
                    .arg(power.verb())
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                {
                    Ok(mut child) => {
                        std::thread::spawn(move || {
                            let _ = child.wait();
                        });
                    }
                    Err(_) => self.error = power.failed().into(),
                }
            }
            None => {}
        }
    }

    /// A pointer event for the view. The window takes the size of the output the pointer is over,
    /// so the position means the same thing there as it was drawn.
    fn pointer_event(&mut self, event: &PointerEvent) {
        use slint::platform::{PointerEventButton, WindowEvent};
        // Linux's BTN_LEFT: the only button that presses anything here.
        const LEFT: u32 = 0x110;
        let Some((_, (w, h))) = self.surfaces.iter().find(|(s, _)| *s.wl_surface() == event.surface) else { return };
        if *w == 0 || *h == 0 {
            return;
        }
        self.window.set_size(slint::PhysicalSize::new(*w, *h));
        let position = slint::LogicalPosition::new(event.position.0 as f32, event.position.1 as f32);
        let slint_event = match event.kind {
            PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => WindowEvent::PointerMoved { position },
            PointerEventKind::Leave { .. } => WindowEvent::PointerExited,
            PointerEventKind::Press { button: LEFT, .. } => WindowEvent::PointerPressed { position, button: PointerEventButton::Left },
            PointerEventKind::Release { button: LEFT, .. } => WindowEvent::PointerReleased { position, button: PointerEventButton::Left },
            _ => return,
        };
        self.window.dispatch_event(slint_event);
    }

    fn key(&mut self, event: &KeyEvent) {
        match event.keysym {
            Keysym::Return | Keysym::KP_Enter => self.ask_shell(),
            Keysym::BackSpace => {
                self.entry.pop();
            }
            Keysym::Escape => self.entry.clear(),
            _ => {
                if let Some(text) = &event.utf8 {
                    for c in text.chars().filter(|c| self.ask.takes(*c)) {
                        if self.entry.chars().count() < MOST_CHARS {
                            self.entry.push(c);
                        }
                    }
                    if !text.is_empty() {
                        self.error.clear();
                    }
                }
            }
        }
        if self.exit.is_none() {
            self.draw();
        }
    }
}

impl SessionLockHandler for App {
    fn locked(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _lock: SessionLock) {
        self.locked = true;
        self.tell("locked");
        self.draw();
        // Now, with the lock held and the charcoal on screen, the picture: pre-blurred by the
        // shell, checked and capped by `wallpaper::load`. One that will not load leaves the
        // charcoal, which is still a lock screen.
        if let Some(image) = self.wallpaper.take().and_then(|p| wallpaper::load(std::path::Path::new(&p))) {
            self.view.set_wallpaper(image);
            self.view.set_has_wallpaper(true);
            self.draw();
        }
    }

    fn finished(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _lock: SessionLock) {
        // The compositor refused or ended the lock. Before it was ever locked, that is a
        // compositor that will not lock for us; after, the session is no longer ours to hold.
        eprintln!("yantrik-lock: the compositor ended the lock");
        self.exit = Some(if self.locked { EXIT_FAILED } else { EXIT_UNSUPPORTED });
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: SessionLockSurface,
        configure: SessionLockSurfaceConfigure,
        _serial: u32,
    ) {
        for (s, size) in &mut self.surfaces {
            if s.wl_surface() == surface.wl_surface() {
                *size = configure.new_size;
            }
        }
        self.draw();
    }
}

impl KeyboardHandler for App {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32, _: &[u32], _: &[Keysym]) {}
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32) {}
    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        self.key(&event);
    }
    fn repeat_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        if event.keysym == Keysym::BackSpace {
            self.key(&event);
        }
    }
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}
    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: Modifiers, _: RawModifiers, _: u32) {}
    fn update_repeat_info(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: RepeatInfo) {}
    fn update_keymap(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: Keymap<'_>) {}
}

impl PointerHandler for App {
    fn pointer_frame(&mut self, conn: &Connection, _: &QueueHandle<Self>, _: &wayland_client::protocol::wl_pointer::WlPointer, events: &[PointerEvent]) {
        for event in events {
            if matches!(event.kind, PointerEventKind::Enter { .. }) {
                if let Some(pointer) = &self.pointer {
                    let _ = pointer.set_cursor(conn, CursorIcon::Default);
                }
            }
            self.pointer_event(event);
        }
        self.act_on_request();
        if self.exit.is_none() && self.locked {
            self.draw();
        }
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seats
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self.seats.get_keyboard(qh, &seat, None).ok();
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            // A themed pointer, so the person sees a cursor on the lock surface: without one the
            // compositor may draw none, and the buttons would be there with nothing to aim.
            let cursor = self.compositor.create_surface(qh);
            self.pointer = self
                .seats
                .get_pointer_with_theme::<App, SurfaceData>(qh, &seat, self.shm.wl_shm(), cursor, ThemeSpec::default())
                .ok();
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Keyboard {
            if let Some(k) = self.keyboard.take() {
                k.release();
            }
        }
        if capability == Capability::Pointer {
            self.pointer = None;
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
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_compositor!(App);
delegate_output!(App);
delegate_session_lock!(App);
delegate_shm!(App);
delegate_seat!(App);
delegate_keyboard!(App);
delegate_pointer!(App);
delegate_registry!(App);
wayland_client::delegate_noop!(App: ignore wl_buffer::WlBuffer);

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_shell_tells_the_lock_who_and_what_but_never_what_a_notification_says() {
        let look = Look::from_args(&args(&["--name", "Pranab", "--wallpaper", "/c/lock.png", "--network", "Harbor", "--notifications", "3"]));
        assert_eq!((look.name.as_str(), look.network.as_str(), look.notifications), ("Pranab", "Harbor", 3));
        assert_eq!(look.wallpaper.as_deref(), Some("/c/lock.png"));
        // An older shell sends none of it: the lock still draws, with nothing in the status line.
        let bare = Look::from_args(&args(&["--ask", "password"]));
        assert!(bare.name.is_empty() && bare.network.is_empty() && bare.wallpaper.is_none());
        assert_eq!(bare.notifications, 0);
        assert_eq!(Look::from_args(&args(&["--notifications", "-4"])).notifications, 0, "a count is never negative");
        assert!(Look::from_args(&args(&["--wallpaper", ""])).wallpaper.is_none(), "an empty path is no wallpaper");
        assert!(bare.can_restart, "Restart is offered unless the shell says not to");
        assert!(!Look::from_args(&args(&["--no-restart"])).can_restart, "a start-open machine draws no Restart");
    }

    #[test]
    fn the_power_buttons_run_logind_verbs_and_unlock_nothing() {
        assert_eq!(Power::Suspend.verb(), "suspend");
        assert_eq!(Power::Restart.verb(), "reboot");
        assert_eq!(Power::ShutDown.verb(), "poweroff");
        // Power is a different request from Submit: no button on the screen sends a secret.
        assert_ne!(Request::Power(Power::Suspend), Request::Submit);
    }
}
