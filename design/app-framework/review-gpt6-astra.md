# YantrikApp: one application contract, composed from features

I would build **`yantrik-app` at L2**, exposed through `yantrik-sdk`.

It is not a superclass, a UI toolkit, or a new service layer. It is the owner of an application’s lifetime:

> **One application identity, one UI-thread model, one command dispatcher, one control surface, and one supervised lifetime.**

Features supply domain behavior. Views project that behavior into Slint. The framework supplies the machinery that all 18 applications currently rebuild.

I would make five decisions immediately:

1. **Application state has one owner: the UI thread.** No application-wide `Rc<RefCell<State>>` or `Arc<Mutex<State>>`.
2. **Commands are the single behavioral entry point.** Buttons, menus, shortcuts, launch requests, and `act` use the same dispatcher.
3. **A surface action is an explicitly exported command**, not a second implementation of that command.
4. **The framework owns startup and shutdown.** Apps cannot quietly implement a second instance lock, IPC server, theme watcher, or event loop.
5. **Shared behavior is composition, not inheritance.** The agent rail, settings, recovery, operations, and notifications are framework facilities.

The API below is a proposed new SDK contract, not a claim that these APIs already exist in Slint or the current runtime. Ordinary builder setters and existing IPC protocol types are omitted; the extension points and ownership boundaries are explicit.

---

# 1. What an application is

## 1.1 Identity and manifest

There must be one ID, used everywhere.

```rust
pub struct AppManifest {
    pub id: AppId,
    pub display_name: MessageKey,
    pub version: &'static str,
    pub instance: InstancePolicy,
    pub launch: LaunchSupport,
    pub capabilities: &'static [Capability],
}

pub enum InstancePolicy {
    Single,
    Multiple,
}

pub struct LaunchSupport {
    pub files: bool,
    pub uris: bool,
}

pub enum LaunchTarget {
    File(PathBuf),
    Uri(Url),
}

pub struct Launch {
    pub request_id: RequestId,
    pub targets: Vec<LaunchTarget>,
    pub disposition: LaunchDisposition,
}

pub enum LaunchDisposition {
    Present,
    Open,
}
```

`AppId` is a validated package identity, not a convenient string picked independently by each subsystem.

The framework derives:

- instance ownership;
- socket registration;
- shell registration;
- settings, state, and data directories;
- notification identity;
- tracing fields;
- problem-report attribution;
- approval attribution;

from this manifest ID.

Production packaging verifies that the executable’s registered identity matches its manifest. A caller cannot become another app by selecting its ID in a builder.

## 1.2 Application trait

```rust
pub trait Application: Sized + 'static {
    type Model: 'static;
    type Event: Send + 'static;
    type Settings: SettingsSchema;
    type View: AppView<Self>;

    fn manifest() -> AppManifest;

    fn register(registry: &mut AppRegistry<Self>) -> AppResult<()>;

    fn restore(
        context: RestoreContext<Self>,
    ) -> BoxFuture<'static, AppResult<Self::Model>>;

    fn launch(
        model: &Self::Model,
        launch: &Launch,
        commands: &mut LaunchCommands<Self>,
    ) -> AppResult<()>;

    fn update(
        model: &mut Self::Model,
        event: Self::Event,
        context: &mut UpdateContext<Self>,
    ) -> AppResult<()>;

    fn ready(
        model: &mut Self::Model,
        context: &mut UpdateContext<Self>,
    ) -> AppResult<()> {
        Ok(())
    }

    fn settings_changed(
        model: &mut Self::Model,
        settings: &Self::Settings,
        context: &mut UpdateContext<Self>,
    ) -> AppResult<()> {
        Ok(())
    }

    fn close_decision(
        model: &Self::Model,
        reason: CloseReason,
    ) -> CloseDecision {
        CloseDecision::Allow
    }

    fn checkpoint(
        model: &Self::Model,
        context: &CheckpointContext,
    ) -> AppResult<RecoveryBatch> {
        Ok(RecoveryBatch::empty())
    }

    fn shutdown(
        model: &mut Self::Model,
        context: &mut ShutdownContext<Self>,
    ) -> AppResult<()> {
        Ok(())
    }
}
```

The application trait deliberately does **not** expose `main`, `wire`, `serve`, or `run_event_loop`.

`restore` performs asynchronous loading before interactive startup. It returns the initial model. It does not create Slint components.

`launch` translates supported launch targets into typed commands. It cannot execute arbitrary domain code directly. The framework preserves the launch’s authenticated origin when dispatching those commands.

`checkpoint` returns owned recovery records. File I/O happens outside the model borrow.

`shutdown` is a brief UI-thread hook. It can register bounded cleanup work with its context; it must not block the event loop.

## 1.3 View contract

```rust
pub trait AppView<A: Application>: WindowPorts + Sized + 'static {
    fn create() -> AppResult<Self>;

    fn bind(&self, commands: UiPort<A>);

    fn render(
        &self,
        model: &A::Model,
        presentation: &Presentation,
    );
}

pub trait WindowPorts {
    fn window(&self) -> &slint::Window;

    fn apply_theme(&self, theme: &ThemeSnapshot);

    fn install_chrome(&self, actions: ChromeActions);

    fn agent_rail(&self) -> Option<AgentRailPort> {
        None
    }
}
```

Only view modules implement or use these Slint-facing contracts.

The UI kit supplies a versioned Slint window contract containing the theme, chrome, notices, and optional rail bridges. An SDK macro generates the mechanical `WindowPorts` implementation:

```rust
yantrik_sdk::ui::impl_window_ports! {
    WeatherView => ui: WeatherWindow,
    theme: Theme,
    chrome: WindowChrome,
    notices: NoticeBridge,
    rail: AgentRailBridge,
}
```

This macro does not generate domain behavior. It replaces repeated generated-global plumbing.

## 1.4 Runner

```rust
pub struct AppRunner<A: Application> { /* private */ }

impl<A: Application> AppRunner<A> {
    pub fn new() -> Self;

    pub fn args_os(
        self,
        args: impl IntoIterator<Item = OsString>,
    ) -> Self;

    pub fn run(self) -> ExitCode;
}

pub fn run<A: Application>() -> ExitCode {
    AppRunner::<A>::new()
        .args_os(std::env::args_os())
        .run()
}
```

An application’s `main.rs` becomes:

```rust
use std::process::ExitCode;
use yantrik_weather::WeatherApp;

fn main() -> ExitCode {
    yantrik_sdk::app::run::<WeatherApp>()
}
```

Five lines. No startup customization is allowed by adding machinery around `run`.

Test-only runner injection lives in `yantrik-app-test`, not in a production builder full of escape hatches.

---

# 2. Exact lifecycle

The runner has an explicit state machine.

```text
Bootstrap
  → Claiming
      → Forwarding → Exit
      → Owner
  → Restoring
  → CreatingView
  → Binding
  → Serving
  → Ready
  → Running
  → Closing
      → Running       [cancelled close]
      → Quiescing
  → Checkpointing
  → Draining
  → Stopped
```

## Startup order

| Step | Framework responsibility | App hook |
|---|---|---|
| 1 | Validate manifest; initialize structured tracing and panic reporting | None |
| 2 | Parse standard CLI and launch envelope | None |
| 3 | Select Slint backend before constructing any component | None |
| 4 | Claim instance ownership and establish an authenticated handoff endpoint | None |
| 5 | Register commands, schemas, surface projections, shared facilities | `register` |
| 6 | Open stores; validate and migrate settings; load recovery | `restore` |
| 7 | Create the Slint window | `View::create` |
| 8 | Apply initial theme; install chrome; bind callbacks | `View::bind` |
| 9 | Register shell identity and publish surface readiness | None |
| 10 | Translate initial launch into commands | `launch` |
| 11 | Start initial work and app-owned schedules | `ready` |
| 12 | Render and show; process queued launch requests | `View::render` |
| 13 | Run event loop, live theme/settings subscriptions, operations | `update`, `settings_changed`, `render` |

The handoff endpoint can acknowledge **queued** requests during restoration, but does not claim that commands have executed. The full surface reports `starting` until the model and registry are ready.

## Single-instance handoff

The framework replaces the seven retry loops and both private `flock` implementations.

- Lock ownership and endpoint registration are one runtime facility.
- The lock is scoped to the appropriate user/session namespace.
- The endpoint is inside a protected runtime directory.
- The second process sends a versioned `Launch` envelope—not an improvised `act`.
- Handoff has a deadline, request ID, and explicit acknowledgement.
- The owner deduplicates retried launch request IDs.
- If the owner dies during handoff, the contender rechecks ownership and may become the new owner.
- Stale socket paths do not prove ownership.
- The owner validates peer credentials and launch payloads.
- Protocol incompatibility produces a typed failure, not silent exit.

There is no second `show` action. The standard action is `app.present`. A legacy `show` alias may exist during migration, but is not listed as a new action.

`present` requests shell activation using available activation authority. It does not promise focus stealing that Wayland forbids.

## Backend selection

