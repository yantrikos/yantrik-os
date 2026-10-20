# Yantrik Platform SDK

Yantrik should become a platform of **small, named frameworks over shared system services**, not a collection of applications that happen to reuse some crates.

The central architectural rule should be:

> An application imports public frameworks. A framework either performs local work or calls an authenticated system service. It never imports the shell, the Mind implementation, or another application.

I would keep Rust, Slint, Wayland, labwc, and the existing services. The first release should mostly **name, narrow, document, and standardise what already exists**, not replace it.

All API names, manifests, and examples below are proposed target APIs, not claims about APIs available today.

---

## 1. Framework catalogue

### Naming convention

Use three distinct names for each framework:

- **Product/documentation name:** `YantrikNotifications`
- **Cargo package:** `yantrik-notifications`
- **Rust namespace:** `yantrik_sdk::notifications`, or directly `yantrik_notifications`

Public framework packages must not also be daemon binaries. For example, the notifications service becomes an internal package such as `yantrik-notificationsd`; `yantrik-notifications` is the public client.

In the table:

- **Local** means code running in the application.
- **Existing service** means a daemon already present.
- **New broker** means proposed infrastructure, introduced incrementally.
- “New” explicitly means there is no supplied existing implementation to rename.

### Core and application frameworks

| Framework / package | Purpose and principal public APIs | Contains—and deliberately excludes | Execution/backend | Today → tomorrow |
|---|---|---|---|---|
| **YantrikFoundation** `yantrik-foundation` | Shared platform vocabulary: `AppId`, `ResourceId`, `LocaleId`, `Timestamp`, `Deadline`, `Cancellation`, `PlatformError`, `Availability`. | Cross-framework identifiers and semantics. **Not** a replacement for Rust `std`, a general utilities bucket, or an IPC runtime. | Local. | Extract genuinely shared types from app-runtime and IPC contracts; add missing types. |
| **YantrikUI** `yantrik-ui` | Design language and Slint components: `YButton`, `YInput`, `AppHeader`, `Dialog`, `Theme`, `ThemeMode`, semantic icons, focus and layout conventions. | Tokens, components, small Rust bridges, accessibility defaults. **Not** desktop chrome, chat, application lifecycle, or business logic. | Local Slint resources; theme updates through preferences. | `yantrik-design-tokens` + `yantrik-ui-kit` → YantrikUI. **Rename the current shell package first** to free `yantrik-ui`. |
| **YantrikApplication** `yantrik-application` | Launch and lifecycle: `Application`, `AppContext`, `LaunchRequest`, `Activation`, `WindowHandle`, `Menu`, `Command`, `BackgroundLease`. | Window registration, activation, single-instance routing, menus and command dispatch. **Not** compositor implementation or arbitrary persistent background execution. | Local runtime and Slint adapter; proposed session application broker; Wayland/labwc and portals. | Lifecycle/window portions of `yantrik-app-runtime` → YantrikApplication. |
| **YantrikAppIntents** `yantrik-app-intents` | Typed, agent-operable applications: `Intent`, `IntentDescriptor`, `StateSnapshot`, `EntityRef`, `Invocation`, `IntentResult`, derive macros and registry. | `describe`/`act`, schemas, action discovery, state projection and invocation receipts. **Not** an LLM runtime or an approval policy engine. | Local handlers; system discovery/router and Security authorization. | `yantrik-surface` + app-runtime registration → YantrikAppIntents; existing wire protocol retained initially. |
| **YantrikPreferences** `yantrik-preferences` | Typed settings: `Preferences`, `PreferenceKey<T>`, `observe`, `transaction`, `SystemAppearance`, `ManagedValue`. | App-owned settings and controlled system-setting access. **Not** arbitrary document storage or secrets. | App-local durable store for private settings; small shared settings broker for system preferences and policy. | App-runtime theme sync + selected shell Settings infrastructure; new typed storage API. |
| **YantrikLocalization** `yantrik-localization` | Localised messages and formats: `Message`, `Locale`, `DateFormatter`, `NumberFormatter`, plural and direction support. | Message catalogues, Slint integration, formatting and resource fallback. **Not** model-based translation or application content translation. | Local; locale changes from Preferences. | New framework; integrate Slint’s localisation facilities rather than create a competing markup mechanism. |

### Data, communication, and device frameworks

