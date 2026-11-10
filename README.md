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
   The image is 1.84 GiB at v0.1.0-2409 (8 Nov), and it will be about 0.5 GiB larger once Hermes
   is built into it ([#1169](https://github.com/yantrikos/yantrik-os/pull/1169)). `latest.json`
   has the exact `bytes` for the build you are about to download.

   ```bash
   sh install.sh --download     # fetches the image and checks its sha256; writes nothing else
   ```
2. **Boot it** in a VM or from a USB stick. It runs live without touching any disk; the disk
   installer is inside the running desktop.
3. **Point it at a model.**

> **You need a model for the AI parts to be useful, and the image ships none.** That means either
> an **account with a model provider** (a free one works: Settings → AI has a Free AI accounts
> card, for example Groq; or any OpenAI-compatible endpoint,
> or the Claude CLI) or a **local GPU** running Ollama or llama.cpp, on this machine or elsewhere
> on your LAN. Small models on a CPU with no GPU produce a token or two a second, which is not a
> conversation. The desktop itself is fully usable without a model; the agents are what need one.

| | Smallest it has been booted on | Booted in CI on every published image |
|--|---------|------------------------|
| **CPU** | 4 cores | 4 |
| **RAM** | 4 GB | 4 GB |
| **GPU** | none (software rendering) | none (virtio-vga) |
| **Disk** | 6 GB to install (the installer's own minimum) | none, boots live |
| **Firmware** | BIOS in CI; UEFI installed and booted on Proxmox OVMF (one VM, 9 Nov) | BIOS |

Anything smaller than 4 cores or 4 GB has not been tried, so no smaller number is given. x86-64
only. Real hardware and any GPU other than QEMU's have **not been measured**; see
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
  How often you are asked is a mode you set from the chip in the right-hand group of the top bar, which
  reads "Mind · Asks first" (or "Suggests only", "Acts within limits", a bypass's own label with
  its countdown). Its menu has "See what it did without asking". Nothing graded above the
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
  OpenClaw (shipped as source in [`harnesses/`](harnesses)). Hermes is the default Mind: after
  first run the shell installs it in the background (it downloads, about four minutes, and needs
  the internet), and the built-in Companion answers until it is ready. The others are not
  started until you configure them. A Mind keeps its own endpoint and key; the one exception is "use a saved provider", which
  copies a key from the vault into that Mind's own settings, and can be reverted. The Lens names
  the Mind that is answering, and the Mind panel (hidden until you pin it) shows "Answering:" and
  what it runs on, the provider and model. The Yantrik Mind runs under its own system account
  (`yantrik-mind`), and the kernel holds that account to a local egress proxy
  ([design/mind-egress-2026-09-29.md](design/mind-egress-2026-09-29.md)). The default egress mode
  is `audit` (connections are recorded and shown in Settings); `guarded` and `enforce` restrict
  it to places you approve. The rules are on the Mind's account; your own apps are not blocked.
  Per the egress design (29 Sep), the Hermes and Pi harness units run as you, not under that
  account. Running Hermes under the Mind account is in review
  ([#1076](https://github.com/yantrikos/yantrik-os/pull/1076)).
- **Memory.** The built-in Mind keeps what it learns in [YantrikDB](https://github.com/yantrikos/yantrikdb),
  on your machine as plain SQLite you can read, export or delete. Making YantrikDB the default
  memory for every attached Mind, Hermes included, is **planned**
  ([#677](https://github.com/yantrikos/yantrik-os/issues/677)), as is per-Mind scoping of it
  ([#447](https://github.com/yantrikos/yantrik-os/issues/447)).

The crate-by-crate map, the layer rules and the rules every pull request is held to are in
[docs/architecture.md](docs/architecture.md).

## What ships

Notes, Email, Calendar, Weather, Terminal, Text Editor, yDoc, yPresent, Images, Downloads,
Snippets, Containers, Network, System Monitor, Arcade, Studio, Music, ySheets and Browser, each
its own window and each drivable through the control surface (`yos ls` on a booted image lists
them); Files, Settings, Memories, Permissions, Devices, Packages and Skills are screens in the
shell. Music and ySheets were shelved on 20 Sep because nothing real was behind them, and came
back in October once there was
([design/shelved-2026-09-20.md](design/shelved-2026-09-20.md)); nothing is shelved now. The built-in companion also has
proactive "instincts" (email watch, open loops, routines) that decide when something is worth
your attention; see [docs/companion.md](docs/companion.md).

## Updating

```bash
yantrik-update check        # what is installed vs what the channel has
yantrik-update apply        # download, verify, install, restart the session
                            # [--channel C] [--force] [--allow-downgrade] [--reboot] [--no-restart]
yantrik-update rollback     # restore the previous build
```

The update information is a signed file (ed25519) that is checked before any of it is read, the
bundle is checked against it, the current binaries are backed up first, and if the new shell does
not answer its control socket the update rolls back on its own. Only the `nightly` channel has ever had builds published to it;
`beta` and `stable` are names the updater knows and are empty.

## Status and known limits

This is early software. Read this before you put anything you care about on it.

- **Nightly quality.** Every published image passes CI and an automated QEMU boot test, and that
  is the bar. Nothing is promoted through `beta` or `stable`. Things break, and the audits of
  what does not work are written down in [`design/`](design/).
- **Open security work**, in progress, tracked in public:
  - **Done, with limits.** The update manifest is signed with ed25519
    ([#537](https://github.com/yantrikos/yantrik-os/issues/537), 30 Oct) and `yantrik-update`
    refuses one that does not verify, is expired or has an older serial. The signing key for
    nightly is a CI secret. The ISO you first download is checked only against the sha256 in
    `latest.json` from the same host: the image itself is not signed. Mail passwords and OAuth
    tokens of the Email app live in the credential vault, not in `email.json`
    ([#538](https://github.com/yantrikos/yantrik-os/issues/538)); saved AI providers' keys live
    there too, not in `providers.yaml`
    ([#1015](https://github.com/yantrikos/yantrik-os/pull/1015)). Still outside the vault: the
    built-in companion's own mail account keeps its password in `config.yaml` in plain text
    ([#1022](https://github.com/yantrikos/yantrik-os/issues/1022)); the installer leaves the key
    you typed in the wizard in `providers.yaml` until the first start moves it; and "use this
    provider" for a harness copies the key into that harness's own settings.
  - [#1137](https://github.com/yantrikos/yantrik-os/issues/1137): a Mind that runs as your
    account (Hermes and the other harness Minds, and the jobs a Mind starts) can drive your
    signed-in Browser through its local DevTools port, and read what the Browser holds, with no
    approval card. A fix is in review
    ([#1151](https://github.com/yantrikos/yantrik-os/pull/1151)). Also open and public:
    [#1027](https://github.com/yantrikos/yantrik-os/issues/1027) (apps shown in Mind View are
    treated as you), [#548](https://github.com/yantrikos/yantrik-os/issues/548) (your own
    dotfiles start the session, so the boot lock does not hold against a process running as
    you), [#542](https://github.com/yantrikos/yantrik-os/issues/542) (some companion tools read
    credential files or change state while graded Safe) and
    [#885](https://github.com/yantrikos/yantrik-os/issues/885) (recipes give tool output to a
    model without the injection check).
  - The full list is the issues labelled
    [`security`](https://github.com/yantrikos/yantrik-os/labels/security).
- **Agents are held by the gate, not by a sandbox.** The built-in Yantrik Mind runs as its own
  account. An agent you start yourself in a terminal runs as you, and the OS can only hold it
  where it goes through `yos` or `yos-mcp`. Applications are not sandboxed either: software
  installed through the package manager runs with your access, as on any Debian desktop.
- **Hardware.** Tested: QEMU (BIOS, software rendering) in CI and on the project's test VMs, and
  an install plus boot under UEFI on Proxmox OVMF (one VM, 9 Nov; CI still boots BIOS only).
  Not measured: real hardware, UEFI on real firmware, any GPU, and the disk installer on real
  disks. x86-64 only; there is no ARM image.
- **Models.** The image ships none, and the quality of what the agents do depends on the model
  you give them.
- **Privacy.** The OS itself sends no telemetry. It does make some calls you did not type: once,
  when it does not know where it is, it asks ipinfo.io for a location and timezone (the installer
  asks ip-api.com over plain HTTP for a timezone), which tells that service your public IP;
  `YANTRIK_NO_GEOLOCATE=1` turns both off. After first run it downloads and installs Hermes, the
  default Mind, from the internet. Beyond those, it reaches out where you pointed it: the update
  check against `releases.yantrikos.com`, whatever your weather, mail and calendar are configured
  against, and a problem report you read and chose to send. A Mind you point at a cloud provider
  talks to that provider.

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