There is one implementation of the `LIBGL_ALWAYS_SOFTWARE` compatibility behavior.

Selection order is documented and tested:

1. explicit supported diagnostic override;
2. platform software-rendering requirement;
3. normal platform default.

Selection happens before Slint initialization. Unsupported combinations produce an actionable startup error. Apps never change backend environment variables themselves.

## Closing and shutdown

Window close, shell quit, logout, SIGTERM, and test shutdown enter the same path.

```rust
pub enum CloseDecision {
    Allow,
    Confirm(Confirmation),
    Block(UserProblem),
}

pub enum CloseReason {
    Window,
    Shell,
    Logout,
    Signal,
    Test,
}
```

For a normal close:

1. Evaluate `close_decision`.
2. If necessary, display a framework confirmation without destroying the window.
3. Stop accepting mutating commands. Return `AppClosing` to IPC callers.
4. Cancel recurring timers and ordinary task scopes.
5. Snapshot recovery via `checkpoint`.
6. Run the app’s bounded shutdown hook.
7. Flush settings/recovery and drain registered cleanup work.
8. Close service connections, surface, and window.
9. Release instance ownership last.

The Slint event loop remains alive while draining. No joining worker threads on the UI thread.

SIGTERM gets a bounded checkpoint opportunity; SIGKILL does not. Unsaved work must therefore be checkpointed during normal operation, not only at exit.

Panic reporting is best effort. It records a sanitized problem report; it does not attempt to continue with potentially inconsistent model state. OOM, abort, and process-kill reporting also require the OS supervisor—an in-process framework cannot catch everything.

---

# 3. Features and the command model

## 3.1 Feature layout

The default feature has these files:

```text
features/<feature>/
    mod.rs       # pub(crate) facade and registration
    model.rs     # domain state and pure rules
    commands.rs  # typed commands and handlers
    view.rs      # Slint binding and projection
    surface.rs   # action exports and describe projection
    tests.rs     # behavioral tests
```

The accepted five-file pattern remains the base. `commands.rs` is the first intentional split when behavior would otherwise crowd `mod.rs` or `model.rs`.

Additional files are named for a concept: `forecast.rs`, `search.rs`, `draft.rs`. Never `utils.rs`, `common.rs`, or `manager.rs`.

## 3.2 Registration

```rust
pub trait Feature<A: Application> {
    const ID: FeatureId;

    fn install(registry: &mut FeatureRegistry<A>) -> AppResult<()>;
}

impl<A: Application> AppRegistry<A> {
    pub fn feature<F: Feature<A>>(&mut self) -> AppResult<()>;

    pub fn agent_rail(&mut self, options: AgentRailOptions);

    pub fn recovery(&mut self, policy: RecoveryPolicy);
}

impl<A: Application> FeatureRegistry<A> {
    pub fn command<C: Command>(
        &mut self,
        spec: CommandSpec<A, C>,
        handler: CommandHandler<A, C>,
    ) -> AppResult<()>;

    pub fn expose<C: Command>(
        &mut self,
        exposure: Exposure,
    ) -> AppResult<()>;

    pub fn describe<T>(
        &mut self,
        name: StateSection,
        projection: fn(
            &A::Model,
            &DescribeContext,
        ) -> AppResult<T>,
    ) -> AppResult<()>
    where
        T: Serialize + JsonSchema + 'static;
}
```

Registration rejects duplicate command IDs, duplicate state sections, unsupported exposure, and reserved `app.*` names.

Feature registration does not depend on order. A handler dispatching another command enqueues it after the current turn; it does not recursively invoke another feature.

## 3.3 State composition

The app owns an ordinary Rust aggregate:

```rust
pub(crate) struct Model {
    pub(crate) weather: weather::Model,
    pub(crate) search: locations::Model,
    pub(crate) settings: Settings,
}
```

There is no reference-counted state handle given to each feature.

During one dispatch, a handler gets one short-lived `&mut Model`. Rust’s normal field borrowing handles composition.

Features in an app may know the app model through its `pub(crate)` facade. That is an intentional choice: these are modules, not independently deployed plugins.

Cross-feature behavior has two acceptable forms:

1. An application-level command coordinates two fields in one turn.
2. One feature emits a typed command or event consumed in a later turn.

It does **not** use a global event bus with string topics, mutable shared services, or back references between feature objects.

If a feature is reusable across applications, it receives an explicit host interface or typed ports. The agent rail is such a feature.

## 3.4 Commands

```rust
pub trait Command:
    Serialize + DeserializeOwned + JsonSchema + Send + 'static
{
    type Output:
        Serialize + DeserializeOwned + JsonSchema + Send + 'static;

    const ID: CommandId;
}

pub type CommandHandler<A, C> = fn(
    &mut <A as Application>::Model,
    C,
    &mut CommandContext<A>,
) -> AppResult<Outcome<<C as Command>::Output>>;

pub enum Outcome<T> {
    Done(T),
    Accepted(OperationId),
}
```

A command contains typed domain arguments:

```rust
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SelectLocation {
    pub location: LocationId,
}

impl Command for SelectLocation {
    type Output = ();
    const ID: CommandId = CommandId::new("weather.select-location");
}
```

The UI may construct this Rust value directly. IPC deserializes it once at the boundary.

There is no JSON parsing inside a handler.

### Commands and actions: related, not identical

**Every public action is a command. Not every command is public.**

Examples:

- `weather.refresh`: UI, shortcut, menu, and Mind.
- `editor.replace-selection`: potentially public.
- `editor.update-composition`: internal UI implementation detail.
- `app.apply-theme-snapshot`: framework internal event, never a public action.

Public action schemas come from the command and output types. Surface exposure adds documentation, availability hints, and description visibility—not another handler or another authorization rule.

Menus and shortcuts bind command IDs and typed arguments. They do not carry closures containing business logic.

## 3.5 One authorization path

```rust
pub struct CommandSpec<A: Application, C: Command> { /* private */ }

impl<A: Application, C: Command> CommandSpec<A, C> {
    pub fn new(title: MessageKey) -> Self;

    pub fn policy(
        self,
        policy: fn(
            &A::Model,
            &C,
            &Caller,
        ) -> AppResult<Intent>,
    ) -> Self;

    pub fn availability(
        self,
        predicate: fn(&A::Model, &C) -> Availability,
    ) -> Self;
}

pub enum Risk {
    Standard,
    Sensitive,
    Dangerous,
}

pub struct Intent {
    pub risk: Risk,
    pub permissions: Vec<Permission>,
    pub resources: Vec<ResourceIntent>,
}
```

A policy is required before registration succeeds. There is no accidental “missing policy means allow.”

The dispatch path is:

```text
decode / construct typed command
  → validate arguments
  → resolve resources and required permissions
  → authorize authenticated caller
  → obtain approval if required
  → revalidate approval-sensitive state
  → execute handler on UI thread
  → start committed effects
  → render and publish state revision
  → reply
```

Approval runs asynchronously. No model borrow survives the wait.

Approval binds the command ID, normalized arguments, resolved resources, caller, relevant state fingerprint, expiry, and nonce. When state changes, the runtime rechecks the intent and either safely reuses or invalidates the approval.

A changed path must not turn an approved harmless operation into a different destructive one. Path-based operations use resolved resource handles/capabilities where available, not merely a string comparison followed by a later unrestricted open.

An approval is not a substitute for ordinary access control.

### Caller identity

```rust
pub struct Caller { /* runtime-created; no public constructor */ }

impl Caller {
    pub fn principal(&self) -> &Principal;
    pub fn origin(&self) -> InvocationOrigin;
    pub fn agent(&self) -> Option<&AgentIdentity>;
    pub fn delegation(&self) -> Option<&VerifiedDelegation>;
}
```

Peer credentials prove a process identity, not automatically “this is the Mind.” The platform identity resolver maps the authenticated process to a registered principal and role.

If an intermediary acts for an agent, it must propagate verified delegation. Otherwise the call is attributed to the intermediary. Payload fields such as `"agent": false` have no authority.

The UI port carries a framework-created local UI origin. IPC clients cannot forge it.

`agent_is_calling()` disappears as ambient thread-local policy. Handlers receive explicit invocation context where needed; most authorization belongs in their command specification.

## 3.6 Callback binding without weak upgrades

```rust
impl<A: Application> UiPort<A> {
    pub fn dispatch<C: Command>(&self, command: C);

    pub fn action<C: Command>(
        &self,
        make: impl Fn() -> C + 'static,
    ) -> impl Fn() + 'static;

    pub fn action1<T: 'static, C: Command>(
        &self,
        make: impl Fn(T) -> C + 'static,
    ) -> impl Fn(T) + 'static;
}
```

A view binds:

```rust
ui.on_refresh(port.action(|| Refresh {}));

ui.on_search(port.action1(|text: slint::SharedString| {
    SearchLocations {
        query: text.to_string(),
    }
}));
```

These closures capture only a dispatcher endpoint. They neither capture the window nor borrow the model.

