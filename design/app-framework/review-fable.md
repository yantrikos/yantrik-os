# YantrikApp: the framework every Yantrik OS app is built on

**Answering:** `brief-app.md`, every numbered section.
**Grounded in:** `c:\Users\sync\codes\yantrik-os-found` at `0a8c1678` (20 Oct 2026): `crates/yantrik-app-runtime/src/*`, `apps/weather/src/*`, `apps/notes/src/main.rs`, `apps/email/src/main.rs`, `apps/spreadsheet/src/{surface,session}.rs`, `crates/yantrik-surface/src/*`, `crates/yantrik-ipc-contracts/src/control_surface.rs`, `frameworks/yantrik-{foundation,app-build}`, `design/platform-sdk-2026-10-21.md`, `docs/architecture.md`, `tools/xtask/layers.txt`.

---

## 0. Where the brief and the code disagree (and what I follow)

I follow the code in each case.

| Brief says | Code says | Consequence for this design |
|---|---|---|
| "Frameworks that exist: `yantrik-design-tokens`, `yantrik-ui-kit`, `yantrik-app-runtime`, `yantrik-surface`, `yantrik-ipc-*`" | Only `yantrik-foundation` and `yantrik-app-build` are under `frameworks/`. The rest are in `crates/` (`layers.txt`: tokens and kit L1, app-runtime and surface L2, ipc L0). | §8 moves them; nothing here assumes the move has happened. |
| "typed errors (`thiserror`)" | No crate in the workspace depends on `thiserror` (no `Cargo.toml` mentions it). Every error today is a `String` or a hand-written enum with a manual `Display`. | Step 1 of §10 adds `thiserror` to `[workspace.dependencies]`. |
| Theme: "3-line load, never live-reloaded" (as a defect) | `theme.rs` argues it on purpose: "the shell relaunches nothing on a theme change … which is how every other desktop behaves". | I add live reload anyway (§5), because it costs one shared watcher thread, but I say where the code's position came from. |
| "`control::App::new(id)…serve()` (adds `present`)" | Correct. Note what the brief does not say: the handlers are `Fn + 'static` and **not `Send`** (`LocalRegistry`); they run on the UI thread, reached by `slint::invoke_from_event_loop` with a 3 s round-trip budget (`UI_ROUNDTRIP`). A grant is spent on the RPC thread *before* the handler. | Intents (§2) run on the UI thread; anything slow must go through a job and `Settle::AfterJob`, which is `answer_later` underneath. |
| Accepted design's `App::builder().window(AppWindow::new()?)` | Slint chooses its backend when the first component is created. The 7 apps that switch to `winit-software` set `SLINT_BACKEND` *before* `XApp::new()`. A builder handed an already-made window cannot do the backend switch, and the single-instance claim must also come before any window exists (a second launch must never open one). | The runner takes a window *constructor*, not a window (§1.3). The accepted design's shape is kept in spirit, its order is corrected. |
| "18 apps" | `apps/` has 21 folders; 18 are workspace members (`blender`, `browser`, `desktop-files` are not). | The migration counts 18. |
| `instance::claim -> Option<InstanceGuard>` | Correct, with a detail worth keeping: on a held slot it asks the running copy to `present` over its surface and says in a sentence what came of it (`present::Presented::sentence`). Notes and Calendar hand-roll `flock`. | One `AppId` from the manifest feeds the claim, the socket, and the `.desktop` surface id, which ends the 3 mismatched-id apps. |
| Weather today: "weather-service" | `apps/weather/src/main.rs` uses raw `SyncRpcClient::for_service("weather")` (not `service::client`), makes **five sequential RPCs** per refresh, falls back to Open-Meteo with `ureq` directly, keeps state in **five `Arc<Mutex<_>>` fields**, polls a `Mutex<Option<WeatherData>>` every 100 ms from a timer it `mem::forget`s, runs `add_location`'s geocode **on the UI thread inside the 3 s budget**, and the `refresh` action calls `ui.invoke_refresh_pressed()` (the Mind drives the button, not the model). | The worked example (§9) is written so that each of those is structurally impossible. |

Slint 1.17 constraints I hold every signature to (§1.6 has the checklist): single UI thread; generated component and **generated global types** (so the framework cannot name `ThemeMode` or `WindowChrome`, hence one macro per app, like today's `window_chrome!`); `ComponentHandle`/`Weak`; `Timer` is `!Send` and stops on drop; `invoke_from_event_loop` needs `Send + 'static`; `set_platform` once per process.

---

## 1. The app model

### 1.1 What an app *is*

An app is **one process, one window, one model, one command type, one surface**:

- a **model**: a plain Rust struct composed of feature models, owned by the framework, borrowed only inside a command;
- a **command**: an enum, `Send`, carrying plain data; every change to the model is a command, whether it came from a button, a key, a menu, a timer, a worker thread, or the Mind;
- an **update** function: `(model, command, effects) -> Result<(), Refused>`, pure except for the effects it *requests*;
- a **render** function: the one place that writes Slint properties, run by the framework after every command, with a `Changed` set saying which features moved;
- a **describe** function: the `View` for the Mind, read from the same model, on the same thread, after the same command;
- **intents**: the subset of commands the Mind, the keyboard and the menu may raise by name, each with its wire contract (name, typed arguments, grade, agent rule, how it settles, what it replies).

There is no `BaseApp`. There is a trait the app implements and a runner the framework owns. Nothing is inherited; everything the framework does for every app is done *by the framework*, in one place, and the app is told through hooks.

This is the Elm shape (model / update / effects / render), chosen for one reason that matters here more than anywhere: **the Mind's `act` and the person's button must do the same thing**, and the only way to make that true structurally is for both to be the same command applied by the same function to the same model, with the UI drawn from the result. The survey's largest defects (`refresh` invoking the button; `add_location` saying "added" before the lookup landed; two paths for units) are all "the button and the action are two code paths".

### 1.2 The trait

```rust
// frameworks/yantrik-app/src/app.rs
pub trait App: Sized + 'static {
    /// The generated Slint root component. `yantrik_app::window!(WeatherApp)` makes it a `Window`.
    type Ui: Window;
    /// Plain Rust. Composed of feature models. Never holds a Slint handle.
    type Model: 'static;
    /// Plain data, `Send`: it crosses from worker threads and from the RPC thread.
    type Command: Send + 'static;
    /// Typed, versioned, atomically written (§5).
    type Settings: Settings;

    /// `include_str!("../yantrik.toml")`: id, name, purpose, entitlements. One source for the
    /// claim id, the socket name, the .desktop surface id, the tracing target.
    const MANIFEST: &'static str;

    /// Build the model before any window exists. Settings are loaded, paths known, services
    /// constructed (real or fake), the launch arguments parsed. No UI.
    fn model(boot: &Boot<Self>) -> Result<Self::Model, Startup>;

    /// Bind the window's callbacks to commands. Each feature's `view::wire` is called here.
    /// Runs once, on the UI thread, after the window is made and themed.
    fn wire(cx: &Cx<Self>);

    /// Declare the control surface, shortcuts and menu items (§2.3).
    fn intents(table: &mut Intents<Self>);

    /// The first commands: initial fetch, timers. Runs after `wire`, before the surface is served.
    fn start(model: &Self::Model, fx: &mut Effects<Self>) { let _ = (model, fx); }

    /// The only function that changes the model.
    fn update(model: &mut Self::Model, command: Self::Command, fx: &mut Effects<Self>)
        -> Result<(), Refused>;

    /// The only function that writes Slint properties. Called after every successful `update`
    /// with the parts that changed; also once at start with `Changed::all()`.
    fn render(ui: &Self::Ui, model: &Self::Model, changed: &Changed);

    /// The Mind's view, from the same model `render` just drew. Keep it cheap: it runs on every
    /// `describe` and around every `act`.
    fn describe(model: &Self::Model) -> View;

    /// Framework events the app may turn into commands (launch handoff, settings file changed,
    /// window activated, Mind reach changed). Theme changes never reach here: the framework
    /// applies them itself.
    fn event(event: Event, model: &Self::Model) -> Option<Self::Command> { let _ = (event, model); None }

    /// Asked when the window is asked to close. `Close::Later` keeps it open and the app must
    /// later run a command that ends in `fx.close()`.
    fn close(model: &Self::Model) -> Close { let _ = model; Close::Now }

    /// After the event loop ended, however it ended (closed, logout, failure): checkpoint,
    /// flush. Timers are already stopped and jobs cancelled when this runs.
    fn stop(model: &mut Self::Model) { let _ = model; }
}

pub enum Close { Now, Later }

pub enum Event {
    /// A second launch handed its arguments to this, the running copy.
    Launch(Launch),
    /// This app's settings file changed on disk (another process, or `yos`). Already reloaded.
    SettingsChanged,
    /// The window became the active one (Calendar checks the date).
    Activated,
    ReachChanged(yantrik_mind::Reach),
}
```

### 1.3 The runner and the `main.rs` an app ends up with

```rust
// apps/weather/src/main.rs — the whole file
fn main() -> std::process::ExitCode {
    yantrik_app::run::<yantrik_weather::Weather>()
}
```

Three lines. Nothing else is permitted in a `main.rs`; a guard test in `yantrik-testing` (`main_is_three_lines`) replaces `no_main_unwraps_its_event_loop`.

```rust
// frameworks/yantrik-app/src/run.rs
pub fn run<A: App>() -> std::process::ExitCode {
    Runner::<A>::new().run()
}

