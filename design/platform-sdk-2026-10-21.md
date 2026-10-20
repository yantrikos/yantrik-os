# The Yantrik Platform SDK, and the code organisation under it

**Status:** accepted, 21 Oct 2026. Pranab approved all four decisions marked ★ (section 8).
**Why:** Pranab asked for code that is "modular, organized, like professional and enterprise grade system and not slobs", organised with "standard libraries like Windows, Mac". He also asked for a consultation with Fable and GPT-6 Astra. The briefs and both reviews, kept as written, are in [`platform-sdk/`](platform-sdk/):
- GPT-6 Astra answered from the briefs: `review-gpt6-astra-architecture.md` and `review-gpt6-astra-sdk.md`.
- Fable read the repository: `review-fable.md`.

This document records where they agree, where they differ and what we do.

## 1. What the platform is

Yantrik OS becomes a platform the way macOS and Windows are: a set of **named, standard frameworks** that every app, ours and third-party, is built on. An app imports frameworks and nothing else. A framework either does its work in the app's process or calls an authenticated system service. **No app imports the shell, the Mind's implementation, a service's implementation or another app.**

Both reviewers say the same thing about where we stand: the platform is half built. The pieces exist (`yantrik-surface` and its golden vectors, `app-runtime`, the kit, typed IPC contracts, caller identity from the kernel), but they are not named, not tiered, and not separated from the OS internals. So the first releases **name, narrow, document and standardise what exists**; they do not rewrite it.

## 2. The frameworks

Each framework has three names, as Apple's do: a product name for the docs, a crate, and a namespace in the `yantrik-sdk` facade.