Dispatch always enqueues a turn. It never reenters the reducer from inside a Slint callback.

The framework owns the one weak event-loop bridge. A closed app makes its ports inert and rejects outstanding requests appropriately.

---

# 4. State, effects, threads, and time

## 4.1 The execution rule

> **Read and mutate app state only on the UI thread. Send owned inputs to workers. Return owned events.**

The runner owns:

```text
UI thread:
    Model
    View
    command registry
    render scheduler
    authorization continuations
    operation state

Background:
    shared async I/O executor
    bounded blocking pool
    IPC transport
    storage I/O
```

`Model` need not be `Send`. `Event` must be `Send`.

No lock is held while rendering, calling Slint, awaiting I/O, or waiting for approval.

The async executor is one runtime per process, not one per feature. CPU-heavy or blocking libraries use the bounded blocking pool. Apps do not call `thread::spawn` by default.

## 4.2 Effects API

```rust
impl<A: Application> CommandContext<A> {
    pub fn caller(&self) -> &Caller;

    pub fn spawn<T: Send + 'static>(
        &mut self,
        spec: JobSpec,
        work: impl FnOnce(TaskContext)
            -> BoxFuture<'static, AppResult<T>>
            + Send
            + 'static,
        completed: impl FnOnce(AppResult<T>) -> A::Event
            + Send
            + 'static,
    ) -> OperationId;

    pub fn enqueue<C: Command>(&mut self, command: C);

    pub fn persist_settings(
        &mut self,
        settings: A::Settings,
    ) -> AppResult<OperationId>;
}

pub struct JobSpec {
    pub key: JobKey,
    pub concurrency: Concurrency,
    pub timeout: Duration,
    pub visibility: OperationVisibility,
}

pub enum Concurrency {
    Parallel,
    RejectIfRunning,
    ReplacePrevious,
}

impl TaskContext {
    pub fn cancellation(&self) -> &CancellationToken;

    pub fn services(&self) -> &Services;

    pub fn storage(&self) -> &AppStorage;
}
```

`UpdateContext` exposes the same effect-producing operations, plus owned scheduling.

Effects are staged during the handler and released only after it returns successfully.

This is **not** automatic database-style rollback of arbitrary Rust mutations. Handlers must validate and calculate before mutating. Complex commands use a pure `prepare` function that returns a change to apply. Tests enforce that failed commands leave state unchanged.

That is preferable to cloning an entire mail database before every command.

## 4.3 Completion, cancellation, and stale results

For `ReplacePrevious` jobs, the runtime assigns generations.

- Starting a new location refresh cancels the previous refresh.
- A late completion from the previous generation cannot update the model.
- Cancellation is cooperative for already-running blocking work.
- Results arriving after the owning scope closes are discarded.
- The operation record still reaches a terminal state.

For uninterruptible native engines, use an isolated worker process if hard cancellation is required. Rust threads cannot be safely killed.

A background completion becomes an `Event`; `Application::update` applies it on the UI thread. The runtime marks an operation successful only after its completion event has been accepted and applied successfully.

`Outcome::Accepted` means **accepted**, not completed. The surface exposes operation status. It does not fabricate a successful final result before the network request finishes.

## 4.4 Timers have owners

```rust
impl<A: Application> UpdateContext<A> {
    pub fn every(
        &mut self,
        owner: ScopeId,
        key: TimerKey,
        interval: Duration,
        make_event: impl Fn() -> A::Event + 'static,
    ) -> TimerId;

    pub fn cancel_timer(&mut self, id: TimerId);
}
```

Timer identity is `(owner, key)`, so installing the same schedule twice does not silently create duplicate timers.

The scheduler owns its timer handles. Apps do not keep timers alive through `_binding` variables.

Closing a feature scope cancels its timers and jobs. Closing the app closes every ordinary scope. Shutdown work uses a separate bounded scope.

Debouncing is a scheduler operation, not logic in `.slint`.

## 4.5 Rendering

The dispatcher increments a state revision and schedules at most one render per event-loop turn.

The initial implementation uses ordinary projection methods, with equality checks or stable `ModelRc` updates for large collections. It does not introduce a custom reactive programming language.

For a mail list with 100,000 rows, use a dedicated Slint model adapter and incremental updates. Do not rebuild the entire list because a notice changed.

---

# 5. Surface protocol and shared facilities

## 5.1 Standard surface envelope

Every application exposes a consistent envelope:

```json
{
  "app": {
    "id": "org.yantrik.weather",
    "lifecycle": "running",
    "revision": 42
  },
  "state": {
    "weather": {},
    "locations": {}
  },
  "operations": [],
  "problems": []
}
```

The runtime owns the envelope. Features own named state sections.

Standard actions include:

- `app.present`;
- `app.close`;
- operation inspection and cancellation where supported.

These do not count as the app’s domain actions.

`describe` is a projection, not serialization of the whole model. It excludes caches, credentials, private engine objects, and irrelevant UI internals.

`DescribeContext` carries authenticated caller visibility. State authorization is independent of action authorization. An agent should not learn private content merely because it can discover an action schema.

Descriptions have explicit size limits and pagination for large domains. An email app cannot dump every message into one JSON response.

## 5.2 Typed clients

The framework supplies transport policy and typed client construction. It does not expose L3 implementation crates.

```rust
impl Services {
    pub fn client<C: ServiceClient>(&self) -> C;
}

pub struct CallOptions {
    pub timeout: Duration,
    pub cancellation: CancellationToken,
    pub retry: RetryPolicy,
}

pub enum RetryPolicy {
    Never,
    Idempotent {
        max_attempts: u8,
        max_elapsed: Duration,
    },
}
```

For example, the public L2 weather contract contains:

```rust
pub trait WeatherService: ServiceClient {
    fn search(
        &self,
        query: SearchQuery,
        options: CallOptions,
    ) -> BoxFuture<'_, Result<Vec<Location>, WeatherServiceError>>;

    fn forecast(
        &self,
        location: LocationId,
        days: ForecastDays,
        options: CallOptions,
    ) -> BoxFuture<'_, Result<ForecastBundle, WeatherServiceError>>;
}
```

The actual weather service remains L3.

All calls have deadlines. Only explicitly idempotent calls get automatic retries. Service discovery and activation use the platform registry; a client does not shell out to start a daemon.

A service sees the app’s authenticated identity. Delegated agent authority is attached only through a verified invocation capability, not by forwarding arbitrary caller JSON.

Notifications follow the same model:

```rust
impl Notifications {
    pub async fn send(
        &self,
        notification: Notification,
        options: CallOptions,
    ) -> Result<NotificationId, NotificationError>;
}
```

Notification actions route back through typed app commands. They do not introduce a second command implementation.

## 5.3 The agent rail

The shared rail has two parts:

1. **L1:** one Slint component and bridge in `yantrik-ui-kit`.
2. **L2:** one controller in `yantrik-app`, using public Mind protocol clients.

It is not a dependency on the L5 Mind implementation.

```rust
pub struct AgentRailOptions {
    pub context: AgentContextPolicy,
    pub suggestions: SuggestionPolicy,
    pub allow_ask: bool,
}

pub enum SuggestionPolicy {
    OnStateChange { debounce: Duration },
    Subscribe,
    Poll { interval: Duration },
}
```

The rail owns:

- ask-box state;
- outstanding request and cancellation;
- suggestion refresh/subscription;
- proposal cards;
- loading and error presentation;
- recall/reach handling;
- redaction and context limits;
- approval routing;
- a feature task scope.

An app provides a safe contextual description, not a clone of the rail controller.

Proposal execution goes through the command dispatcher with the proposal’s verified provenance. Pressing “Apply” can provide explicit approval for that specific proposal; it does not erase the fact that the proposal originated with an agent.

The existing 4-second poll is a compatibility fallback. Prefer subscriptions or debounced revision changes.

---

# 6. Settings, data, and recovery

## 6.1 One directory rule

For app ID `org.yantrik.weather`:

```text
$XDG_CONFIG_HOME/yantrik/apps/org.yantrik.weather/settings.json
$XDG_DATA_HOME/yantrik/apps/org.yantrik.weather/
$XDG_STATE_HOME/yantrik/apps/org.yantrik.weather/
$XDG_CACHE_HOME/yantrik/apps/org.yantrik.weather/
$XDG_RUNTIME_DIR/yantrik/...
```

XDG fallback rules are implemented once.

- **Config:** preferences.
- **Data:** durable user/application data.
- **State:** recovery, drafts, checkpoints, operation journals where needed.
- **Cache:** disposable downloaded or derived material.
- **Runtime:** sockets and transient coordination.

No new `$YANTRIK_WEATHER_DIR` overrides. Tests and developer sessions use an explicit sandbox root supplied by the framework.

User-selected documents are not forced into app storage.

## 6.2 Typed settings