/// The same lifecycle with pieces swapped: tests use it headless with fake services.
pub struct Runner<A: App> { /* services: Services, home: Option<PathBuf>, headless: bool */ }
impl<A: App> Runner<A> {
    pub fn new() -> Self;
    pub fn services(self, services: Services) -> Self;
    pub fn home(self, dir: impl Into<PathBuf>) -> Self;      // YANTRIK_HOME for this run
    pub fn run(self) -> std::process::ExitCode;
}
```

### 1.4 What the framework does for every app, and the exact lifecycle

Order matters and is fixed. Each step names the hook, if any, and what it replaces in today's mains.

| # | Step (framework) | Hook | Replaces today |
|---|---|---|---|
| 1 | **Manifest** parsed from `A::MANIFEST` → `AppId`, name, purpose, entitlements. A malformed manifest is a startup failure printed as a sentence. | — | the three inconsistent ids |
| 2 | **Boot**: `--version` answered (`yantrik_version::handle_version_flag`), `SLINT_FULLSCREEN` stripped, tracing with target = app id, panic hook → problem record, log ring for `log_tail`. | — | `init_tracing(name)` ×18 |
| 3 | **Backend**: if `LIBGL_ALWAYS_SOFTWARE=1` and `SLINT_BACKEND` is unset or `winit`, select `winit-software` (`slint::BackendSelector::new().backend_name("winit-software").select()`, falling back to the env var when the selector refuses because a platform is already set). Before any component exists. | — | the 5-line switch ×7 |
| 4 | **Launch** parsed from `argv`: files, `--open`, unknown flags refused with a sentence. | — | argv parsing ×7 |
| 5 | **Single instance**: `instance::claim(id)`. Held → `handoff` (§1.5) → exit 0. | — | `claim` ×16, `flock` ×2, the 20×100 ms loop ×7 |
| 6 | **Paths and settings**: `Paths::for_app(&id)`, `Store::<A::Settings>::open` (migrating), `Drafts` opened. Then `Services` built from entitlements (real clients; fakes under `Runner::services`). | — | 8 `home()`s, 7 atomic writers |
| 7 | **Model**: `A::model(&boot)`. Failure → sentence on stderr, problem record, exit 1. | `model` | "building state" |
| 8 | **Window**: `A::Ui::make()`; `apply_theme(&Theme::current())`; `connect_chrome()`; `connect_shortcuts()`. | — | `XApp::new().unwrap()`, `window_chrome!`, 3 theme lines ×18 |
| 9 | **Cx** built: weak window, the model, the runtime (timers, job pool, intents registry). `A::wire(&cx)`. | `wire` | `wire()` (minus its logic) |
| 10 | **Intents** collected: `A::intents(&mut table)`; `present` added; declaration problems (`Registry::problems`) are fatal in debug and a problem record in release. Shortcuts and menu built from the same table. | `intents` | `publish_control` / `surface()` |
| 11 | **First render**: `A::render(ui, model, Changed::all())`. Then `A::start(model, fx)` and its effects applied. | `start` | "first refresh" |
| 12 | **Surface served** on `app-<id>.sock` (the `control.rs` hop, unchanged). **Watchers** started (one housekeeping thread: settings.yaml mtime for theme, the app's settings file, Mind reach). | — | surface published in `main` *or* `wire` |
| 13 | **Launch delivered**: `A::event(Event::Launch(launch))` → command. | `event` | per-app argv handling |
| 14 | **Event loop**: `run_until_closed` (unchanged: logout is not a crash). Close requests go to `A::close`. | `close` | 18 ignored bools, 2 `on_close_requested` copies |
| 15 | **Shutdown**: timers stopped and dropped, jobs cancelled, pool drained for ≤2 s then detached, `A::stop(model)`, checkpoints flushed, instance guard dropped, exit code from how the loop ended (0 closed, 0 display gone, 1 failure with a record). | `stop` | the 8 hand-written shutdowns |

**Crash and problem reporting.** Unchanged in substance, moved to Foundation: panic hook, `record_failure`, scrubbing, the log tail. New: `fx.report(what, &error)` for a failure the app survived, and `cx.run` writes a record for any `Refused` whose `kind` begins with `internal_` (a bug, not a refusal).

### 1.5 Launch handoff

A second launch, finding the slot held, asks the running copy over its surface, in this order, each bounded (`present::BIND_WAIT` 2 s, `ANSWER_WAIT` 3 s, reused):

1. `present` (already on every surface; refused for agents; the sentence on failure is `Presented::sentence`, unchanged);
2. if the launch carries arguments and the app declared a launch intent, `act <intent> {args}`.

The app declares which intent takes a launch: `table.launch("open", |launch| json!({"path": launch.files[0]}))`. No app writes the loop again.

### 1.6 Slint 1.17 constraints, and how each signature respects them

- **Generated types.** `WeatherApp`, `ThemeMode`, `AccentPreset`, `WindowChrome`, `AgentContextItem` are generated into the *app's* crate. The framework names none of them. It names `A::Ui: Window` and the app makes its component a `Window` with one macro (§1.7), exactly as `window_chrome!` does today, because `macro_rules!` resolves item paths at the invocation site.
- **`ComponentHandle` is a trait, components are not trait objects.** Everything generic is `A::Ui: ComponentHandle + 'static` (as `run_until_closed` already is). No `Box<dyn ComponentHandle>` anywhere.
- **Single UI thread.** `Cx` is `!Send` (`Rc` inside) and lives on the UI thread. The model is reached only through `Cx`. Worker threads never see `Cx`, the model, or the window; they get `&Services`, a `Cancel`, and return a `Command`.
- **The hop.** A job's result comes back through `slint::invoke_from_event_loop(Box<dyn FnOnce + Send>)`; `Weak::upgrade_in_event_loop` is used only by the `present` installer. The surface's `on_ui_thread` is unchanged.
- **Timers.** `slint::Timer` is `!Send` and stops on drop. The runtime owns them in `HashMap<TimerId, slint::Timer>` on the UI thread; no app ever holds a `_binding` or `mem::forget`s one again.
- **`set_platform` once.** The headless harness installs one platform per test process; window tests take a process-wide lock (§7).
- **`slint::spawn_local`** exists and works on winit; it is *not* the baseline (§3), because every service client is blocking and two concurrency models in one app is the soup we are removing.

### 1.7 `Window`: the per-app macro

```rust
// frameworks/yantrik-app/src/window.rs
pub trait Window: slint::ComponentHandle + 'static {
    fn make() -> Result<Self, slint::PlatformError>;
    fn apply_theme(&self, theme: &yantrik_design::Theme);
    fn connect_chrome(&self);
    fn connect_shortcuts(&self, pressed: Box<dyn Fn(&str) + 'static>);
    /// The one notice line every AppWindow draws. Replaces the 8 `say`/`settle` copies.
    fn set_notice(&self, text: &str);
}

/// One line per app. Requires the root .slint to export `ThemeMode`, `AccentPreset`,
/// `WindowChrome` and `Shortcuts` from the kit, and to inherit `AppWindow` (which has `notice`).
#[macro_export]
macro_rules! window {
    ($ui:ident) => {
        impl $crate::Window for $ui {
            fn make() -> Result<Self, $crate::slint::PlatformError> { $ui::new() }
            fn apply_theme(&self, t: &$crate::design::Theme) {
                use $crate::slint::ComponentHandle as _;
                self.global::<ThemeMode>().set_dark(t.dark);
                self.global::<AccentPreset>().set_index(t.accent_index);
            }
            fn connect_chrome(&self) { $crate::chrome::connect(self, self.global::<WindowChrome>()) }
            fn connect_shortcuts(&self, pressed: Box<dyn Fn(&str) + 'static>) {
                self.global::<Shortcuts>().on_pressed(move |chord| pressed(chord.as_str()));
            }
            fn set_notice(&self, text: &str) { self.set_notice(text.into()) }
        }
    };
}
```

`chrome::connect` is today's `window_chrome!` body as a function generic over the global handle (`WindowChrome<'_>` is a generated type too, so the macro passes it in and `connect` is generic over a small `Chrome` trait the macro also implements; the five callbacks are wired once). The kit's `AppWindow` gains a `Shortcuts` global (`callback pressed(string)`) fed by one `FocusScope` in `app_window.slint` that formats the chord (`"Ctrl+R"`), and a `notice` property drawn by `NoticeBar`. Chord formatting is "visual state", allowed in `.slint`.

---

## 2. Features inside an app

### 2.1 The directory, made concrete

```
src/features/<name>/
    mod.rs       pub use model::{Model, Command}; pub const PART: &str = "<name>"; re-exports update/wire/render/intents
    model.rs     Model, Command, update(), pure rules. No Slint, no JSON, no IO. Tests at the bottom or in tests.rs.
    view.rs      wire(cx) binds callbacks → cx.run(Command); render(ui, &Model) writes properties. The only file with `use slint`.
    surface.rs   intents(table): Intent::new(Action, parse) .reply() .agents() .settles() .key() .menu()
    tests.rs     behavioural tests of update() through a TestApp or directly on the Model
```

Rules held by `xtask check-architecture` (new checks, cheap greps): `model.rs` and `surface.rs` import no `slint`; `view.rs` imports no `serde_json`; only `view.rs` and `lib.rs` may call `cx.run`; `main.rs` is three lines.

### 2.2 How a feature declares its parts

```rust
// features/forecast/model.rs (shape; the full one is in §9)
pub struct Model { pub reading: Option<Reading>, pub loading: bool, pub fetched_at: Option<Instant>, pub degraded: Option<String>, pub job: Option<JobId> }

