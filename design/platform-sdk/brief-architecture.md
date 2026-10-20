# Architecture review: make Yantrik OS's code enterprise grade, modular and reusable

You are a principal software architect. You have built large desktop and OS codebases: think of the people behind the Chromium and Fuchsia component architectures, GNOME's libadwaita, the Zed editor's GPUI and crate layout, and Apple's SwiftUI design-system teams. The owner asked for "the best possible software architecture and reusability and libraries". He wants code that reads as professional and enterprise grade, "not slop": modular, organised, consistent. Be concrete and opinionated, name crates and modules, and say what to delete as readily as what to add.

## What Yantrik OS is
A Linux desktop OS built around an AI companion called the Mind. The Mind remembers, notices, and acts on the person's behalf, asking before risky things. It is written in **Rust**, and the UI is **Slint 1.17**: `.slint` markup compiled to Rust. The shell uses the GL renderer and Alt+Tab uses the software renderer. The compositor is labwc (Wayland), and we carry a patch to it. The target is Debian 13 ISOs, and it also runs in VMs with software GL.

It is one Cargo workspace (repo `yantrik-os`) of **38 crates, 21 apps and 10 services**, plus git dependencies from three sibling repos (`yantrikdb`, the memory database; `yantrik-ml`; `yantrik-companion`), patched to local paths. A known wart: two copies of `yantrik-ml` exist, one in this repo and one in the companion repo, and the Mind builds from the companion's.

### Main pieces and their size (Rust lines; Slint lines where it has any)
- `crates/yantrik-ui`, **131,722** lines: the shell binary. About **180 flat modules** in `src/` (`control_*.rs` ×30, `wire/*.rs` ×40, `approvals.rs`, `bridge.rs`, `mind_mode.rs`, `agents/`, `lens.rs`, …). It owns Slint wiring, the control surface (an RPC an agent uses to describe and act on the shell), approvals, the bridge to the companion, the window list, notifications and more.
- `crates/yantrik-ui-slint`, **58,492** lines of `.slint` in 137 files: every shell screen, compiled into one huge generated crate. rustc takes about 14 GB of RAM for it and rebuilds the whole thing on any `.slint` edit.
- `crates/yantrik-ui-kit`, 6,574 Slint lines and 2,251 Rust: **40 shared components** (YButton, YInput, AppHeader, AppTile, dialogs, tabs, sliders, charts) and guard tests.
- `crates/yantrik-design-tokens`: one `theme.slint` (787 lines) with colours, a type scale, spacing, radii, shadows, light and dark, and theme files (TOML).
- `crates/yantrik-app-runtime`, 4,792 lines: the scaffolding every app binary uses (window, control surface, IPC).
- `crates/yantrik-companion`, 71,912 lines; `-tools`, 22,295; `-instincts`, 8,131; `-cortex`, 5,055; `-core`, 5,132: the Mind's brain (tools, memory, recipes, proactive engine).
- `crates/yantrik-harness`, 9,463 lines: hosts third-party "minds" (agent harnesses).
- `crates/yantrik-ipc-contracts`, 5,570, and `yantrik-ipc-transport`, 6,444: typed IPC between shell, apps and services.
- `crates/yantrik-surface`, 3,828: the describe/act control-surface protocol shared by apps.
- `apps/*`: 21 apps, each a thin binary (`main.rs` plus `ui/app.slint`) that wraps a screen. Several `main.rs` files are 2,000–2,700 lines (email, calendar, download-manager).
- `services/*`: 10 background services (notifications, email, calendar, network, perception, weather, …).

### The largest files
`companion.rs` 6,898 · `harness/host.rs` 4,539 · `control_approvals.rs` 4,003 · `settings.slint` 3,955 · `app.slint` 3,862 · `control.rs` 3,794 · `bridge.rs` 3,737 · `mind_mode.rs` 3,629 · `recipe_executor.rs` 3,217 · `wire/agents.rs` 3,188 · `browser.rs` 2,860 · `agents/store.rs` 2,785 · `email main.rs` 2,689 · `approvals.rs` 2,613 · `intent_lens.slint` 2,551 · `wire/dock.rs` 2,540.