```rust
pub trait SettingsSchema:
    Serialize
    + DeserializeOwned
    + Clone
    + Send
    + Sync
    + Default
    + 'static
{
    const VERSION: u32;

    fn migrate(
        from: u32,
        document: serde_json::Value,
    ) -> Result<serde_json::Value, SettingsError>;

    fn validate(&self) -> Result<(), SettingsError>;
}

pub struct SettingsSnapshot<T> {
    pub revision: SettingsRevision,
    pub value: Arc<T>,
}

impl<T: SettingsSchema> SettingsStore<T> {
    pub fn snapshot(&self) -> SettingsSnapshot<T>;

    pub async fn replace(
        &self,
        expected: SettingsRevision,
        value: T,
    ) -> Result<SettingsSnapshot<T>, SettingsError>;

    pub fn watch(&self) -> SettingsChanges<T>;
}
```

The on-disk envelope carries a schema version.

Writes use:

1. validation;
2. a temporary file in the same directory;
3. restrictive permissions;
4. complete write and file sync;
5. atomic rename;
6. directory sync when durability is requested.

File watching handles replacement, not just in-place modification. Changes are debounced, parsed, migrated, and validated before publication.

An invalid external edit does not replace the last valid in-memory settings. It creates a typed problem. The runtime does not silently overwrite a malformed or newer-schema file with defaults.

Concurrent edits use revisions; lost updates are reported rather than silently accepted.

Per-app migration code handles old path imports once, records completion, and preserves a backup.

## 6.3 Data and recovery APIs

```rust
impl AppStorage {
    pub fn data(&self) -> &DataDirectory;
    pub fn cache(&self) -> &CacheDirectory;
    pub fn recovery(&self) -> &RecoveryStore;
}

impl RecoveryStore {
    pub async fn write<T: Serialize + Send + Sync>(
        &self,
        key: RecoveryKey,
        schema: u32,
        value: &T,
    ) -> Result<(), StorageError>;
}
```

Settings are JSON. Small app records and recovery may also use versioned JSON. The framework does **not** require a future mail index or large document store to be JSON.

Atomic file replacement has one implementation. Tilde expansion has one parser at the user-input boundary; path-bearing internal APIs use `PathBuf` or resource handles.

Draft recovery is periodic and debounced, and also triggered at meaningful transitions. “Save everything during shutdown” is not a recovery strategy.

## 6.4 Theme

The global theme belongs to the OS settings service, not each app’s settings file.

The framework:

1. loads the current theme;
2. subscribes to updates;
3. applies complete snapshots on the UI thread;
4. redraws all registered windows;
5. retains the last valid snapshot during service interruption.

An app may have a declared preference such as “follow system” or an approved per-app appearance override. It cannot maintain an independent copy of system accent state.

---

# 7. Errors and presentation

Domain code produces typed errors:

```rust
#[derive(Debug, thiserror::Error)]
pub enum WeatherError {
    #[error("no location selected")]
    NoLocation,

    #[error("unknown location")]
    UnknownLocation,

    #[error("invalid search query")]
    InvalidQuery,

    #[error(transparent)]
    Service(#[from] WeatherServiceError),
}
```

The `Display` strings are diagnostic text, not the user-facing UI contract.

```rust
pub trait DomainError: std::error::Error + Send + Sync {
    fn code(&self) -> ErrorCode;

    fn message(&self) -> MessageDescriptor;

    fn details(&self) -> ErrorDetails;

    fn retry(&self) -> RetryAdvice;
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Storage(#[from] StorageError),

    #[error(transparent)]
    Service(#[from] ServiceError),

    #[error(transparent)]
    Authorization(#[from] AuthorizationError),

    #[error(transparent)]
    Domain(Box<dyn DomainError>),

    #[error("application is closing")]
    Closing,
}

pub type AppResult<T> = Result<T, AppError>;
```

A presentation boundary maps the error to:

- a localized human sentence and suitable UI presentation;
- a stable machine code and structured details for the Mind;
- a structured tracing event;
- a problem record if warranted.

Example wire error:

```json
{
  "code": "weather.location_required",
  "message": "Choose a location before refreshing.",
  "retryable": false,
  "details": {},
  "request_id": "..."
}
```

There is one notice presenter instead of eight `settle`/`say` implementations. It handles command failures from UI dispatch and supplies the notice bridge.

Expected failures—offline, cancelled, permission denied—are not automatically crashes or telemetry incidents.

Logs carry app, feature, command, request, and operation IDs. Arguments and described state are not logged wholesale. Secrets, message content, and precise location require explicit redaction decisions.

---

# 8. Testing

## 8.1 Three harness levels

### A. Pure model tests

No framework, Slint, IPC, or async executor.

Test domain rules, units conversion, ordering, validation, draft transformations, and migrations directly.

### B. Application harness

```rust
pub struct AppHarness<A: Application> { /* private */ }

impl<A: Application> AppHarness<A> {
    pub async fn start(
        sandbox: Sandbox,
        services: FakeServices,
        clock: TestClock,
    ) -> AppResult<Self>;

    pub async fn ui<C: Command>(
        &mut self,
        command: C,
    ) -> AppResult<Outcome<C::Output>>;

    pub async fn act(
        &mut self,
        caller: TestCaller,
        action: &str,
        arguments: serde_json::Value,
    ) -> SurfaceReply;

    pub async fn describe(
        &mut self,
        caller: TestCaller,
    ) -> DescribeReply;

    pub async fn settle(&mut self);

    pub async fn close(&mut self, reason: CloseReason) -> AppResult<()>;
}
```

It runs the real registry, policy checks, command handlers, update logic, settings, and operation state without constructing a Slint window.

The fake service transport implements the same public client contracts. Fake Mind records asks, emits proposals, and simulates unavailable or slow responses.

Tests use a fake clock, explicit approvals, deterministic task completion, and a temporary XDG sandbox. No sleeps waiting for a real 4-second timer.

The surface harness passes through the real JSON codec and router. Separate transport tests verify kernel peer credentials, framing, limits, and disconnection behavior.

### C. UI harness

`yantrik-app-test` owns the only headless Slint test platform.

Slint platform initialization is process-global, so the harness enforces a test-thread/process policy instead of having six tests each attempt to install their own platform.

It supports:

- view creation;
- callback invocation;
- inspecting accessibility and semantic UI state;
- fixed fonts, locale, scale, theme, and clock;
- rendered-image capture through a pinned renderer configuration.

Pixel tests are run in a controlled rendering environment. They are not expected to be portable bit-for-bit across arbitrary GPU drivers.

## 8.2 What to test

Every exported action gets tests for:

- valid arguments;
- malformed and unknown fields;
- unavailable state;
- unauthorized caller;
- agent approval required, denied, expired, and accepted;
- cancellation and timeout;
- duplicate request handling;
- stale asynchronous completion.

Every application gets:

- initial launch and second-instance handoff;
- settings migration and live reload;
- close cancellation;
- recovery after interrupted operation;
- shutdown with work outstanding;
- describe redaction.

UI snapshots cover:

- light and dark;
- narrow and wide layouts;
- loading, empty, stale, offline, and error states;
- keyboard focus and large text.

## 8.3 `lib + bin`

Tests import the library normally:

```rust
use yantrik_weather::WeatherApp;
```

The nine `#[path]` test crates are deleted as their apps migrate.

Private unit tests live beside the feature. Integration tests use the public app entry type and harness. Do not make the entire model public merely to satisfy tests.

Tests that search `main.rs` for text are replaced by behavioral tests or architectural lints:

- no runtime/transport bypass dependencies;
- no forbidden Slint imports;
- main line limit;
- no duplicate IDs;
- complete action schemas.

---

# 9. Crates and layers

## Public framework layout

```text
frameworks/
    yantrik-foundation/          # L0
    yantrik-design-tokens/       # L1
    yantrik-ui-kit/              # L1
    yantrik-app-build/           # build-time tooling

    yantrik-ipc-contracts/       # L2 public wire contracts
    yantrik-ipc-transport/       # L2 authenticated transport
    yantrik-surface/             # L2 surface wire model
    yantrik-platform/            # L2 typed platform/service clients
    yantrik-app-storage/         # L2 paths, settings, atomic storage
    yantrik-app/                 # L2 application model and runner
    yantrik-app-test/            # L2 development/test support
    yantrik-sdk/                 # L0–L2 curated facade

    yantrik-app-runtime/         # temporary compatibility facade
```

These are responsibility splits, not one crate per class.

## `yantrik-app` modules

```text
src/
    lib.rs
    application.rs
    manifest.rs
    runner/
        mod.rs
        bootstrap.rs
        lifecycle.rs
        shutdown.rs
    instance/
        mod.rs
        claim.rs
        handoff.rs
    command/
        mod.rs
        registry.rs
        dispatch.rs
        authorization.rs
        operation.rs
    feature.rs
    state/
        mod.rs
        describe.rs
        revision.rs
    task/
        mod.rs
        executor.rs
        scope.rs
        scheduler.rs
    window/
        mod.rs
        ports.rs
        chrome.rs
    theme.rs
    presentation/
        mod.rs
        notice.rs
        error.rs
    agent_rail/
        mod.rs
        model.rs
        controller.rs
        proposals.rs
    recovery.rs
```