| Framework | Crate | One purpose | Tier at 1.0 | Built from today's |
|---|---|---|---|---|
| **YantrikFoundation** | `yantrik-foundation` | Shared types and rules: ids, time and "ago", text, places, version, problem records, the error type, i18n lookup, a small JSON store with atomic writes | Stable | `yantrik-shown-address`, `yantrik-version`, `app-runtime::{place, problems}`, `i18n.rs`, the **nine** `ago` functions, the hand-rolled JSON stores |
| **YantrikDesign** | `yantrik-design` | The design tokens: colour roles, type scale, spacing, radii, levels, motion, text scale; light and dark | Stable (the token *names*) | `yantrik-design-tokens`, `app-runtime/theme.rs`, `colour_roles` |
| **YantrikUI** | `yantrik-ui-kit` | The component library, in tiers (§4) | Stable | `yantrik-ui-kit` |
| **YantrikApp** | `yantrik-app` | The app model: manifest, lifecycle, `App::builder`, `AppWindow`, single instance, menus and commands, per-app settings, theme following | Stable | `yantrik-app-runtime` (renamed, minus notify, companion and service) and the dormant `yantrik-manifest` |
| **YantrikIntents** | `yantrik-surface` | What an app lets the Mind and agents see and do (`describe`, `act`), as Apple's App Intents | Stable | `yantrik-surface`, `ipc-contracts::control_surface`, the dispatch vectors |
| **YantrikNotifications** | `yantrik-notifications` | Tell the person something: the builder, actions, provenance | Stable | `app-runtime/notify.rs`, `ipc-contracts::notifications` |
| **YantrikServices** | `yantrik-services` | Typed clients for the OS services (calendar, mail, notes, music, weather, network, system), one module each, with the right deadline per method | Preview | the `ipc-contracts` domain modules, `service::client`, `call_typed` |
| **YantrikMind** | `yantrik-mind` | Ask the person's mind from an app (`ask`, `recall`, `tool`, `reach`), with labelled output | Preview | `app-runtime/companion.rs`, `companion_rpc.rs` |
| **YantrikFiles** | `yantrik-files` | Where an app and an agent may read and write; following a moved file; open-with | Preview | `home_paths`, `yantrik-file-follow`, `place`, `mime_dispatch.rs` |
| **YantrikSettings** | `yantrik-settings` | The one schema of `settings.yaml` with typed readers and a watch; per-app settings | Preview | `wire/settings.rs` and the **19** private parsers (the gate's included) |
| **YantrikSecurity** | `yantrik-security` | Who is calling, the gate (grades, ceiling, mode, grant), agent reach, scrubbing, a narrow vault client | Preview | `ipc-transport::{gate, reach, peer_identity, privacy, plain_text}`, `caller_identity.rs`, `vault_unlock.rs` |
| **YantrikServiceKit** | `yantrik-service` | What a background service implements | Stable | `yantrik-service-sdk` |
| **YantrikMindKit** | `yantrik-harness-sdk` | What a mind implements to attach to the desktop | Preview | `harness/{protocol, redact, notice}.rs` |
| **YantrikIPC** | `yantrik-ipc` | JSON-RPC over Unix sockets. Apps reach it through Services, not directly | Preview | `yantrik-ipc-transport` |
| **YantrikTesting** | `yantrik-testing` | Fake services, a deterministic clock, an intents test harness, Slint test helpers | Preview, dev-only | new; the stand-in dispatch in `vectors.rs` |
| — | `yantrik-sdk` | The facade: re-exports only, every framework behind a feature, so a service never links Slint | Stable | new |

**Later, when a real backend or a second consumer exists:** Documents (open, save and recovery, extracted from Notes and the office trio), Contacts, Media, Accessibility and Localization (full catalogues; the lookup is in Foundation from the start).

**Internal, never imported by an app:** the shell, `shell-core`, `yantrik-os`, the companion stack, the harness host, gateway, egress, lock, switcher, chat, brain, `yantrikdb-server`, and every `services/*` daemon. A framework is a client; the daemon behind it is internal (`notifications-service` serves `yantrik-notifications`).

## 3. How an app looks

```toml
# Cargo.toml
[dependencies]
yantrik-sdk = { version = "1", features = ["app", "ui", "intents", "notifications"] }
[build-dependencies]
yantrik-app-build = "1"        # one call; replaces the ten copied build.rs files
```

```slint
import { AppWindow } from "@yantrik-kit/templates.slint";
import { YButton }   from "@yantrik-kit/controls.slint";
```

The kit and tokens are Slint *libraries*, mapped by `with_library_paths`, which Slint 1.17 has and we use nowhere. An app can no longer reach into the shell's `ui/` folder, and the 30 shim files and the include-order collision go.

```toml
# yantrik.toml, the manifest: one declaration, from which the .desktop file is generated
[app]
id = "notes"
name = "Notes"
version = "1.2.0"
sdk = "1.0"
purpose = "quick markdown notes kept in the notes library"
mime = ["text/markdown"]

[entitlements]                 # what it may ever ask for; a declaration is not a grant
notifications = ["reminder"]
mind = ["ask", "recall"]
files = ["home:documents:rw"]
network = false

[intents]
grades-max = "sensitive"       # the highest grade it will publish; yos check holds it
```

```rust
// src/main.rs: at most 40 lines. The app's logic is in src/lib.rs, Slint-free and tested.
fn main() -> yantrik_sdk::Result<()> {
    yantrik_sdk::app::App::builder()
        .manifest(include_str!("../yantrik.toml"))
        .window(AppWindow::new()?)
        .intents(notes::intents)          // describe and act, derived from the same state the view draws
        .run()
}
```

**The app model's rules:**
- **Identity is the launcher's word, not the app's.** The service manager binds a process to its installed bundle. An app that calls itself `settings` gains nothing.
- **Entitlements are checked where a call lands, before the gate.** They are log-only for one beta, then enforced.
- **A permission ("may Calendar read your calendars") is not an approval ("may this agent send this invitation").** Both stay system UI, never the app's.
- **The same typed command backs a button, a shortcut, a menu item and an intent.** An agent's invocation carries who it came from, and is never a trusted gesture.

## 4. YantrikUI: tiers, contract, enforcement

**Tiers**, as directories in the kit:
- `primitives`: YText with roles, Icon, YSurface with levels, YScrim, YDivider, YScrollView, YListView, YKeycap.
- `controls`: YButton, YIconButton, YInput, YField, YTextArea, YSwitch, YSlider, YSegmented, YTabs, YChip, YBadge, YStatusPill, YStepper.
- `components`: YListRow, YCard, YTile, YMetric, YSheet, YDialog, YPopover, YContextMenu, YTooltip, YEmptyState, YSectionHeader, YToolbarButton, YMenuButton, the charts.
- `patterns`: AppHeader, YSidebar, YReading, ToastBanner, NoticeBar, MessageBubble, ToolCall, MindMark, YApprovalCard (a frame only: the approvals feature owns the state and the executor owns the authority).
- `templates`: AppWindow, SettingsWindow, ListDetailWindow, DocumentWindow.

**Every public component provides:**
1. Enum-typed properties: `kind: ButtonKind`, not `variant: int`, so a retired value is a compile error.
2. Every state (rest, hover, pressed, focus, disabled, selected, loading, error), drawn from tokens.
3. Keyboard operation and accessible role, name and value.
4. Light and dark.
5. Text scale.
6. Control-surface hooks (`action-id`, `surface-key`) where it stands for an action or a state.
7. A doc block, a preview scene and a catalogue entry.

**Screens are built only from the kit,** held there by:
- **library paths:** a screen cannot import a shell-private file;
- **`YText` roles:** these remove the 122 pixel font sizes mechanically;
- **`AppWindow` as the only exported `Window`;**
- **the design-system lint (in CI since 21 Oct), widened:**
  - `private-control` also counts `Row|Tile|Item|Sheet|Field|Switch|Toggle|Tab|Menu|Dialog|Modal`, which closes the 124 Fable counted, not just 55;
  - `inherits Window` outside the kit;
  - `deprecated-kit`;
  - a cap on private components per file.

  A domain composition built from kit parts (an `EmailMessageRow` made of `YListRow` and `YBadge`) stays allowed; that is Astra's correction.
- **snapshots:** every component in both themes at 1.0 and 1.25, rendered in CI by the preview harness, which joins the workspace;
- **the catalogue,** generated from the kit's doc blocks and the snapshots;
- **a kit owner:** a CODEOWNER on the kit.

**The Yantrik Human Interface Guidelines** live in `docs/hig/`. They gather what is scattered today:
- the art direction, desk-and-mind, the colour roles and the levels;
- the writing rules;
- the component catalogue and the app model;
- grades and accessibility;
- "what a window costs" (the idle-redraw rule).

A change to a component's behaviour changes its HIG page and its snapshot in the same PR.

## 5. The code organisation under it

```
frameworks/   the public SDK: yantrik-sdk, -foundation, -design, -ui-kit, -app, -surface, -notifications,
              -services, -mind, -files, -settings, -security, -ipc, -service, -harness-sdk, -testing, -app-build
crates/       OS internals: yantrik-shell, shell-core, yantrik-os, the companion stack, harness host,
              gateway, egress, lock, switcher, …
apps/  services/  templates/  examples/
tools/        kit-preview, design-lint, xtask (the architecture checks)
docs/         hig/, frameworks/, architecture.md
```

★ **Folders.** Fable proposes `frameworks/` against `crates/`. Astra proposes `sdk/ platform/ os/ mind/`. We take Fable's: two folders make the public/private line visible in `ls`, and fewer moves are needed.

**The layers.** Dependencies point down, and nothing points at the shell or the Mind:

```
L5  Minds        the built-in companion as its own process; third-party harnesses
L4  Shell, Apps  yantrik-shell, lock, switcher;  apps/* (apps → L0–L2 only)
L3  OS services  services/*, shell-core, yantrik-os, gateway, egress, harness host
L2  Platform     app, intents, notifications, services, mind, files, settings, service, harness-sdk
L1  Design       design, ui-kit
L0  Foundation   foundation, ipc, security
```

★ **The largest move: the Mind becomes its own process.** Today the shell links the companion, `yantrik-ml`, `yantrikdb-core`, IMAP, SMTP and the audio stack, so the trusted UI that draws approval cards links the model runtime. The harness path already runs minds (Hermes) as attached processes behind egress and the gateway. The built-in Mind should attach the same way. Fable rates it the biggest security and build win; Astra's `security-core` and "the executor authorises" point the same way. It is large and high-risk, so it comes late, behind a flag, with both paths running on nightly first.

**One feature pattern, everywhere.** In the shell and in every app:

```
features/<name>/  mod.rs (the facade: wire, actions, State)
                  model.rs   (State and pure rules, no Slint)
                  view.rs    (the only file that touches ui)
                  surface.rs (intents, built from the same State, calling the same model functions)
                  tests.rs
```

- `wire/*.rs`, `control_*.rs` and the flat 180 modules dissolve into these directories.
- Each screen's state is a Slint `export global <Screen>State` in its own file, not 559 properties on the `App` root.
- Screen ids become `export enum Screen`, which deletes `SCREENS` and its mirror tests.

Astra would put the pure models in a separate `yantrik-shell-model` crate. We start with the directories, which are cheaper, and extract the crate only if a second consumer appears.

**Compile memory.** Splitting files is not splitting compilation units, and both reviewers say so. The split has to happen at the process boundary, which the OS already uses for apps, the lock screen and Alt+Tab. About half the shell's reachable 38.7k Slint lines are windows in all but name: Settings, the installer and first run, Permissions, Devices, Agents, packages, About, Files, Recipes, System, Memory and Problems. They move out one per PR, Settings first, each measured as a drop in the shell's build memory. The 17 app screens already compiled by apps move into their apps.

**The rules every PR is held to** (Pranab, 21 Oct; the 7-point checklist):
- **One home per concept.**
- **File size:** 600 lines soft and 1,200 hard, as a ratchet baseline like the lint's that may only shrink.
- **`main.rs`:** at most 40 lines for an app, 80 for the shell.
- **Callbacks** dispatch to the model and return; there is no workflow in a closure, and no logic in a `.slint` file beyond layout and visual state.
- **Errors are typed** with `thiserror`. Strings appear only at the wire, where the protocol wants sentences, and nowhere else.
- **Visibility:** `pub(crate)` facades, with `unreachable_pub` warned on.
- **No grab bags:** no `utils`, `common`, `manager` or a single global error type.
- **Behavioural tests.** The 59 files of source-reading tests shrink to about 10 in `guards/`, keeping only what no type can hold: consent buttons, idle redraw, dependency direction, the rc.xml keys.

**CI enforces, in one `cargo xtask check-architecture`:**
- the dependency direction and tiers (an app depends on no `crates/*`, Tier 1 depends on no Tier 2, no `build.rs` joins `../../crates`), plus `deny.toml`;
- the file-size and design-lint ratchets;
- `cargo public-api` and `semver-checks` on the frameworks;
- a sample app that builds outside the workspace;
- clippy blocking, crate by crate;
- a pinned `rust-toolchain.toml`;
- `CARGO_BUILD_JOBS` capped so two UI crates never compile together.

## 6. Where the code contradicted my brief (Fable checked)

- `yantrik-ml`: only the in-tree copy ever builds (the git dependency is patched to it). The fix is to **delete the git dependency and the patch now**, and remove the in-tree copy once the Mind is a process. It is not a repo reconciliation.
- The shell's Slint crate compiles only `app.slint` (38.7k lines). The other 17 app screens (18.5k lines) live in the shell's folder and are compiled by apps through a relative path.
- `onboarding.slint` (1,426 lines) looks dead.
- Apps are binary-only, so 14 `tests/*-core` crates `#[path]`-include their sources to test them.
- `crates/yantrik-manifest` is used by nothing. `yantrik-mcp` is not a workspace member.
- `examples/hello-window` imports `std-widgets`.
- `docs/architecture.md` is from March and says "5 crates".

## 7. The migration, in order (none of it stops nightly)

| # | Step | Size, risk | Unblocks |
|---|---|---|---|
| 1 | **The rules, written and failing:** `docs/architecture.md` (this design), `deny.toml`, the dependency-direction and file-size ratchets in `xtask`, a pinned toolchain. Report first, then fail only new violations | S, low | every later step |
| 2 | **Library paths and no shims:** `@yantrik-design` and `@yantrik-kit`, delete the 30 shims, `yantrik-app-build` | S, low | the kit tiers; screens that can move |
| 3 | **App screens into their apps;** delete `onboarding.slint` if dead | S–M, low, one PR per app | apps as independent compile units |
| 4 | **YantrikFoundation and one `ago`** (delete the nine), one text module, the JSON store | S, low | the SDK facade; the "one rule" fixes |
| 5 | **Apps become `lib + bin`; delete `tests/*-core`** | M, low | `App::builder`, `yos new app` |
| 6 | **YantrikApp, Notifications, Mind, Services and the `yantrik-sdk` facade;** `App::builder`; `app-runtime` kept as deprecated re-exports for one release; `templates/rust-app` and `yos new app` | M, low | third-party apps; the template |
| 7 | **Kit tiers and the missing components:** `YText` first, then YListRow, YCard, YChip/YBadge, YSheet, YScrollView/YListView/YTextArea, YSectionHeader, YToolbarButton, YIconButton, MindMark; enum kinds; the lint widened | L overall, S per PR, low | "screens only from the kit" |
| 8 | **The preview harness in the workspace, snapshots in CI, the catalogue generated;** HIG v0 in `docs/hig/` | M, low | review by pixels; the HIG |
| 9 | **The shell becomes a library with feature directories,** one directory per PR, mechanical moves only | M (many PRs), low–medium | size limits, visibility, the feature pattern |
| 10 | **YantrikSettings,** with the gate reading the ceiling through it, byte-identical and proven by settings vectors | M, medium | live theme in apps; per-app settings |
| 11 | **Per-screen globals and the `Screen` enum,** each `describe` snapshotted first | M, medium | moving screens out |
| 12 | **Screens out of the shell, Settings first,** one per PR, each measured | L, medium | the 14 GB problem; CI's 12 GB swap |
| 13 | **Manifest and entitlements:** log-only for one beta, then enforced, with a full security review | M, high | a real third-party app story |
| 14 | **The Mind as its own process,** behind a flag with both paths on nightly; the shell drops the companion, ML, yantrikdb-core, mail and audio | L, high | the security story; build times; one `yantrik-ml` |
| 15 | **Close the loop:** blocking clippy, public-API checks, the external sample build, SDK 1.0 | S, low | a platform that cannot drift back |

**The gate for SDK 1.0** (Astra), which we keep:
- several first-party apps use only public frameworks;
- one third-party Rust app builds outside the workspace;
- an app built against an older SDK runs on today's nightly;
- permission denial, revocation and a service restart behave correctly;
- a hostile app cannot take another app's identity;
- a confined mind cannot get around egress;
- an approval cannot be replayed for a changed action;
- the build fits the host's memory budget.

## 8. Decisions (approved by Pranab, 21 Oct 2026) ★

1. **Adopt this platform shape and the framework names** (YantrikFoundation, YantrikUI, YantrikApp, YantrikIntents, …).
2. **Folders:** `frameworks/` (public) against `crates/` (internal).
3. **The Mind as its own process** (step 14): the largest move, done late and behind a flag.
4. **Steps 1–5 start now,** in parallel with the UI work. They are small, low-risk PRs that remove most of the slop signal: shims, path hacks, duplicate helpers and the test-crate workarounds.
