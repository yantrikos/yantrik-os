# Fable's review (21 Oct 2026), with the repository open

Fable (Claude Fable 5.1) read the workspace at the head of `feat/ui-kit-controls`. It had read access to every crate and was asked the architecture brief, then the platform-SDK brief (both in this folder). This is its answer, kept as written. The synthesis and the decisions are in `../platform-sdk-2026-10-21.md`. Paths are repo-relative.

## 0. Where the code contradicts the brief

1. **The sibling-repo story is wrong in the direction that matters.**
   - In the root `Cargo.toml`, `yantrikdb` is a git dependency pinned to a revision, with no patch.
   - `yantrik-ml` is a git dependency *patched to the in-tree copy* (`[patch."https://github.com/yantrikos/yantrik-ml.git"] yantrik-ml = { path = "crates/yantrik-ml" }`), so in this workspace the in-tree copy is the only one that ever builds.
   - `yantrik-companion` is not a sibling repo here at all. It is `crates/yantrik-companion`, an in-tree workspace member.

   The "two copies" wart is real, but the fix is a one-line deletion, not a repo reconciliation.
2. **The giant Slint crate is smaller than stated, and the app screens are not in it.** `crates/yantrik-ui-slint/build.rs` compiles only `ui/app.slint`, whose import graph reaches 122 files and 38,725 lines.
   - The other 17 top-level files (18,470 lines) are never reached from the shell. They are compiled by each app, which reaches into the shell's directory by a relative path in `apps/*/build.rs` (`../../crates/yantrik-ui-slint/ui`), with no Cargo dependency.
   - `onboarding.slint` (1,426 lines) is imported by nothing found; it is probably dead.
3. **Apps are not thin.** `apps/email/src/main.rs` is 2,689 lines and `apps/notes/src/main.rs` 1,904. They are `[[bin]]`-only crates with no library target. That is why `tests/email-core/lib.rs` `#[path]`-includes app sources to test them without Slint. Fourteen such `tests/*-core` crates are workspace members.
4. **The kit is duplicated by shims, with a known name collision.** `crates/yantrik-ui-slint/ui/components/` holds 30 two-to-four-line files that re-export the kit by `../../../yantrik-ui-kit/slint/...`. `app_header.slint` exists in both places, and include-path order decides which one an app compiles.
5. **The lint sees 55 of the 124 private components.** The bulk is `*Row`, `*Tile`, `*Item`, `*Sheet`, `*Field`, `ToggleSwitch` and `ViewToggle`, none of which it counts.

Also:
- `crates/yantrik-manifest` is used by nothing.
- `crates/yantrik-mcp` is not a workspace member.
- `examples/hello-window` imports `Button` from `std-widgets.slint`.
- `docs/architecture.md` is dated March and describes "5 crates".
- The shell's `App` root declares 559 properties, 245 callbacks and 34 `current-screen == N` branches, mirrored by an integer table `SCREENS` in `control.rs` that a source-reading test keeps in step.

The good news is real too:
- `yantrik-surface`, with its golden `dispatch-vectors.json`;
- `yantrik-app-runtime::control`;
- the `AppWindow` + `AppHeader` data-driven frame;
- `services/*/tests/yos_check.rs`;
- 62 of the shell's 117 top-level modules never mention Slint.

The platform is already half built. It is not named, not tiered, and not separated from the OS internals.

## 1. The framework catalogue

A framework is a named crate with one purpose, a documented public surface and a stability tier, and an app depends on frameworks only.