Internal implementation crates, if needed:

```text
crates/
    yantrik-app-macros/
    yantrik-app-test-backend/
```

They carry L2-compatible dependencies. Putting code under `crates/` is not permission to depend upward on L3–L5.

## Current runtime migration map

| Existing API | New home |
|---|---|
| `init_tracing` | Runner bootstrap |
| `instance::*` | Runner instance subsystem |
| `theme::load` | Theme subscription owned by runner |
| `window_chrome!` | Versioned window ports |
| `control::App` | Typed command registry and surface adapter |
| `agent_is_calling`, `caller` | Explicit invocation context |
| `answer_later` | Operations and deferred command outcomes |
| `companion::*` | L2 typed Mind client |
| `notify::send` | Typed notification client |
| `service::*` | Typed platform client registry |
| `place::home` | Canonical foundation/path facility, re-exported |
| `problems::record_failure` | Typed problem client and runner reporting |
| `run_until_closed` | Runner lifecycle |

`yantrik-app-runtime` initially re-exports moved low-level facilities and retains legacy adapters. New apps cannot depend on it. It receives no new features.

## Facade discipline

`yantrik-sdk` is curated:

```rust
pub mod app;
pub mod command;
pub mod services;
pub mod storage;
pub mod ui;
pub mod prelude;
```

It does not re-export every transport type into the prelude. Direct socket clients remain possible for framework implementation, not routine app code.

## Build-host discipline

Do not put every app’s Slint UI into one mega-crate.

- Keep each app’s generated UI in its own app library or private UI crate.
- Keep the application runtime independent of generated application UI.
- Compile shared kit markup through the supported Slint build path; do not assume generated code is freely reusable across unrelated component compilations.
- Cache by UI inputs and SDK version.
- Have CI limit concurrent heavy Slint compilations, especially when the shell is compiling.
- Start with a conservative heavy-build concurrency limit and measure peak RSS before increasing it.

On a 48 GB host with a 14 GB shell UI compile, unconstrained parallel Slint codegen is not a scheduling policy.

---

# 10. Worked example: Weather

The example uses two domain features:

- **weather:** selected location, current conditions, seven-day forecast, units;
- **locations:** location search and search results.

It exports four domain actions:

1. `weather.refresh`
2. `weather.search-locations`
3. `weather.select-location`
4. `weather.set-units`

The shared agent rail is registered once. Framework `app.*` actions are additional.

The snippets below sketch all files. Boilerplate imports and repetitive conversion code are abbreviated; each file is designed to remain below 300 lines.

## 10.1 Tree

```text
apps/weather/
    Cargo.toml
    build.rs
    ui/
        weather.slint
    src/
        main.rs
        lib.rs
        app.rs
        model.rs
        event.rs
        settings.rs
        recovery.rs
        view.rs
        features/
            mod.rs
            weather/
                mod.rs
                model.rs
                commands.rs
                surface.rs
                view.rs
                tests.rs
            locations/
                mod.rs
                model.rs
                commands.rs
                surface.rs
                view.rs
                tests.rs
    tests/
        surface.rs
        lifecycle.rs
        visual.rs
```

## 10.2 Package and build

### `Cargo.toml`

```toml
[package]
name = "yantrik-weather"
version = "0.1.0"
edition = "2024"

[lib]
name = "yantrik_weather"

[[bin]]
name = "yantrik-weather"
path = "src/main.rs"

[dependencies]
yantrik-sdk = { workspace = true, features = ["weather-client"] }
slint = { workspace = true }
serde = { workspace = true, features = ["derive"] }
schemars = { workspace = true }
thiserror = { workspace = true }

[build-dependencies]
yantrik-app-build = { workspace = true }

[dev-dependencies]
yantrik-app-test = { workspace = true }
```

### `build.rs`

```rust
fn main() {
    yantrik_app_build::compile("ui/weather.slint")
        .expect("Weather UI must compile");
}
```

### `src/main.rs`

```rust
use std::process::ExitCode;
use yantrik_weather::WeatherApp;

fn main() -> ExitCode {
    yantrik_sdk::app::run::<WeatherApp>()
}
```

### `src/lib.rs`

```rust
mod app;
mod event;
mod features;
mod model;
mod recovery;
mod settings;
mod view;

pub use app::WeatherApp;
```

There is one public application entry type. Features remain private to the crate.

## 10.3 Composition

### `src/features/mod.rs`

```rust
pub(crate) mod locations;
pub(crate) mod weather;
```

### `src/model.rs`

```rust
use crate::{features, settings::Settings};

pub struct Model {
    pub(crate) weather: features::weather::Model,
    pub(crate) search: features::locations::Model,
    pub(crate) settings: Settings,
}

impl Model {
    pub(crate) fn new(settings: Settings) -> Self {
        Self {
            weather: features::weather::Model::new(
                settings.location.clone(),
                settings.units,
            ),
            search: features::locations::Model::default(),
            settings,
        }
    }
}
```

`Model` is `pub` because it is an associated type of the public application trait implementation, but its module and fields are not an application API.

### `src/event.rs`

```rust
use yantrik_sdk::prelude::*;

pub enum Event {
    ForecastFinished(AppResult<ForecastBundle>),
    SearchFinished(AppResult<Vec<Location>>),
    AutoRefresh,
}
```

### `src/app.rs`

```rust
use crate::{
    event::Event,
    features::{locations, weather},
    model::Model,
    recovery,
    settings::Settings,
    view::WeatherView,
};
use yantrik_sdk::prelude::*;

pub struct WeatherApp;

impl Application for WeatherApp {
    type Model = Model;
    type Event = Event;
    type Settings = Settings;
    type View = WeatherView;

    fn manifest() -> AppManifest {
        AppManifest {
            id: AppId::new("org.yantrik.weather"),
            display_name: message_key!("weather.app_name"),
            version: env!("CARGO_PKG_VERSION"),
            instance: InstancePolicy::Single,
            launch: LaunchSupport {
                files: false,
                uris: false,
            },
            capabilities: &[
                Capability::Weather,
                Capability::MindCompanion,
            ],
        }
    }

    fn register(reg: &mut AppRegistry<Self>) -> AppResult<()> {
        reg.feature::<weather::Feature>()?;
        reg.feature::<locations::Feature>()?;
        reg.agent_rail(AgentRailOptions {
            context: AgentContextPolicy::VisibleAppState,
            suggestions: SuggestionPolicy::Subscribe,
            allow_ask: true,
        });
        reg.recovery(RecoveryPolicy::on_change());
        Ok(())
    }

    fn restore(
        ctx: RestoreContext<Self>,
    ) -> BoxFuture<'static, AppResult<Model>> {
        Box::pin(async move {
            let mut model = Model::new(ctx.settings().value.as_ref().clone());
            recovery::restore(&mut model, ctx.storage()).await?;
            Ok(model)
        })
    }

    fn launch(
        _model: &Model,
        launch: &Launch,
        _commands: &mut LaunchCommands<Self>,
    ) -> AppResult<()> {
        launch.require_no_targets()
    }

    fn ready(
        model: &mut Model,
        ctx: &mut UpdateContext<Self>,
    ) -> AppResult<()> {
        ctx.every(
            ScopeId::app(),
            TimerKey::new("weather.auto-refresh"),
            Duration::from_secs(15 * 60),
            || Event::AutoRefresh,
        );

        if model.weather.location.is_some() {
            ctx.enqueue(weather::Refresh {});
        }
        Ok(())
    }

    fn update(
        model: &mut Model,
        event: Event,
        ctx: &mut UpdateContext<Self>,
    ) -> AppResult<()> {
        match event {
            Event::ForecastFinished(result) => {
                model.weather.finish_refresh(result)?;
            }
            Event::SearchFinished(result) => {
                model.search.finish(result)?;
            }
            Event::AutoRefresh => {
                if model.weather.location.is_some() {
                    ctx.enqueue(weather::Refresh {});
                }
            }
        }
        Ok(())
    }

    fn settings_changed(
        model: &mut Model,
        settings: &Settings,
        ctx: &mut UpdateContext<Self>,
    ) -> AppResult<()> {
        let changed_location =
            model.weather.location != settings.location;

        model.settings = settings.clone();
        model.weather.apply_settings(settings);

        if changed_location && settings.location.is_some() {
            ctx.enqueue(weather::Refresh {});
        }
        Ok(())
    }

    fn checkpoint(
        model: &Model,
        _ctx: &CheckpointContext,
    ) -> AppResult<RecoveryBatch> {
        recovery::snapshot(model)
    }
}
```

Weather needs no custom close or shutdown hook. Its jobs, timers, stores, sockets, and rail are framework-owned.

### `src/settings.rs`