### The UI consistency audit (21 Oct 2026)
- 124 private components defined inside screens: 11 buttons, 30 list rows, 13 chips or badges, 11 cards.
- 81 raw hex colours in 13 files, and 122 pixel font sizes in 16 files.
- 16 files import Slint's stock widgets.
- Duplicates found and fixed this week: three keycap components, and the same reading-column layout math in three screens.
- A design-system lint now runs in CI. It fails on raw colours, pixel font sizes, private controls and std-widgets imports, with a ratchet baseline that may only shrink.

### How the code works today
- **Rust ↔ Slint:** state lives in Slint globals and `in-out` properties. `wire/*.rs` modules set them from Rust and register callbacks. Models are `VecModel<Struct>`, built from Rust structs each refresh.
- **Logic:** many rules are already pure Rust functions with unit tests (e.g. `mind_panel::describe_of`, `memory_rows::rows`). Others live inside `wire` closures or inside `.slint` expressions.
- **Tests:**
  - about 1,700 shell unit tests and 560 companion tests;
  - many source-reading tests that open `.slint` or `.rs` files and assert on text, to hold invariants such as "this screen uses the kit's button";
  - a `tests/ui-preview` harness that renders components to PNGs, outside the workspace and not in CI;
  - screenshots from a VM (520) for visual checks.
- **The security model is central:**
  - approval cards for risky actions, a per-machine ceiling and modes;
  - caller identity from the kernel;
  - model-written text always labelled;
  - prompt-injection resistance.

  Security reviews run on every PR and routinely find issues.
- **Process:** small PRs, each with a security review and a test on VM 520, merged into nightly, beta and stable channels.

## Constraints
- Rust and Slint stay. No web stack.
- It must keep building on one 48 GB build host and in CI. The giant `yantrik-ui-slint` crate is already a memory and time problem.
- The control surface (agents describe and act on every screen and app) is a core product feature, not an afterthought. It must stay complete and typed.
- Security boundaries must stay explicit: approvals, caller identity, egress, the vault.
- Migration must be incremental: the OS ships nightly, so there is no big-bang rewrite.

## What I want back
1. **The target architecture in one page.** The layers (for example foundation, design system, platform services, shell, apps, the Mind), what each may depend on, and the rule that keeps dependencies pointing one way. Name the crates, existing and new, and their single responsibility.
2. **The UI component library.**
   - How to structure the kit: tiers such as tokens → primitives → components → patterns → templates.
   - What a component must provide: states, accessibility, its control-surface hooks, light and dark, text scale.
   - How screens are built only from it.
   - The concrete list of components the kit is missing, given the audit.
   - How to stop drift, beyond the lint we have.
3. **The Rust ↔ Slint pattern.**
   - One way to structure a screen or feature: view model, state, actions and effects, wiring.
   - Where logic lives and where it must never live.
   - How to make each feature a testable unit.
   - How to keep generated-code size and compile memory down (splitting `yantrik-ui-slint`?).
4. **Shared libraries beyond UI.** What should be one shared crate instead of copies: time and "ago" formatting, text cleaning, IPC client patterns, settings and config, persistence, error types, logging, i18n. Also the `yantrik-ml` duplicate.
5. **Module and file rules.** Size limits, naming, one concept per module, public-API discipline (`pub(crate)` and facades), and how to split the 180-module flat shell into feature modules or crates. A concrete target tree for `crates/yantrik-ui/src`.
6. **Testing and enforcement.**
   - Which tests at which layer: pure model tests, component snapshot tests, contract tests for IPC and the control surface, end-to-end on a VM.
   - Which source-reading tests to keep and which to replace.
   - What CI enforces automatically: dependency direction, file size, the lint, public-API checks.
7. **A prioritised migration plan of 10–15 steps,** most value per effort first. Each step gets a size (small, medium or large), a risk, what it unblocks, and how to do it without stopping nightly releases.
8. **What to delete or stop doing.**
