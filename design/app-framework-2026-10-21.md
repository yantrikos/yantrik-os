# YantrikApp: the framework every app is built on

**Status:** accepted, 21 Oct 2026 (revision 3). GPT-6 Astra's verdict on this revision with the adapter evidence: APPROVE ([review-gpt6-astra-approve.md](app-framework/review-gpt6-astra-approve.md); its sign-off pass is [review-gpt6-astra-signoff.md](app-framework/review-gpt6-astra-signoff.md)). The decisions marked ★ (§12) were taken by Y under the delegation Pranab gave on 20 Oct ("putting you in the driving seat"); any of them is his to overturn.

**Asked for:** "first we need to define a well-structured framework — BaseApp etc — then start working" (Pranab, 20 Oct). The goal is that each app can then be made the best-looking of any OS, with code that is modular and easy to maintain.

**Built from:**
- a survey of all 18 apps' `main.rs` and of `yantrik-app-runtime`;
- two independent designs answering the same [brief](app-framework/brief.md): GPT-6 Astra ([review-gpt6-astra.md](app-framework/review-gpt6-astra.md)) and Fable, which was checked against the code ([review-fable.md](app-framework/review-fable.md));
- Astra's review of revision 1 ([review-gpt6-astra-final.md](app-framework/review-gpt6-astra-final.md)), verdict REVISE, and its confirmation review of revision 2 ([review-gpt6-astra-confirm.md](app-framework/review-gpt6-astra-confirm.md)). This revision makes their contract repairs B1–B10, adopts their "decide now" list (§10) and gated migration (§11), and applies the confirmation review's replacement text word for word. The one item text cannot settle, the two-crate build of the window adapter, stays a gate (§2.3).

This design sits under the accepted [platform SDK design](platform-sdk-2026-10-21.md) and is its step 6, done before the apps are worked on, as asked.

## 1. Why

Every app rebuilds the same machinery by hand, differently each time:
- startup, written into 18 separate `main()` functions;
- the Mind's actions, declared three different ways, with about 6 copies each of `act`/`arg`/`needed`/`refuse`;
- an "ask the Mind" rail copied into 8 apps;
- 7 atomic file writers, 7 `~` expanders and 8 private `home()` functions;
- a headless test platform copied into 6 test files;
- three apps whose instance id and surface id disagree.

State lives in `Rc<RefCell<…>>` or `Arc<Mutex<…>>`, reached through up to 33 weak-upgrade closures per file. The longest functions run 749, 687 and 627 lines.

The worst defect is structural: **the person's button and the Mind's action are two code paths.** Weather's `refresh` action presses the button instead of changing the model. Its `add_location` geocodes on the UI thread and says "added" before the lookup lands. A framework can fix this only by making both paths the same command, applied by the same function to the same model.

## 2. What an app is

**One process per app and profile, one model, one command type, one surface, and zero or more windows.** Windows have stable `WindowId`s and draw the app's state. Closing a window is not the same as ending the app. The first implementation supports one primary window, but nothing in command routing, job ownership or the lifecycle assumes it is the only one, because Notes and Email will need document and composition windows. (B1)

There is no `BaseApp` class. Rust has no inheritance, and a base struct holding every service and hook would become the grab bag our rules forbid. Instead:
- The app implements one trait, `App`.
- The framework's runner drives it through a fixed lifecycle. Everything every app needs is done once, by the framework.
- **Model:** a plain Rust struct composed of feature models. The framework owns it, and code borrows it only while applying a command. App code has no `Rc<RefCell>`.
- **Command:** a `Send` enum of plain data. Every change to the model is a command, wherever it came from: a button, a key, a menu, a timer, a job result, a second launch, a window event or the Mind.
- **`update(model, command, effects) -> Result<(), Refused>`:** the only function that changes the model. It *requests* effects and performs none.
- **`render(ui, model, changed)`:** the only function that writes Slint properties.
- **`describe(model)`:** the Mind's view, a projection of the same committed model.
- **Intents:** the commands the Mind, keys and menus may raise by name, each with its wire contract. **Every intent is a command; not every command is an intent.**

This is the Elm shape (model, update, effects, render), which both designs chose independently.

### 2.1 The trait