#[derive(Debug, Clone)]
pub enum Command {
    Refresh,
    Fetched(Result<Reading, FetchError>),
    Tick,
}

pub fn update(m: &mut crate::Model, cmd: Command, fx: &mut Effects<Weather>) -> Result<(), Refused> { … }
```

A feature's `update` takes the **whole app model** (`&mut crate::Model`) and owns one field of it. It may *read* sibling fields (forecast reads `m.places.active()` and `m.settings.get().units`); it may *change* a sibling only by `fx.then(sibling::Command::X.into())`. That is a convention with a review rule, not a type system trick: a generic `Lens` would double every signature for a rule a reviewer can see in one line.

### 2.3 Commands and intents: one thing or two?

**Two things, with one direction.** Every intent *is* a command; not every command is an intent. `Edited(String)` at every keystroke, `Fetched(..)` from a worker, `Tick` from a timer are commands and nothing else. `Refresh`, `AddLocation { name }`, `SetUnits(Units)` are commands *and* intents: the intent is the command plus its wire contract.

```rust
// frameworks/yantrik-app/src/intents/mod.rs
pub struct Intents<A: App> { /* Vec<Intent<A>>, state_rule, launch */ }
impl<A: App> Intents<A> {
    pub fn add(&mut self, intent: Intent<A>);
    /// A rule every call is held to before parsing (the shell's "locked desktop").
    pub fn state_rule(&mut self, rule: fn(&A::Model) -> Result<(), Refused>);
    /// Which intent takes a second launch's arguments (§1.5).
    pub fn launch(&mut self, action: &'static str, args: fn(&Launch) -> serde_json::Value);
}

pub struct Intent<A: App> {
    action: Action,                 // yantrik_surface::Action: name, description, params, grade, defers, explain
    parse: fn(&Args<'_>) -> Result<A::Command, Refused>,
    reply: fn(&A::Model) -> serde_json::Value,
    agents: Agents<A>,
    settle: Settle,
    key: Option<&'static str>,
    menu: Option<&'static str>,
}

pub enum Agents<A: App> {
    /// Any caller.
    Allowed,
    /// Refused for an agent with this sentence (e.g. `present`).
    Refused(&'static str),
    /// Checked for an agent only, before parse: the per-app `agent_rule.rs` of 4 apps, in one place.
    Rule(fn(&A::Model, &Args<'_>) -> Result<(), Refused>),
}

pub enum Settle {
    /// The command finished the work: reply from the model now.
    OnReturn,
    /// The command only started it (`Action::defers`): reply now, `settled: false`.
    Deferred,
    /// Wait for the job the command spawned, then reply from the model (`answer_later` underneath).
    AfterJob { within: Duration },
}

impl<A: App> Intent<A> {
    pub fn new(action: Action, parse: fn(&Args<'_>) -> Result<A::Command, Refused>) -> Self;
    pub fn reply(self, f: fn(&A::Model) -> serde_json::Value) -> Self;   // default: `{}`
    pub fn agents(self, a: Agents<A>) -> Self;                             // default: Allowed
    pub fn settles(self, s: Settle) -> Self;                               // default: OnReturn
    pub fn key(self, chord: &'static str) -> Self;                         // "Ctrl+R": parse(&{}) must succeed; checked at declaration
    pub fn menu(self, path: &'static str) -> Self;                         // "View/Refresh"
}

/// Typed, once. The dispatch has already checked presence and type against `Action`'s params
/// and filled defaults, so these never fail for a declared argument; they fail for an empty
/// required string and for a value outside a declared set.
pub struct Args<'a>(&'a serde_json::Value);
impl Args<'_> {
    pub fn text(&self, name: &str) -> Result<&str, Refused>;        // non-empty after trim
    pub fn text_opt(&self, name: &str) -> Option<&str>;
    pub fn integer(&self, name: &str) -> Result<i64, Refused>;
    pub fn number(&self, name: &str) -> Result<f64, Refused>;
    pub fn flag(&self, name: &str) -> Result<bool, Refused>;
    pub fn one_of<E: OneOf>(&self, name: &str) -> Result<E, Refused>;  // an enum with `fn names() -> &[&str]`, `fn parse(&str)`
    pub fn raw(&self) -> &serde_json::Value;
}
```

**What one declaration serves.** From `Intent { action: "refresh", key: "Ctrl+R", menu: "View/Refresh" }` the framework derives: the `app.act` handler (parse → `cx.run` → reply), the `describe` schema (unchanged `Action::schema`), the shortcut table (`Shortcuts.pressed("Ctrl+R")` → `parse(&{})` → `cx.run`), and the menu model (`AppWindow.header-actions` rows with `action-id`, as the kit's component contract asks). The button is wired in `view.rs` as `cx.run(Command::Refresh)` directly, not through the intent: a button is typed Rust and needs no parse, and the audit question ("was this an agent?") is answered by `agent_is_calling()` in the surface path only, where it is true.

**The dispatch, per call, on the UI thread** (the surface's existing order is kept in front of it: private mode, standing, reach, argument checks, ceiling, mode, grant spent, revision guard):

1. `state_rule(model)`;
2. if `agent_is_calling()`: `agents` check;
3. `parse(args)` → command;
4. `cx.run_for_surface(command)` → `Err(Refused)` becomes the `-32602` message *and* the notice line, in the same words;
5. settle: `OnReturn`/`Deferred` → `reply(model)` now; `AfterJob` → the runtime registers a waiter on the first `JobId` the command spawned and hands `answer_later` a closure that `recv_timeout(within)`s the reply the UI thread computes once that job's completion command has been applied and rendered.

The 4 `agent_rule.rs` files become `Agents::Rule(fn)` on the intents that need them, calling the same `home_paths::may_read_file` rules; `agent_is_calling()` is no longer called from inside app code.

### 2.4 Registration

In `lib.rs` (§9): `wire` calls each feature's `view::wire(cx)`; `intents` calls each feature's `surface::intents(table)`; `update` is a one-`match` dispatch over the app `Command` enum; `render` a one-`if` per `PART`. A feature is "registered" by appearing in those four places and in the `Model` and `Command` types. A missing arm is a compile error, which is the registration check.

### 2.5 Sharing state without `Rc<RefCell>` soup

There is exactly one `Rc<RefCell<A::Model>>` in the process and it is private to `Cx`. Nobody clones it, nobody captures it, nobody holds a borrow across a callback. Features see the model as `&mut` inside `update` and `&` inside `render`/`describe`/`reply`; that is all.

Re-entrancy: `cx.run` inside `update` (or inside `render`, where `ui.invoke_*` could fire a callback) does not panic on the `RefCell`; the runtime queues it and drains the queue after the current command's render. `fx.then` is the stated way; the queue is the safety net, and it logs at `debug` so a reviewer finds the stray call.

### 2.6 A callback reaches the model without the weak-upgrade boilerplate

```rust
// features/forecast/view.rs
pub fn wire(cx: &Cx<Weather>) {
    let Some(ui) = cx.ui() else { return };
    ui.on_refresh_pressed(cx.cmd(|| Command::Refresh.into()));
    ui.on_weather_select_location(cx.cmd1(|row: i32| places::Command::Select(row.max(0) as usize).into()));
}
```

```rust
impl<A: App> Cx<A> {
    pub fn run(&self, command: impl Into<A::Command>);
    pub fn read<R>(&self, f: impl FnOnce(&A::Model) -> R) -> R;
    pub fn ui(&self) -> Option<A::Ui>;
    /// Closures for Slint callbacks of arity 0, 1, 2: each is `move || cx.run(f(..))` with the
    /// `Cx` cloned once. No `Weak`, no `upgrade`, because `run` holds the window itself.
    pub fn cmd(&self, f: impl Fn() -> A::Command + 'static) -> impl Fn() + 'static;
    pub fn cmd1<T>(&self, f: impl Fn(T) -> A::Command + 'static) -> impl Fn(T) + 'static;
    pub fn cmd2<T, U>(&self, f: impl Fn(T, U) -> A::Command + 'static) -> impl Fn(T, U) + 'static;
    pub fn services(&self) -> &Services;
    pub fn mind(&self) -> &Mind;
    pub fn paths(&self) -> &Paths;
    pub fn id(&self) -> &AppId;
}
```

The 33 copies of `let weak = …; let Some(ui) = weak.upgrade() else { return };` per app become zero. `cx.ui()` exists for the rare view that needs the handle outside a callback (binding a `VecModel` at wire time); using it inside `update` is a lint failure.

---

## 3. State, threads and async

**One pattern:**

- **UI-thread state** is the model, reached through commands (§2).
- **Background work** is a *job*: `fx.spawn(|services, cancel| -> Command)`. Jobs run on a small fixed pool of named threads (`<id>-job-0..3`), take `&Services` (blocking typed clients) and a `Cancel`, and *return a command*. The pool posts `Completed { id, command }` to the UI thread with `invoke_from_event_loop`; the runtime runs it through `cx.run`, then wakes any `Settle::AfterJob` waiter on that id.
- **Results back to the UI** are therefore commands, applied by `update`, drawn by `render`. There is no `Mutex<Option<Data>>` polled by a timer, and no `upgrade_in_event_loop` closure that pokes properties from a thread.
- **Timers have an owner**: the runtime. `fx.every(TimerId, period, command)` and `fx.after(TimerId, delay, command)` (re-arming the same id restarts it: the 750 ms autosave debounce of Notes is one line). `fx.stop_timer(id)`. Stopped and dropped at shutdown.
- **Cancellation**: `JobId` + `Cancel` (an `Arc<AtomicBool>` the job polls between RPCs); `fx.cancel(id)`; a cancelled job's completion is dropped by the runtime; at shutdown every job is cancelled, the pool is drained for 2 s, then the threads are detached (a blocked `connect` must not hold the exit).

```rust
// frameworks/yantrik-app/src/effects.rs
pub struct Effects<A: App> { /* queued, private */ }
impl<A: App> Effects<A> {
    pub fn changed(&mut self, part: &'static str);
    pub fn then(&mut self, command: impl Into<A::Command>);
    pub fn spawn(&mut self, job: impl FnOnce(&Services, &Cancel) -> A::Command + Send + 'static) -> JobId;
    pub fn cancel(&mut self, job: JobId);
    pub fn every(&mut self, id: TimerId, period: Duration, command: A::Command) where A::Command: Clone;
    pub fn after(&mut self, id: TimerId, delay: Duration, command: A::Command);
    pub fn stop_timer(&mut self, id: TimerId);
    pub fn notify(&mut self, notification: yantrik_notifications::Notification);
    pub fn checkpoint(&mut self, key: &'static str, bytes: Vec<u8>);   // atomic, off-thread, state dir
    pub fn report(&mut self, what: &'static str, error: &dyn std::error::Error);
    pub fn close(&mut self);                                            // finish a `Close::Later`
}
pub struct JobId(u64);
pub struct TimerId(pub &'static str);
pub struct Cancel(Arc<AtomicBool>);
impl Cancel { pub fn is_set(&self) -> bool; }
```

**With Slint's model in mind:** every function in §1–2 that touches `A::Ui` or `A::Model` runs on the thread that created the component, because `Cx` cannot leave it (`Rc`). Every function that may block (a job) cannot reach either, because it is given neither. The compiler enforces the split; the pool size enforces the ceiling on threads.

**Async.** Not in apps. The transport's sync client is what every service speaks; the surface's own RPC server keeps its private two-worker tokio runtime inside `yantrik-surface`, invisible to the app. `slint::spawn_local` stays available for a future async client, and will be introduced only if a service gains a streaming method (a Mind token stream is the one candidate). Two concurrency models in one app is refused.

**Notes' Job/Event worker thread** maps one-to-one: `Job::Save(note)` becomes `fx.spawn(move |_, _| Command::Saved(store::save(&dir, &note)))`; the `mpsc` pair, the `Stop` job, the 100 ms `receive` timer and the `JoinHandle` all go.

---

## 4. Services, Mind and notifications

Three framework crates (per the accepted design), all L2, all `Send + Sync`, all blocking with a deadline per method, all usable from a job and from a test with a fake behind them.

```rust
// frameworks/yantrik-services/src/lib.rs
pub struct Services { weather: WeatherClient, calendar: CalendarClient, notes: NotesClient, /* … one per ipc-contracts domain */ }
impl Services {
    /// Real clients, each lazily `service::ensure`d on first use (the shell starts it).
    pub fn real() -> Services;
    pub fn weather(&self) -> &WeatherClient;
    /* … */
}

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("the {service} service could not be reached: {reason}")]
    Unreachable { service: &'static str, reason: String },
    #[error("the {service} service did not answer {method} within {after:?}")]
    Timeout { service: &'static str, method: &'static str, after: Duration },
    #[error("{message}")]
    Refused { code: i32, message: String },
    #[error("the {service} service answered {method} in a shape this app does not read: {reason}")]
    Shape { service: &'static str, method: &'static str, reason: String },
}

// frameworks/yantrik-services/src/weather.rs
pub trait WeatherApi: Send + Sync {
    fn forecast(&self, place: &Place, units: Units, days: u32) -> Result<Forecast, ServiceError>;
    fn geocode(&self, query: &str) -> Result<Vec<Place>, ServiceError>;
}
pub struct WeatherClient(Arc<dyn WeatherApi>);
impl WeatherClient {
    pub fn forecast(&self, place: &Place, units: Units, days: u32) -> Result<Forecast, ServiceError>; // deadline 8 s
    pub fn geocode(&self, query: &str) -> Result<Vec<Place>, ServiceError>;                            // deadline 1.2 s (today's GEOCODE_TIMEOUT)
}
```

`forecast` is **one method** that the service answers with current + hourly + daily + alerts + air quality (the service gains `weather.forecast`; its five methods stay for one release). The direct Open-Meteo fallback moves *into the service* (the one owner of that domain); the app's job reports `ServiceError::Unreachable` and the model says `degraded`, as today's `reading_from` does.

**Caller identity** is not an API: the service reads `SO_PEERCRED` at accept. The app can neither claim nor forge it. What an app *does* get is the Mind side: `yantrik_surface::caller()` / `agent_token()` inside an intent, which the framework folds into `Agents::Rule`.

```rust
// frameworks/yantrik-mind/src/lib.rs   (today's companion.rs, typed)
pub struct Mind { /* client with 90 s ask deadline */ }
pub struct Prompt { pub text: String, pub context: Vec<String>, pub max_lines: Option<u8> }
pub struct Answer { pub text: String, pub model: String }   // labelled output, as the design asks
impl Mind {
    pub fn ask(&self, prompt: &Prompt) -> Result<Answer, AskError>;             // from a job only
    pub fn recall_relevant(&self, query: &str, want: usize) -> Vec<Recalled>;    // floor 0.35, unchanged
    pub fn reach(&self) -> Reach;                                                // non-blocking, from the shared watcher
}
pub use companion::{AskError, Reach, Recalled, NO_MODEL_HINT, OFFLINE_HINT};   // sentences unchanged
```

`yantrik-notifications` is `notify.rs` renamed; `fx.notify(Notification::new(..))` is the only way an app sends one (the builder gains the app's name from the manifest, so `Notification::new("Downloads", …)` loses its first argument).

### 4.1 The agent rail, once

```rust
// frameworks/yantrik-app/src/features/agent_rail/{mod,model,view,surface}.rs
pub struct Model { pub context: Vec<ContextItem>, pub suggestions: Vec<Suggestion>, pub proposal: Option<Proposal>, pub working: bool, pub reach: Reach }
#[derive(Debug, Clone)]
pub enum Command { Tick, Activate(String), Dismiss, Apply, Answered(Result<Answer, AskError>) }
pub const PART: &str = "agent_rail";
pub const TICK: TimerId = TimerId("agent_rail");

/// What the app supplies: what the rail says about this app's state, and what to ask.
pub trait Source: Sized {
    fn context(&self) -> Vec<ContextItem>;
    fn suggestions(&self, reach: Reach) -> Vec<Suggestion>;
    fn prompt(&self, suggestion: &str) -> Option<Prompt>;
    fn apply(&mut self, proposal: &Proposal) -> Option<Proposal>;   // for proposals with an impact; None = nothing to apply
}
pub trait HasRail { fn rail(&self) -> &Model; fn rail_mut(&mut self) -> &mut Model; }

pub fn start<A: App>(fx: &mut Effects<A>) where A::Command: From<Command>;        // every 4 s: Tick
pub fn update<A: App>(model: &mut A::Model, cmd: Command, fx: &mut Effects<A>) -> Result<(), Refused>
    where A::Model: HasRail + Source, A::Command: From<Command>;
pub fn wire<A: App>(cx: &Cx<A>) where A::Ui: RailView, A::Command: From<Command>;
pub fn render<A: App>(ui: &A::Ui, model: &Model) where A::Ui: RailView;
pub fn describe(model: &Model) -> serde_json::Value;   // the rail's part of `describe`

/// Implemented by `yantrik_app::agent_rail!(WeatherApp)`: maps the plain structs to the
/// generated `AgentContextItem`/`AgentSuggestion`/`AgentProposal` and binds the four callbacks.
pub trait RailView: slint::ComponentHandle {
    fn set_rail(&self, context: &[ContextItem], suggestions: &[Suggestion], proposal: Option<&Proposal>, working: bool, unavailable: &str);
    fn connect_rail(&self, activated: Box<dyn Fn(String)>, dismissed: Box<dyn Fn()>, applied: Box<dyn Fn()>);
}
```

`Tick` reads `mind.reach()` (non-blocking), rebuilds context and suggestions from `Source`, marks `PART`. `Activate(id)` sets `working`, builds the prompt, `fx.spawn(move |_, _| Command::Answered(mind.ask(&prompt)).into())`. `Answered` fills the proposal or the hint sentence from `AskError::hint()`. Eight copies of `refresh_agent_rail`, eight 4 s timers and the 45-line ask handlers become an `impl Source for Model` of ~40 lines per app.

---

## 5. Settings and storage

### 5.1 One path rule

```rust
// frameworks/yantrik-app/src/paths.rs
pub struct Paths { config: PathBuf, data: PathBuf, state: PathBuf }
impl Paths {
    /// $YANTRIK_HOME/{config,data,state}/<id> when set (tests, a second profile);
    /// else $XDG_CONFIG_HOME|~/.config /yantrik/<id>.json, $XDG_DATA_HOME|~/.local/share /yantrik/<id>/,
    /// $XDG_STATE_HOME|~/.local/state /yantrik/<id>/.
    pub fn for_app(id: &AppId) -> Result<Paths, paths::Error>;
    pub fn settings_file(&self) -> &Path;   // ~/.config/yantrik/<id>.json      (Weather already uses this)
    pub fn data_dir(&self) -> &Path;        // ~/.local/share/yantrik/<id>/      (Notes' library already lives here)
    pub fn state_dir(&self) -> &Path;       // ~/.local/state/yantrik/<id>/      (drafts, recovery)
}
```

`$YANTRIK_<APP>_DIR` overrides are honoured for one release with a `warn!` and then removed. The 8 private `home()`s and 7 `~` expanders go: `yantrik_foundation::home::expand` is the one.

### 5.2 Typed settings with migrations, atomic writes, live change

```rust
// frameworks/yantrik-app/src/settings.rs
pub trait Settings: Serialize + DeserializeOwned + Default + Clone + PartialEq + 'static {
    /// Bumped when the shape changes; `migrate` is given every older version.
    const SCHEMA: u32;
    fn migrate(from: u32, raw: serde_json::Value) -> Result<serde_json::Value, settings::Error> { let _ = from; Ok(raw) }
}

pub struct Store<T: Settings> { path: PathBuf, current: T, seen_mtime: Option<SystemTime> }
impl<T: Settings> Store<T> {
    /// Missing → defaults (and nothing written yet). Unreadable → defaults + a problem record +
    /// the file left in place (Weather's rule, kept). Older schema → migrated and rewritten once.
    pub fn open(path: &Path) -> Store<T>;
    pub fn get(&self) -> &T;
    /// Change, write atomically (temp in the same dir + rename, from Foundation's json_store),
    /// and on a failed write put memory back (Weather's rule, kept) and return why.
    pub fn update(&mut self, change: impl FnOnce(&mut T)) -> Result<(), settings::Error>;
    /// Re-read when the file's mtime moved under us. `Ok(true)` when the value changed.
    pub fn reload_if_changed(&mut self) -> Result<bool, settings::Error>;
}
```

File shape: `{"schema": 2, "settings": {…}}`. Weather's existing `{"fahrenheit", "active", "locations"}` with no envelope is read as schema 1 and migrated (§9's `settings.rs` shows it).

**Live change notification** is the housekeeping thread (one per process; it already exists in substance as `companion::ReachWatch`): every 2 s it stats the app's settings file and the shell's `settings.yaml`; a moved mtime posts `Event::SettingsChanged` (after `reload_if_changed`) or applies the theme, through the hop. mtime polling, not inotify: no new dependency, works on every filesystem including the SSHFS/9p mounts dev builds run on, and two seconds is under the rail's own tick. If a settings write ever needs to be seen faster, inotify is a drop-in behind the same `Watch` API.

### 5.3 App data, drafts, recovery

- **Data** (a library of notes, a gallery) stays the app's own files under `data_dir()`, written with `yantrik_foundation::json_store::write_atomically` / `text::write_atomically`. No framework store for domain data: Notes' `store.rs` is right as it is; it just stops carrying its own atomic writer.
- **Drafts and recovery**: `Drafts` in `state_dir()`: `checkpoint(key, bytes)`, `restore(key) -> Option<Vec<u8>>`, `clear(key)`. `fx.checkpoint` writes off-thread; `A::stop` is where the last one is written synchronously. A draft found at start is handed to `A::model` through `Boot::drafts()`.

### 5.4 Where the theme lives

In the shell's `settings.yaml` (`dark_mode`, `accent_color`), as today, read by `yantrik_design::Theme::current()` (today's `theme.rs`: `ThemeSettings`, `ACCENTS`, `offered_accent`, moved into the tokens crate's Rust side, where `AccentPreset`'s numbering is already documented). When YantrikSettings lands (step 10) that reader moves behind its typed `settings.yaml` schema; the `Theme` type and the watcher do not change. Apps never write it.

---

## 6. Errors and logging

**Typed, per module, no global error.** `thiserror` enters the workspace. Each framework module has its own enum (`settings::Error`, `paths::Error`, `drafts::Error`, `ServiceError`, `AskError`, `Startup`). Each feature has its own (`forecast::FetchError`, `places::PlaceError`). `Display` is written as the sentence the person reads, in the writing rules' voice, because the person and the Mind get the same sentence.

**The wire.**

```rust
// frameworks/yantrik-app/src/refused.rs
/// A refusal as it crosses to the person (notice line) and to the Mind (`-32602` message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused { pub kind: &'static str, pub sentence: String }

/// What an app error adds to its Display: a stable snake_case kind the Mind can branch on.
pub trait Explain: std::error::Error {
    fn kind(&self) -> &'static str;
}
impl<E: Explain> From<E> for Refused {
    fn from(e: E) -> Refused { Refused { kind: e.kind(), sentence: e.to_string() } }
}
impl Refused {
    pub fn new(kind: &'static str, sentence: impl Into<String>) -> Refused;
}
```

Flow: `update` returns `Err(e.into())`; `cx.run` puts `sentence` on the notice line (the one `say`), logs `warn!(kind, %sentence)`, and if `kind` starts with `internal_` writes a problem record. The surface path returns the same `sentence` as the `-32602` message (the dispatch's contract: "a handler's own `Err` is the caller's refusal, word for word") and `describe` carries `last_refusal: {kind, sentence}` so a Mind can read the kind after the fact. The wire message itself stays a sentence: the protocol's `ServiceError { code, message }` has no data field, and I do not widen a protocol the Python SDK mirrors for a field only a branch would read.

**Logging.** `tracing` with the app id as target (boot sets `<id>=info,yantrik_app=info`); the log ring feeds `log_tail`; the dispatch's own `app.act` line (action, id, ceiling, mode, caller pid) is unchanged and is the audit line. Apps log with `tracing::` and never `eprintln!` except in `main`'s absence of a window (the framework does it).

---

## 7. Testing

### 7.1 Apps become `lib + bin`

`src/lib.rs` holds `slint::include_modules!()`, the `App` impl and `features/`; `src/main.rs` is the three lines. Integration tests in `apps/<name>/tests/` link the lib. **`tests/*-core` crates are deleted** (step 5); their tests move next to the code they test (`features/<name>/tests.rs`) or into `apps/<name>/tests/`. Cost: the lib is compiled twice for unit tests (normal and `cfg(test)`), which includes the app's own Slint codegen (1–2k lines, seconds), not the shell's 38k.

### 7.2 `yantrik-testing` (dev-only, L2)

```rust
pub struct TestApp<A: App> { /* headless platform, runner, fakes, virtual clock */ }
impl<A: App> TestApp<A> {
    /// Headless Slint platform (MinimalSoftwareWindow + an in-process event queue, today's
    /// notes/terminal/text-editor copy, once), fake services, a temp YANTRIK_HOME, jobs run
    /// inline on `tick()`, timers on a virtual clock (`Platform::duration_since_start`).
    pub fn start() -> TestApp<A>;
    pub fn with_services(self, f: impl FnOnce(&mut FakeServices)) -> Self;
    pub fn with_mind(self, f: impl FnOnce(&mut FakeMind)) -> Self;
    pub fn with_settings(self, settings: A::Settings) -> Self;
    pub fn run(&self, command: A::Command) -> Result<(), Refused>;
    /// Drain the event queue, run queued jobs inline (in order), fire timers that are due.
    pub fn tick(&self);
    pub fn advance(&self, by: Duration);
    pub fn model<R>(&self, f: impl FnOnce(&A::Model) -> R) -> R;
    pub fn ui<R>(&self, f: impl FnOnce(&A::Ui) -> R) -> R;
    pub fn notice(&self) -> String;
    /// Through the real `LocalRegistry::act` with a pinned `Authority` (ceiling/mode of the
    /// test's choosing), the real argument checks, the real revision guard, no socket.
    pub fn describe(&self) -> serde_json::Value;
    pub fn act(&self, action: &str, args: serde_json::Value) -> Result<serde_json::Value, Refusal>;
    pub fn act_as_agent(&self, token: &str, action: &str, args: serde_json::Value) -> Result<serde_json::Value, Refusal>;
    pub fn act_with_grant(&self, grant: &str, action: &str, args: serde_json::Value) -> Result<serde_json::Value, Refusal>;
    pub fn allow(&self, grant: &str, action: &str, args: serde_json::Value);   // the surface's `stand_in::allow`, public
    /// Render with the software renderer and compare with tests/snapshots/<name>.png (both
    /// themes, 1.0 and 1.25 text scale when asked). `UPDATE_SNAPSHOTS=1` rewrites.
    pub fn snapshot(&self, name: &str);
}
pub struct Refusal { pub code: i32, pub message: String }
```

**Headless harness.** One `set_platform` per test process (`OnceLock`); window-creating tests hold a process-wide `Mutex` so two never create components on two threads. Model tests (most tests) use no `TestApp` at all: `forecast::update(&mut model, cmd, &mut Effects::recording())` with `Effects::recording()` capturing what was requested, asserted as data.

**Fake services and Mind.** `FakeServices` implements every `*Api` trait with scripted answers (`weather.forecast_returns(Ok(..))`, `geocode_fails(ServiceError::Timeout{..})`); `FakeMind` scripts `ask` and `reach`. They are the same traits the real clients implement, so a feature cannot tell.

**Control-surface harness.** `act` → assert on `describe`, as the brief asks; because `act` runs through the registry, the tests also pin the grade, the argument sentences, the agent rule and the `STALE:` guard with no socket.

**Snapshot/pixel tests.** The tokens crate embeds the fonts, the software renderer is deterministic, so a PNG per screen per theme is stable across machines; tolerance 0.1 % of pixels; CI stores diffs as artifacts. This is the preview harness the design's step 8 wants, reused.

**What replaces `#[path]`:** nothing. The lib is a normal dependency.

---

## 8. Crate layout

```
frameworks/
  yantrik-foundation   L0  + json_store (atomic write), home (expand ~), problems, place, text   (from app-runtime)
  yantrik-app-build    L2  unchanged (+ reads yantrik.toml to generate the .desktop file)
  yantrik-app          L2  NEW: app.rs run.rs boot.rs cx.rs effects.rs jobs.rs timers.rs window.rs chrome.rs
                           instance.rs launch.rs event_loop.rs theme.rs paths.rs settings.rs drafts.rs refused.rs
                           intents/{mod,intent,args,serve,shortcuts}.rs   (serve.rs = control.rs's hop, unchanged)
                           features/agent_rail/{mod,model,view,surface}.rs   watch.rs (the housekeeping thread)
  yantrik-services     L2  NEW: service.rs (ensure/client, from transport) + one typed client per contracts domain + fakes behind a feature
  yantrik-mind         L2  companion.rs, typed
  yantrik-notifications L2 notify.rs
  yantrik-testing      L2  dev-only: TestApp, Headless, FakeServices, FakeMind, snapshot
  yantrik-surface      L2  (moved from crates/, unchanged: YantrikIntents)
crates/
  yantrik-app-runtime  L2  DEPRECATED for one release: `pub use yantrik_app::…` under the old paths, `#[deprecated]`
  yantrik-design-tokens L1 + src/theme.rs (Theme, ACCENTS, offered_accent)  → later renamed yantrik-design, frameworks/
  yantrik-ui-kit       L1  + Shortcuts global, notice on AppWindow, agent_rail!-ready exports
  yantrik-manifest     L0  revived as the `yantrik.toml` reader (it already parses TOML into typed structs); loses the unused package half
```

**What moves where:** `instance`, `chrome`, `event_loop`, `present`, `control` (→ `intents/serve.rs`) and `theme` (reader only) into `yantrik-app`; `place`, `problems` into Foundation; `notify` → notifications; `companion` → mind; `service` → services. `yantrik-app` depends on `slint` (runtime only, no codegen), `yantrik-surface`, `yantrik-services`, `yantrik-mind`, `yantrik-notifications`, `yantrik-foundation`, `yantrik-manifest`, `yantrik-design-tokens`. It does **not** depend on the kit (it names the kit's globals only through the macro, at the app's site). `layers.txt` gains the new lines in the same PRs.

**Deprecated:** `yantrik_app_runtime::*` (all of it, by re-export), `window_chrome!`, `init_tracing`, `run_until_closed` as a public call (it is the runner's), `control::App` (an `Intents` table replaces it; `yantrik_surface::Surface` stays for windowless programs), `theme::load` (→ `Theme::current`), `$YANTRIK_<APP>_DIR`, `tests/*-core`.

**Compile memory:** `yantrik-app` adds no Slint codegen. The apps' own screens move into their crates (step 3) regardless of this design; the design neither helps nor hurts the 14 GB shell problem, which is the process-boundary split (step 12).

---

## 9. A worked example: Weather

Current conditions + 7-day forecast from weather-service, a location search, refresh, 4 Mind actions (`refresh`, `show_location`, `add_location`, `set_units`; `present` comes free), the agent rail. Every file under 300 lines; the sketches below are abridged where a line is obvious, and complete where it is load-bearing.

```
apps/weather/
  Cargo.toml                      lib + bin; deps: yantrik-app, yantrik-services, yantrik-design-tokens, yantrik-ui-kit, serde, slint
  yantrik.toml                    [app] id="weather" name="Weather" purpose="…" ; [entitlements] mind=["ask"] network=false ; [intents] grades-max="sensitive"
  build.rs                        yantrik_app_build::UiBuild::new().compile("ui/app.slint")
  ui/
    app.slint                     WeatherApp inherits AppWindow; exports ThemeMode, AccentPreset, WindowChrome, Shortcuts; composes the two screens      (~90)
    forecast.slint                ForecastView: current, hourly strip, 7-day rows; structs Current, Hour, Day                                             (~260)
    places.slint                  PlacesPanel: saved list, search field, suggestions; struct Place                                                       (~180)
  src/
    main.rs                       3 lines
    lib.rs                        Weather: App; Model; Command; dispatch                                                                                 (~120)
    settings.rs                   WeatherSettings (schema 2) + migration from the v1 file                                                                (~70)
    rail.rs                       impl Source + HasRail for Model                                                                                        (~60)
    features/mod.rs               pub mod forecast; pub mod places;
    features/forecast/mod.rs      (~20)   model.rs (~190)   view.rs (~110)   surface.rs (~40)   tests.rs (~120)
    features/places/mod.rs        (~20)   model.rs (~200)   view.rs (~100)   surface.rs (~110)  tests.rs (~140)
  tests/
    surface.rs                    act → describe, grades, agent rules, STALE                                                                             (~150)
    snapshots/forecast@dark.png   forecast@light.png   places@dark.png
```

### `src/lib.rs`

```rust
#![warn(unreachable_pub)]
//! Weather: what it is like outside, where the person cares about.
slint::include_modules!();
yantrik_app::window!(WeatherApp);
yantrik_app::agent_rail!(WeatherApp);

mod features;
mod rail;
mod settings;

use std::time::Duration;
use yantrik_app::{agent_rail, App, Boot, Changed, Cx, Effects, Intents, Refused, Startup, View};
use features::{forecast, places};
pub use settings::WeatherSettings;

pub struct Weather;

pub struct Model {
    pub settings: yantrik_app::Store<WeatherSettings>,
    pub forecast: forecast::Model,
    pub places: places::Model,
    pub rail: agent_rail::Model,
}

#[derive(Debug, Clone)]
pub enum Command {
    Forecast(forecast::Command),
    Places(places::Command),
    Rail(agent_rail::Command),
}
impl From<forecast::Command> for Command { fn from(c: forecast::Command) -> Self { Command::Forecast(c) } }
impl From<places::Command> for Command { fn from(c: places::Command) -> Self { Command::Places(c) } }
impl From<agent_rail::Command> for Command { fn from(c: agent_rail::Command) -> Self { Command::Rail(c) } }

impl App for Weather {
    type Ui = WeatherApp;
    type Model = Model;
    type Command = Command;
    type Settings = WeatherSettings;
    const MANIFEST: &'static str = include_str!("../yantrik.toml");

    fn model(boot: &Boot<Self>) -> Result<Model, Startup> {
        let settings = boot.settings();
        Ok(Model {
            places: places::Model::from_settings(settings.get()),
            forecast: forecast::Model::default(),
            rail: agent_rail::Model::default(),
            settings,
        })
    }

    fn wire(cx: &Cx<Self>) {
        forecast::view::wire(cx);
        places::view::wire(cx);
        agent_rail::wire(cx);
    }

    fn intents(table: &mut Intents<Self>) {
        forecast::surface::intents(table);
        places::surface::intents(table);
    }

    fn start(_: &Model, fx: &mut Effects<Self>) {
        fx.then(forecast::Command::Refresh);
        fx.every(forecast::REFETCH, Duration::from_secs(30 * 60), forecast::Command::Refresh.into());
        fx.every(forecast::CLOCK, Duration::from_secs(30), forecast::Command::Tick.into());
        agent_rail::start(fx);
    }

    fn update(m: &mut Model, cmd: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        match cmd {
            Command::Forecast(c) => forecast::update(m, c, fx),
            Command::Places(c) => places::update(m, c, fx),
            Command::Rail(c) => agent_rail::update::<Self>(m, c, fx),
        }
    }

    fn render(ui: &WeatherApp, m: &Model, changed: &Changed) {
        if changed.has(forecast::PART) { forecast::view::render(ui, &m.forecast, m.settings.get().units) }
        if changed.has(places::PART) { places::view::render(ui, &m.places) }
        if changed.has(agent_rail::PART) { agent_rail::render::<Self>(ui, &m.rail) }
    }

    fn describe(m: &Model) -> View {
        let place = m.places.active();
        let summary = match (&m.forecast.reading, m.forecast.loading) {
            (_, true) => format!("Weather — loading {}", place.name),
            (Some(r), _) => format!("Weather — {} in {}, {}", r.now.temperature(m.settings.get().units), place.name, r.now.condition),
            (None, _) => format!("Weather — no reading for {}", place.name),
        };
        View::new(summary)
            .with("place", place.name.clone())
            .with("units", m.settings.get().units.name())
            .with("reading", forecast::model::describe(&m.forecast, m.settings.get().units))
            .with("saved_places", places::model::describe(&m.places))
            .with("agent_rail", agent_rail::describe(&m.rail))
            .with("config_file", m.settings.path().display().to_string())
    }
}
```

### `src/settings.rs`

```rust
use serde::{Deserialize, Serialize};
use yantrik_app::settings::{self, Settings};
use yantrik_services::weather::{Place, Units};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WeatherSettings {
    pub units: Units,                 // Celsius | Fahrenheit, serde(rename_all = "lowercase")
    pub places: Vec<Place>,
    pub active: usize,
}

impl Settings for WeatherSettings {
    const SCHEMA: u32 = 2;
    /// Schema 1 is the file Weather wrote until now: `{"fahrenheit": bool, "active": n,
    /// "locations": [{name, lat, lon}]}` with no envelope. Nothing is lost.
    fn migrate(from: u32, raw: serde_json::Value) -> Result<serde_json::Value, settings::Error> {
        match from {
            1 => Ok(serde_json::json!({
                "units": if raw["fahrenheit"] == true { "fahrenheit" } else { "celsius" },
                "places": raw["locations"],
                "active": raw["active"],
            })),
            other => Err(settings::Error::UnknownSchema { found: other, latest: Self::SCHEMA }),
        }
    }
}
```

### `src/features/forecast/model.rs`

```rust
//! The reading on screen: fetched by a job, kept here, formatted by pure rules. No Slint, no JSON.
use std::time::{Duration, Instant};
use yantrik_app::{Effects, JobId, Refused, TimerId, Explain};
use yantrik_services::weather::{Forecast, Place, ServiceError, Units};
use crate::{Model as AppModel, Weather};

pub const PART: &str = "forecast";
pub const REFETCH: TimerId = TimerId("forecast.refetch");
pub const CLOCK: TimerId = TimerId("forecast.clock");

#[derive(Default)]
pub struct Model {
    pub reading: Option<Reading>,
    pub loading: bool,
    pub fetched_at: Option<Instant>,
    pub degraded: Option<String>,
    pub job: Option<JobId>,
}
pub struct Reading { pub place: Place, pub forecast: Forecast }

#[derive(Debug, Clone)]
pub enum Command {
    Refresh,
    Fetched { place: Place, result: Result<Forecast, ServiceError> },
    Tick,
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("the weather service did not answer: {0}")]
    Unavailable(#[from] ServiceError),
}
impl Explain for FetchError { fn kind(&self) -> &'static str { "forecast_unavailable" } }

pub fn update(m: &mut AppModel, cmd: Command, fx: &mut Effects<Weather>) -> Result<(), Refused> {
    match cmd {
        Command::Refresh => {
            if let Some(job) = m.forecast.job.take() { fx.cancel(job) }      // one fetch in flight, the latest
            let place = m.places.active().clone();
            let units = m.settings.get().units;
            m.forecast.loading = true;
            m.forecast.job = Some(fx.spawn(move |services, _cancel| {
                let result = services.weather().forecast(&place, units, 7);
                Command::Fetched { place, result }.into()
            }));
            fx.changed(PART);
            Ok(())
        }
        Command::Fetched { place, result } => {
            m.forecast.job = None;
            m.forecast.loading = false;
            match result {
                Ok(forecast) => {
                    m.forecast.reading = Some(Reading { place, forecast });
                    m.forecast.fetched_at = Some(Instant::now());
                    m.forecast.degraded = None;
                }
                Err(e) => {
                    m.forecast.degraded = Some(e.to_string());
                    fx.changed(PART);
                    return Err(FetchError::from(e).into());   // the notice line and the Mind get one sentence
                }
            }
            fx.changed(PART);
            Ok(())
        }
        Command::Tick => { fx.changed(PART); Ok(()) }        // "Updated 3 min ago" moves
    }
}

/// Pure rules the view and `describe` share. Tested without a window.
pub fn updated_text(fetched_at: Option<Instant>, now: Instant) -> String { /* "Updated just now" … */ }
pub fn day_label(index: usize, date: &str) -> String { /* "Today", "Tue" */ }
pub fn temperature_range(days: &[Day]) -> (f64, f64) { /* for the range bars */ }
pub fn describe(m: &Model, units: Units) -> serde_json::Value { /* current, forecast[7], alerts, updated, degraded */ }
```

### `src/features/forecast/view.rs`

```rust
//! The only file in this feature that knows Slint.
use slint::{ComponentHandle, ModelRc, VecModel};
use yantrik_app::Cx;
use crate::{Weather, WeatherApp, Current, Day, Hour};    // Current/Day/Hour are the generated structs from forecast.slint
use super::{model, Command, Model};

pub fn wire(cx: &Cx<Weather>) {
    let Some(ui) = cx.ui() else { return };
    ui.on_refresh_pressed(cx.cmd(|| Command::Refresh.into()));
    ui.set_hours(ModelRc::new(VecModel::<Hour>::default()));   // bound once; render fills it
    ui.set_days(ModelRc::new(VecModel::<Day>::default()));
}

pub fn render(ui: &WeatherApp, m: &Model, units: yantrik_services::weather::Units) {
    ui.set_loading(m.loading);
    ui.set_updated(model::updated_text(m.fetched_at, std::time::Instant::now()).into());
    ui.set_degraded(m.degraded.clone().unwrap_or_default().into());
    let Some(r) = &m.reading else { return };
    ui.set_current(current_row(&r.forecast.now, &r.place, units));
    fill(&ui.get_hours(), r.forecast.hourly.iter().map(|h| hour_row(h, units)).collect());
    let range = model::temperature_range(&r.forecast.daily);
    fill(&ui.get_days(), r.forecast.daily.iter().enumerate().map(|(i, d)| day_row(i, d, range, units)).collect());
}

/// Refill the VecModel bound at wire time instead of allocating a new ModelRc per render.
fn fill<T: Clone + 'static>(model: &ModelRc<T>, rows: Vec<T>) {
    if let Some(v) = model.as_any().downcast_ref::<VecModel<T>>() { v.set_vec(rows) }
}
fn current_row(..) -> Current { /* formatting only */ }
fn hour_row(..) -> Hour { .. }
fn day_row(..) -> Day { .. }
```

### `src/features/forecast/surface.rs`

```rust
use yantrik_app::intents::{Intent, Intents, Settle};
use yantrik_surface::Action;
use crate::Weather;
use super::Command;

pub fn intents(table: &mut Intents<Weather>) {
    table.add(
        Intent::new(
            Action::new("refresh", "Fetch the current conditions and the week's forecast again").defers(),
            |_args| Ok(Command::Refresh.into()),
        )
        .settles(Settle::Deferred)
        .reply(|m| serde_json::json!({ "refreshing": m.places.active().name }))
        .key("Ctrl+R")
        .menu("View/Refresh"),
    );
}
```

### `src/features/places/model.rs` (the shape)

```rust
pub const PART: &str = "places";
pub struct Model { pub saved: Vec<Place>, pub active: usize, pub query: String, pub suggestions: Vec<Place>, pub search: Option<JobId> }

#[derive(Debug, Clone)]
pub enum Command {
    Search(String),                                    // typed into the field (debounced by fx.after)
    Found { query: String, result: Result<Vec<Place>, ServiceError> },
    Add(String),                                       // the Add button and `add_location`: a name to look up
    Added { query: String, result: Result<Place, PlaceError> },
    Select(usize),                                     // the list row and `show_location` (after a name match)
    Remove(usize),
    SetUnits(Units),                                   // the switch and `set_units`
}

#[derive(Debug, thiserror::Error)]
pub enum PlaceError {
    #[error("no place called \u{201c}{0}\u{201d} was found")] NotFound(String),
    #[error("the geocoder could not be reached: {0}")] Unreachable(#[from] ServiceError),
    #[error("\u{201c}{wanted}\u{201d} is already saved, as \u{201c}{existing}\u{201d}")] AlreadySaved { wanted: String, existing: String },
    #[error("the last saved place cannot be removed; add another first")] LastPlace,
    #[error("there is no saved place {0}")] NoSuchRow(usize),
    #[error("the choice could not be kept: {0}")] NotKept(#[from] yantrik_app::settings::Error),
}
impl Explain for PlaceError { fn kind(&self) -> &'static str { match self { NotFound(_) => "place_not_found", Unreachable(_) => "geocoder_unreachable", AlreadySaved{..} => "place_already_saved", LastPlace => "last_place", NoSuchRow(_) => "no_such_place", NotKept(_) => "settings_not_written" } } }

pub fn update(m: &mut AppModel, cmd: Command, fx: &mut Effects<Weather>) -> Result<(), Refused> {
    match cmd {
        Command::Add(name) => {
            let query = name.trim().to_string();
            if query.is_empty() { return Err(Refused::new("empty_name", "A place needs a name.")) }
            fx.spawn(move |services, _| {
                let result = services.weather().geocode(&query).map_err(PlaceError::from)
                    .and_then(|found| found.into_iter().next().ok_or(PlaceError::NotFound(query.clone())));
                Command::Added { query, result }.into()
            });
            Ok(())
        }
        Command::Added { result, .. } => {
            let place = result?;                                            // PlaceError → Refused, one sentence
            if let Some(existing) = m.places.saved.iter().find(|p| p.is_same_place(&place)) {
                return Err(PlaceError::AlreadySaved { wanted: place.name, existing: existing.name.clone() }.into());
            }
            m.places.saved.push(place);
            m.places.active = m.places.saved.len() - 1;
            m.places.persist(&mut m.settings)?;                             // Store::update, rollback on failure
            fx.changed(PART);
            fx.then(forecast::Command::Refresh);
            Ok(())
        }
        Command::Select(row) => { /* bounds → NoSuchRow; active = row; persist; changed; then Refresh */ }
        Command::Remove(row) => { /* LastPlace rule; follow the same place, not the same row (today's rule, kept) */ }
        Command::SetUnits(units) => { /* persist; changed(PART); then Refresh */ }
        Command::Search(q) => { /* fx.after(SEARCH, 300ms, …) then a geocode job → Found */ }
        Command::Found { .. } => { /* suggestions; changed */ }
    }
}
pub fn row_matching(saved: &[Place], wanted: &str) -> Result<usize, Refused> { /* today's contains-match with the "there is: …" sentence */ }
pub fn describe(m: &Model) -> serde_json::Value { .. }
```

### `src/features/places/surface.rs`

```rust
pub fn intents(table: &mut Intents<Weather>) {
    table.add(
        Intent::new(
            Action::new("show_location", "Switch to one of the saved places").risk("standard").arg(Param::text("name")),
            |args| { let want = args.text("name")?; Ok(Command::SelectByName(want.to_string()).into()) },
        )
        .reply(|m| { let p = m.places.active(); json!({ "showing": p.name, "lat": p.lat, "lon": p.lon }) }),
    );
    table.add(
        Intent::new(
            // sensitive: it writes a line to the person's stored configuration that outlives the turn (today's reasoning, kept)
            Action::new("add_location", "Look a place up and save it").risk("sensitive").arg(Param::text("name")).expected_seconds(3),
            |args| Ok(Command::Add(args.text("name")?.to_string()).into()),
        )
        // The geocode runs on a job; the reply waits for `Added` to be applied, then reads what
        // was actually saved. Nothing runs on the UI thread, and "added" is never said early.
        .settles(Settle::AfterJob { within: Duration::from_secs(4) })
        .reply(|m| { let p = m.places.active(); json!({ "added": p.name, "lat": p.lat, "lon": p.lon, "saved_to": m.settings.path(), "saved_places": m.places.saved.len() }) }),
    );
    table.add(
        Intent::new(
            Action::new("set_units", "Show temperatures in Celsius or Fahrenheit").risk("sensitive").arg(Param::one_of("units", &["celsius", "fahrenheit"])),
            |args| Ok(Command::SetUnits(args.one_of::<Units>("units")?).into()),
        )
        .reply(|m| json!({ "units": m.settings.get().units.name(), "saved_to": m.settings.path() })),
    );
}
```

(`SelectByName(String)` is a places command that resolves the row with `row_matching` inside `update`, so the "no saved location matches … there is: …" sentence is the model's, and the list row's `Select(usize)` shares the rest.)

### `src/rail.rs`

```rust
impl HasRail for Model { fn rail(&self) -> &agent_rail::Model { &self.rail } fn rail_mut(&mut self) -> &mut agent_rail::Model { &mut self.rail } }
impl agent_rail::Source for Model {
    fn context(&self) -> Vec<ContextItem> { /* "21°C, Clear" + "Updated 3 min ago", from forecast */ }
    fn suggestions(&self, reach: Reach) -> Vec<Suggestion> { /* "What should I plan for?" when Ready and there is a reading */ }
    fn prompt(&self, id: &str) -> Option<Prompt> { (id == "advise").then(|| forecast::model::conditions_prompt(self.forecast.reading.as_ref()?, self.settings.get().units)) }
    fn apply(&mut self, _: &Proposal) -> Option<Proposal> { None }   // answers to read, nothing to apply
}
```

### `tests/surface.rs` (two of the tests)

```rust
#[test]
fn add_location_answers_with_what_was_saved_not_what_was_asked() {
    let app = TestApp::<Weather>::start().with_services(|s| s.weather.geocode_returns("paris", vec![place("Paris, France", 48.85, 2.35)]));
    let reply = app.act("add_location", json!({"name": "paris"})).unwrap();   // AfterJob: tick() runs inline under the hood
    assert_eq!(reply["result"]["added"], "Paris, France");
    assert_eq!(app.describe()["state"]["saved_places"].as_array().unwrap().len(), 2);
    assert!(app.model(|m| m.settings.get().places.iter().any(|p| p.name == "Paris, France")));
}

#[test]
fn a_geocoder_that_is_down_is_refused_in_one_sentence_on_screen_and_on_the_wire() {
    let app = TestApp::<Weather>::start().with_services(|s| s.weather.geocode_fails(ServiceError::Timeout { service: "weather", method: "geocode", after: Duration::from_millis(1200) }));
    let err = app.act("add_location", json!({"name": "paris"})).unwrap_err();
    assert_eq!(err.code, -32602);
    assert_eq!(err.message, app.notice());
    assert!(err.message.starts_with("the geocoder could not be reached"));
}
```

**What the example demonstrates against the survey:** 3-line `main`; no `init_tracing`/claim/chrome/theme in the app; the surface declared once with typed args and the agent rule in one place; `refresh` via the model not the button; `add_location` off the UI thread with an honest reply; no `Arc<Mutex>`, no polled slot, no forgotten timer; one settings file with a migration; the rail as `impl Source`; tests with no `#[path]`.

---

## 10. Migration (18 apps, nightly never stops)

Ordered to match steps 4–6 of the accepted plan; every PR leaves every app building.

| # | Lands | Size | Codemod? |
|---|---|---|---|
| 1 | `thiserror` in the workspace; `yantrik-app` crate created by **moving** `instance`, `chrome`, `event_loop`, `present`, `control`, `theme` out of app-runtime unchanged; `place`/`problems` to Foundation; app-runtime becomes re-exports. `layers.txt` updated. No app changes. | S | — |
| 2 | `Window` trait + `window!`, `boot()`, `Launch`, `Paths`, `Store`, the housekeeping watcher. Old API still works. | M | — |
| 3 | **Startup codemod**: a script replaces the recognisable startup block in each `main` (`init_tracing` … theme lines, the `LIBGL` switch, hand-rolled locks, the 20×100 ms loop) with `let boot = yantrik_app::boot::<X>();` + `window!(X)`. Mechanical, reviewed per app, 18 small PRs or 3 batches. Main shrinks ~30 lines each. | S×18 | yes |
| 4 | **lib + bin**: `main.rs` → `lib.rs` with `pub fn main()`, 3-line `main.rs`; `tests/*-core` tests moved in and the crates deleted. | S×18 | yes |
| 5 | `yantrik-services`, `yantrik-mind`, `yantrik-notifications`, `yantrik-testing`; `weather.forecast` on the service. | M | — |
| 6 | **Pilot: Weather** on the full model (§9), with the kit redesign in the same "app pass" (as planned on 20 Oct). Snapshot tests on. Measure: lines, build time, `yos check weather`, 520 evidence. | M | no |
| 7 | **Notes** (worker thread → jobs, 687-line surface → `surface.rs` per feature), then **Calendar** (627-line `publish_control`), then **Email** (749-line `wire`): each one app pass. These three carry most of the long functions. | L each | partly: `settle/say` → notice; `act/arg/needed` → `Intent`/`Args` |
| 8 | The remaining 14, smallest first, one PR each; `agent_rule.rs` ×4 → `Agents::Rule`. | S–M ×14 | partly |
| 9 | Delete `yantrik-app-runtime`; the baseline shrinks; `xtask` gains the feature-directory checks (§2.1). | S | — |

**What can be codemodded:** the startup block; `run_until_closed(&app, ..)` (deleted); the theme lines; `window_chrome!`; atomic writers and `home()`s → Foundation calls; `say/settle` → `ui.set_notice` / `Refused`; `#[path]` test crates. **What cannot:** turning callbacks into commands and surfaces into intents is modelling work, done per app in its pass, which is also where the redesign happens, so it is not extra passes.

**Nightly safety:** nothing is renamed without a re-export for one release; a migrated app and an unmigrated app coexist in one nightly; the surface protocol and every sentence it refuses with are unchanged (the dispatch is not touched); `yos check <app>` and the dispatch vectors are the regression gate per app.

---

## 11. Trade-offs, and what I refuse to build

**Trade-offs made**

- **A render pass after every command.** Costs a function call and some property sets per command; buys the one path. Mitigated by `Changed` parts and refilling bound `VecModel`s. If a feature ever shows it in a profile, that feature renders finer parts; the pattern does not change.
- **Commands are `Send` plain data.** No `SharedString`-laden structs across the boundary; a view converts at the edge. More `.to_string()`; no thread-safety surprises.
- **Features take `&mut` the whole model.** Reviewers hold the "own one field, read siblings, change siblings by `fx.then`" rule; the type system does not. The alternative (lenses/generics per feature) doubles every signature.
- **mtime polling (2 s) for live settings and theme** over inotify: no new dependency, filesystem-agnostic; a change is seen within a rail tick.
- **Blocking jobs on a small pool** over async: matches the sync transport and the Slint thread model; the one future async case (streamed Mind tokens) is a later addition behind `spawn_local`, not a rewrite.
- **Buttons call commands directly; only the Mind, keys and menus go through intents.** A second `agent_is_calling()`-style audit for buttons is not needed because a button cannot be an agent.
- **The lib is compiled twice for unit tests.** Seconds per app; the price of deleting 14 `#[path]` crates.

**Refused**

- A `BaseApp` struct with `Box<dyn Fn>` "virtual" hooks, or any inheritance emulation: Rust has traits and the runner; a base struct would become the grab bag the rules forbid.
- A proc-macro DSL for intents or features (`#[intent(name = …)]`): hides the registry, breaks `yos check`'s reading of declarations, and the `fn`-pointer table is already one line per intent.
- Async everywhere (tokio in apps): two concurrency models in one process.
- A cached `describe`: it reads the live model on the UI thread, as `control.rs` insists; a cache is the stale account the surface exists to replace.
- A global `Error` type or a `utils`/`common` crate.
- Letting an agent call `present`, or widening the wire `ServiceError` with a data field for `kind` in this round.
- Hot-reloading `.slint`, a plugin system, or cross-process UI composition in 1.0.
- A codemod that turns callbacks into commands automatically: it would produce commands named after buttons, which is the thing we are trying to stop.
