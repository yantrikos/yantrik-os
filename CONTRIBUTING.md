# Contributing to Yantrik OS

Yantrik OS is a Debian-based Linux desktop built so that AI agents can work on it beside a person,
safely: every app publishes what it holds and what it can be asked to do, and a permission gate
decides what runs without asking. It is GPL-3.0, written mostly in Rust with a Slint UI, and small
enough that the people who wrote the code read every pull request.

You do not need to understand all of it to help. Most of the useful work below touches one app,
one harness or one doc.

## Where to start

Pick whichever fits what you know:

- **A starter issue.** The ones labelled
  [`good first issue`](https://github.com/yantrikos/yantrik-os/issues?q=is%3Aopen+label%3A%22good+first+issue%22)
  are scoped, real and unassigned. Comment with the one you want and it is yours.
- **Make an app agent-ready.** An app becomes something a mind can use by publishing a *surface*:
  `describe` (what it holds) and `act` (what it can be asked to do, each action graded). Start
  with [`docs/sdk/python-quickstart.md`](docs/sdk/python-quickstart.md) or
  [`docs/sdk/rust-quickstart.md`](docs/sdk/rust-quickstart.md), then
  [`docs/sdk/wrap-an-app.md`](docs/sdk/wrap-an-app.md) for an existing program.
- **Improve a harness.** Minds attach through [`harnesses/`](harnesses): Hermes, Pi, DeepSeek and
  OpenClaw, each a small Python adapter. If you use one of those agents day to day, you know it
  better than we do. The protocol is in [`docs/harness.md`](docs/harness.md); the shared tests are in
  `harnesses/tests`.
- **Add a tool to the built-in companion** (Rust), or a **YAML plugin / theme** (no Rust): see
  [Recipes](#recipes) below.
- **Run it on real hardware.** Every image is boot-tested in QEMU, and real machines and UEFI are
  untested ([`docs/hardware-requirements.md`](docs/hardware-requirements.md)). A report of what worked
  and what did not on your laptop is a real contribution: open an issue with the machine, the
  image version and what you saw.
- **Docs.** If something here or in `docs/` was wrong or unclear when you tried it, a fix to the
  doc is as welcome as a fix to the code.

## Setting up

You need Linux (Debian or Ubuntu; WSL2 on Windows works) and stable Rust 1.92 or newer (Slint 1.17
requires it). The system libraries are the ones CI installs:

```bash
sudo apt-get install -y --no-install-recommends \
  pkg-config libasound2-dev libudev-dev libssl-dev libspeechd-dev libwayland-dev \
  libxkbcommon-dev libfontconfig1-dev libfreetype6-dev libinput-dev libgbm-dev libegl1-mesa-dev
```

Build and test the way CI does:

```bash
cargo build --workspace --locked
cargo test --workspace --locked
```

The whole workspace takes a while the first time. While working on one part, test just that part,
for example `cargo test -p yantrik-ui --bin yantrik-ui` for the shell or
`cargo test -p yantrik-companion-tools` for the companion's tools.

The memory engine, YantrikDB, is a git dependency pinned to an exact revision in the root
`Cargo.toml`. Leave that pin as it is unless you are changing the engine too; the comment beside it
says how to point the build at a local checkout of it.

### Seeing it run

The quickest way to see a change is to boot the published image in QEMU and work against it,
rather than to build an image (a full ISO build takes about an hour):

```bash
sh install.sh --download                       # the current nightly image, with its checksum
qemu-system-x86_64 -enable-kvm -m 4096 -smp 4 \
  -cdrom yantrik-os-<version>.iso -boot d -device virtio-vga -display gtk
```

4 GB and 4 CPUs is what CI boots every published image with. To build an image from your own
tree, `deploy/yantrik-os/build-debian-iso.sh` is what CI runs, and
`deploy/yantrik-os/boottest.py <iso> <outdir>` is the boot check it has to pass. (The Alpine-era
scripts beside them, `setup-alpine-vm.sh`, `build-iso.sh`, the `*vbox*` ones and
`deploy-stack.sh`, are history and no current path uses them.)

For UI work, `tests/ui-preview/validate.sh` draws the real Slint screens headlessly, presses
buttons and writes screenshots under `target/ui-validation/`. On a running machine,
`scripts/screen-survey.sh` photographs every screen. Look at the pixels: most UI defects in this
project were found by looking at a running machine, not by reading the source.

## What CI checks

Every pull request runs two jobs, and both must pass:

- **test**: `cargo build --workspace --locked` and `cargo test --workspace --locked`.
- **shell scripts**: every script with a `#!` line is executable and parses, and the tools written
  in Python and shell run their own selftests:

  ```bash
  python3 deploy/yantrik-os/yos-mcp-selftest.py
  python3 deploy/yantrik-os/yos-selftest.py
  python3 deploy/yantrik-os/yantrik-mind-launch-selftest.py
  python3 deploy/yantrik-os/server/publish_selftest.py
  bash deploy/yantrik-os/yantrik-update selftest
  sh deploy/yantrik-os/yantrik-shell selftest
  bash deploy/yantrik-os/service-bins.sh selftest
  python3 -m unittest discover -s harnesses/tests -v
  ```

  A new script needs its executable bit in git, or CI fails:
  `git update-index --chmod=+x path/to/script`.

## Making a change

- **One concern per pull request**, on a branch. A small PR is reviewed the same day; a large one
  waits.
- **A bug fix comes with the test that would have caught it.** Many tests here exist to stop one
  specific mistake coming back, and say so in their names.
- **Comments say why.** The code around you will show the style: a comment explains the reason for
  a choice, often with what went wrong without it, not what the next line does.
- **Small modules.** Prefer a new focused file to growing a large one, and reuse a shared component
  rather than writing a second copy of it.
- **Words a person reads are true and plain.** UI text and docs state what the software does,
  with its limits beside the claim. No marketing words, and no number that is not measured.
- **Commit messages** say what changed and why, in sentences.

Some areas are security boundaries, and a change to them gets a careful review: the control surface
and the permission gate (`crates/yantrik-ui/src/control*.rs`, `crates/yantrik-ipc-transport`), the
companion's tools and taint rules (`crates/yantrik-companion-core`), the MCP bridge
(`deploy/yantrik-os/yos-mcp`), the vault, and the updater (`deploy/yantrik-os/yantrik-update`).
Changes there are welcome; expect questions.

## Reporting a security problem

Please do not open a public issue for a vulnerability. Use GitHub's private report: the
**Security** tab of the repository, then **Report a vulnerability**. See [`SECURITY.md`](SECURITY.md).

## Recipes

### A companion tool (Rust)

Each tool category lives in `crates/yantrik-companion-tools/src/<name>.rs`:

```rust
pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(MyTool));
}

struct MyTool;

impl Tool for MyTool {
    fn name(&self) -> &'static str { "my_tool" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "my_category" }
    fn definition(&self) -> serde_json::Value { /* the function schema the model sees */ }
    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String { /* ... */ }
}
```

Grades: `Safe` (reads only) < `Standard` (reversible writes) < `Sensitive` (system changes) <
`Dangerous` (destructive). Add `pub mod mytool;` to `lib.rs` and call `mytool::register(reg)` from
`register_all()`; [`git.rs`](crates/yantrik-companion-tools/src/git.rs) is a full example. Use
`validate_path()` for any file access, keep output short enough for a model to read, and grade
honestly: the grade decides whether a person is asked first.

### A UI feature in the shell

Each feature's wiring lives in `crates/yantrik-ui/src/wire/<name>.rs` as a
`pub fn wire(ui: &App, ctx: &AppContext)` registering its Slint callbacks, called once from
[`wire/mod.rs`](crates/yantrik-ui/src/wire/mod.rs). Its markup is in
`crates/yantrik-ui-slint/ui/`, built from the shared components in `crates/yantrik-ui-kit/slint/`.

### A YAML plugin (no Rust)

Plugins add tools from `~/.config/yantrik/plugins/*.yaml`:

```yaml
name: "my-tools"
version: "1.0"
tools:
  - name: "check_vpn"
    description: "Check if VPN is connected"
    permission: "safe"
    category: "network"
    parameters: {}
    command: "mullvad status"
```

Parameters substitute into the command as `{param_name}`, and their values are sanitized (no `;`,
`&`, `|`, `` ` ``, `$`, `>`, `<`). The command template itself is trusted, so a plugin is only as
safe as its author; the machine's permission ceiling still applies to every tool.

### A theme (no Rust)

`~/.config/yantrik/theme-override.yaml` overrides the default theme's (Calm Graphite) key colours:

```yaml
name: "Nord"
enabled: true
bg_deep: "#2e3440"
bg_surface: "#3b4252"
bg_card: "#434c5e"
bg_elevated: "#4c566a"
accent: "#81a1c1"
text_primary: "#eceff4"
text_secondary: "#d8dee9"
text_dim: "#4c566a"
amber: "#ebcb8b"
cyan: "#88c0d0"
```

Set `enabled: false`, or delete the file, to go back to the default.

## Known issues

- **rustc 1.93.x internal compiler error around dead-code lints.** A compiler bug, not this code.
  `yantrik-ui` carries mitigations (`#![allow(unused)]` at the crate root and `#[allow(dead_code)]`
  on the `mod` lines its comments name). If another crate hits it, add `#[allow(dead_code)]` to the
  `mod` in the stack trace, or update to a newer stable toolchain.

## Talking to us

The Discord, [discord.gg/7cDw3jd3Xf](https://discord.gg/7cDw3jd3Xf), is read by the people who
wrote the code: `#apps-and-surfaces` for the control protocol, `#minds` for harnesses, and the
**help** forum for anything that broke. Issues are at
[github.com/yantrikos/yantrik-os/issues](https://github.com/yantrikos/yantrik-os/issues).