```rust
// frameworks/yantrik-app/src/app.rs
pub trait App: Sized + 'static {
    type Ui: Window;            // the primary window's generated component; `yantrik_app::window!(WeatherApp)`
    type Model: 'static;        // plain Rust, never holds a Slint handle or a list model
    type Command: Send + 'static;
    type Settings: Settings;

    const MANIFEST: &'static str;   // include_str!("../yantrik.toml"): id, name, purpose, entitlements, lifetime
    const LIFETIME: Lifetime = Lifetime::WhileWindowsOpen;

    fn model(boot: &Boot<Self>) -> Result<Self::Model, Startup>;      // settings, drafts; reads the launch, does not apply it
    fn wire(cx: &Cx<Self>, window: WindowId);                          // bind one window's callbacks to commands
    fn intents(table: &mut Intents<Self>);
    fn start(model: &Self::Model, fx: &mut Effects<Self>) {}
    fn update(model: &mut Self::Model, command: Self::Command, fx: &mut Effects<Self>) -> Result<(), Refused>;
    fn render(ui: &Self::Ui, window: WindowId, model: &Self::Model, changed: &Changed);
    fn describe(model: &Self::Model, page: &Page) -> View;
    fn event(event: Event, model: &Self::Model) -> Option<Self::Command>;
}

pub enum Lifetime { WhileWindowsOpen, ExplicitExit }
pub enum Event {
    Launch(Launch),                          // the first launch and every handed-over one, applied only here
    CloseRequested(WindowId),
    ExitRequested(ExitReason),               // quit, logout, shutdown
    SettingsCommitted(Revision), SettingsConflict, SettingsChangedOutside,
    Activated(WindowId),
    Presented(WindowId),                     // a second launch showed this window (or reopened it during a linger)
    ReachChanged(Reach),
}
```

Rules that differ from revision 1:
- **Close and exit (B8).** These are events that become commands. `update` sets any confirmation state ("Save changes?") and requests the save. A successful save then requests `fx.close(window)` or `fx.exit()`. Cancelling leaves the window open.
- **Final checkpoints** are requested while commands are still processed. After the event loop ends, cleanup touches nothing in the model. There is no `stop(&mut Model)`.
- **Logout** gets a bounded save window and is never treated as a completed interactive save.
- **`main.rs` is three lines,** held by a guard test:

  ```rust
  fn main() -> std::process::ExitCode { yantrik_app::run::<yantrik_weather::Weather>() }
  ```

### 2.2 The lifecycle the framework owns

1. **Manifest and intent declarations are validated in every build,** not only in debug builds. A failure is a startup sentence and exit 1. (B10)
2. **Boot:** `--version`, tracing and the panic hook.
3. **Profile:** resolved from `$YANTRIK_HOME`, the default profile otherwise. The instance claim and the socket are keyed by `(AppId, ProfileId)`, so a second profile is a second app instance, not a contender for the first. (B10)
4. **Single instance.** If another copy holds the claim, the launch goes to it as a versioned `Launch` with a request id. The owner acknowledges and de-duplicates by id, so a retry never opens a document twice. This process then exits. If the owner dies mid-handoff, the contender re-checks and may become the owner. This replaces the two `flock`s and the seven 20 × 100 ms loops.
5. **Execution mode.** The initial window set is resolved independently of `Lifetime`. With no windows, the runner constructs no component and selects no graphical backend; it uses a backend-independent command pump and wake mechanism. Otherwise the graphical backend (the `LIBGL_ALWAYS_SOFTWARE` switch to winit-software, once) is initialised before the first window is constructed, which is why the runner takes a window *constructor*.
6. **Paths, settings and services:** `Paths::for_app`, `Store::<Settings>::open` (migrating or entering recovery, §6), drafts, and `Services` from the entitlements.
7. **`A::model(&boot)`.**
8. **Initial windows:** construct only the requested windows; apply theme, chrome and shortcuts, wire callbacks (`A::wire(cx, id)`), and render each. An empty initial window set is valid.
9. **`A::intents`;** `present` is added. **`A::start`.**
10. **Command processing starts.** Only then is the surface published as ready on `app-<id>[-<profile>].sock`. A request that arrives earlier waits and is never admitted into a runner that cannot apply it. (B10)
11. **The launch is applied once, as `Event::Launch`.** `Boot` only reads it.
12. **Event loop.** Logout is not a crash.
13. **Exit,** once the last window closes (`WhileWindowsOpen`) or `fx.exit()` runs:
    - reject new external work;
    - complete or cancel outstanding invocations with defined outcomes;
    - stop timers and cancel jobs;
    - flush the persistence lane only within the shutdown grace deadline. At expiry the process exits without an unbounded join or flush; unfinished writes are not reported as durable, and recovery uses the last durably committed revision;
    - unwind the socket and claim.

    The command that closes the last window may ask to **linger** instead (`fx.linger(backstop)`, capped at 60 s), so work still in flight can land and its outcome be known. The process then runs with no window until `fx.exit()` or the backstop, which ends it and logs. A second launch reopens the first window, drawn whole, and raises `Event::Presented`, where the app cancels its exit. A logout still ends it within the signal deadline.

    A failure anywhere in startup unwinds what it took.