| Framework / package | Purpose and principal public APIs | Contains—and deliberately excludes | Execution/backend | Today → tomorrow |
|---|---|---|---|---|
| **YantrikNotifications** `yantrik-notifications` | User-visible notifications: `NotificationCenter`, `Notification`, `Category`, `NotificationAction`, `post`, `withdraw`. | Declared categories, quiet-mode behaviour, action routing and history access scoped to the caller. **Not** approval cards or arbitrary shell overlays. | Existing notifications daemon; compatible freedesktop notification entry point. | Notifications IPC contract/client code → public framework; daemon remains internal. |
| **YantrikFiles** `yantrik-files` | User-mediated file access: `FilePicker`, `FileGrant`, `FileHandle`, `DirectoryHandle`, `MimeType`, `FileWatch`. | Pick/open/save, granted handles, metadata, streams, portal integration. **Not** unrestricted filesystem authority disguised as a safe API. | Local I/O on granted handles; XDG desktop portals and launch broker. | Extract Files/Downloads and runtime helpers where suitable; new grant abstraction. |
| **YantrikDocuments** `yantrik-documents` | Document-oriented applications: `Document`, `DocumentSession`, `DocumentCodec`, `DirtyState`, `SaveCoordinator`, `RecoveryStore`. | Open/save/save-as, atomic saves where supported, conflict handling, recovery and recent-document references. **Not** DOCX, PDF or spreadsheet engines. | Local; Files and application activation. | Extract reusable behaviour from Notes and the office trio; initially preview. |
| **YantrikStorage** `yantrik-storage` | App-private persistence: `AppDirectories`, `Database`, `Migration`, `Transaction`, `BlobStore`, `Cache`. | XDG-aligned app directories, SQLite-based persistence, migrations and cache policy. **Not** a cross-app object graph, credentials, or Mind memory. | Local SQLite/filesystem, constrained by sandbox and app identity. | New standard wrapper around app storage patterns. **Do not expose `yantrikdb` here.** |
| **YantrikCalendar** `yantrik-calendar` | Calendar data: `CalendarStore`, `Calendar`, `Event`, `Recurrence`, `query`, `create`, `observe`. | Permissioned events, time zones and recurrence semantics. **Not** Calendar’s UI or task execution by the Mind. | Existing calendar daemon. | Calendar IPC/client extraction → YantrikCalendar. |
| **YantrikContacts** `yantrik-contacts` | Person and address-book records: `ContactStore`, `Contact`, `ContactId`, `ContactPicker`, `query`, `update`. | Permissioned contact records and selection. **Not** inferred social graphs or automatic harvesting of email correspondents. | New contact provider/service, or an adapter to an available system provider. | New; explicitly preview until a real backend exists. |
| **YantrikMail** `yantrik-mail` | Permissioned mail integration: `MailStore`, `Mailbox`, `MessageSummary`, `Draft`, `ComposeRequest`, `SendReceipt`. | Scoped queries, composing, drafts and explicitly authorized sending. **Not** UI rendering of untrusted HTML or raw account credentials. | Existing email daemon; Security/vault. | Email IPC/client extraction → YantrikMail. |
| **YantrikNetwork** `yantrik-network` | Connectivity and policy-aware requests: `Connectivity`, `HttpClient`, `Request`, `NetworkScope`, `EgressContext`, `Response`. | Connectivity observation, HTTP, cancellation, redirect policy and attributed egress. **Not** network configuration, firewall bypass, or a promise that SDK use alone prevents raw sockets. | Existing network daemon for status; egress proxy for controlled requests. | Client side of network IPC + `yantrik-egress` → framework; proxy and network management remain internal. |
| **YantrikMedia** `yantrik-media` | Playback, media sessions and capture: `Player`, `MediaSource`, `MediaSession`, `CaptureSession`, `AudioDevice`, `TransportCommand`. | Common playback controls, MPRIS integration, permitted capture and device selection. **Not** a new codec suite, DAW, image editor, or unrestricted microphone access. | Local adapters to established media libraries; PipeWire and capture portals. | Extract useful Music/Images code; new common API, preview initially. |
| **YantrikSystem** `yantrik-system` | Read-only system facts: `SystemInfo`, `PowerState`, `ResourceSample`, `DeviceSummary`, `observe`. | Version, power, resource metrics and appropriately scoped device information. **Not** unrestricted process inspection, process termination or privileged administration. | Existing system-monitor daemon; standard Linux services where appropriate. | System monitor IPC/client extraction → YantrikSystem. |
| **YantrikAccessibility** `yantrik-accessibility` | Accessible applications and assistive integrations: `AccessibleNode`, `Role`, `AccessibleAction`, `Announcement`, `AssistiveSession`. | Semantic metadata, accessible actions, announcements and consented assistive access. **Not** a general unpermissioned input injector or a replacement for AT-SPI. | Slint accessibility bridge, AT-SPI and existing a11y daemon; backend coverage must be audited. | UI-kit semantics + a11y IPC → YantrikAccessibility. |
| **YantrikWeather** `yantrik-weather` | Shared weather data: `Forecast`, `WeatherLocation`, `WeatherClient`, `observe`. | Cached forecast access with explicit location selection/permission. **Not** continuous location tracking. | Existing weather daemon. | Weather IPC/client extraction → small optional framework. |

