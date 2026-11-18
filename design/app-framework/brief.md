# Design YantrikApp: the framework every Yantrik OS app is built on

You are the architect who led the Cocoa/AppKit application layer at Apple, or the Windows App SDK app model. The owner of Yantrik OS asked, in his words: "first we need to define a well-structured framework — BaseApp etc — then start working [on each app]". He wants each app's UI to become the best-looking of any OS. He also wants each app's code to be "modular and easy to maintain, not a huge chunk of file, classes, inheritance, functions".

Design the app framework that will hold for the next 20 apps. Be concrete and opinionated: give real Rust signatures, the module layout, and a worked example app.

## The platform (facts)
- Rust with Slint 1.17 UI (`.slint` markup compiled to Rust) on Linux (Debian 13, Wayland, labwc).
- Each app is its own process and binary under `apps/<name>`. It talks to the shell and to services over a typed Unix-socket IPC, and caller identity comes from kernel peer credentials.
- The **control surface** is a defining feature. Every app publishes `describe` (its state as JSON for the Mind, the OS's AI companion) and `act` (typed actions). The Mind and other agents can therefore operate any app, under approval rules: some actions are standard, some dangerous, and an agent calling is detectable.
- The platform SDK design has already been accepted:
  - **Folders:** `frameworks/` (public) and `crates/` (internal).
  - **Layers:** L0 foundation, L1 design, L2 platform, L3 OS services, L4 shell, L5 minds. Apps may use L0–L2 only.
  - **Frameworks that exist:**
    - `yantrik-foundation` (L0: `time::ago` and the like),
    - `yantrik-design-tokens` and `yantrik-ui-kit` (L1: Theme tokens and ~40 Slint components, with a design lint that fails new raw colours, px fonts, private copies of kit controls and std-widgets),
    - `yantrik-app-build` (how a UI is compiled),
    - `yantrik-app-runtime` (L2, today's grab-bag runtime, listed below),
    - `yantrik-surface` (the control-surface protocol),
    - `yantrik-ipc-contracts` / `-transport`.
  - **Planned:** a `yantrik-sdk` facade, apps as `lib + bin`, and `features/<name>/{mod,model,view,surface,tests}.rs` per feature.
- **Rules every PR is held to:**
  - one home per concept;
  - files at most 600 lines soft and 1,200 hard;
  - `main.rs` at most 40 lines for an app;
  - no logic in `.slint` beyond layout and visual state;
  - callbacks dispatch to the model and return;
  - typed errors (`thiserror`), with sentences only at the wire;
  - `pub(crate)` facades;
  - no `utils`/`common`/`manager` grab bags;
  - behavioural tests.
- A 48 GB build host. Slint codegen is the expensive part, and the shell's UI crate alone takes about 14 GB.

## What the 18 apps do today (survey of every main.rs, 20 Oct 2026)
**Startup, repeated by hand in each `main()`:**
- `init_tracing(name)`;
- a 5-line `LIBGL_ALWAYS_SOFTWARE` → winit-software backend switch (7 apps);
- argv path parsing (7);
- `instance::claim(name)` (16 apps). Two others hand-roll `flock` locks;
- a 20 × 100 ms loop that hands a launch argument to the running copy through `act` (7 apps, each different);
- `XApp::new().unwrap()` and `window_chrome!(app)` (18);
- a 3-line theme load that sets the `ThemeMode`/`AccentPreset` globals (18, identical, never live-reloaded);
- building state, `wire()`, and a first refresh;
- publishing the control surface, either in `main` or inside `wire` (both seen);
- companion "ask" handlers of about 45 lines (3 nearly identical copies in `main`, 4 in `wire`);
- timers created in `main` or returned from `wire` and kept alive by a `_binding`;
- `run_until_closed(&app, name)`, whose bool every app ignores;
- shutdown in 8 apps: stop timers, checkpoint drafts, join worker threads, shut engines down.

**Runtime API apps use (yantrik-app-runtime):**
- `init_tracing`;
- `instance::{claim -> Option<InstanceGuard>, present_running}`;
- `theme::load() -> ThemeSettings{dark, accent_index}`;
- `window_chrome!` (needs a `WindowChrome` Slint global);
- `control::App::new(id).describe(Fn()->View).state_rule(..).action(Action, Fn(&Value)->Result<Value,String>).serve()` (adds `present`), plus `agent_is_calling()`, `caller()`, `answer_later`;
- `companion::{ask, recall, reach()->Reach}`;
- `notify::send(Notification::new(..))`;
- `service::{client(id), ensure, is_up}`;
- `place::home()`;
- `problems::record_failure`;
- `run_until_closed`.

Apps re-implement several of these by hand anyway: 8 private `home()`s, raw `SyncRpcClient::for_service` instead of `service::client`, and hand-added `show` actions that overlap `present`.

**State and callbacks:**
- **State:** mostly `type State = Rc<RefCell<X>>`; some `Arc<Mutex<..>>`; cloneable engines.
- **Callbacks:** every `on_*` closure repeats `let weak = app.as_weak(); … let Some(ui) = weak.upgrade() else { return };`, up to 33 times per main.rs.
- **Background work:** `thread::spawn` + `upgrade_in_event_loop`. Two apps run `mpsc` Job/Event worker threads.
- **Largest functions:** email `wire` 749 lines, notes `surface` 687, calendar `publish_control` 627, text-editor `surface` 608, then several of 380–480 lines.
- **Largest main.rs files:** email 2,689, calendar 2,271, text-editor 1,988, notes 1,904.

**The control surface, declared three ways:**
1. One long builder chain where every action re-clones a `ui_for` closure and the state, and parses `args["k"].as_str().unwrap_or_default()` by hand (11 apps).
2. A `surface() -> Vec<(Action, Handler)>` with copied helpers `act/arg/needed/given/refuse` (6 apps, about 6 copies of each).
3. A table of `fn(&mut Session, &Json)` with a central rule check (spreadsheet).

Apps have 3–28 actions each. Agent path rules live in a per-app `agent_rule.rs` (4 apps) built on `agent_is_calling`.

**Persistence:**
- Each app picks its own path: `~/.local/share/yantrik/<app>`, `$XDG_STATE_HOME/yantrik/<app>/draft.json`, `$XDG_CONFIG_HOME/yantrik/<app>.json` or `~/.config/yantrik/weather.json`, some with `$YANTRIK_<APP>_DIR` overrides.
- The formats are all JSON.
- 7 copies of an atomic file write and 7 copies of `~/` expansion.

**Copied UI glue:**
- An "agent rail" (a side panel of Mind suggestions, with an ask box and a proposal card) in 8 apps: the same `refresh_agent_rail`, a 4 s timer and an ask/proposal flow.
- 8 copies of `settle(ui, result)`/`say` helpers that write a result to a notice.
- A headless Slint test platform copied into 6 test files.

**Tests:**
- Apps are binary-only, so 9 `tests/*-core` crates `#[path]`-include app sources to test them.
- Several tests read `main.rs` as text.

**Inconsistent ids:** the claim id and the control-surface id differ for 3 apps.

## What I want back
1. **The app model.** What an app *is* in code. In Rust there is no inheritance, so give the equivalent of BaseApp:
   - a trait (or traits) an app implements;
   - a builder/runner the framework owns;
   - what the framework does for every app (startup, single instance, launch handoff, backend choice, theme including live reload, window chrome, control surface, shutdown, crash/problem reporting);
   - the exact lifecycle with hooks.

   Give the full public API with Rust signatures, and the `main.rs` an app ends up with (target ≤10 lines).
2. **Features inside an app.** The `features/<name>/` pattern made concrete:
   - how a feature declares its model (state and pure rules), its view binding (the only code that touches Slint), its commands, and its surface actions;
   - how features are registered with the app;
   - how they share state without `Rc<RefCell>` soup;
   - how a callback reaches the model without the weak-upgrade boilerplate.

   Say whether commands and surface actions are one thing. One declaration could serve a button, a keyboard shortcut, a menu item and the Mind's `act`, with typed arguments parsed once and the agent/approval rules in one place.
3. **State, threads and async.** One pattern for UI-thread state, background work (thread pool, async, or both), results back to the UI, timers with an owner, and cancellation on shutdown. Explain it with Slint's single-threaded model in mind.
4. **Services, Mind and notifications** as typed clients through the framework, with timeouts, errors and caller identity. Include the agent rail as one shared feature, not 8 copies.
5. **Settings and storage.**
   - A typed per-app settings store with one path rule, migrations, atomic writes and live change notification.
   - App data and drafts or recovery.
   - Where the theme lives.
6. **Errors and logging.** Typed errors and how they become a sentence for the person and an error for the Mind.
7. **Testing.** Apps become `lib + bin`. Cover:
   - a headless test harness;
   - fake services and Mind;
   - a control-surface harness: call `act`, assert on `describe`;
   - snapshot/pixel tests;
   - what replaces the `#[path]` crates.
8. **Crate layout.** Which crates hold this (`yantrik-app`? split from `yantrik-app-runtime`?), what moves where, and what is deprecated. Respect the layers.
9. **A worked example.** A small but real app (Weather: current conditions plus a 7-day forecast from weather-service, a location search, a refresh, 4 Mind actions, the agent rail). Show it as the full file tree with each file's contents sketched, demonstrating every piece above. Keep every file under 300 lines.
10. **Migration.** How the 18 apps move onto this one at a time without breaking nightly. Cover what lands first, the pilot, and what can be codemodded.

Name the trade-offs you are making, and what you would refuse to build.