```rust
use yantrik_sdk::prelude::*;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    pub location: Option<LocationId>,
    pub units: Units,
}

impl SettingsSchema for Settings {
    const VERSION: u32 = 2;

    fn migrate(
        from: u32,
        mut value: serde_json::Value,
    ) -> Result<serde_json::Value, SettingsError> {
        match from {
            1 => {
                let metric = value["metric"]
                    .as_bool()
                    .ok_or(SettingsError::InvalidLegacyDocument)?;

                let object = value
                    .as_object_mut()
                    .ok_or(SettingsError::InvalidLegacyDocument)?;

                object.remove("metric");
                object.insert(
                    "units".into(),
                    serde_json::json!(
                        if metric { "metric" } else { "imperial" }
                    ),
                );
                Ok(value)
            }
            other => Err(SettingsError::UnsupportedVersion(other)),
        }
    }

    fn validate(&self) -> Result<(), SettingsError> {
        if let Some(location) = &self.location {
            location.validate()?;
        }
        Ok(())
    }
}
```

The store performs sequential migrations until the current version. Current documents are deserialized and validated directly.

### `src/recovery.rs`

```rust
use crate::model::Model;
use yantrik_sdk::prelude::*;

const SEARCH: RecoveryKey = RecoveryKey::new("location-search");

#[derive(Serialize, Deserialize)]
struct SearchDraft {
    query: String,
}

pub(crate) async fn restore(
    model: &mut Model,
    storage: &AppStorage,
) -> AppResult<()> {
    if let Some(draft) = storage
        .recovery()
        .read::<SearchDraft>(SEARCH, 1)
        .await?
    {
        model.search.restore_query(draft.query);
    }
    Ok(())
}

pub(crate) fn snapshot(model: &Model) -> AppResult<RecoveryBatch> {
    RecoveryBatch::json(
        SEARCH,
        1,
        &SearchDraft {
            query: model.search.query.clone(),
        },
    )
}
```

A real Weather app may also cache forecast data with a timestamp in `cache/`. That is not a settings field and not a recovery draft.

## 10.4 Weather feature

### `features/weather/mod.rs`

```rust
mod commands;
mod model;
mod surface;
pub(crate) mod view;

#[cfg(test)]
mod tests;

pub(crate) use commands::{Refresh, SelectLocation, SetUnits};
pub(crate) use model::Model;

use crate::WeatherApp;
use yantrik_sdk::prelude::*;

pub(crate) struct Feature;

impl yantrik_sdk::app::Feature<WeatherApp> for Feature {
    const ID: FeatureId = FeatureId::new("weather");

    fn install(reg: &mut FeatureRegistry<WeatherApp>) -> AppResult<()> {
        commands::register(reg)?;
        surface::register(reg)
    }
}
```

### `features/weather/model.rs`

```rust
use crate::settings::Settings;
use yantrik_sdk::prelude::*;

pub(crate) struct Model {
    pub(crate) location: Option<LocationId>,
    pub(crate) units: Units,
    pub(crate) forecast: Option<ForecastBundle>,
    pub(crate) refreshing: bool,
}

impl Model {
    pub(crate) fn new(
        location: Option<LocationId>,
        units: Units,
    ) -> Self {
        Self {
            location,
            units,
            forecast: None,
            refreshing: false,
        }
    }

    pub(crate) fn apply_settings(&mut self, settings: &Settings) {
        if self.location != settings.location {
            self.forecast = None;
        }
        self.location = settings.location.clone();
        self.units = settings.units;
    }

    pub(crate) fn finish_refresh(
        &mut self,
        result: AppResult<ForecastBundle>,
    ) -> AppResult<()> {
        self.refreshing = false;
        self.forecast = Some(result?);
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("no location selected")]
    NoLocation,

    #[error("location is not a current search result")]
    UnknownLocation,
}

// Implements DomainError with stable codes:
// weather.location_required
// weather.location_unknown
//
// Converts into AppError::Domain.
```

The current forecast remains visible during a failed refresh. The view can show it as stale using its observation timestamp and operation failure.

### `features/weather/commands.rs`

```rust
use super::model::Error;
use crate::{event::Event, model::Model, WeatherApp};
use yantrik_sdk::prelude::*;

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Refresh {}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SelectLocation {
    pub location: LocationId,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetUnits {
    pub units: Units,
}

impl Command for Refresh {
    type Output = ();
    const ID: CommandId = CommandId::new("weather.refresh");
}

impl Command for SelectLocation {
    type Output = ();
    const ID: CommandId =
        CommandId::new("weather.select-location");
}

impl Command for SetUnits {
    type Output = ();
    const ID: CommandId = CommandId::new("weather.set-units");
}

pub(crate) fn register(
    reg: &mut FeatureRegistry<WeatherApp>,
) -> AppResult<()> {
    reg.command(
        CommandSpec::new(message_key!("weather.refresh"))
            .policy(policies::weather_read),
        refresh,
    )?;

    reg.command(
        CommandSpec::new(message_key!("weather.select_location"))
            .policy(policies::change_app_preference),
        select_location,
    )?;

    reg.command(
        CommandSpec::new(message_key!("weather.set_units"))
            .policy(policies::change_app_preference),
        set_units,
    )
}

fn refresh(
    model: &mut Model,
    _: Refresh,
    ctx: &mut CommandContext<WeatherApp>,
) -> AppResult<Outcome<()>> {
    let location = model.weather.location
        .clone()
        .ok_or_else(|| AppError::from(Error::NoLocation))?;

    model.weather.refreshing = true;

    let operation = ctx.spawn(
        JobSpec::replace_previous(
            "weather.forecast",
            Duration::from_secs(12),
        ),
        move |task| Box::pin(async move {
            let client = task.services()
                .client::<WeatherClient>();

            let forecast = client.forecast(
                location,
                ForecastDays::Seven,
                task.call_options(Duration::from_secs(10)),
            ).await?;

            Ok(forecast)
        }),
        Event::ForecastFinished,
    );

    Ok(Outcome::Accepted(operation))
}

fn select_location(
    model: &mut Model,
    command: SelectLocation,
    ctx: &mut CommandContext<WeatherApp>,
) -> AppResult<Outcome<()>> {
    if !model.search.contains(&command.location) {
        return Err(Error::UnknownLocation.into());
    }

    let mut settings = model.settings.clone();
    settings.location = Some(command.location);

    // The store commits first; settings_changed then applies the
    // new location and queues refresh. Failure leaves selection intact.
    let operation = ctx.persist_settings(settings)?;
    Ok(Outcome::Accepted(operation))
}

fn set_units(
    model: &mut Model,
    command: SetUnits,
    ctx: &mut CommandContext<WeatherApp>,
) -> AppResult<Outcome<()>> {
    let mut settings = model.settings.clone();
    settings.units = command.units;

    let operation = ctx.persist_settings(settings)?;
    Ok(Outcome::Accepted(operation))
}
```

Preference writes use the current settings revision and are serialized by the settings facility. A conflicting edit fails explicitly; it does not overwrite another edit.

Changing units reformats the existing forecast. It does not refetch identical meteorological data.

The weather-read policy also accounts for transmitting a location to the service. “Read-only” is not synonymous with “privacy-free.”

### `features/weather/surface.rs`

```rust
use super::{Refresh, SelectLocation, SetUnits};
use crate::{model::Model, WeatherApp};
use yantrik_sdk::prelude::*;

#[derive(Serialize, JsonSchema)]
struct WeatherDescription {
    location: Option<LocationId>,
    units: Units,
    refreshing: bool,
    current: Option<CurrentConditions>,
    forecast: Vec<DailyForecast>,
}

pub(crate) fn register(
    reg: &mut FeatureRegistry<WeatherApp>,
) -> AppResult<()> {
    reg.expose::<Refresh>(Exposure::public())?;
    reg.expose::<SelectLocation>(Exposure::public())?;
    reg.expose::<SetUnits>(Exposure::public())?;
    reg.describe(StateSection::new("weather"), describe)
}

fn describe(
    model: &Model,
    caller: &DescribeContext,
) -> AppResult<WeatherDescription> {
    caller.require(Visibility::Location)?;

    let forecast = model.weather.forecast.as_ref();

    Ok(WeatherDescription {
        location: model.weather.location.clone(),
        units: model.weather.units,
        refreshing: model.weather.refreshing,
        current: forecast.map(|f| f.current.clone()),
        forecast: forecast
            .map(|f| f.days.iter().take(7).cloned().collect())
            .unwrap_or_default(),
    })
}
```

The service contract validates its forecast bundle. The app also caps the exported seven-day list.

### `features/weather/view.rs`

```rust
use super::{Refresh, SelectLocation, SetUnits};
use crate::{model::Model, view::WeatherWindow, WeatherApp};
use yantrik_sdk::prelude::*;

pub(crate) fn bind(
    ui: &WeatherWindow,
    port: &UiPort<WeatherApp>,
) {
    ui.on_refresh(port.action(|| Refresh {}));

    ui.on_select_location(port.action1(|id: slint::SharedString| {
        SelectLocation {
            location: LocationId::from_untrusted(id.as_str()),
        }
    }));

    ui.on_set_metric(port.action1(|metric| SetUnits {
        units: if metric { Units::Metric } else { Units::Imperial },
    }));
}

pub(crate) fn render(ui: &WeatherWindow, model: &Model) {
    ui.set_refreshing(model.weather.refreshing);
    ui.set_has_location(model.weather.location.is_some());

    // Formats with locale-aware SDK formatters.
    // Updates current-condition properties and keyed forecast rows.
    // No service calls or domain decisions.
    project_forecast(ui, &model.weather);
}
```