### Trust and intelligence frameworks

| Framework / package | Purpose and principal public APIs | Contains—and deliberately excludes | Execution/backend | Today → tomorrow |
|---|---|---|---|---|
| **YantrikSecurity** `yantrik-security` | Platform trust: `Principal`, `PermissionRequest`, `PermissionState`, `CredentialRef`, `VaultClient`, `AuditReceipt`, `DataLabel`. | Identity inspection, scoped permission requests, vault references and authorization results. **Not** developer-defined approval bypasses, public arbitrary cryptographic primitives, or raw vault enumeration. | Proposed security authority, initially adapting existing approvals/vault/egress policy; trusted shell consent UI. | Existing peer-credential verification, vault and approval interfaces → public clients; policy implementation stays private. |
| **YantrikMind** `yantrik-mind` | Ask the Mind and contribute capabilities: `MindClient`, `Conversation`, `ModelOutput<T>`, `ToolDescriptor`, `MemoryCollection`, `MemoryQuery`, `MemoryProposal`. | Streaming requests, cancellation, explicitly contributed context, scoped memory and opt-in tool registration. **Not** cortex internals, unrestricted personal memory, model execution authority or direct `yantrikdb` access. | Existing companion/harness services through a versioned gateway; `yantrikdb` and `yantrik-ml` remain implementation details. | Selected companion/harness contracts → public client; no dependency on the 110k-line implementation. |

Two additional SDK products support these frameworks:

- **YantrikServiceKit — `yantrik-service-kit`:** service registration, authenticated endpoints, health checks, cancellation, tracing and graceful shutdown. Built from `yantrik-service-sdk` and internal IPC infrastructure. Initially preview; shipping a third-party persistent service requires installation policy.
- **YantrikTesting — `yantrik-testing`:** fake framework services, deterministic time, permission simulation, intent conformance testing and Slint test helpers. Development-only.

Not every existing service warrants a public framework:

- **Notes** remains an application domain, using Documents/Storage and exposing App Intents.
- **Perception and perception-journal** remain private initially. Applications receive narrowly scoped features through explicit permissioned APIs, not a public surveillance event stream.
- The **harness** remains an agent-host implementation behind Mind and Security.

---

## 2. Naming, namespaces, and dependencies

### Use both framework crates and a thin facade

Each framework gets its own package, API documentation, owners and tests. `yantrik-sdk` is a convenience facade containing **re-exports, not implementation**.

```rust
use yantrik_sdk::{
    application::Application,
    files::FilePicker,
    notifications::{Notification, NotificationCenter},
};
```

This is equivalent to importing the individual packages. A library author may prefer direct dependencies.

The facade must have:

```toml
[features]
default = []
```

Each framework is opt-in. Enabling Notifications must not pull in Slint, the Mind, SQLite or a media backend.

### Application Cargo manifest

```toml
[package]
name = "org.example.hello"
version = "0.1.0"
edition = "2024"
build = "build.rs"

[dependencies]
yantrik-sdk = {
    version = "1",
    default-features = false,
    features = ["application", "ui", "intents", "notifications"]
}
slint = "=1.17.0" # Illustrative: the SDK release pins a tested 1.17.x patch.

[build-dependencies]
yantrik-build = "1"

[dev-dependencies]
yantrik-testing = "1"
```

The generated template uses the actual tested patch release, not a floating untested Slint range.

`yantrik-build` compiles the manifest, validates schemas, configures Slint library paths and invokes the supported Slint compiler:

```rust
fn main() {
    yantrik_build::app("app.toml")
        .slint("ui/app.slint")
        .compile()
        .unwrap();
}
```

Keep Slint compiler dependencies in build tooling, not every runtime framework.

### Slint imports

```slint
import { YButton, AppHeader, Theme } from "@yantrik-ui";
```

Also provide stable, narrower entry points where useful:

```slint
import { DocumentToolbar } from "@yantrik-ui/documents.slint";
```

The build helper supplies these library mappings. Applications must not import:

```text
../../crates/yantrik-ui-kit/components/button.slint
```

Public Slint exports are versioned API, just like Rust types.

The SDK must document separately:

1. Rust source compatibility;
2. Slint markup compatibility;
3. runtime service protocol compatibility;
4. supported Slint compiler/backend versions.

Rust libraries do **not** provide a stable Rust ABI.

---

## 3. Public versus private

### Stability tiers

