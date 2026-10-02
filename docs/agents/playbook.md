# Playbook for agents building the UI/UX overhaul

Read this whole file before you touch code. Your story brief says what to build; this file says how to work in this repository so the result can be merged without rework. It applies to agents running locally and to cloud agents.

## What you're building toward
Yantrik OS is a Debian desktop where AI minds work beside a person. It is written in Rust and Slint, on the labwc Wayland compositor.

A reviewer compared its UI with Omarchy and called it "really really bad". The design work since then is in `design/`:
- `design/ui-overhaul-2026-10-01.md`: the audit, the benchmark against other desktops and the phased plan. Read §1 for your area, §4.3 for the control-surface fields, and §6 for your phase.
- `design/ui-review-gpt6-astra-2026-10-02.md`: an outside review whose rules are adopted: opaque panels, honest copy, teal only for minds, amber only for "needs you".
- `design/minds-surfaces-spec-2026-10-02.md`: the spec for the chat, the Agents workroom, the dock and Alt+Tab, with sizes, states and copy.
- **The chosen look** comes from GPT image renders (owner's choice):
  - a photographic wallpaper;
  - solid charcoal surfaces (#151A1E panels, #101417 bars) with 1px neutral borders and 16px radii;
  - white line icons and Barlow type;
  - a slim icons-only dock grounded on the bottom edge like the macOS Dock (not floating): centred, as wide as its icons, and maximised apps end above it.

**The bar for "done":**
- A person sees the difference in the first minute.
- It holds up beside GNOME, KDE and Omarchy.
- It never lies about the machine. A slider shows the real level, and an indicator for hardware that isn't there isn't drawn.

## Where to work
- Work on the branch your brief names, created from current `origin/main`.
- Never push to `main`. Push your branch and open a PR with `gh pr create --base main`.
- Don't touch any machine other than your own build environment. No ssh, and nothing on VMs 520, 560 or 561, or on the family box. The parent session checks the result on a real machine after merge.
- Never read, print or commit secrets.

## Building and testing
- `cargo check --offline -p yantrik-ui` catches most errors fast. If `--offline` fails in a fresh environment, drop it for the first fetch.
- Run the full suite with `cargo test -p yantrik-ui --bin yantrik-ui`, plus the crates you touched. CI also runs `cargo build --workspace --locked`, `cargo test --workspace --locked` and the selftests listed in `docs/CONTRIBUTING.md`. Run the parts your change touches before opening the PR.
- `yantrik-ui-slint` is one huge generated crate. Any `.slint` change recompiles it in about 10 minutes and uses about 14 GB of RAM, so batch your Slint edits before building. Use `CARGO_INCREMENTAL=0` if disk is tight.
- **Rendered UI checks.** `tests/ui-preview` draws the real Slint screens headlessly. Read `tests/ui-preview/validate.sh` and `tests/ui-preview/src/main.rs` to see how a scene is written; `verify-controls`, `verify-idle`, `verify-taskbar-menu` and `verify-cards-waiting` are good models.
  - Run a check with `cargo run --manifest-path tests/ui-preview/Cargo.toml --profile fast -- target/ui-validation/<name>.png 1280 800 <verify-name>`.
  - **Look at the PNGs you produce.** Most UI bugs in this project were found by looking at pixels, not by reading source.
- **Shared target directories:** if you share a `CARGO_TARGET_DIR` with other worktrees, cargo can build another worktree's copy of a workspace crate. If you get an error about code you didn't change, build in a private target directory.
- **Rebase before you open the PR,** and again if asked: `git fetch origin && git rebase origin/main`, keep both sides, rebuild, re-run, then `git push --force-with-lease`. Other stories merge while you work.
- **Executable scripts:** any new script with a `#!` line needs `git update-index --chmod=+x <file>`, or CI fails.

## How code is written here (from docs/CONTRIBUTING.md; enforced in review)
- **One concern per PR.** If you find something else wrong, note it in the PR description. Don't fix it in this PR.
- **Reuse, never duplicate.** One shared component or module, used everywhere. Kit components live in `crates/yantrik-ui-kit/slint/` (YIndicator, YPopover, YSlider, YToggleTile and others), and the shell re-exports them through a shim in `crates/yantrik-ui-slint/ui/components/`. Never write a second copy of anything.
- **Small files.** Add a focused new file rather than growing a large one. `app.slint` is about 3,000 lines; add only the wiring you must.
- **Tokens, not literals.** Colours, sizes, radii and durations come from `crates/yantrik-design-tokens/slint/theme.slint`. Add tokens there; no hex colours or magic px in components. Use the `fs-*` type scale.
- **Idle CPU is a feature.** The machine often runs Slint's software renderer.
  - No repeating Slint timers for state. Data arrives from Rust on events, and a hide-after-delay is one single-shot timer.
  - Animate only while something changes. A `verify-idle`-style check showing 0 redraws once settled is the proof.
  - The kit's `an_idle_window_stops_drawing` test reads every Timer. Its interval must be a readable literal, and its `running` must stop it after its job.
- **Off the UI thread.** No blocking D-Bus, process or network call runs on the UI thread or inside a control-action handler. Use the runtime's `answer_later` with a worker, and a timeout of about 2 s on every D-Bus connection. This exact bug was found in two PRs in one day.
- **Comments say why,** often naming what went wrong without the code. Read a few nearby files and match their density and voice.
- **Words a person reads are true and plain.** No marketing words and no number that isn't measured. A control for absent hardware is hidden, never greyed with a fake value.
- **Parity: what a pointer can do, a mind can ask for.** Every new control is also on the shell's control surface: fields in `describe shell`, actions in `act shell`, with honest grades (safe < standard < sensitive < dangerous) and answers that read the result back.
  - Follow `crates/yantrik-ui/src/control.rs` and its tests.
  - An action that brings a window or overlay over the shell must call `crate::card_watch::hold_windows("<action>")?` first. A source-scan test enforces it.
- **A bug fix comes with the test that would have caught it.** Many tests here are named for the mistake they prevent.
- **Security boundaries:** the control surface and permission gate (`control*.rs`), approvals and cards, the companion's tools and taint rules, `yos-mcp`, memory grants, the vault, the updater and harness installs. If your story touches one, say so at the top of the PR, because it gets a security review.

## Git
- **Commit messages** say what changed and why, in sentences. The subject is a plain statement of the user-visible result.
- **End every commit message** with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`, or with your own model line if you're a different model.
- **End the PR description** with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
- **The PR description covers:**
  - what a person sees now;
  - what changed underneath;
  - the control-surface fields and actions added;
  - the tests and preview checks, with their results;
  - the screenshot paths;
  - anything left out, or found but not fixed.

## Your final report to the parent (under 300 words)
- the PR URL and the branch;
- what a person now sees;
- the files added and changed;
- the test and preview results, with exact commands and pass counts;
- the screenshots;
- the control-surface fields and actions added;
- anything you couldn't verify, and anything you found that belongs in another story.

Don't claim anything you didn't run.