### 2.3 The UI-thread bridge and the Slint adapter (B2)

- `Window: slint::ComponentHandle + 'static`. The runtime keeps the app's state UI-thread-local.
- Workers receive owned inputs and a **bounded, `Send` command endpoint**.
- A wake closure passed to `invoke_from_event_loop` captures only `Send` routing data (`AppId`, the sequence and the command) and looks up the app's state on the UI thread.
- No worker captures `Cx`, a component, a timer or the model. Posting after shutdown fails harmlessly.
- Slint generates the components, globals and rail structs into **each app's crate**. `window!` expands there and adapts those generated types to framework-owned plain Rust types (theme, chrome, shortcuts, notice, rail). This adapter contract is explicit: a window must export the kit's `ThemeMode`, `AccentPreset`, `WindowChrome`, `Shortcuts`, `Notice` and, if it has a rail, the rail's types.
- **UI adapters own retained Slint list models** (`VecModel` and friends). These never live in the app's model.
- **Approval gate:** the adapter design remains provisional until passing release builds from two separate generated UI crates are attached as evidence (§11 step 2). Neither production adoption nor adapter approval precedes that proof. "One line per app" is the goal; the two-crate build is the proof.
- **Evidence (21 Oct 2026):** `yantrik_app::Window` and `window!` exist (`frameworks/yantrik-app/src/window.rs`, commit `1c271c86` on `arch/step6a-yantrik-app`). Weather and Notes, two separate generated UI crates, implement it with `yantrik_app::window!(WeatherApp)` and `yantrik_app::window!(NotesApp)`, and `cargo build --release -p yantrik-weather -p yantrik-notes` finished on the build host (rustc 1.92.0, x86_64 Linux) in 2 m 19 s with no error. Their tests pass (Weather 6 and 9, Notes 5, `yantrik-app` 55). The log is attached to the design's pull request. The gate is passed; the adapter is no longer provisional.

## 3. Features, commands and intents

```
src/features/<name>/
    mod.rs       facade: pub use model::{Model, Command}; PART
    model.rs     Model, Command, update(), pure rules. No Slint, no JSON, no I/O.
    view.rs      wire(cx, window): callbacks → commands; render(ui, &Model); the list adapters. The only file importing slint.
    surface.rs   intents(table)
    tests.rs     behavioural tests
```

- **Ownership.** A feature owns one field of the model. It may read sibling fields, and changes a sibling only through `fx.then(sibling::Command::…)`, which is a separate, fallible command and not a transaction. A change that must be atomic across features is made in one `update`.
- **Naming.** Extra files are named for a concept (`search.rs`, `draft.rs`), never `utils.rs`.
- **Registration.** A missing match arm is a compile error. A missing `wire`, `render` or intent call is not, so `TestApp` checks that each feature's intents are declared and that each part renders (§9).
- **Enforcement.** `xtask check-architecture` gains these checks **with the first migrated feature**, not at final cleanup:
  - `model.rs` and `surface.rs` import no `slint`;
  - `view.rs` imports no `serde_json`;
  - `main.rs` is three lines.

**Commits (B3).** Commands validate before they change anything. A `Refused` leaves the model unchanged, and this is tested for every intent. Effects are staged and committed only when `update` returns `Ok`. The framework does not roll back arbitrary Rust mutations, which is exactly why validating first is the rule. Every successful command advances the model **revision**. `render` and `describe` both observe that same committed revision. `Changed` defaults to every part unless an `update` narrows it.