| Tier | Contract |
|---|---|
| **Stable** | Documented, reviewed, semver-governed APIs with conformance tests and a supported runtime compatibility range. |
| **Preview** | Explicit opt-in, separately versioned `0.x` packages or preview-only modules; breakage allowed only in announced preview releases. |
| **Internal** | No external compatibility promise; not importable by applications. |

The catalogue is the destination, not an instruction to declare twenty immature packages stable immediately. Stabilise a small useful slice first: UI, Application, Foundation, Notifications and basic App Intents.

### Versioning policy

- Give the stable SDK an annual major release and regular additive minor releases.
- Keep stable framework packages on a coordinated SDK release train initially.
- Existing API removal requires a major version.
- Deprecate for **at least two minor releases and twelve months**, whichever is longer.
- Support the current and preceding stable protocol major for at least twelve months.
- Provide `min_platform`, feature availability checks and a published SDK/runtime matrix.
- Unknown optional wire fields are tolerated. Unsupported operations return `Unsupported`, not silent no-ops.
- Security fixes may change enforcement immediately; compatibility is not a promise to preserve a vulnerability.

A nightly OS must keep running applications built against the supported stable SDK.

### CI enforcement

Maintain `sdk/public-api.toml` listing public packages, tiers and allowed dependencies.

CI checks:

- Application direct dependencies may target public SDK packages, approved external libraries and their own app-domain libraries—not OS-private packages.
- Public framework implementations may use narrowly designated internal client infrastructure; apps cannot depend on that infrastructure directly.
- No public signature leaks an internal type.
- No `sdk/` runtime library depends on `os/`, an app, or companion implementation crates.
- No app imports shell Slint files.
- Rust API diffs, Slint export diffs and protocol schema diffs are reviewed.
- A sample application builds **outside the workspace using published artifacts**.
- Old-client/new-service and new-client/old-service compatibility suites run.
- First-party apps have no blanket exemption.

The external-build test catches accidental path dependencies and workspace-only assumptions that architecture diagrams miss.

---

## 4. The application model

### Bundle

Use a simple bundle layout inside signed distribution packages:

```text
org.example.Notes.yapp/
  app.toml
  bin/notes
  resources/
  ui/
  locales/
  intents.json
  licenses/
```

The bundle is a packaging unit—not itself a sandbox or proof of trust.

Installation produces:

- a verified installation record;
- a registered application identity;
- `.desktop` entries and MIME associations;
- a resource index;
- approved entitlement ceilings;
- sandbox launch configuration;
- a validated intent catalogue.

For development, `yantrik run` registers a development installation with conspicuous trust status.

### Manifest

```toml
manifest_version = 1
id = "org.example.Notes"
name = "Notes"
version = "1.0.0"
exec = "bin/notes"
min_platform = "1.0"

[application]
single_instance = true
background = "lease"

[capabilities]
provides = ["documents", "intents"]
requires = ["application.v1", "files.v1"]

[permissions]
files = "user-selected"
notifications = ["reminder", "export-complete"]
calendar = "none"
contacts = "none"
mind = ["request", "memory.app-owned"]
network = ["https://sync.example.org"]
credentials = ["notes-sync"]
background = ["document-sync"]

[[documents]]
mime_types = ["text/markdown", "text/plain"]
extensions = ["md", "txt"]
role = "editor"

[intents]
schema = "intents.json"

[[notification_categories]]
id = "reminder"
actions = ["open-note", "dismiss"]
```

Important distinctions:

- **Capabilities** say what an app provides or requires.
- **Entitlements** define what it may ever request.
- **Permissions** are actual grants, often narrower and revocable.
- A declaration is **not** a grant.
- Installer, administrator and per-machine ceilings may deny a declared permission.

`intents.json` is generated from typed action definitions and validated during packaging. Do not hand-maintain Rust handlers, manifest signatures and agent schemas independently.

### Identity must be stronger than an app-supplied string

Kernel peer credentials establish process credentials. They do **not**, by themselves, prove which app a process represents: many desktop apps share a UID.

The authority must bind peer credentials to a broker-managed launch record and sandbox/security domain. That binding must account for process lifetime, avoid PID-reuse races and define controlled child-process delegation.

An app saying `AppId("org.yantrik.settings")` must gain nothing.

Services derive caller identity from the authenticated connection. They do not accept a trusted `caller_id` field from request bodies.

### Lifecycle

The platform standardises these events:

1. **Launch:** construct the app, register intents and resources.
2. **Activate:** deliver a launch, file-open, URL, notification or intent activation.
3. **Foreground/background:** report visibility and activity; do not pretend Linux has suspended the process.
4. **Background lease:** bounded authorization to finish a task, with cancellation and expiry.
5. **Quit request:** allow document-save handling and bounded shutdown.
6. **Termination/recovery:** recover after crashes or forced termination.