`from_untrusted` preserves input for normal command validation; it must not panic or silently substitute a default ID.

### `features/weather/tests.rs`

```rust
#[test]
fn changing_units_does_not_discard_forecast() {
    let mut model = fixture_weather_model();
    let previous = model.forecast.clone();

    model.apply_settings(&settings_with_units(Units::Imperial));

    assert_eq!(model.forecast, previous);
}

#[test]
fn failed_refresh_retains_previous_forecast() {
    let mut model = fixture_weather_model();
    let previous = model.forecast.clone();

    assert!(model.finish_refresh(Err(timeout_error())).is_err());
    assert!(!model.refreshing);
    assert_eq!(model.forecast, previous);
}
```

## 10.5 Locations feature

### `features/locations/mod.rs`

```rust
mod commands;
mod model;
mod surface;
pub(crate) mod view;

#[cfg(test)]
mod tests;

pub(crate) use commands::SearchLocations;
pub(crate) use model::Model;

use crate::WeatherApp;
use yantrik_sdk::prelude::*;

pub(crate) struct Feature;

impl yantrik_sdk::app::Feature<WeatherApp> for Feature {
    const ID: FeatureId = FeatureId::new("locations");

    fn install(reg: &mut FeatureRegistry<WeatherApp>) -> AppResult<()> {
        commands::register(reg)?;
        surface::register(reg)
    }
}
```

### `features/locations/model.rs`

```rust
use yantrik_sdk::prelude::*;

#[derive(Default)]
pub(crate) struct Model {
    pub(crate) query: String,
    pub(crate) results: Vec<Location>,
    pub(crate) searching: bool,
}

impl Model {
    pub(crate) fn contains(&self, id: &LocationId) -> bool {
        self.results.iter().any(|location| &location.id == id)
    }

    pub(crate) fn restore_query(&mut self, query: String) {
        self.query = query;
    }

    pub(crate) fn finish(
        &mut self,
        result: AppResult<Vec<Location>>,
    ) -> AppResult<()> {
        self.searching = false;
        self.results = result?;
        Ok(())
    }
}
```

### `features/locations/commands.rs`

```rust
use crate::{event::Event, model::Model, WeatherApp};
use yantrik_sdk::prelude::*;

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchLocations {
    pub query: String,
}

impl Command for SearchLocations {
    type Output = ();
    const ID: CommandId =
        CommandId::new("weather.search-locations");
}

pub(crate) fn register(
    reg: &mut FeatureRegistry<WeatherApp>,
) -> AppResult<()> {
    reg.command(
        CommandSpec::new(message_key!("weather.search_locations"))
            .policy(policies::location_search),
        search,
    )
}

fn search(
    model: &mut Model,
    command: SearchLocations,
    ctx: &mut CommandContext<WeatherApp>,
) -> AppResult<Outcome<()>> {
    let query = SearchQuery::parse(&command.query)?;

    model.search.query = query.as_str().to_owned();
    model.search.searching = true;

    let operation = ctx.spawn(
        JobSpec::replace_previous(
            "weather.location-search",
            Duration::from_secs(8),
        ),
        move |task| Box::pin(async move {
            let client = task.services().client::<WeatherClient>();

            Ok(client.search(
                query,
                task.call_options(Duration::from_secs(6)),
            ).await?)
        }),
        Event::SearchFinished,
    );

    Ok(Outcome::Accepted(operation))
}
```

For search-as-you-type, the Rust view binding uses the owned debounce facility. This example submits on activation, so there is no need for a debounce timer.

### `features/locations/surface.rs`

```rust
use super::SearchLocations;
use crate::{model::Model, WeatherApp};
use yantrik_sdk::prelude::*;

#[derive(Serialize, JsonSchema)]
struct SearchDescription {
    query: String,
    searching: bool,
    results: Vec<Location>,
}

pub(crate) fn register(
    reg: &mut FeatureRegistry<WeatherApp>,
) -> AppResult<()> {
    reg.expose::<SearchLocations>(Exposure::public())?;

    reg.describe(
        StateSection::new("locations"),
        |model: &Model, caller| {
            caller.require(Visibility::Location)?;

            Ok(SearchDescription {
                query: model.search.query.clone(),
                searching: model.search.searching,
                results: model.search.results.clone(),
            })
        },
    )
}
```

### `features/locations/view.rs`

```rust
use super::SearchLocations;
use crate::{model::Model, view::WeatherWindow, WeatherApp};
use yantrik_sdk::prelude::*;

pub(crate) fn bind(
    ui: &WeatherWindow,
    port: &UiPort<WeatherApp>,
) {
    ui.on_search(
        port.action1(|text: slint::SharedString| SearchLocations {
            query: text.to_string(),
        }),
    );
}

pub(crate) fn render(ui: &WeatherWindow, model: &Model) {
    ui.set_searching(model.search.searching);
    ui.set_search_query(model.search.query.clone().into());

    // Converts Location values to keyed Slint presentation rows.
    ui.set_locations(location_rows(&model.search.results));
}
```

### `features/locations/tests.rs`

```rust
#[test]
fn selection_requires_a_current_result() {
    let model = fixture_search_results();
    assert!(model.contains(&LocationId::fixture("london")));
    assert!(!model.contains(&LocationId::fixture("unknown")));
}

#[test]
fn invalid_query_is_rejected_before_state_changes() {
    assert!(SearchQuery::parse("").is_err());
    assert!(SearchQuery::parse(&"x".repeat(10_000)).is_err());
}
```

## 10.6 Root view and Slint

### `src/view.rs`

```rust
use crate::{features, model::Model, WeatherApp};
use yantrik_sdk::prelude::*;

slint::include_modules!();

pub struct WeatherView {
    ui: WeatherWindow,
}

yantrik_sdk::ui::impl_window_ports! {
    WeatherView => ui: WeatherWindow,
    theme: Theme,
    chrome: WindowChrome,
    notices: NoticeBridge,
    rail: AgentRailBridge,
}

impl AppView<WeatherApp> for WeatherView {
    fn create() -> AppResult<Self> {
        Ok(Self {
            ui: WeatherWindow::new()?,
        })
    }

    fn bind(&self, port: UiPort<WeatherApp>) {
        features::weather::view::bind(&self.ui, &port);
        features::locations::view::bind(&self.ui, &port);
    }

    fn render(&self, model: &Model, presentation: &Presentation) {
        features::weather::view::render(&self.ui, model);
        features::locations::view::render(&self.ui, model);
        presentation.apply_notices(&self.ui);
    }
}
```

### `ui/weather.slint`

Illustrative markup using the versioned kit contract:

```slint
import {
    AppWindow,
    PageHeader,
    SearchField,
    Button,
    CurrentWeatherCard,
    ForecastStrip,
    LocationResults,
    EmptyState,
    NoticeStack,
    AgentRail,
    Theme,
    WindowChrome,
    NoticeBridge,
    AgentRailBridge
} from "yantrik-ui-kit.slint";

export { Theme, WindowChrome, NoticeBridge, AgentRailBridge }

export struct LocationRow {
    id: string,
    title: string,
    subtitle: string,
}

export struct ForecastRow {
    day: string,
    condition: string,
    high: string,
    low: string,
}

export component WeatherWindow inherits AppWindow {
    in property <bool> refreshing;
    in property <bool> searching;
    in property <bool> has-location;
    in-out property <string> search-query;

    in property <[LocationRow]> locations;
    in property <[ForecastRow]> forecast;

    in property <string> current-temperature;
    in property <string> current-condition;
    in property <string> location-title;

    callback refresh();
    callback search(string);
    callback select-location(string);
    callback set-metric(bool);

    content := HorizontalLayout {
        VerticalLayout {
            PageHeader {
                title: @tr("Weather");
                subtitle: root.location-title;
            }

            HorizontalLayout {
                SearchField {
                    text <=> root.search-query;
                    busy: root.searching;
                    submitted(text) => { root.search(text); }
                }

                Button {
                    text: @tr("Refresh");
                    busy: root.refreshing;
                    enabled: root.has-location;
                    clicked => { root.refresh(); }
                }
            }

            LocationResults {
                rows: root.locations;
                selected(id) => { root.select-location(id); }
            }

            if root.has-location: CurrentWeatherCard {
                temperature: root.current-temperature;
                condition: root.current-condition;
            }

            if root.has-location: ForecastStrip {
                rows: root.forecast;
            }

            if !root.has-location: EmptyState {
                title: @tr("Weather, wherever you are");
                description: @tr("Search for a location to begin.");
            }

            NoticeStack { }
        }

        AgentRail { }
    }
}
```

