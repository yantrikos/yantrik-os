# YantrikApp: the framework every app is built on

**Status:** proposed, 21 Oct 2026. Decisions for Pranab are marked ★ and listed in §10.
**Asked for:** "first we need to define a well-structured framework — BaseApp etc — then start working" (Pranab, 20 Oct), so that each app can then be made the best-looking of any OS and its code modular and easy to maintain.
**Built from:**
- a survey of all 18 apps' `main.rs` and `yantrik-app-runtime`;
- two independent designs: GPT-6 Astra ([review-gpt6-astra.md](app-framework/review-gpt6-astra.md)) and Fable, checked against the code ([review-fable.md](app-framework/review-fable.md)), both answering the same [brief](app-framework/brief.md).

This document is where they agree, and the choice made where they differ.

It sits under the accepted [platform SDK design](platform-sdk-2026-10-21.md). It is step 6 of that plan ("YantrikApp, `App::builder`"), designed before the apps are worked on, as asked.

## 1. Why

Every app rebuilds the same machinery by hand, differently each time:
- startup in 18 `main()`s;
- three ways of declaring the Mind's actions, with about 6 copies each of `act`/`arg`/`needed`/`refuse`;
- an "ask the Mind" rail copied into 8 apps;
- 7 atomic file writers, 7 `~` expanders and 8 private `home()`s;
- a headless test platform copied into 6 test files;
- three apps whose instance id and surface id disagree.

State is `Rc<RefCell<…>>` or `Arc<Mutex<…>>` reached through up to 33 weak-upgrade closures per file. The longest functions are 749, 687 and 627 lines.

The worst defect is structural. **The person's button and the Mind's action are two code paths.** Weather's `refresh` action presses the button instead of changing the model. Its `add_location` geocodes on the UI thread and says "added" before the lookup lands. A framework fixes this only if both are the same command, applied by the same function to the same model.

## 2. What an app is

**One process, one window, one model, one command type, one surface.** There is no `BaseApp` class. Rust has no inheritance, and a base struct of every service and hook becomes the grab bag the rules forbid. Instead:

- **The app implements one trait, `App`,** and the framework's runner drives it through a fixed lifecycle. Everything every app needs is done once, by the framework.
- **Model:** a plain Rust struct composed of feature models. The framework owns it, and code borrows it only inside a command. No `Rc<RefCell>` in app code.
- **Command:** a `Send` enum of plain data. Every change to the model is a command, whether it comes from a button, a key, a menu, a timer, a background job, a second launch or the Mind.
- **`update(model, command, effects) -> Result<(), Refused>`:** the only function that changes the model. It requests effects (jobs, timers, notifications, checkpoints) and does not perform them.
- **`render(ui, model, changed)`:** the only function that writes Slint properties. It runs after every command, for the parts that changed.
- **`describe(model)`:** the Mind's view, read from the same model after the same command.
- **Intents:** the commands the Mind, the keyboard and the menu may raise by name, each with its wire contract. **Every intent is a command; not every command is an intent.** Keystrokes, timer ticks and job results are commands only.

This is the Elm shape (model, update, effects, render). Both designs chose it independently.

### 2.1 The trait

```rust
// frameworks/yantrik-app/src/app.rs
pub trait App: Sized + 'static {
    type Ui: Window;            // the generated Slint root; `yantrik_app::window!(WeatherApp)` implements it
    type Model: 'static;        // plain Rust, composed of feature models, never holds a Slint handle
    type Command: Send + 'static;
    type Settings: Settings;    // typed, versioned, atomically written (§6)

    const MANIFEST: &'static str;   // include_str!("../yantrik.toml"): id, name, purpose, entitlements

    fn model(boot: &Boot<Self>) -> Result<Self::Model, Startup>;     // before any window: settings, drafts, launch
    fn wire(cx: &Cx<Self>);                                           // bind callbacks to commands, once
    fn intents(table: &mut Intents<Self>);                            // the surface, shortcuts and menu (§3)
    fn start(model: &Self::Model, fx: &mut Effects<Self>) {}          // first fetch, timers
    fn update(model: &mut Self::Model, command: Self::Command, fx: &mut Effects<Self>) -> Result<(), Refused>;
    fn render(ui: &Self::Ui, model: &Self::Model, changed: &Changed);
    fn describe(model: &Self::Model) -> View;
    fn event(event: Event, model: &Self::Model) -> Option<Self::Command> { None } // launch handoff, settings changed, activated, Mind reach
    fn close(model: &Self::Model) -> Close { Close::Now }             // `Later` keeps the window for "save changes?"
    fn stop(model: &mut Self::Model) {}                               // after the loop: final checkpoint
}
```