Single-instance behaviour is owned by the session application broker. A second launch forwards an activation request to the registered instance.

A persistent daemon is not “an app that never quits.” It uses ServiceKit and a separately approved service declaration.

### Windows and menus

Application owns:

- logical window IDs;
- document-to-window associations;
- activation and focus requests;
- menu/command models;
- standard shortcuts;
- recent document routing;
- graceful close.

Wayland/labwc remains responsible for placement, focus policy and window management. The SDK must not promise arbitrary global positioning, focus stealing or screen capture.

Define menu commands once. The same command can back a toolbar button, keyboard shortcut, context menu and permitted intent.

### Settings

- App-private preferences are durable and namespaced by verified app identity.
- Cross-process access uses a coordinated store; do not let Settings and apps race by rewriting the same TOML file.
- System preferences have typed keys, subscriptions and authorization.
- Machine policy overrides are visible as managed values.
- Secrets use the vault, never Preferences.
- Document content uses Documents/Storage, never Preferences.

### Permissions and approvals

There are two different interactions:

1. **Permission grant:** “Allow Calendar to read these calendars?”
2. **Action approval:** “Allow this agent to send this particular invitation?”

The SDK requests authorization. Only trusted system UI renders the authoritative consent surface.

An approval should bind to:

- initiating actor and target app;
- action and canonical parameters;
- resource/version references where necessary;
- consequences and data destinations;
- expiry and replay protection.

A materially changed action needs a new approval. Approval is an opaque authorization result, not `approved: true` supplied by a client.

### How the Mind sees an application

App Intents should make ordinary app commands discoverable without exposing the entire UI tree.

Each application supplies:

- **State projection:** a small, labelled, permission-filtered description.
- **Typed intents:** actions with schemas, results and declared effects.
- **Entities:** stable scoped references such as `NoteId`, not screen coordinates.
- **Preconditions:** applicable state revisions or other constraints.
- **Execution results:** success, structured error, or asynchronous operation receipt.

The SDK generates discovery metadata and routes invocations through the same domain commands used by the UI.

```text
Button / shortcut ────────┐
                         ├── typed app command ── domain operation
Agent → intent router ───┘
         │
         └── identity + authorization + audit
```

Agent invocations carry a non-forgeable attribution/delegation chain. The target service must not treat an agent-induced action as ordinary fully trusted app activity.

A developer’s `read_only` annotation is useful metadata, not a security guarantee. Backend services still authorize their operations.

### Third-party participation

Support three explicit levels:

| Level | Integration |
|---|---|
| **Ordinary Linux application** | `.desktop`, MIME, portals, freedesktop notifications, MPRIS and accessibility. No fiction that it has typed native intents. |
| **Yantrik-integrated application** | Manifest and optional intent endpoint; can be Rust, C++, Python, GTK, Qt or another toolkit. |
| **Native Yantrik application** | Application + UI + App Intents and other frameworks, normally Rust/Slint. |

For non-Rust applications, publish:

- a versioned, toolkit-independent intent/service protocol;
- language-neutral schemas;
- a small C client ABI and generated bindings where justified;
- an optional local sidecar adapter.

Do not make JSON over a socket a parallel security model. Non-Rust clients use the same broker, identity rules and authorization.

Automation adapters for legacy apps are useful, but remain visibly less reliable than native typed intents.

---

## 5. YantrikUI and the Human Interface Guidelines

YantrikUI is the implementation of the design language. The **Yantrik Human Interface Guidelines**, or **Yantrik HIG**, explain when and why to use it.

### YantrikUI contains

- Semantic colour tokens: surface, elevated surface, primary text, warning, selection—not app-specific hex values.
- Typography and density scales.
- Spacing, radii, shadows and motion.
- Light, dark, high-contrast and reduced-motion behaviour.
- The existing component set, rationalised rather than rewritten.
- Keyboard navigation, focus rings and accessible naming conventions.
- Standard loading, empty, unavailable, denied and error states.
- Icon naming and sizing.
- Consistent presentation of model-generated content and provenance.

### The HIG covers

1. App structure, navigation and document workflows.
2. Window behaviour and dialog modality.
3. Typography, density, colour and motion.
4. Keyboard-first operation and accessibility.
5. Notifications versus dialogs versus approval cards.
6. Permission requests, denial and revocation.
7. Mind interactions: generated-content labels, uncertainty, review and undo.
8. App Intent design: semantic actions, understandable consequences, idempotency and reversibility.
9. Localisation, text expansion and bidirectional layouts.
10. Destructive actions, recovery, offline operation and synchronization conflicts.