**Callbacks.**

```rust
ui.on_refresh_pressed(cx.cmd(|| Command::Refresh.into()));
ui.on_select_place(cx.cmd1(|id: SharedString| {
    Ok(places::Command::Select(PlaceId::parse(&id)?).into())
}));
```

`cmd` and `cmd1` accept infallible converters and fallible ones returning `Result<A::Command, Refused>`.

- These replace the 33 weak-upgrade closures per file.
- Lists are addressed by **stable item ids**, never by row index, which is not a durable command target (B9).
- A conversion that fails is a refusal, not a panic.
- A command raised while another is being applied is queued, never re-entered.

### 3.1 Intents: a command plus its wire contract

```rust
table.add(Intent::new(Action::new("add_location", "Add a place").param(Param::text("name")), |args| {
        Ok(places::Command::Add { name: args.text("name")?.to_owned() }.into())
    })
    .settles(Settle::Explicit { within: secs(8) })
    .agents(Agents::Allowed));
table.local(places::Command::FocusSearch.into()).key("Ctrl+L").menu("Places/Add…");   // collects the name first
```

- **Arguments** are checked against the declared parameters before `parse`. Handlers never parse JSON by hand: `Args::{text, integer, number, flag, one_of}`.
- **Parameterised intents get their arguments locally (B9).** A shortcut or menu item either raises a complete command, or starts the workflow that collects the arguments: Ctrl+L focuses the search, and submitting it raises `Add { name }`. Shortcuts respect the focused window and editable fields, and follow platform conventions. Availability is shared with the UI (a disabled menu item, a greyed button); authorisation is still checked when the command runs.
- **Settlement (B4).** Each accepted Mind or surface call gets an **`InvocationId`**:
  - `Settle::OnReturn` replies once the command commits.
  - `Settle::Deferred` replies at once with an operation id the Mind can query later.
  - `Settle::Explicit { within }` replies when the invocation reaches **exactly one terminal outcome**, which the app reports with `fx.settle(invocation, result)` after the result command commits.

  Jobs, `then` chains and continuations carry the invocation id. Timeout, cancellation, replacement, worker panic and shutdown each produce a defined terminal outcome. A deadline never claims that an external operation was undone. The reply describes *that invocation's* committed result, not whatever happens to be selected when the reply is built.
- **Agents.** Each intent declares `Agents::Allowed`, `Refused(sentence)` or `Rule(fn)`. The four `agent_rule.rs` files become `Rule`s, and app code stops calling `agent_is_calling()`.

### 3.2 Authorisation and provenance (B5)

- **The surface's existing dispatch stays the external admission authority:** private mode, standing, ceiling, mode, approvals and grants.
- **Invocation context.** Dispatch issues an internal, unforgeable `Invocation` holding the authenticated principal, the origin, the authorised operation and arguments, and the grant and revision constraints.
- **Checks at dequeue.** When the command is taken off the queue on the UI thread, the runner checks the revision and the intent's state-dependent rules before `update`, as one atomic step. An approval is bound to the approved operation and arguments. Changed arguments, or a condition that no longer holds, need fresh authorisation.
- **Provenance travels** with jobs, continuations and rail proposals. An agent's proposal re-enters through the authorised intent path, never through the local-input path. Buttons call `cx.run` directly only for genuinely local UI events. Validation applies to every origin.
- **Identity.** Peer credentials provide transport evidence, including PID, UID and GID on Linux; they do not by themselves authenticate an application identity, including among processes sharing a UID. App identity and grants follow the platform's authenticated identity mechanism, and a flag a caller sets is never authoritative. Socket ownership, permissions, peer validation and request limits are mandatory. Manifest entitlements configure clients; they are not a security boundary.

## 4. Jobs, queues, timers

**The rule:** the model is read and changed only on the UI thread. Workers get owned inputs and return commands.