`main.rs` is the whole of this, and a guard test holds it there:

```rust
fn main() -> std::process::ExitCode { yantrik_app::run::<yantrik_weather::Weather>() }
```

### 2.2 The lifecycle the framework owns

The order is fixed.
1. **Manifest:** `yantrik.toml` gives one `AppId` for the instance claim, the socket, the `.desktop` surface id, tracing and paths. This ends the three mismatched ids.
2. **Boot:** `--version`, tracing and the panic hook. This replaces `init_tracing` ×18.
3. **Backend:** the `LIBGL_ALWAYS_SOFTWARE` switch to winit-software, once, before any component exists. This replaces 7 copies. For this reason the runner takes a window *constructor*, not a window: the accepted design's `App::builder().window(AppWindow::new()?)` would make the window too early.
4. **Launch:** files and flags parsed from argv.
5. **Single instance:** `instance::claim(id)`. If another copy holds it, the launch is handed to that copy (`present`, then the app's declared launch intent, each with a deadline), and this process exits. This replaces the two `flock`s and the seven 20 × 100 ms loops.
6. **Paths and settings:** `Paths::for_app`, `Store::<Settings>::open` (migrating), drafts, and `Services` built from the entitlements.
7. **`A::model(&boot)`.**
8. **Window:** make it, apply the theme, connect the chrome and the shortcuts.
9. **`A::wire(&cx)`.**
10. **`A::intents`:** `present` is added; declaration problems are fatal in debug builds.
11. **First `render`, then `start`.**
12. **Serve:** the surface goes up on `app-<id>.sock`. One housekeeping watcher follows the app's settings file, the shell's theme and the Mind's reach.
13. **Launch delivered** as `Event::Launch`.
14. **Event loop:** `run_until_closed` (a logout is not a crash). Close requests go to `A::close`.
15. **Shutdown:** stop timers, cancel jobs, drain the pool for at most 2 s, `A::stop`, flush checkpoints, release the instance last. The exit code says how the loop ended.

### 2.3 Slint's real constraints

Fable checked these against Slint 1.17:
- Slint generates the component and its globals (`ThemeMode`, `WindowChrome`, the rail's structs) **into each app's crate**. The framework cannot name them, so each app has one line, `yantrik_app::window!(WeatherApp)`, which implements `Window` at the app's site. This is the same shape as today's `window_chrome!`.
- Components are generic `A::Ui: ComponentHandle`, never trait objects.
- `Cx` holds an `Rc`, so it is `!Send` and cannot leave the UI thread. Jobs are never given the model or the window. The compiler enforces the thread split.
- `slint::Timer` is `!Send` and stops when dropped, so the runtime owns every timer. No `_binding` and no `mem::forget`.

## 3. Features, commands and intents

```
src/features/<name>/
    mod.rs       the facade: pub use model::{Model, Command}; PART
    model.rs     Model, Command, update(), pure rules. No Slint, no JSON, no I/O.
    view.rs      wire(cx): callbacks → commands; render(ui, &Model). The only file that imports slint.
    surface.rs   intents(table): this feature's actions for the Mind, keys and menu
    tests.rs     behavioural tests
```

- **Registration.** A feature is registered by appearing in the app's `Model`, its `Command` enum and the four calls in `lib.rs`. A missing match arm is a compile error, and that compile error is the registration check.
- **Ownership.** A feature owns one field of the model. It may read its siblings, and changes a sibling only by `fx.then(sibling::Command::…)`.
- **Extra files.** When a feature grows, the new files are named for a concept (`search.rs`, `draft.rs`), never `utils.rs`.
- **Enforcement.** `xtask check-architecture` gains these rules:
  - `model.rs` and `surface.rs` import no `slint`;
  - `view.rs` imports no `serde_json`;
  - `main.rs` is three lines.

**Callbacks without the boilerplate.**

```rust
ui.on_refresh_pressed(cx.cmd(|| Command::Refresh.into()));
ui.on_select_location(cx.cmd1(|row: i32| places::Command::Select(row as usize).into()));
```

These replace the 33 `let weak = …; let Some(ui) = weak.upgrade() else { return };` per file. A command raised while another is being applied is queued, never re-entered.

**Intents: a command plus its wire contract.** Each is declared once:

```rust
table.add(Intent::new(Action::new("add_location", "Add a place").param(Param::text("name")), |args| {
        Ok(places::Command::Add { name: args.text("name")?.to_owned() }.into())
    })
    .settles(Settle::AfterJob { within: secs(8) })   // reply only once the lookup has landed
    .reply(places::reply)
    .agents(Agents::Allowed)
    .key("Ctrl+L")
    .menu("Places/Add"));
```

- **Arguments** are checked against the declared parameters before `parse`, so handlers never parse JSON by hand. `Args::{text, integer, number, flag, one_of}`.
- **`Settle`** says when the Mind is answered:
  - `OnReturn`: when the command returns;
  - `Deferred`: the work has only started;
  - `AfterJob`: when the job the command started has landed (`answer_later` underneath).

  A slow action never runs on the UI thread, and it never answers early.
- **`Agents`** is `Allowed`, `Refused(sentence)` or `Rule(fn)`. The four `agent_rule.rs` files become `Rule`s, and app code no longer calls `agent_is_calling()`.
- **Policy.** The surface's existing dispatch stays unchanged and runs first: private mode, standing, ceiling, mode, approvals and grants, and the revision guard. The intent adds only the app's own rules after it.
- **Shortcuts and menus** are built from the same table, so a key, a menu item and the Mind raise the identical command.
- **Buttons** call `cx.run(command)` directly. They are typed Rust, and a button cannot be an agent.

## 4. State, threads, timers

**One rule: the model is read and changed only on the UI thread. Workers get owned inputs and return commands.**

```rust
impl<A: App> Effects<A> {
    pub fn then(&mut self, command: impl Into<A::Command>);
    pub fn spawn(&mut self, job: impl FnOnce(&Services, &Cancel) -> A::Command + Send + 'static) -> JobId;
    pub fn spawn_keyed(&mut self, key: &'static str, mode: Concurrency, job: impl FnOnce(&Services, &Cancel) -> A::Command + Send + 'static) -> JobId;
    pub fn cancel(&mut self, job: JobId);
    pub fn every(&mut self, id: TimerId, period: Duration, command: A::Command) where A::Command: Clone;
    pub fn after(&mut self, id: TimerId, delay: Duration, command: A::Command);   // re-arming restarts: a debounce is one line
    pub fn stop_timer(&mut self, id: TimerId);
    pub fn notify(&mut self, notification: Notification);
    pub fn checkpoint(&mut self, key: &'static str, bytes: Vec<u8>);
    pub fn report(&mut self, what: &'static str, error: &dyn std::error::Error);
    pub fn changed(&mut self, part: &'static str);
    pub fn close(&mut self);
}
pub enum Concurrency { Parallel, RejectIfRunning, ReplacePrevious }
```

- **Jobs** run on a small named pool (4 threads per app) with blocking typed clients and a `Cancel`. They return a command, which the runtime posts to the UI thread and applies through `update`.
- **`ReplacePrevious`** gives each job a generation. A new refresh cancels the old one, and the old one's late result is dropped rather than drawn. This is Astra's addition.
- **Timers** are owned by the runtime and identified by `TimerId`. Installing the same timer twice does not make two.
- **★ No async in apps.** Every service client in the workspace is blocking, and the surface keeps its private runtime inside `yantrik-surface`. Two concurrency models in one app is exactly the mix this removes. `slint::spawn_local` stays available for one future case, streamed Mind tokens.

  Astra proposed one async executor per process. Fable showed the transport and every client are synchronous, so we take the pool.

## 5. Services, the Mind, notifications, the rail

- **`yantrik-services` (L2)** holds one typed client per service domain (`WeatherClient`, `CalendarClient`, …).
  - Each method is blocking and has a deadline.
  - Errors are a typed `ServiceError` (unreachable, timeout, refused, wrong shape).
  - Each client implements an `*Api` trait, so tests swap in fakes.
  - Raw `SyncRpcClient::for_service` leaves app code.
- **`yantrik-mind` (L2)** is today's `companion.rs`, typed: `ask` (from a job only), `recall_relevant`, and `reach` (non-blocking, from the watcher). Its sentences are unchanged.
- **`yantrik-notifications` (L2)** is today's `notify.rs`. The app's name comes from the manifest. Notification buttons route back as commands.
- **The agent rail, once.** The rail is one framework feature with its model, view and timer. An app supplies `impl rail::Source for Model`:
  - `context()`;
  - `suggestions(reach)`;
  - `prompt(suggestion)`;
  - `apply(proposal)`.

  That is about 40 lines in place of 8 copies of `refresh_agent_rail`, eight 4 s timers and the 45-line ask handlers. An applied proposal is a command that keeps its agent provenance.
- **Caller identity** is the kernel's (`SO_PEERCRED` at accept). An app can neither claim nor forge it.

## 6. Settings, data, drafts, theme

**One path rule, for app id `<id>`:**

| Kind | Path | Holds |
|---|---|---|
| config | `$XDG_CONFIG_HOME/yantrik/<id>.json` | preferences |
| data | `$XDG_DATA_HOME/yantrik/<id>/` | the app's own data |
| state | `$XDG_STATE_HOME/yantrik/<id>/` | drafts and recovery |
| cache | `$XDG_CACHE_HOME/yantrik/<id>/` | disposable |

- `$YANTRIK_HOME` roots all four for tests and second profiles.
- ★ The per-app `$YANTRIK_<APP>_DIR` overrides warn for one release and are then removed.
- Existing files are imported once: backed up, never deleted.

**Typed settings.** `trait Settings: Serialize + DeserializeOwned + Default + Clone + PartialEq { const SCHEMA: u32; fn migrate(from, raw) }`. The file is `{"schema": n, "settings": {…}}`.
- **Writes:** temp file, fsync, rename.
- **A malformed file, or one newer than this app knows:** keeps the last good value in memory and writes a problem record. Nothing is overwritten with defaults.
- **A failed write** puts the in-memory value back and says why.
- **★ Live change:** one housekeeping thread checks file mtimes every 2 s and posts `Event::SettingsChanged`. This is polling, not inotify, because it needs no dependency and works on the 9p/SSHFS dev mounts. If faster is ever needed, inotify can go behind the same API.

**Data** stays the app's own files under `data/`, written with `yantrik_foundation::json_store` (the one atomic writer). **Drafts** go through `fx.checkpoint`, which is debounced and written off-thread. `stop` writes the last one. A draft found at start is handed to `model` through `Boot::drafts()`. Recovery is not "save everything at exit".

**★ Theme, live.** The theme stays in the shell's `settings.yaml`, read by `yantrik_design::Theme::current()`. The framework applies changes to every open app while it runs, without a restart. Today an app keeps the theme it started with until relaunched. Apps never write the theme.

## 7. Errors

- **Typed per module, with no global error type.** `thiserror` joins the workspace.
- **`Display` is the sentence the person reads,** in the writing rules' voice. The person and the Mind get the same sentence.
- **`Refused { kind: &'static str, sentence: String }`** crosses the wire. An app error implements `Explain { fn kind() }`, and `From` does the rest.
- **What `cx.run` does with a refusal:**
  - puts `sentence` on the notice line, replacing the 8 `settle`/`say` helpers;
  - logs `kind`;
  - writes a problem record when the kind begins `internal_`, because that is a bug, not a refusal.
- **What the surface returns:** the same sentence. `describe` carries `last_refusal`.
- **`describe` is a projection, not the model:**
  - each feature contributes a named section;
  - large domains page (Email cannot dump every message);
  - credentials, caches and private engine state are never in it.

## 8. Testing

Apps become **`lib + bin`**. The 9 `#[path]` test crates are deleted as each app moves, and tests that read `main.rs` as text become behavioural tests or architecture rules.

| Level | What | How |
|---|---|---|
| Model | rules, conversions, migrations | `update(&mut model, cmd, &mut Effects::recording())`, asserting on the requested effects as data. No Slint, no threads. Most tests are here. |
| App | intents, agents, settle, shutdown | `TestApp::<A>` (in `yantrik-testing`, dev-only): fake services and Mind, a temp `$YANTRIK_HOME`, jobs run inline, timers on a virtual clock. `act`/`act_as_agent`/`act_with_grant` go through the real surface registry with no socket; `describe` and `notice()` are asserted. |
| Pixels | every screen | `snapshot(name)` with the software renderer, both themes, 1.0 and 1.25 text scale. `UPDATE_SNAPSHOTS=1` rewrites. This replaces the 6 copied headless platforms. |

Every exported intent is tested for: good arguments, missing and wrong arguments, an unavailable state, an agent refused, an approval required and given, and a slow job settling.

## 9. Crates and migration

```
frameworks/
  yantrik-foundation    L0  + json_store (one atomic writer), home::expand, problems, place
  yantrik-app           L2  NEW  app, run, boot, cx, effects, jobs, timers, window!, chrome, instance,
                                 launch, event_loop, theme, paths, settings, drafts, refused, watch,
                                 intents/{intent,args,serve,shortcuts}, features/agent_rail
  yantrik-services      L2  NEW  one typed client per domain, fakes behind a feature
  yantrik-mind          L2  NEW  companion.rs, typed
  yantrik-notifications L2  NEW  notify.rs
  yantrik-testing       L2  NEW  dev-only: TestApp, Headless, FakeServices, FakeMind, snapshot
crates/
  yantrik-app-runtime   L2  DEPRECATED: re-exports for one release, then deleted
```

`yantrik-app` adds no Slint code generation and does not depend on the kit; it names the kit's globals only through `window!`. It does not help or hurt the shell's 14 GB compile, which is the process split in step 12.

**Migration. Every PR leaves every app building, and nightly never stops.**

| # | Lands | Size |
|---|---|---|
| 1 | `yantrik-app` created by **moving** instance, chrome, event loop, present, control and theme out of app-runtime unchanged; `place` and `problems` to Foundation; `json_store`; `thiserror`. app-runtime becomes re-exports. No app changes. | S |
| 2 | `App`, `Runner`, `Cx`, `Effects`, jobs, timers, `window!`, `Intents` and `Args`, `Paths`, `Store`, the watcher, `Refused`. `yantrik-testing`. Old APIs still work. | M |
| 3 | `yantrik-services`, `yantrik-mind`, `yantrik-notifications`; the rail as a feature; the `weather.forecast` method on the service. | M |
| 4 | **Pilot: Weather**, as one app pass: on the framework, lib + bin, screen moved into the app, kit-only (zero design-lint entries), redesigned to the Alt+Tab bar with 520 evidence, snapshots on. | M |
| 5 | **Prove persistence: Notes**: drafts, unsaved-close (`Close::Later`), a dangerous action with approval, launch handoff with a path. | L |
| 6 | **Calendar, then Email**: the two longest functions. | L each |
| 7 | The remaining 14, smallest first, one app pass each. | S–M ×14 |
| 8 | Delete `yantrik-app-runtime` and the `#[path]` crates; the architecture rules gain the feature checks. | S |

**Can be codemodded:** the startup block, theme lines, `window_chrome!`, `run_until_closed`, the atomic writers and `home()`s, `say`/`settle`, and the `#[path]` crates.

**Cannot be codemodded:** turning callbacks into commands and surfaces into intents. That is modelling work, done in each app's pass, alongside its redesign.

## 10. Decisions for Pranab ★

1. **Adopt this app model:** an `App` trait and a framework-owned runner (the equivalent of BaseApp, by composition, not inheritance), Elm-style model/update/render, and buttons and the Mind on the same commands.
2. **No async runtime in apps:** a small job pool returning commands. Async only later, for streamed Mind tokens.
3. **One path rule for app files,** with existing files imported once and backed up. The `$YANTRIK_<APP>_DIR` overrides go after one release.
4. **Live theme:** apps follow theme changes while open, without a relaunch.
5. **The order:** framework steps 1–3, then Weather as the pilot (framework, code split and redesign together), then Notes to prove drafts and approvals, then Calendar, Email and the rest.

## What we refuse to build

- A `BaseApp` struct with virtual-hook fields, or any inheritance emulation.
- A proc-macro DSL for features or intents.
- A string-topic event bus.
- A global `Arc<Mutex<AppState>>`.
- A thread or runtime per feature.
- A surface handler that skips the dispatch's policy.
- Agent detection from a caller-supplied flag.
- A cached or whole-model `describe`.
- One generated UI crate for all apps.
- A codemod that turns callbacks into commands, which would name commands after buttons.
- A framework that "solves" modularity by moving a 2,000-line `main` into a 2,000-line `App::wire`.

**The real test** is not that Weather's `main.rs` is three lines; that is easy. It is that the twentieth app gets these by adopting the contract, and still reads as a small Rust program anyone can follow:
- launch handoff;
- a live theme;
- typed actions;
- agent approval;
- owned jobs and timers;
- recovery;
- notices;
- tests.