### Keep the specification executable

Each component has:

- a stable component identifier;
- linked HIG guidance;
- interactive examples;
- accessibility requirements;
- visual regression fixtures;
- approved and prohibited usage examples.

A change to a component’s behaviour, token semantics or accessibility contract must update the corresponding HIG section and tests in the same change.

Extend the existing design-system lint to detect raw colour/spacing values, missing accessible labels, prohibited private imports and misuse of modal/approval components.

---

## 6. Developer experience

### One CLI

```text
yantrik new app org.example.Hello
yantrik dev
yantrik check
yantrik test
yantrik intents inspect
yantrik intents invoke org.example.Hello greet
yantrik ui catalogue
yantrik ui preview ui/app.slint
yantrik package
```

`yantrik dev` should provide:

- development installation and launch;
- resource/UI rebuilding;
- service diagnostics;
- manifest and permission diagnostics;
- intent inspection;
- a clear distinction between fake and real services.

It must not quietly grant production privileges.

### Minimal app: about 30 lines

The generated template includes the Cargo manifest, build script, bundle manifest and intent metadata generation. Application code can then look like this proposed API:

```rust
use yantrik_sdk::{
    application::Application,
    intents::{IntentReply, NoArgs},
};

slint::include_modules!();

fn main() -> yantrik_sdk::Result<()> {
    Application::from_manifest().run(|ctx| {
        let window = AppWindow::new()?;

        let greet = ctx.intents().register(
            "greet",
            |_invocation, _: NoArgs| async {
                Ok(IntentReply::message("Hello from Yantrik"))
            },
        )?;

        window.on_greet(move || {
            greet.dispatch_from_ui(NoArgs);
        });

        ctx.present(window)
    })
}
```

```slint
import { YButton } from "@yantrik-ui";

export component AppWindow inherits Window {
    title: @tr("Hello");
    callback greet();
    YButton {
        text: @tr("Say hello");
        clicked => { root.greet(); }
    }
}
```

Here:

- Registration validates the action against generated bundle metadata.
- The runtime supplies basic non-sensitive `describe` state.
- Dispatch is nonblocking; runtime-owned task tracking surfaces failures.
- `dispatch_from_ui` identifies the local invocation path; it does not authorize privileged service work.
- App-specific content enters `describe` only through explicit projections.

For richer apps, typed intent derives generate schemas from command types. UI callbacks should call domain commands, not implement those commands inline.

### Documentation and samples

Publish one developer site containing:

- Getting started.
- Framework reference.
- App model and manifest reference.
- HIG.
- Security and permissions guide.
- Intent design guide.
- Runtime compatibility matrix.
- Migration notes.

Maintain small samples for:

- Hello app.
- Document editor.
- Notification with action.
- Calendar integration.
- Permission denial and revocation.
- Streaming Mind request.
- Scoped memory.
- Non-Rust intent provider.
- Background lease.

### Testing

`yantrik-testing` should offer:

- fake notification, calendar, preferences, Mind and vault clients;
- a deterministic clock and scripted transport faults;
- disconnect/reconnect and service-restart scenarios;
- intent schema and handler conformance tests;
- labelled-content propagation checks;
- UI snapshots and keyboard-navigation tests.

The intent harness tests replay, cancellation, stale state, excessive payloads, redaction and approval invalidation—not just successful actions.

Kernel identity, sandbox boundaries and real authorization require separate OS integration tests. A fake service cannot prove those properties.

---

## 7. Services behind the frameworks

### Do not create a daemon for every framework

**Mostly local:** Foundation, UI, Localization, Storage and Documents.

**Local plus system integration:** Application, Files, Preferences, Media, Accessibility and App Intents.

**Primarily service clients:** Notifications, Calendar, Contacts, Mail, Network status/egress, System, Mind and Security.

Framework ownership and process topology are different things. A new Contacts API does not automatically justify a permanently running Contacts daemon.

### One client pattern

Expose typed async clients with common connection semantics:

```rust
let client = NotificationCenter::connect(ctx.services()).await?;

let receipt = client
    .post(notification)
    .deadline(Deadline::after_seconds(2))
    .await?;

let mut changes = client.subscribe(Default::default()).await?;
```

Underneath, every service client uses the same private infrastructure:

1. **Discover and connect** to an authenticated endpoint.
2. **Negotiate** protocol versions and optional capabilities.
3. **Derive caller identity** from the connection and broker launch binding.
4. **Attach attribution** and authorized delegation, not arbitrary caller claims.
5. **Apply a deadline** to every operation.
6. **Propagate cancellation**, while acknowledging that an already committed operation cannot always be undone.
7. **Return standard errors** plus domain-specific details.
8. **Reconnect and resynchronize** subscriptions using cursors/revisions where supported.
9. **Apply backpressure** with bounded queues.
10. **Emit correlated, privacy-conscious diagnostics.**