| Framework (crate) | Purpose | Built from today's |
|---|---|---|
| `yantrik-foundation` | Value types and rules every process shares: time and "ago", text, shown addresses, places, ids, version, problem records, i18n lookup, the error type. | `yantrik-shown-address`, `yantrik-version`, `yantrik-app-runtime::{place, problems}`, `yantrik-ui/src/i18n.rs`, `notification_title.rs`, and the nine `ago` functions (bridge, clipboard, filebrowser, mind_panel, agents_workroom, memory_rows, the switcher's model, weather-service recent, companion connector) |
| `yantrik-design` | The tokens (`theme.slint`) plus the Rust side of the same facts (ThemeSettings, the accent table, colour roles). | `yantrik-design-tokens`, `app-runtime/src/theme.rs`, `ui-kit/src/colour_roles.rs`, `themes/*.toml` |
| `yantrik-ui-kit` (kept) | The component library, imported as a Slint library `@yantrik-kit/...` and tiered primitives → controls → components → patterns → templates. | `crates/yantrik-ui-kit/slint/*` |
| `yantrik-app` | The app model: AppWindow chrome, lifecycle, single instance, init and tracing, per-app settings, open-file and theme-change hooks, and the window-side control surface. | `yantrik-app-runtime` minus notify, companion and service |
| `yantrik-surface` (kept) | The describe, act and explain protocol: registry, grades, argument checks, revision guard, gate, grants, `answer_later`. | `yantrik-surface`, `ipc-contracts::control_surface` |
| `yantrik-notifications` | Tell the person something: the builder, actions, provenance, the typed contract. | `app-runtime/src/notify.rs`, `ipc-contracts::notifications` |
| `yantrik-services` | Typed clients for the OS services, one module each (calendar, mail, notes, music, weather, network, system). | the `ipc-contracts` domain modules, `ipc-transport::service::client`, `call_typed` |
| `yantrik-mind` | Ask the person's mind from an app: `ask`, `recall`, `tool`, `reach`, `AskError`. | `app-runtime/src/companion.rs`, `yantrik-ui/src/companion_rpc.rs` |
| `yantrik-harness-sdk` | What a mind implements to attach. | `harness/src/{protocol, redact, notice}.rs` (`host.rs` stays internal) |
| `yantrik-files` | Where an app and an agent may look and write, following a moved file, open-with. | `ipc-contracts::home_paths`, `yantrik-file-follow`, `app-runtime::place`, `mime_dispatch.rs` |
| `yantrik-settings` | One schema of `settings.yaml` with typed readers, plus per-app settings and a change watch. | `wire/settings.rs::UserSettings` and the 19 ad-hoc parsers |
| `yantrik-security` | Who is calling, the gate (grades, ceiling, mode, grant), agent reach, scrubbing, a narrow vault client. | `ipc-transport::{gate, reach, peer_identity, owner, privacy, plain_text}`, `caller_identity.rs`, `vault_unlock.rs` |
| `yantrik-ipc` | JSON-RPC over Unix sockets (Tier 2; apps reach it through services). | `yantrik-ipc-transport` |
| `yantrik-service` | What a background service implements. | `yantrik-service-sdk` |
| `yantrik-sdk` | The facade: one crate and one version, feature-gated so a service never links Slint. | new |

Everything else stays internal: the shell, shell-core, `yantrik-os`, the companion stack, the harness host, the gateway, egress, lock, switcher, chat, brain, yantrikdb-server and the services.

## 2. Naming and namespaces
- **Crates and the facade:** separate `yantrik-*` framework crates, plus one feature-gated `yantrik-sdk` facade. `ipc-transport/src/service.rs` already records why one mega-crate failed: the companion "must not carry a toolkit to make a socket call".
- **An app's dependency:** `yantrik-sdk = { version = "1", features = ["app", "ui", "notifications", "mind", "services-calendar"] }`. A service uses `default-features = false, features = ["service", "surface", "settings"]`.
- **Slint imports:** the kit and tokens are imported as Slint libraries via `CompilerConfiguration::with_library_paths` (in 1.17, and used nowhere today): `import { YButton } from "@yantrik-kit/controls.slint";`. This removes the include-order problem and the 30 shims, and a third-party app cannot import a shell-private component.
- **Rust names:** `yantrik_sdk::<framework>::<noun>`, verbs as free functions, builders past three options. No `control_*`, `wire_*` or `*_rs`. Wire names stay dotted, in `method::` modules.
- **Folders:** public frameworks live in `frameworks/`, internals stay in `crates/`, so the boundary is visible in `ls`.

## 3. Tiers, versions, deprecation
- **Tier 1, stable:** foundation, design (the token names), ui-kit, app, surface, notifications, service. These follow semver, carry `#![deny(missing_docs)]`, are diffed with `cargo public-api` in CI, and keep a deprecation for two minor releases. The wire protocol has its own number (`PROTOCOL = 1`), and its golden vectors are the compatibility test.
- **Tier 2, preview:** services, mind, files, settings, security, ipc, harness-sdk. They sit behind a `preview` feature and may break between minors, with a changelog entry.
- **Tier 3, internal:** everything in `crates/`, `publish = false`.
- **One SDK version** for all frameworks, released with the OS channel. The manifest carries `sdk = "1.3"`, and the OS can refuse an app built against a newer SDK.
- **Retiring a Slint component:** `deprecated.slint` re-exports it under its old name, and a `deprecated-kit` lint rule ratchets the uses down.
- **Enforcement:** a workspace test fails when an app, template, example or framework depends on `crates/*`, when Tier 1 depends on Tier 2, or when a `build.rs` joins `../../crates`.

## 4. The app model

**What an app does today.** A `Cargo.toml` with eleven copied dependencies. A `build.rs` identical to nine others. `ui/app.slint` with raw sizes, a hand-rolled modal and `std-widgets`. A `main.rs` that re-orders the same ten runtime calls. A `.desktop` file kept separately. No declaration of what the app needs. It is a workable runtime, not an app model.

**The bundle.** `/opt/yantrik/apps/<id>/` holds `yantrik.toml`, the binary, `icon.svg`, `i18n/*.yaml` and an optional `adapter`. The `.desktop` file is generated from the manifest. Revive `crates/yantrik-manifest` inside `yantrik-app`.

```toml
[app]
id = "notes"
name = "Notes"
version = "1.2.0"
sdk = "1.3"
purpose = "quick markdown notes kept in the notes library"
mime = ["text/markdown"]

[entitlements]
notifications = true
mind = ["ask", "recall"]
services = ["calendar:read"]
files = ["home:documents:rw"]
network = false
vault = []

[surface]
grades-max = "sensitive"
```

**Entitlements are enforced where a call lands, by the identity the kernel gives:** `peer_identity`, then the bundle, then the manifest, checked before the gate. They are log-only for one beta, then enforced. This step needs a full security review.

**Lifecycle.** `App::builder("notes").window(ui).manifest(include_str!("../yantrik.toml")).on_open(..).on_present(..).on_theme_changed(..).on_close_requested(..).surface(..).run()`. Every piece is already a function in `app-runtime`; the builder is the missing twenty lines.

**Windows and menus.** `AppWindow` with `header-actions` and `YContextMenu`. Add `app-commands` so an app's commands also appear in the command palette and in `describe`.

**Everything else:**
- **Notifications:** the app id is taken from the manifest, never typed by the app.
- **The control surface:** `describe` is derived from the same Rust state struct the view is drawn from.
- **Settings:** `settings::<NotesSettings>()` reads `~/.config/yantrik/apps/notes.toml` and is published read-only in `describe`.

## 5. Developer experience
- **Templates:** `templates/rust-app` holds a manifest, a kit-only `ui/app.slint`, a Slint-free `src/lib.rs`, a `main.rs` of at most 40 lines, and tests. `examples/hello-window` must stop importing `std-widgets`.
- **The `yos` CLI:** `yos new app`, `yos lint` (the design lint as a binary), and `yos preview <component|app> [--light] [--scale 1.25]`.
- **`docs/hig/`:** it consolidates the art direction, desk-and-mind, the colour roles, the levels system, the writing rules, the component catalogue, the app model, grades, accessibility and "what a window costs" (the idle-redraw rule).
- **The catalogue is generated** from each kit file's doc block plus preview PNGs per theme.
- **`tools/kit-preview`:** the preview harness joins the workspace, rendering every component in both themes at 1.0 and 1.25 with the software renderer, in CI.

## 6. Internal organisation

```
L5  Minds          the built-in companion as a process; third-party harnesses
L4  Shell          yantrik-shell, yantrik-lock, yantrik-switcher
L4' Apps           apps/*                       → L0–L2 only
L3  OS services    services/*, shell-core, yantrik-os, gateway, egress, harness host, yantrikdb-server
L2  Platform SDK   app, surface, notifications, services, mind, files, settings, service, harness-sdk
L1  Design system  design, ui-kit
L0  Foundation     foundation, ipc, security
```

Dependencies point down, and nothing points at L4 or L5. Two edges break that today, and both are load-bearing:
- **`yantrik-ui` → `yantrik-companion`, `yantrik-ml`, `yantrikdb-core`, `imap`, `lettre`, `cpal`, `rubato`, `pty-process`.** The trusted UI that draws approval cards links the model runtime, the mail client and the audio stack. The harness path already runs minds as attached processes behind egress and gateway policy, and the companion should attach the same way. This is the single largest architectural move and the largest security win: the shell becomes the thing that only asks and draws.
- **`apps/*/build.rs` → `crates/yantrik-ui-slint/ui` by relative path.**

**The kit tiers.**
- `primitives/`: YText (role enum), Icon, YSurface (level enum), YScrim, YFocusRing, YScrollView, YListView, YDivider, YKeycap.
- `controls/`: YButton, YIconButton, YInput, YField, YTextArea, YSwitch, YSlider, YSegmented, YTabs, YChip, YBadge, YStatusPill, YToggleTile, YWheel, YStepper.
- `components/`: YListRow, YCard, YTile, YMetric, YSheet, YDialog, YPopover, YContextMenu, YTooltip, YEmptyState, YIndicator, YSectionHeader, YToolbar, YToolbarButton, YMenuButton, YOsd and the charts.
- `patterns/`: AppHeader, YSidebar, YReading, ToastBanner, NoticeBar, MessageBubble, ToolCall, AgentProposal, AgentRail, ModelPicker, MindMark, JumpToPresent, LockView.
- `templates/`: AppWindow, SettingsWindow, ListDetailWindow, DocumentWindow.

**The component contract:**
1. Enum-typed properties (`kind: ButtonKind`, not `variant: int`).
2. Every state, from tokens.
3. Accessibility.
4. Light and dark.
5. Text scale.
6. Control-surface hooks (`action-id`, `surface-key`).
7. A doc block, a preview scene and a catalogue entry.

Screens are built only from the kit because:
- library paths keep screens out of shell-private files;
- `AppWindow` is the only exported `Window` subclass;
- `YText` roles replace every `Text { font-size: 13px }`.

**The Rust ↔ Slint pattern.** `features/<name>/{mod.rs, model.rs, view.rs, surface.rs, tests.rs}`:
- The model is Slint-free.
- The view is the only place that touches `ui`.
- The surface builds from the same State and calls the same model functions as the callbacks.

Per-screen state goes in a Slint `export global <Screen>State` in the screen's own file. Screen ids become `export enum Screen`, which deletes `SCREENS` and its mirror tests.

**Compile memory.** Slint cannot compose components across crates, so the split has to happen at the process boundary. About half the reachable 38.7k lines are windows in all but name: Settings, Installer and first run, Permissions, Devices, Agents, the package manager, About, Files, Recipes, the System dashboard, Memory and Problems. Each becomes an app on `yantrik-app`, and `BECAME_WINDOWS` tells old callers where it went.

**Shared libraries beyond UI:**
- one `ago`, replacing nine;
- `foundation::text`;
- `yantrik_services::<domain>::Client`, with the right timeout per method;
- `yantrik-settings`, which the gate also reads through;
- `foundation::store::{JsonFile, atomic_write, Bounded}`;
- `foundation::Error`/`Refusal` built on `thiserror`, with `Result<_, String>` only at the wire edge;
- `foundation::problems` together with `yantrik_app::init`;
- i18n through `foundation::i18n` plus the kit's `Tr`;
- for `yantrik-ml`: delete the git dependency and the patch now, and the in-tree copy once the Mind is a process.

**Files and modules:**
- 600 lines soft, 1,200 hard, as a ratchet baseline.
- One concept per module.
- No `control_*.rs` or `wire/*.rs`.
- The shell becomes a library crate with a main of at most 80 lines.
- `pub(crate)` facades.
- `ShellContext` holds per-feature state instead of 30 `Rc<RefCell>` fields.

The target tree is `crates/yantrik-shell/src/{main.rs, lib.rs, shell/{context, wiring, surface, screens, render_backend}.rs, features/{desktop, bar, lens, approvals, mind_mode, minds, agents, recipes, notifications, memory, files, session, settings, system, capture, companion}/, guards/}`.

**Tests:**
- Unit tests sit with each feature.
- Kit snapshots and `verify-controls` run in CI.
- Each screen renders from a fixture State.
- Contract tests cover the dispatch vectors, `yos check` on every app and a golden `describe` per app.
- The VM is for release gates.
- The 59 files of source-reading tests become about 10 in `guards/`.

**CI enforces:**
- dependency direction, with `deny.toml`;
- the file-size and design-lint ratchets;
- clippy, blocking crate by crate;
- `public-api` and `semver-checks` on the frameworks;
- the snapshots;
- a pinned `rust-toolchain.toml`;
- a cap of `CARGO_BUILD_JOBS`.

## 7. Migration plan (none of these stops nightly)
1. Write the rules down and make them fail: the architecture doc, `deny.toml`, the dependency-direction test, `rust-toolchain.toml`. Small, low risk.
2. Library paths and no shims: `with_library_paths`, delete the 30 shims, and a `yantrik-app-build` crate so the ten `build.rs` files become one call. Small, low risk.
3. App screens into their apps, and delete `onboarding.slint` if it is dead. Small to medium, low risk, one PR per app.
4. `yantrik-foundation` and one `ago`. Small, low risk.
5. Apps become `lib + bin`; delete `tests/*-core`. Medium, low risk.
6. The shell becomes a library with feature directories: mechanical moves, one directory per PR. Medium, low to medium risk.
7. `yantrik-app`, `-notifications`, `-mind`, `-services` and the `yantrik-sdk` facade, with deprecated re-exports for one release, and `App::builder`. Medium, low risk.
8. `yantrik-settings`, with the gate reading the ceiling through it byte-identically, tested by settings vectors. Medium risk.
9. Kit tiering and the missing components, with the lint widened: `YText` first, then YCard, YListRow, YChip/YBadge, YSheet, YScrollView/YListView/YTextArea, YSectionHeader, YToolbarButton, YMeter and MindMark, plus enum kinds. Large overall, small per PR.
10. The preview harness in the workspace, snapshots in CI, the catalogue generated. Medium.
11. Per-screen globals and the `Screen` enum, with each `describe` snapshotted first. Medium risk.
12. Screens out of the shell, Settings first, then the Installer and the rest. Large, one screen per PR. This fixes the 14 GB problem.
13. Manifest and entitlements: log-only for one beta, then enforced. High risk, with a full security review.
14. The Mind as a process. The shell drops the companion, ml, yantrikdb-core and the mail and audio stacks. Run both paths behind a flag for two weeks and re-run the guards. High risk.
15. Blocking clippy, public-API checks and the file-size ratchet.

## 8. Delete or stop
- **Delete:**
  - the 30 shims;
  - the relative include paths and the ten copies of `build.rs`;
  - the `tests/*-core` crates;
  - `onboarding.slint`, if it is dead;
  - the `yantrik-ml` git dependency and patch, and later the in-tree copy;
  - `yantrik-mcp` (or make it a member);
  - the standalone `yantrik-manifest`;
  - `test_brain.rs` from the shell;
  - `SCREENS` and its mirror tests;
  - `wire/callbacks.rs`;
  - the March architecture doc;
  - the `#[allow]`s for the rustc ICE, once the toolchain is pinned.
- **Stop:**
  - new `control_*.rs` and `wire/*.rs` files;
  - `Result<_, String>` inside crates;
  - integer-coded kinds in Slint;
  - pixel font sizes and private controls;
  - `std-widgets` anywhere;
  - mirror tests;
  - private `settings.yaml` parsers;
  - a shell that links the model runtime, the mail client and the audio stack;
  - parallel Slint builds on a 16 GB runner;
  - clippy that only reports.