```rust
impl<A: App> Effects<A> {
    pub fn then(&mut self, command: impl Into<A::Command>);
    pub fn spawn(&mut self, key: JobKey, mode: Concurrency, job: impl FnOnce(&Services, &Cancel) -> A::Command + Send + 'static) -> JobId;
    pub fn cancel(&mut self, job: JobId);
    pub fn every(&mut self, id: TimerId, period: Duration, command: A::Command) where A::Command: Clone;
    pub fn after(&mut self, id: TimerId, delay: Duration, command: A::Command);   // re-arming restarts: a debounce is one line
    pub fn stop_timer(&mut self, id: TimerId);
    pub fn persist(&mut self, key: PersistKey, revision: Revision, bytes: Vec<u8>); // the ordered lane, §6
    pub fn notify(&mut self, notification: Notification);
    pub fn settle(&mut self, invocation: InvocationId, result: Result<serde_json::Value, Refused>);
    pub fn desktop(&mut self, request: DesktopRequest);                              // pickers, clipboard, open-URL, §10
    pub fn report(&mut self, what: &'static str, error: &dyn std::error::Error);
    pub fn changed(&mut self, part: &'static str);
    pub fn close(&mut self, window: WindowId);
    pub fn exit(&mut self);
}
pub struct JobKey { pub feature: &'static str, pub scope: Scope }   // Scope::App | Window(WindowId) | Entity(String)
pub enum Concurrency { Parallel, RejectIfRunning, ReplacePrevious }
```

- **★ No async runtime in apps.** Jobs run on a small named pool of 4 threads per app, with blocking typed clients that have finite deadlines, plus a cooperative `Cancel`. Every service client in the workspace is blocking. Async is reserved for a later need, such as a bounded stream of Mind tokens arriving as commands.
- **Bounded admission.** The command and job queues are bounded; when full, a call gets an overload refusal ("busy, try again") instead of growing memory.
  - Execution capacity is reserved before applying a command whose acceptance depends on it. A capacity rejection leaves the model unchanged.
  - Failures discovered after commit are operation failures, not command refusals.
  - Job completion, cancellation and terminal settlement use reserved, bounded control capacity, so saturation cannot lose an accepted invocation's terminal outcome.
  - The UI thread never blocks waiting for queue space.
  - A worker panic is caught: its job reaches a terminal outcome, and the app keeps running.
  - The UI thread drains commands fairly, in batches, between frames, so one flood cannot freeze drawing.
- **Keyed.** Jobs are scoped by feature and by window or entity, never a global `"refresh"`. `ReplacePrevious` gives each job a generation: a new refresh cancels the old one, and the old result is rejected, not drawn.
- **Shutdown (B6).** Rust cannot kill a blocking thread safely, so the framework does not pretend to:
  - new work is rejected first;
  - jobs are cancelled cooperatively;
  - after a 2 s grace period, late results are dropped and the process exits;
  - nothing claims an unresponsive worker was killed or rolled back.

  Workers never write settings or checkpoints directly. Those go through the ordered persistence lane.
- **Timers** are owned by the runtime and identified by `TimerId`. Arming the same id twice does not make two.

## 5. Services, the Mind, notifications, the rail

- **`yantrik-services` (L2).** One typed client per service domain. Each is blocking with a deadline per method, and returns a typed `ServiceError` (unreachable, timeout, refused, wrong shape). Each implements an `*Api` trait, so tests use fakes. App code stops using raw `SyncRpcClient::for_service`.
- **`yantrik-mind` (L2)** is today's `companion.rs`, typed:
  - `ask` runs from a job only;
  - `recall_relevant` is as today;
  - `reach` is non-blocking, fed by the watcher.

  Its sentences are unchanged.
- **`yantrik-notifications` (L2)** is today's `notify.rs`. The app's name comes from the manifest, and notification buttons come back as commands.
- **The agent rail, once.** It is one framework feature with its own model, view adapter and timer. An app supplies `impl rail::Source for Model` with four methods: `context()`, `suggestions(reach)`, `prompt(suggestion)` and `apply(proposal)`. That is about 40 lines, in place of 8 copies of `refresh_agent_rail`, eight 4 s timers and the 45-line ask handlers. Applying a proposal keeps its agent provenance and goes through the authorised intent path (§3.2). Suggestions refresh on a model revision change, debounced, with the 4 s tick kept as a fallback.

## 6. Settings, data, drafts, theme

**One path rule, for app `<id>` in profile `<p>`:**

