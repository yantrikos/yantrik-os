# Design the Yantrik OS platform SDK: standard frameworks, like macOS and Windows

You are the architect who would lead Apple's frameworks group or Microsoft's Windows App SDK team. The owner of Yantrik OS asked for this explicitly, in his words: "organizing code, standard libraries like Windows, Mac etc."

He wants Yantrik OS organised as a real platform: a set of **named, standard, documented frameworks** that every app, ours and third-party, is built on. The model is macOS, with Foundation, AppKit and SwiftUI, UserNotifications, EventKit, Contacts, CoreData, Security, App Intents and the Human Interface Guidelines. Or Windows, with the Windows App SDK, WinUI 3, and namespaces such as `Windows.Storage`, `Windows.UI.Notifications`, `Windows.ApplicationModel` and `Windows.Security`.

Be concrete and opinionated: give names, and say what each framework contains and does not.

## The platform today (facts)
Yantrik OS is a Linux desktop (Debian 13, Wayland, the labwc compositor) built around an AI companion called the Mind. It is written in **Rust**, and the UI is **Slint 1.17**: `.slint` markup compiled to Rust. Everything is one Cargo workspace of 38 crates, 21 apps and 10 services, plus three sibling repos (`yantrikdb`, the memory database; `yantrik-ml`; `yantrik-companion`).

What already exists, without a platform shape:
- `yantrik-design-tokens`: `theme.slint`, with colours, the type scale, spacing, radii, shadows, light and dark, plus TOML theme files.
- `yantrik-ui-kit`: 40 Slint components (YButton, YInput, AppHeader, AppTile, dialogs, tabs, sliders, charts, an icon set) and a design-system lint in CI.
- `yantrik-app-runtime` (4.8k lines): what every app binary uses for window bootstrap, its control surface, IPC to services and theme sync.
- `yantrik-ipc-contracts` (5.6k) and `yantrik-ipc-transport` (6.4k): typed messages and a Unix-socket transport, with caller identity from kernel peer credentials.
- `yantrik-surface` (3.8k): the "control surface" protocol. Every app publishes `describe` (its state) and `act` (typed actions), so the Mind and other agents can operate any app, under approval rules.
- 10 services, each a daemon with a typed IPC API: notifications, email, calendar, network, perception, weather, system monitor, a11y, notes, perception-journal.
- `yantrik-service-sdk` (547 lines): helpers for writing a service.
- The Mind: `yantrik-companion*` (about 110k lines across core, cortex, instincts and tools). It holds memory (yantrikdb), tools, recipes, the proactive engine and the approval gate.
- `yantrik-harness` (9.5k): hosts third-party "minds" (agent harnesses such as Hermes, OpenClaw and Pi) as separate processes.
- `yantrik-egress` (the proxy every network call from a mind goes through), the vault (credentials), and approvals: a person approves risky actions on a card; there are modes and a per-machine ceiling.
- 21 apps (Notes, Email, Calendar, Files, Terminal, Music, Images, Weather, System Monitor, Network, Downloads, Containers, the office trio yDoc/ySheets/yPresent, Snippets, Studio, Arcade, Browser and others). Each is `src/main.rs` plus `ui/app.slint`, built on app-runtime. Several `main.rs` files are 2,000–2,700 lines.
- The shell (`yantrik-ui`, 131k Rust lines in about 180 flat modules, and `yantrik-ui-slint`, 58k Slint lines): desktop, dock, top bar, Lens (chat), Mind panel, Settings, Notifications, Memory, approvals, Alt+Tab (a separate binary), lock screen and the installer.
- Third-party apps exist today only as ordinary Linux programs (Chromium, Blender). They have no SDK, and they reach the Mind only through freedesktop interfaces.

## Constraints
- Rust and Slint stay, and so does Linux with Wayland/labwc. Freedesktop standards (D-Bus notifications, `.desktop` files, portals) should be honoured, not replaced.
- Security is first-class: caller identity from the kernel, approvals, egress control, the vault, and labelled model output. An SDK must make the safe path the easy path.
- The control surface (agents can describe and act on every app) is a defining product feature, so the SDK must make it nearly free for an app to support.
- Migration must be incremental, because the OS ships nightly.
- The workspace builds on one 48 GB build host. A giant UI crate already costs about 14 GB to compile.

## What I want back
1. **The framework catalogue.** The named frameworks of the Yantrik platform, as Apple and Microsoft name theirs. For each: its name, its purpose in one line, its main public types and APIs, which process or service backs it, and which existing crates it is built from. Present it as a table, with a column mapping today's crate to tomorrow's framework. Cover at least: core types and utilities (a Foundation), UI (the kit and design language), app lifecycle and windows, notifications, files and documents, calendar and contacts, settings and preferences, storage and persistence, networking and egress, security (identity, permissions, vault), the Mind (asking it things, giving it tools, memory), App Intents (the control surface), media, system information, accessibility, and localisation.
2. **Naming and namespaces.** One crate per framework, an umbrella `yantrik-sdk` facade, or both? The Rust naming (`yantrik_sdk::notifications::…`), the Slint import paths, and how an app's `Cargo.toml` and `.slint` imports look.
3. **Public vs private.** The stable SDK surface versus OS-internal crates. Stability tiers (stable, preview, internal), versioning, semver, deprecation policy, and how CI enforces that private crates are not used by apps.
4. **The app model.**
   - An app bundle and manifest (identity, entitlements and capabilities, the control-surface intents it exposes, notifications it may post, file types it opens);
   - its lifecycle (launch, activate, background, quit, single-instance);
   - windows and menus;
   - settings storage;
   - how permissions are requested and shown to the person;
   - how the Mind and agents see the app through App Intents;
   - how a third-party Rust or non-Rust app participates.
5. **The design language as a framework.** The tokens and components as "YantrikUI", plus a Human Interface Guidelines document: what it covers and how it is kept in sync with the kit.
6. **Developer experience.** A `yantrik new app` template, the minimal app in about 30 lines, docs, samples, a component catalogue and preview tool, and testing helpers (fake services, a control-surface test harness).
7. **Services behind the frameworks.** Which frameworks are in-process libraries and which are clients of a service daemon, and the one standard client pattern (connect, subscribe, deadlines, errors, caller identity).
8. **The internal code organisation that supports this,** in brief: the repo layout (sdk/, os/, apps/, services/, mind/ …), dependency direction, and file and module rules.
9. **A prioritised migration plan of 10–15 steps** from today's workspace to this platform. Each step gets a size, a risk, what it unblocks, and how it ships without breaking nightly. Start with what gives apps the most consistency soonest.