Common error categories:

```text
Unavailable
Unsupported
PermissionDenied
ApprovalRequired
Cancelled
DeadlineExceeded
Conflict
RateLimited
InvalidArgument
ResourceGone
```

Mutating requests need idempotency keys or explicit uncertain-outcome handling. A timeout after sending an email must not trigger an automatic duplicate send.

Subscriptions must say whether they are snapshots, state changes or durable events. After gaps, a state stream can demand a fresh snapshot; it must not pretend nothing was missed.

All Slint-facing adapters marshal updates onto the UI thread. No framework performs blocking socket I/O inside a Slint callback.

### Security enforcement is below the SDK

The safe API is necessary but insufficient:

- A malicious process can avoid `YantrikNetwork`.
- Same-UID desktop programs can often reach resources outside a voluntary API.
- A label in a Rust type is not a containment boundary.

Therefore:

- Managed apps and hosted minds need OS-enforced filesystem/process/network isolation.
- Egress-only execution requires network namespace/firewall policy, including DNS and other bypass paths.
- Vault operations should prefer narrowly scoped credential references or broker-performed authentication.
- File authority should use grants/handles wherever possible.
- Unconfined legacy applications remain a separately identified trust class.

`ModelOutput<T>` carries provenance and trust labels through serialization and IPC. Rendering and intent boundaries preserve those labels; model output never becomes executable authority merely by being parsed as structured data.

The security authority—not the Mind—must ultimately own approval enforcement. Initially it can delegate to existing code, but the long-term dependency must not be “all permissions require importing the companion.”

---

## 8. Internal code organisation

Use this logical layout; it need not all be moved physically on day one:

```text
sdk/
  foundation/
  ui/
  application/
  app-intents/
  notifications/
  files/
  documents/
  security/
  mind/
  ...
  facade/
  build/
  testing/

platform/
  ipc-contracts/
  ipc-transport/
  service-host/
  identity/
  policy/
  protocol-schemas/

os/
  shell/
  shell-ui/
  session/
  settings/
  lock-screen/
  installer/
  switcher/

services/
  notifications/
  email/
  calendar/
  network/
  ...

apps/
  notes/
  files/
  calendar/
  ...

mind/
  gateway/
  harness/
  companion-adapters/

tools/
  yantrik-cli/
  architecture-lints/
  compatibility-tests/

docs/
  hig/
  frameworks/
  architecture/
```

The sibling repositories can remain sibling repositories. Pin their versions and put platform-facing adapters around them; repository consolidation is not a prerequisite.

### Dependency direction

```text
Applications / shell feature clients
                ↓
       Public framework APIs
                ↓
 Narrow private client infrastructure
                ↓
       Authenticated protocols
                ↓
    Service/domain implementations
```

Separately:

```text
Services → shared contracts + ServiceKit
Mind implementation → provider libraries + approved framework clients
```

Protocol schema packages must not depend on daemon implementation packages.

The shell is a privileged platform client, not the library every app imports. It may have explicitly isolated private administrative interfaces; ordinary views should use public clients where possible.

### Module rules

- `main.rs`: composition, argument handling and launch; target under 150 lines.
- Split applications into `app`, `model`, `commands`, `views`, `persistence` and `tests`.
- UI callbacks delegate immediately to typed commands.
- No IPC wire types in Slint view models.
- No database queries inside views.
- No unowned background tasks; tasks belong to an app, window or operation lifetime.
- Files above roughly 500–800 lines require review for a useful split; this is a review trigger, not a mechanical hard limit.
- Crate boundaries follow compilation and ownership boundaries, not every folder.

For the shell, extract independent binaries and feature packages where useful—installer, lock screen, switcher, Settings and Mind panel—but avoid turning 180 flat modules into 180 crates.

### Build-host discipline

On the 48 GB host:

- Do not create a giant `yantrik-sdk` implementation crate.
- Keep UI resources out of non-UI framework dependencies.
- Split large generated Slint compilation units where architecture permits.
- Avoid “all frameworks” default features.
- Cache build artifacts and pin dependency versions.
- Record peak compiler RSS and incremental build time in CI.
- Initially schedule only one known ~14 GB UI-heavy compilation at a time, with lower-memory work beside it; tune from measurements.
- Keep a full integration build, but use dependency-aware targeted checks for fast feedback.

Moving code into a facade without reducing generated UI compilation would not solve the current memory problem.

---

## 9. Prioritised migration: 13 steps

Sizes are relative: **S** roughly days, **M** a small multi-week project, **L** a substantial cross-cutting project. They are not promises without repository inspection.

