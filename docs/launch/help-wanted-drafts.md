# Help-wanted issue drafts

**Drafts. None of these has been filed.** Each is a bigger, well-bounded area an outside
contributor could own. Before filing, re-check that the files named still exist and that no open
issue already covers it (the references were current on 29 Oct 2026). Suggested labels are in each
header.

---

## 1. Write a new app on YantrikApp, or bring a small one onto it

**Labels:** `help wanted`, `area/apps`, `enhancement`

**What.** Yantrik apps are built on the YantrikApp framework: a plain-Rust model, one command
type, and a surface a Mind can read and act on, so a person's button and an agent's action are
the same code path. We would like outside people to write new apps, or to bring a small existing
program onto the framework. Candidates: a calculator, a clock and timers app, a contacts app, a
PDF viewer front-end.

**Where to start.**
- Read `design/app-framework-2026-10-21.md` (sections 1 and 2) and `CONTRIBUTING.md` ("Writing an
  app: YantrikApp").
- Copy the shape of `apps/weather`.
- Grade every action honestly (`docs/sdk/grades.md`) and design `describe` for a model to read
  (`docs/sdk/describe.md`).
- `yos check <app>` must pass.

**Done when.** The app builds in the workspace, has headless tests through `yantrik-testing`,
wears the shared `AppHeader`, has a `.desktop` file, passes `tests/app-lints`, and is placed in
`tools/xtask/layers.txt`. Open the issue first with a one-paragraph proposal, so we can say
whether it overlaps with work in flight.

**Not in scope.** Anything that stores or handles credentials (the vault work is in progress:
#537, #538, #1015), or that changes the permission gate.

---

## 2. A theme (or three)

**Labels:** `help wanted`, `area/desktop-shell`

**What.** The default theme is Calm Graphite. A theme is a YAML file of tokens
(`~/.config/yantrik/theme-override.yaml`; the keys are listed in `CONTRIBUTING.md`). We want a
small gallery of themes people actually like: a high-contrast one (this also moves #21), a warm
light theme, a dark one that is not graphite-grey.

**Done when.** Each theme is a file in a location the maintainers pick in the thread, with
screenshots of the shell and three apps in it, and text meeting a 4.5:1 contrast ratio (7:1 for
the high-contrast one). A screen where a token does not reach is a bug worth reporting: those are
real findings (`design/ui-sweep-2026-10-17.md` shows the method).

**Related.** #21 (high-contrast variant), #220 (light-mode wallpaper scrim).

---

## 3. Real-hardware testing and reports

**Labels:** `help wanted`, `area/installer`, `parity`

**What.** Every published image is booted in QEMU with BIOS firmware, and that is all that is
measured. Real hardware, UEFI, GPUs and Wi-Fi chips are untested
(`docs/hardware-requirements.md`). We would like a compatibility table filled in by people with
real machines.

**How.** Boot the nightly live, run `yos describe shell` and `scripts/first-boot-check.sh`, try the
disk installer on a spare disk, and open an issue with the machine model, firmware mode, GPU,
Wi-Fi chip, image version and what happened. A small script that collects those details without
personal data (no serial numbers, MAC addresses or hostnames) would be a good first contribution
in itself.

**Done when.** `docs/hardware-requirements.md` gains a "reported working / reported broken"
table sourced from linked issues, each with the date and image version.

**Related.** #403, #270 (ARM64).

---

## 4. Translations and right-to-left layout

**Labels:** `help wanted`, `area/i18n`

**What.** The interface is English, with some Arabic strings present and no right-to-left
layout (#6). Two pieces: (a) get one screen's strings out of the Slint markup and Rust into a
translatable form (Slint supports `@tr()` and gettext), as a pilot, and (b) RTL layout mirroring
for that screen.

**Where to start.** Ask in the issue which screen to pilot; Settings or Notes are reasonable
choices. Words a person reads in this project are plain and true, and translations follow that
rule (`CONTRIBUTING.md`, "Making a change").

**Done when.** One screen is translatable end to end with a test that fails when a string is
added without being marked; one non-English language is complete for it, and RTL mirrors it
correctly in a screenshot.

**Not in scope.** Machine-translating the whole UI in one pass.

---

## 5. A harness for another agent

**Labels:** `help wanted`, `area/ai-companion`

**What.** A Mind attaches to the desktop through a small set of JSON-RPC methods
(`docs/harness.md`). Hermes, Pi, DeepSeek and OpenClaw have adapters in `harnesses/`. If you use
another agent, write the adapter, run the shared tests in `harnesses/tests`, and document what the
agent could and could not do through `yos-mcp`.

**Where to start.** `crates/yantrik-harness/examples/echo_harness.rs` is a working harness end to
end; the only part a real harness replaces is the function that produces the answer.

**Done when.** The adapter attaches, answers a turn, streams, handles cancel and appears in the
mind picker; the shared tests cover it; the harness's README says what is untested.

**Not in scope.** Storing the agent's key in the OS: the protocol deliberately has nowhere to put
one.
