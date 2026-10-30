# Yantrik OS

[![Chat on Discord](https://img.shields.io/badge/Discord-join%20the%20server-5865F2?logo=discord&logoColor=white)](https://discord.gg/7cDw3jd3Xf)
[![Nightly image](https://img.shields.io/badge/nightly-iso.yantrikos.com-informational)](https://iso.yantrikos.com/nightly/)
[![CI](https://github.com/yantrikos/yantrik-os/actions/workflows/ci.yml/badge.svg)](https://github.com/yantrikos/yantrik-os/actions/workflows/ci.yml)
[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-blue)](LICENSE)

**A Debian-based Linux desktop built so AI agents can work on it beside you, safely.** Every app
and screen publishes what it holds and what it can be asked to do, so an agent uses your desktop
through a typed interface instead of guessing at screenshots. A permission gate decides what runs
without asking, and anything risky puts an approval card in front of you.

It is early software, published as a nightly live ISO. [Status and known limits](#status-and-known-limits)
says what is rough, in plain words.

## See it work in 60 seconds

<!-- LAUNCH: replace with the demo video / GIF once captured on a test VM. -->
> **Demo: coming soon.** *(placeholder: video or GIF link goes here)*

What the demo shows, and what you can run yourself on any booted image:

```bash
yos describe shell                       # what the desktop is showing, and what it can be asked to do
yos act shell open_app name=notes        # an agent opens an app, through the same path a person uses
yos describe notes                       # the app answers for itself: its state and its actions
yos act notes append text='hello'        # a standard action runs; a sensitive one raises an approval card
```

An agent that does something above its grade does not run it quietly. A card says who is asking,
what it will do and with which arguments, and you choose **Allow once**, **Deny**, or (for
anything recoverable) **Allow for this session**.

## Try it

1. **Download** the newest nightly: <https://iso.yantrikos.com/latest.iso>
   (`https://iso.yantrikos.com/nightly/latest.json` names the current file, its sha256 and size).
   The image is about 1.8 GiB.

   ```bash
   sh install.sh --download     # fetches the image and checks its sha256; writes nothing else
   ```
2. **Boot it** in a VM or from a USB stick. It runs live without touching any disk; the disk
   installer is inside the running desktop.
3. **Point it at a model.**

> **You need a model for the AI parts to be useful, and the image ships none.** That means either
> a **cloud model key** (any OpenAI-compatible endpoint, or the Claude CLI) or a **local GPU**
> running Ollama or llama.cpp, on this machine or elsewhere on your LAN. Small models on a CPU
> with no GPU produce a token or two a second, which is not a conversation. The desktop itself
> is fully usable without a model; the agents are what need one.

| | Minimum | Booted in CI on every published image |
|--|---------|------------------------|
| **CPU** | 2 cores | 4 |
| **RAM** | 4 GB | 4 GB |
| **GPU** | none (software rendering) | none (virtio-vga) |
| **Disk** | 6 GB to install | none, boots live |
| **Firmware** | BIOS (UEFI is built, untested) | BIOS |

x86-64 only. Real hardware and any GPU other than QEMU's have **not been measured**; see
[docs/hardware-requirements.md](docs/hardware-requirements.md) for which numbers are measured and
which are not. The whole path from download to attached mind is in
[docs/getting-started.md](docs/getting-started.md).

## How it works

```
 you ──────────────┐                    ┌── a Mind: Yantrik Mind, Hermes, Pi, OpenClaw, DeepSeek ...
 (keyboard, mouse) │                    │   (attaches to the desktop; keeps its own model and key)
                   ▼                    ▼
        ┌─────────────────────────────────────────┐
        │  control surface   yos describe / act   │  every app and screen: state + graded actions
        ├─────────────────────────────────────────┤
        │  permission gate   mode + ceiling       │  safe / standard / sensitive / dangerous
        │  approval cards    mind-audit.jsonl     │  you decide; what ran unasked is logged
        ├─────────────────────────────────────────┤
        │  apps · shell · services                │  Rust + Slint, Debian 13, labwc (Wayland)
        └─────────────────────────────────────────┘
          YantrikDB: persistent memory for the Minds      yantrik-egress: where the Mind account may connect
```

- **Control surface.** The shell and each app answer two calls on a unix socket: `app.describe`
  (what are you holding?) and `app.act` (do this). `yos` is the command line for it, and
  `yos-mcp` exposes the same surface over MCP. A program nobody here wrote can join by publishing
  one socket; Blender does. See [docs/app-control.md](docs/app-control.md) and the normative
  [docs/surface-protocol.md](docs/surface-protocol.md).
- **Permission gate.** Each action has a grade: `safe`, `standard`, `sensitive` or `dangerous`.
  How often you are asked is a mode you set from the status bar. Nothing graded above the
  machine's own ceiling runs or is even asked about, in any mode. No action on any surface can
  grant a permission, and `set_mind_mode` can only tighten.

  | mode | the Mind may... |
  |---|---|
  | `plan` | read only; every change is refused and it has to say what it *would* do |
  | `ask` | routine things run; sensitive ones put a card in front of you (the default) |
  | `auto` | sensitive things run; destructive ones still ask |
  | `bypass` / `bypass_all` | little or nothing is asked. Time-boxed, never written to disk, so no machine boots into either |

- **Minds.** A Mind is any agent that attaches to the desktop through the
  [harness protocol](docs/harness.md): the built-in Yantrik Mind, and Hermes, Pi, DeepSeek and
  OpenClaw (shipped as source in [`harnesses/`](harnesses); none is started until you configure it).
  The OS never holds a Mind's endpoint or key. The status bar names which Mind is answering and
  whether it is local or cloud. The Yantrik Mind runs under its own system account
  (`yantrik-mind`), and the kernel holds that account to a local egress proxy
  ([design/mind-egress-2026-09-29.md](design/mind-egress-2026-09-29.md)). The default egress mode
  is `audit` (connections are recorded and shown in Settings); `guarded` and `enforce` restrict
  it to places you approve. The rules are on the Mind's account; your own apps are not blocked.
  Per the egress design (29 Sep), the Hermes and Pi harness units run as you, not under that
  account.
- **Memory.** The built-in Mind keeps what it learns in [YantrikDB](https://github.com/yantrikos/yantrikdb),
  on your machine as plain SQLite you can read, export or delete. Making YantrikDB the default
  memory for every attached Mind, Hermes included, is **planned**
  ([#677](https://github.com/yantrikos/yantrik-os/issues/677)), as is per-Mind scoping of it
  ([#447](https://github.com/yantrikos/yantrik-os/issues/447)).

The crate-by-crate map, the layer rules and the rules every pull request is held to are in
[docs/architecture.md](docs/architecture.md).

## What ships

Notes, Email, Calendar, Weather, Terminal, Text Editor, yDoc, yPresent, Images, Downloads,
Snippets, Containers, Network, System Monitor, Arcade and Studio, each its own window and each
drivable through the control surface; Files, Settings, Memories, Permissions, Devices, Packages
and Skills are screens in the shell. Two more apps (ySheets, Music) are in the tree and
**shelved**: not in the build, because nothing real was behind them
([design/shelved-2026-09-20.md](design/shelved-2026-09-20.md)). The built-in companion also has
proactive "instincts" (email watch, open loops, routines) that decide when something is worth
your attention; see [docs/companion.md](docs/companion.md).

## Updating

```bash
yantrik-update check        # what is installed vs what the channel has
yantrik-update apply        # download, verify the sha256, install, restart the session
yantrik-update rollback     # restore the previous build
```

The current binaries are backed up first, and if the new shell does not answer its control socket
the update rolls back on its own. Only the `nightly` channel has ever had builds published to it;
`beta` and `stable` are names the updater knows and are empty.

## Status and known limits

This is early software. Read this before you put anything you care about on it.

- **Nightly quality.** Every published image passes CI and an automated QEMU boot test, and that
  is the bar. Nothing is promoted through `beta` or `stable`. Things break, and the audits of
  what does not work are written down in [`design/`](design/).
- **Open security work**, in progress, tracked in public:
  - [#537](https://github.com/yantrikos/yantrik-os/issues/537): the update manifest is **not
    signed**. `yantrik-update` checks the bundle against the manifest's sha256, which proves the
    download matches the manifest and nothing about who wrote the manifest.
  - [#538](https://github.com/yantrikos/yantrik-os/issues/538): mail account passwords are stored
    in plaintext; they belong in the credential vault.
  - [#1015](https://github.com/yantrikos/yantrik-os/pull/1015): saved AI providers' keys move out
    of `providers.yaml` and into the credential vault.
  - The full list is the issues labelled
    [`security`](https://github.com/yantrikos/yantrik-os/labels/security).
- **Agents are held by the gate, not by a sandbox.** The built-in Yantrik Mind runs as its own
  account. An agent you start yourself in a terminal runs as you, and the OS can only hold it
  where it goes through `yos` or `yos-mcp`. Applications are not sandboxed either: software
  installed through the package manager runs with your access, as on any Debian desktop.
- **Hardware.** Tested: QEMU (BIOS, software rendering) in CI and on the project's test VMs.
  Not measured: real hardware, UEFI boot, any GPU, and the disk installer on real disks.
  x86-64 only; there is no ARM image.
- **Models.** The image ships none, and the quality of what the agents do depends on the model
  you give them.
- **Privacy.** The OS itself sends no telemetry and makes no call on its own. It reaches out in
  three places you chose: the update check against `releases.yantrikos.com`, whatever your
  weather, mail and calendar are configured against, and a problem report you read and chose to
  send. A Mind you point at a cloud provider talks to that provider.

## For developers

```bash
sudo apt-get install -y --no-install-recommends \
  pkg-config libasound2-dev libudev-dev libssl-dev libspeechd-dev libwayland-dev \
  libxkbcommon-dev libfontconfig1-dev libfreetype6-dev libinput-dev libgbm-dev libegl1-mesa-dev
cargo build --workspace --locked        # Rust 1.92, pinned in rust-toolchain.toml
cargo test --workspace --locked
```

[CONTRIBUTING.md](CONTRIBUTING.md) covers building, running the shell in a VM, writing an app,
and what a pull request needs. To make a program of your own drivable by a Mind, start with
[docs/sdk/README.md](docs/sdk/README.md).

## Contributing

Start with [CONTRIBUTING.md](CONTRIBUTING.md). Issues labelled
[`good first issue`](https://github.com/yantrikos/yantrik-os/issues?q=is%3Aopen+label%3A%22good+first+issue%22)
and [`help wanted`](https://github.com/yantrikos/yantrik-os/issues?q=is%3Aopen+label%3A%22help+wanted%22)
are real and unassigned: say which one you want and it is yours. A report of how a nightly behaved
on your hardware is a contribution too.

Security problems go to [SECURITY.md](SECURITY.md) (a private GitHub advisory), not to a public
issue. The Discord, [discord.gg/7cDw3jd3Xf](https://discord.gg/7cDw3jd3Xf), is small and read by
the people who wrote the code.

## License

GPL-3.0. See [LICENSE](LICENSE).

## Links

- Images (nightly): <https://iso.yantrikos.com/nightly/>
- Release bundles: <https://releases.yantrikos.com>
- Docs: [getting started](docs/getting-started.md) · [app control](docs/app-control.md) ·
  [harness protocol](docs/harness.md) · [hardware](docs/hardware-requirements.md) ·
  [architecture](docs/architecture.md)
- Issues: <https://github.com/yantrikos/yantrik-os/issues>