| Kind | Path | Holds |
|---|---|---|
| config | `$XDG_CONFIG_HOME/yantrik/apps/<id>/settings.json` | preferences |
| data | `$XDG_DATA_HOME/yantrik/<id>/` | the app's own data |
| state | `$XDG_STATE_HOME/yantrik/<id>/` | drafts and recovery |
| cache | `$XDG_CACHE_HOME/yantrik/<id>/` | disposable |

- `$YANTRIK_HOME` roots all four for a second profile. Tests inject a `Paths` value instead of changing process environment, which parallel tests cannot share safely.
- ★ The per-app `$YANTRIK_<APP>_DIR` overrides warn for one release and are then removed.
- Existing files are imported once, backed up, never deleted.
- The settings live in `yantrik/apps/`, a directory only the framework writes. The first rule, `yantrik/<id>.json`, put them beside the services' own files, and Email's were email-service's account file, which holds the account password. Each file carries a mark (`"kind": "yantrik-app-settings"`, `"app": "<id>"`), and the store never reads past the mark of a file without it, never writes over one, and logs only its path. An app's settings at the old place come across once, only if they are the framework's envelope or the app's own shape from before (`Settings::FORMER_FILE`, `Settings::legacy`), and the old file stays where it was as the backup. `paths/others.rs` lists what every other component keeps under the four roots, and a test holds every app id clear of it.

**Ownership.** The settings store owns the *committed* value. The model holds the value the app is using. Changes go through commands, and write results and outside changes come back as events.

```rust
pub trait Settings: Serialize + DeserializeOwned + Default + Clone + PartialEq + 'static {
    const SCHEMA: u32;
    fn migrate(from: u32, raw: serde_json::Value) -> Result<serde_json::Value, settings::Error>;
}
```

**Persistence (B7).**
- **The ordered lane.** Writes for one destination are serialised and tagged with monotonic revisions. A completion or failure affects only its own revision; an older result never replaces newer state.
- **A failed write** keeps the pending edits, says so ("Not saved: …"), and retries on the next change. It never rolls the model back.
- **Atomic replace:** a temporary file in the same directory, restrictive permissions, file sync, rename, then a sync of the parent directory where supported.
- **A corrupt file, or one from a newer schema,** enters protected recovery. The app runs on defaults or the last good value, and an ordinary save cannot overwrite that file until the person chooses to.
- **Outside changes.** The housekeeping thread polls every 2 s. It fingerprints file contents as well as identity and metadata; content checks occur independently of metadata changes. It handles replacement and deletion and suppresses self-write echoes by committed fingerprint. An outside edit that conflicts with a pending local edit raises `SettingsConflict`; it is never silently last-writer-wins. (★ polling rather than inotify: no new dependency, and it works on the 9p and SSHFS dev mounts.)

**App data and drafts** use the same lane and atomic writer (`yantrik_foundation::json_store`). Apps own application-data and draft schemas and migrations; the framework owns ordered durable replacement and recovery mechanics. Migration preserves the last valid source until the upgraded destination commits durably. Interrupted migration resumes idempotently or enters protected recovery. Future versions are refused without overwriting them. Drafts are checkpointed on change, debounced, and at meaningful transitions. Recovery never depends on a shutdown hook running.

**★ Theme, live.** The theme stays in the shell's `settings.yaml`, read by `yantrik_design::Theme::current()`. The framework applies a change to every open window while the app runs. Today an app keeps its starting theme until relaunched. Apps never write the theme.

## 7. Errors and words

- **Errors are typed per module,** with no global error type. `thiserror` joins the workspace.
- **A public error** carries a stable `kind` and a **message id with arguments**. It renders to a localised sentence, for the person on the notice line and for the Mind as the refusal message, in the writing rules' voice. Diagnostics (the `Debug`/`Display` of internal errors) are separate and never shown as-is.
- **What crosses the wire:** `Refused { kind, message: MessageId, args }`, rendered to a sentence at the boundary.
- **`cx.run` with a refusal:**
  - writes the sentence to the notice line, replacing the 8 `settle`/`say` helpers;
  - logs the kind;
  - writes a problem record for `internal_*` kinds.
- **`describe` is a projection:**
  - named sections per feature;
  - large domains page with a revision and a cursor;
  - credentials, caches, private engine state, document contents and raw arguments are never included.

## 8. Large lists, undo, desktop integration