`CurrentWeatherCard` and `ForecastStrip` may initially be Weather-local visual components rather than global kit controls. Promote them only when a second genuine consumer exists. Their styling still uses kit tokens and primitives.

The `.slint` file contains layout, visual conditions, and callback forwarding. No forecast fetching, unit conversion, policy checks, or persistence.

“Best-looking” comes from a stronger visual contract than merely banning raw colors:

- tokenized typography and spacing;
- consistent focus, hover, pressed, loading, empty, and error states;
- deliberate responsive layouts;
- accessible labels and keyboard traversal;
- restrained, tokenized motion with reduced-motion support;
- visual review of every state, not only the happy-path screenshot.

The framework makes these states easy to render. It cannot replace design work.

## 10.7 Integration tests

### `tests/surface.rs`

```rust
#[test]
fn agent_can_search_select_and_refresh_with_policy() {
    test_runtime(async {
        let weather = FakeWeather::with_london_forecast();
        let mut app = weather_harness(weather).await;

        let reply = app.act(
            TestCaller::approved_agent(),
            "weather.search-locations",
            json!({ "query": "London" }),
        ).await;
        assert!(reply.is_accepted());

        app.settle().await;

        let state = app.describe(TestCaller::approved_agent()).await;
        let id = state.pointer("/state/locations/results/0/id").unwrap();

        app.act(
            TestCaller::approved_agent(),
            "weather.select-location",
            json!({ "location": id }),
        ).await;

        app.settle().await;

        let state = app.describe(TestCaller::approved_agent()).await;
        assert_eq!(
            state.pointer("/state/weather/forecast").unwrap()
                .as_array().unwrap().len(),
            7,
        );
    });
}
```

Also test denial, malformed IDs, late search results, and location privacy.

### `tests/lifecycle.rs`

```rust
#[test]
fn close_cancels_network_work_and_preserves_search_draft() {
    test_runtime(async {
        let mut app = harness_with_blocked_weather_service().await;

        app.ui(SearchLocationsFixture::command("London")).await.unwrap();
        app.close(CloseReason::Test).await.unwrap();

        assert_no_live_tasks();
        assert_recovery_query("London");
    });
}
```

In real integration code, test helpers send the public surface command or a supported test fixture API; private command types need not be exported.

### `tests/visual.rs`

```rust
#[test]
fn weather_states() {
    ui_matrix::<WeatherApp>()
        .themes([Light, Dark])
        .widths([Compact, Wide])
        .text_scales([Normal, Large])
        .fixtures(["empty", "loaded", "refreshing", "offline"])
        .assert_snapshots();
}
```

---

# 11. Migration without breaking nightly

Do not convert 18 applications in one framework PR.

## Phase 0: record the behavior to preserve

For each app, capture:

- canonical identity and legacy aliases;
- current action names and argument formats;
- launch behavior;
- persistence paths;
- shutdown obligations;
- essential surface and UI behavior.

Create golden protocol fixtures and a short compatibility manifest. This prevents “cleanup” from accidentally deleting working automation.

Add CI measurements for:

- app startup;
- peak build memory;
- action coverage;
- main/file size;
- generated UI compilation time.

## Phase 1: extract boring shared primitives

Land independently, with the old runtime forwarding to them:

1. canonical XDG paths;
2. atomic JSON writes and migrations;
3. tracing/problem reporting;
4. typed service-client construction and timeouts;
5. shared headless Slint platform;
6. theme subscription;
7. instance handoff;
8. shared rail visual component.

Existing binaries continue to run unchanged.

These are valuable even before the new app trait exists.

## Phase 2: land the core and Weather pilot

Weather is a good first pilot because it exercises:

- settings;
- asynchronous services;
- cancellation and stale results;
- public actions;
- theme;
- rail;
- instance lifecycle;

without risking document loss.

Migrate Weather behind a build/packaging switch. Do not ship two competing instance owners under one identity.

Run legacy and new implementations in isolated test sandboxes against the same protocol fixtures. The nightly package selects exactly one.

Success gates:

- no regression in its existing supported actions;
- the four typed actions work through the real surface codec;
- no hand-written thread, timer ownership, theme loading, or instance claim;
- live theme and settings changes work;
- shutdown and interrupted-request tests pass;
- build memory remains within budget.

## Phase 3: prove persistence and destructive policy

Next migrate a small document app—not email immediately.

Use Text Editor or a narrowly scoped Notes migration to prove:

- unsaved-close confirmation;
- debounced draft recovery;
- filesystem resource policy;
- dangerous actions and approval;
- launch handoff with paths;
- recovery after termination.

Do not declare the framework complete after a read-only app.

## Phase 4: migrate one app at a time

Suggested order:

1. simple informational apps;
2. settings and utility apps;
3. document apps;
4. calendar and notes;
5. email and other engine-heavy apps.

For each app:

1. convert to `lib + bin` without changing behavior;
2. move model code into feature modules;
3. add harness coverage;
4. adopt the runner;
5. migrate commands feature by feature;
6. replace private storage/service helpers;
7. replace rail copies;
8. remove legacy adapters.

A migrated app has one dispatcher. A compatibility action adapter parses an old wire shape and creates the new typed command; it does not invoke a legacy handler alongside the new one.

## Legacy compatibility

- Legacy action aliases are marked deprecated and omitted from preferred discovery.
- Old claim/surface ID mismatches are reconciled through a migration alias map.
- During the transition, new versions recognize old lock identities and old installed owners.
- There is one authoritative owner, not independently claimed locks that permit two instances.
- Old data paths are imported only when the canonical store is absent and the import journal permits it.
- Existing files are backed up before format migration.

## What can be codemodded

Good candidates:

- `main.rs` extraction into library entry;
- imports from the old runtime to the SDK facade;
- canonical home/path helpers;
- mechanical theme/chrome plumbing;
- common callback-to-command binding shapes;
- repeated notice helpers;
- standard `present` aliases;
- common Slint test-platform setup;
- relocation of identical rail markup.

Not safe to codemod blindly:

- security policy;
- agent rules;
- command argument semantics;
- state ownership;
- shutdown ordering;
- data migrations;
- transactional document operations.

These require behavioral understanding and review.

## Deprecation finish line

Remove the old runtime only after:

- all shipping apps use the new runner;
- no application depends on its legacy control builder;
- supported alias windows have elapsed;
- recovery and instance migration tests cover upgrades from the last legacy release.

A calendar date alone is not a sufficient removal condition.

---

# 12. Trade-offs and explicit refusals

## Trade-offs

### One UI-thread model instead of shared mutable state

**Gain:** predictable ordering, simple ownership, reproducible tests, safe Slint integration.

**Cost:** expensive model calculations must be deliberately moved to workers. Large collections need incremental projections.

### Typed commands instead of arbitrary closures

**Gain:** one implementation for UI and agents, generated schemas, centralized policy, traceability.

**Cost:** small command structs and registration code. That is worthwhile boilerplate.

### Explicit public action exports

**Gain:** a stable, reviewable agent API.

**Cost:** not every internal action is automatically discoverable. That is intentional.

### An application aggregate instead of a plugin-style feature container

**Gain:** ordinary Rust navigation and borrowing, straightforward refactoring.

**Cost:** features are coupled to their host app unless explicitly generalized. We should not pay plugin-system complexity for modules that ship together.

### Framework-owned runtime instead of app-specific executors

**Gain:** bounded resource use and enforceable shutdown.

**Cost:** apps with unusual engines need explicit integration adapters.

### Small supporting macros, not a macro framework

**Gain:** less mechanical Slint plumbing.

**Cost:** some handwritten registration remains. Debuggable Rust is more important than eliminating every line.

### Versioned settings and recovery, not one universal database

**Gain:** clear lifecycle and location rules without imposing a storage engine.

**Cost:** sophisticated apps still design their own durable domain storage.

## What I would refuse to build

- A giant `BaseApp` struct containing every possible service and feature.
- A deep inheritance simulation using traits and blanket implementations.
- A generic “manager” for windows, commands, state, jobs, and persistence.
- A stringly typed event bus as the primary app architecture.
- A surface handler that bypasses the normal command policy.
- Agent detection based on a caller-supplied boolean.
- A default policy that treats every local socket caller as the person.
- Global `Arc<Mutex<AppState>>`.
- A separate Tokio runtime or worker thread per feature.
- A promise to kill arbitrary blocking Rust work safely.
- An assumption that shutdown always runs.
- A universal JSON dump of the entire model.
- A universal UI DSL layered on top of Slint.
- A monolithic generated UI crate for all apps.
- A framework that “solves” modularity by moving a 2,000-line `main` into a 2,000-line `App::wire`.

The acceptance criterion is not that Weather’s `main.rs` becomes five lines. That is easy.

The real criterion is that the twentieth app obtains launch handoff, live theme, typed actions, agent approval, supervised jobs, recovery, notices, and testability **by adopting the application contract—and still reads like a small, understandable Rust program.**
