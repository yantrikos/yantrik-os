# Yantrik OS architecture

How the code is organised, which way dependencies point, and the rules every pull request is held to. The reasoning, the framework catalogue and the full migration are in [design/platform-sdk-2026-10-21.md](../design/platform-sdk-2026-10-21.md), which Pranab accepted on 21 October 2026. The architecture of March 2026, written when the OS was five crates, is kept in [history/architecture-v1-2026-03.md](history/architecture-v1-2026-03.md).

Stack: Debian 13 → labwc (Wayland) → the Yantrik shell, apps and services, all in Rust with Slint UI.

## The platform

Yantrik OS is being shaped as a platform: a set of named, documented frameworks that every app, ours and third-party, is built on. These include YantrikFoundation, YantrikUI (the design tokens and the kit), YantrikApp, YantrikIntents (the control surface), Notifications, Services, Mind, Files, Settings and Security, under one `yantrik-sdk` facade. Today most of them exist as crates under other names. The migration (design §7) moves them into place one small PR at a time, without stopping nightly.

## Folders

| Folder | What lives there |
|---|---|
| `frameworks/` | The public SDK, as the frameworks are formed (migration step 6 on). Apps may use only these. |
| `crates/` | OS internals: the shell, shell-core, the system layer, the companion stack, the harness host, gateway, egress, lock, switcher. |
| `apps/` | One crate per app. |
| `services/` | One daemon per service, each with a typed IPC API. |
| `templates/`, `examples/`, `docs/sdk/samples` | What an app author copies. Held to the same rules as an app. |
| `tools/` | The workspace's own tooling: `xtask` (these checks), evaluations. |
| `tests/` | Test crates that `#[path]`-include app sources. They go away when apps become `lib + bin` (step 5). |
| `docs/` | This file, the HIG (`docs/hig/`, from step 8) and the framework guides. |

## The layers

Dependencies point down, and nothing points at the shell or a mind.

| Layer | Holds | May use |
|---|---|---|
| L5 Minds | the built-in companion and what only it uses (`yantrik-companion*`, `yantrik-ml`, `yantrik-chat`) | L0–L5 |
| L4 Shell | `yantrik-ui`, `yantrik-ui-slint`, lock, switcher | L0–L4 |
| L3 OS services | `services/*`, shell-core, `yantrik-os`, gateway, egress, harness host | L0–L3 |
| L2 Platform | app-runtime, surface, service-sdk | L0–L2 |
| L1 Design | design tokens, ui-kit | L0–L1 |
| L0 Foundation | IPC contracts and transport, version, and small pure libraries | L0 |
| app | `apps/*`, templates, examples | L0–L2 only, never another app |

Where each crate sits is listed in [tools/xtask/layers.txt](../tools/xtask/layers.txt). A crate that is not listed fails the check, so **a new crate is placed on purpose**: add its line in the same PR.

The largest break today is the shell linking the companion, `yantrik-ml` and `yantrik-chat` (L4 → L5). The trusted UI that draws approval cards should not link the model runtime. That is fixed by making the Mind its own process (step 14), late and behind a flag.

## The rules every pull request is held to

1. **One home per concept.** Anything used twice goes into the kit or one shared module, never copied.
2. **File size:** 600 lines is soft, 1,200 hard. A file over the soft limit is split when next touched: the part being worked on moves into its own focused module. No new file passes the hard limit.
3. **`main.rs`** is a few lines that start the app: at most 40 for an app, 80 for the shell.
4. **Logic stays out of `.slint`** beyond layout and visual state. Callbacks dispatch to the model and return. Rules live in pure Rust modules with tests.
5. **Colours, sizes and spacing come from theme tokens,** and screens are drawn from the kit (the design lint in `yantrik-ui-kit`).
6. **Errors are typed** with `thiserror`. Sentences appear only at the wire, where the protocol wants them.
7. **Visibility:** `pub(crate)` facades; nothing is public that another crate does not use.
8. **No grab bags:** no `utils`, `common`, `manager`, and no single global error type.
9. **Tests check behaviour.** Tests that read source text are kept only where the rule is about the source itself (consent buttons, dependency direction, labwc keys).
10. **No dead code,** unused imports, new warnings or stale comments.
11. **The PR body says what it split, moved or deleted,** not only what it added.

## What CI enforces

**`cargo run -q -p xtask -- check-architecture`** (the `architecture` job; source in [tools/xtask](../tools/xtask)):
- **layer:** a dependency that points up, or from an app to anything but L0–L2;
- **build-path:** an app's or a framework's `build.rs` that reaches into another crate's folder by a relative path, instead of using that crate's library;
- **size:** a tracked `.rs` or `.slint` file over 1,200 lines.

What broke a rule on the day the rule landed is listed in [tools/xtask/architecture-baseline.txt](../tools/xtask/architecture-baseline.txt). The check fails on anything new. It also fails when a listed entry is fixed but still listed, so that a fix is kept. The baseline may only shrink. After a fix, rewrite it with:

```sh
cargo run -q -p xtask -- check-architecture --write
```

`--write` refuses to add entries, and on a pull request CI runs the check with `--since` the base branch's baseline, so a line added to it by hand fails too. The check also reports how many files are over the soft limit, and which listed files grew.

**The design lint** (`cargo test -p yantrik-ui-kit design_lint`) holds hex colours, pixel font sizes, private copies of kit controls and `std-widgets` imports to their own baseline, in the same way.

**`deny.toml`** (the `dependencies` job, cargo-deny) checks advisories, licences (compatible with GPL-3.0), duplicate versions and sources. It reports and does not fail yet.

**`rust-toolchain.toml`** pins the Rust the nightly ships with. The build host, CI and a laptop all build with it, and moving it is a PR of its own.

To come, as the migration reaches them: the stability tiers (an app uses no `crates/*`), `cargo public-api` and `semver-checks` on the frameworks, a sample app built outside the workspace, and clippy blocking crate by crate.

## The breaks in the baseline, and the step that removes each

| Break | Step |
|---|---|
| 18 apps compile their screens from `crates/yantrik-ui-slint/ui` (build-path) | 2–3: library paths, app screens into their apps |
| The shell links the companion, `yantrik-ml` and `yantrik-chat` | 14: the Mind as its own process |
| The gateway links `yantrik-ml` | 14 (one `yantrik-ml`) |
| `docs/sdk/samples` uses shell-core and the surface template | 6: the SDK facade |
| 83 files over 1,200 lines | 9 and 12 for the shell; each file whenever it is next touched |

## One feature, one directory

In the shell and in every app (from step 9):

```
features/<name>/  mod.rs      the facade: wire, actions, State
                  model.rs    State and pure rules, no Slint
                  view.rs     the only file that touches the UI
                  surface.rs  the intents, from the same State and model functions
                  tests.rs
```