- **Lists.** The view adapter owns a retained `ModelRc`, updated per row from stable ids and the model revision, never rebuilt whole because an unrelated part changed. Paging has explicit row and byte limits and supports cancellation. Surface cursors bind to the query and committed revision; a cursor from another revision is rejected as stale and requires a restart.
- **Undo.** History is owned by the app and its domain, recording committed user operations. The framework does not infer inverse commands. External, irreversible effects (sent mail, deleted remote events) are marked non-undoable.
- **Desktop integration.** One typed effect boundary, `fx.desktop(DesktopRequest::…)`, covers file pickers (through the portal), clipboard, drag and drop, and opening URLs and files. Results come back as commands. A picked file is a capability handle the app may use, not an unrestricted path.

## 9. Testing

Apps become **`lib + bin`**. The 9 `#[path]` test crates are deleted as each app moves. Tests that read `main.rs` as text become behavioural tests or architecture rules.

| Level | What | How |
|---|---|---|
| Model | rules, conversions, migrations; `Refused` leaves the model unchanged | `update(&mut model, cmd, &mut Effects::recording())`; assert on the requested effects as data. No Slint, no threads. |
| App | intents, agents, settlement, close and exit, persistence ordering | `TestApp::<A>` (`yantrik-testing`, dev-only): fake services and Mind, injected `Paths`, and a **manually scheduled executor**. A test chooses when each job completes, fails, panics or is replaced, and moves a virtual clock. `act`/`act_as_agent`/`act_with_grant` go through the real dispatch registry. It also checks that every feature's intents are declared and every part renders. |
| Transport | identity, approvals | One real-socket suite per release: peer credentials, approvals, grants and request limits through the actual transport. |
| Pixels | every screen | `snapshot(name)` with the software renderer: both themes, 1.0 and 1.25 text scale, keyboard focus visible. This replaces the 6 copied headless platforms. |

**Every exported intent is tested for:**
- good, missing and wrong arguments;
- unavailable state;
- an agent refused;
- an approval required, given and then invalidated by a change;
- duplicate delivery;
- settling with each terminal outcome;
- a stale result rejected.

## 10. Decided now, built when first needed

- **Accessibility.** Every kit control has an accessible role, name and value. Focus order and keyboard operation are part of each view's tests, and #887's gap is closed by the framework's shortcut and focus contract.
- **Localisation.** User-facing text uses message ids with arguments and locale-aware formatting (dates, numbers, `time::ago`). Layouts do not assume left-to-right. English remains the only shipped locale until a second one is added.
- **Deep links and launch.** `Launch` is typed and versioned, with explicit profile and optional window targets, files or intent arguments (`yantrik://<app>/<intent>`). Validate all fields as untrusted input. Reject mismatched instance targets; deep-linked intents enter the authorised intent path.
- **Privacy and diagnostics.** Local and redacted by default. No network telemetry without an explicit policy and consent. Problem records exclude document content, credentials and raw arguments. They expire after 30 days and have a 10 MiB per-profile cap, evicting the oldest first. Users can delete them immediately.
- **Can wait:** printing, rich drag previews, undo grouping, streaming Mind tokens and inotify. Their effect and service boundaries are reserved above, so adding them later is not a rewrite.

## 11. Crates and the gated migration

```
frameworks/
  yantrik-foundation    L0  + json_store (one atomic writer, ordered lane), home::expand, problems, place, message ids
  yantrik-app           L2  NEW  app, run, boot, profile, cx, bridge, effects, jobs, timers, window!, chrome, instance,
                                 launch, event_loop, theme, paths, settings, persist, refused, invocation, watch,
                                 intents/{intent,args,serve,shortcuts}, desktop, features/agent_rail
  yantrik-services      L2  NEW  one typed client per domain, fakes behind a feature
  yantrik-mind          L2  NEW  companion.rs, typed
  yantrik-notifications L2  NEW  notify.rs
  yantrik-testing       L2  NEW  dev-only: TestApp, the scheduled executor, Headless, FakeServices, FakeMind, snapshot
crates/
  yantrik-app-runtime   L2  DEPRECATED: re-exports, removed after every consumer has moved and one release has passed
```

`yantrik-app` adds no Slint code generation and does not depend on the kit.

**Every PR leaves every app building, and nightly never stops.** Each step lands only after its gate passes.