| # | Step | Size / risk | What it unblocks | How it ships without breaking nightly |
|---|---|---|---|---|
| **1** | **Publish YantrikUI v0 and HIG v0.** Rename the current shell package, establish stable Slint import aliases, document the existing 40 components and token rules. | M / Low | Immediate visual and interaction consistency. | Existing kit/token packages forward to the new exports; migrate apps individually. No component rewrite. |
| **2** | **Create thin Application and Foundation boundaries.** Wrap current runtime behaviour and establish the public/private package registry. | M / Low–medium | One documented app entry point; a small facade; enforceable dependency rules. | Existing runtime remains the implementation behind adapters. Begin lints in warning mode, then fail new violations. |
| **3** | **Ship `yantrik new app`, external-build CI and two reference apps.** Use one simple app and one document app. | M / Low | A coherent developer path and proof the SDK is distributable. | Templates target current runtime through the new wrappers; no platform-wide cutover. |
| **4** | **Extract typed commands and App Intents helpers.** Generate schemas and registrations from shared definitions. | M / Medium | Near-free agent support; thinner `main.rs`; fewer UI/agent divergences. | Adapt existing `describe`/`act` messages. Preserve old endpoints and compare descriptors in tests. |
| **5** | **Standardise Notifications and Preferences clients.** Include subscriptions, error states and appearance sync. | M / Low–medium | Consistent everyday app behaviour. | Wrap current services and storage first; support old contract versions during app-by-app migration. |
| **6** | **Adopt one client runtime and protocol contract.** Deadlines, cancellation, bounded subscriptions, tracing and conformance tests. | M / Medium | Predictable service integration and reliable fake services. | Replace transport usage behind framework clients one service at a time; do not replace every socket simultaneously. |
| **7** | **Introduce manifests, bundle tooling and verified launch records.** Register `.desktop` files, MIME types and single-instance activation. | L / Medium–high | Third-party installation identity and capability negotiation. | Generate manifests for current apps; retain legacy launch adapters. Do not claim strong app isolation until enforcement exists. |
| **8** | **Establish Security authority and confined execution.** Bind kernel identity to launches; unify permission/vault interfaces and delegation. | L / High | Trustworthy third-party grants, enforceable egress and approval receipts. | Adapt existing approval UI and gate first. Use observe-only policy for migration diagnostics, never for actually security-critical decisions. Roll out confinement by app class with explicit legacy status. |
| **9** | **Extract Files, Documents and Storage.** Start with Notes and one office application. | L / Medium | Consistent open/save/recovery and scoped persistence. | Keep existing document formats. Add migration backups and old-reader tests; switch individual workflows behind flags. |
| **10** | **Publish domain service clients.** Calendar, Mail, System and Weather first; Contacts only when backed by an implementation. | M / Medium | Apps stop importing daemon contracts directly. | Framework clients translate existing contracts. Keep missing backend capabilities explicitly unavailable. |
| **11** | **Publish the Mind gateway.** Scoped requests, labelled streams, app memory and tools derived from intents. | L / High | Public AI integration without exposing companion internals. | Gateway adapters call existing companion APIs; existing Lens remains working. Expand access only after identity, scope and revocation tests pass. |
| **12** | **Complete accessibility/localisation foundations and preview Media.** Audit real Slint/backend support; expand catalogue testing. | L / Medium | Platform-quality UX and internationalisation. | Localisation scaffolding and component semantics begin in step 1; this step closes audited gaps. Keep old media paths until equivalent behaviour is verified. |
| **13** | **Stabilise SDK 1.0 and progressively dismantle monoliths.** Split shell features, finish app dependency enforcement, document non-Rust integration. | L / Medium | A credible supported platform with sustainable nightly builds. | Stabilise only proven API slices; leave the rest preview. Remove shims only under the deprecation policy. Split compilation hotspots incrementally with RSS gates. |

### Release gate for SDK 1.0

I would not declare SDK 1.0 because the catalogue exists. I would require:

- Several first-party apps use only public application-facing frameworks.
- One third-party Rust app builds outside the workspace.
- One non-Rust app exposes a typed intent.
- An older SDK app runs on the current nightly.
- Permission denial, revocation and service restart work correctly.
- A hostile app cannot claim another app’s identity.
- A confined mind cannot bypass required egress control.
- Intent approvals cannot be replayed for altered actions.
- The build stays within the host’s measured memory budget.

**The first visible result should be consistent apps, not a large repository move.** Start by turning the existing UI kit, runtime and control surface into YantrikUI, YantrikApplication and YantrikAppIntents. Then make the service and security boundaries strong enough that third parties can rely on them.