| # | Lands | Gate |
|---|---|---|
| 1 | **Extraction:** `yantrik-app` made by moving instance, chrome, event loop, present, control and theme out of app-runtime, unchanged; `place`, `problems` and `json_store` to Foundation; `thiserror`. app-runtime re-exports. | Compatibility tests; no re-export cycle; every app builds unchanged. |
| 2 | **Minimal runner:** `App`, `Runner`, `Cx`, the bridge, the bounded command queue, `window!`, and the scheduled test executor. | **`window!` compiles in two separate generated UI crates in release**; a toy app runs; queue overload and posting after shutdown are tested. |
| 3 | **Invocations:** `Intents`, `Args`, `InvocationId`, settlement, provenance, the dequeue-time checks. | The real-dispatch tests: approvals bound and invalidated, agents refused, every terminal outcome. |
| 4 | **Persistence and lifecycle:** paths and profiles, settings with recovery and conflicts, the ordered lane, drafts, close and exit events. | Ordering, save-failure, corrupt-file and conflict tests. |
| 5 | **Clients and the rail:** `yantrik-services`, `yantrik-mind`, `yantrik-notifications`, the rail feature, `fx.desktop`, the `weather.forecast` method. | Fake-backed tests; one real-socket suite. |
| 6 | **Weather on the framework, behaviour unchanged** (lib + bin, screen moved into the app, the feature checks on). | Stale-result rejection, worker panic, queue overload, timeout, shutdown during a refresh, duplicate launch, and an unauthorised proposal refused. `yos check weather` is identical. |
| 7 | **Weather redesign**, kit-only (zero design-lint entries), to the Alt+Tab bar with 520 evidence and snapshots. A separate PR, so a regression is never ambiguous between framework and presentation. | Review and 520 evidence. |
| 8 | **Notes, the framework's acceptance gate:** two document contexts, save failure, an interrupted migration, a conflicting outside edit, close cancelled, recovery after a forced kill. | All of these pass before any other app moves. |
| 9 | **A large-list fixture,** then Calendar, then Email. | List paging and incremental-update tests. |
| 10 | The remaining 14, smallest first. Each app gets two PRs, a framework move and then a redesign. | Each app's own gates. |
| 11 | Delete `yantrik-app-runtime` and the `#[path]` crates. | No consumer remains, and one release has passed. |

**Can be codemodded:** the startup block, theme lines, `window_chrome!`, `run_until_closed`, atomic writers and `home()`s, `say`/`settle`, and the `#[path]` crates. **Cannot be:** turning callbacks into commands, surfaces into intents, security rules, agent rules, shutdown order and data migrations. That is modelling work, reviewed in each app's framework PR.

## 12. Decisions ★ (taken 21 Oct 2026 under Pranab's delegation)

1. **Adopt this app model:** an `App` trait and a framework-owned runner (the equivalent of BaseApp, by composition, not inheritance), Elm-style model/update/render, with buttons and the Mind on the same commands.
2. **No async runtime in apps:** a bounded job pool returning commands.
3. **One path rule for app files,** with existing files imported once and backed up; the `$YANTRIK_<APP>_DIR` overrides go after one release.
4. **Live theme:** apps follow theme changes while open.
5. **The gated order:** framework steps 1–5, then Weather twice (behaviour unchanged, then the redesign), then Notes as the acceptance gate, then Calendar, Email and the rest. Two PRs per app.

## What we refuse to build

- A `BaseApp` struct with virtual-hook fields, or any inheritance emulation.
- A proc-macro DSL for features or intents.
- A string-topic event bus.
- A global `Arc<Mutex<AppState>>`.
- A thread or runtime per feature.
- A surface or rail path that skips the dispatch's policy.
- Agent detection from a caller-supplied flag.
- A cached or whole-model `describe`.
- One generated UI crate for all apps.
- Unbounded queues.
- Promises to kill blocking work.
- A codemod that turns callbacks into commands.
- A framework that "solves" modularity by moving a 2,000-line `main` into a 2,000-line `App::wire`.

**The real test:** the twentieth app gets launch handoff, a live theme, typed actions, bound approvals, owned jobs and timers, ordered persistence, recovery, notices and tests by adopting the contract, and still reads as a small Rust program anyone can follow.
