# Yantrik OS

[![Chat on Discord](https://img.shields.io/badge/Discord-join%20the%20server-5865F2?logo=discord&logoColor=white)](https://discord.gg/7cDw3jd3Xf)
[![Nightly image](https://img.shields.io/badge/nightly-iso.yantrikos.com-informational)](https://iso.yantrikos.com/nightly/)
[![CI](https://github.com/yantrikos/yantrik-os/actions/workflows/ci.yml/badge.svg)](https://github.com/yantrikos/yantrik-os/actions/workflows/ci.yml)
[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-blue)](LICENSE)

**A Debian-based Linux desktop built so AI agents can work on it beside you, safely.** Every app
and screen publishes what it holds and what it can be asked to do, so an agent uses your desktop
through a typed interface instead of guessing at screenshots. A permission gate decides what runs
without asking, and a Mind's risky actions put an approval card in front of you. (The built-in
companion is the exception today: see [Status and known limits](#status-and-known-limits).)

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

A Mind or harness that does something above its grade does not run it quietly. (The built-in
companion runs Sensitive tools with no card at the shipped setting; see the limits below.) A card says who is asking,
what it will do and with which arguments, and you choose **Allow once**, **Deny**, or (for
anything recoverable) **Allow for this session**.

## Try it

1. **Download** the newest nightly: <https://iso.yantrikos.com/latest.iso>
   (`https://iso.yantrikos.com/nightly/latest.json` names the current file, its sha256 and size).
   The image is about **1.85 GiB** as built (1,984,960,512 bytes for build
   `v0.1.0-2690-g92eec0bd`, 16 Nov). The size changes from build to build; `latest.json` has the
   exact `bytes` for the build you are about to download.

   ```bash
   sh install.sh --download     # fetches the image and checks its sha256; writes nothing else
   ```
2. **Boot it** in a VM or from a USB stick. The first page is the installer's Welcome page, where
   you pick the keyboard layout and choose **Try it first** (runs live; touches no disk) or
   **Install**.
3. **To install**, the installer's pages come first, in this order:
   1. **Welcome**: the keyboard layout, then "Try it first" or "Install".
   2. **About you**: your name, a username and a password.
   3. **Where to install**: the disk, and **"Encrypt with my password", which is ticked by
      default**. With it ticked the installed machine asks for that password at every start, on a
      screen that shows your name. Untick it on this page if you do not want that. Without
      encryption, anyone who takes the disk out can read it; with it, a forgotten password means an
      unreadable disk.
   4. **Ready to install**: every answer and the disk together. "Erase disk and install" erases
      the disk you chose. It took about five minutes on a test VM.

   **Then take the installer disc out before the restart.** In a virtual machine, detach the ISO
   (and check the boot order): the page says the disc comes out by itself, but in a VM it does not,
   and the machine starts the installer's Welcome page again
   ([#1165](https://github.com/yantrikos/yantrik-os/issues/1165)). The install is fine; boot from
   the disk.
4. **First run** is on the installed machine's first boot, after you enter the disk password. It
   has four pages. "Not now" on the second one skips choosing a model:
   1. **Welcome**: text size and read-aloud options, then Get started.
   2. **Who should do the thinking?**: where the AI runs. See "Point it at a model" below.
   3. **Getting your minds ready**: a check of each mind. With no model chosen it says it "is not
      set up to think yet".
   4. **Your minds**: one row per mind, with what was seen, and "Add another mind".

   First run does not ask about your interests or where you are. If you choose a provider, a page
   about what your minds share with it comes after the provider page (it is in the code; the
   16 Nov run did not choose one, so did not see it).
5. **Point it at a model.**

> **You need a model for the AI parts to be useful, and the image ships none.** Every card on the
> first-run provider page needs an account: Groq (marked Recommended, free, needs a free Groq
> account), Claude, OpenAI, Google Gemini, DeepSeek, OpenRouter and NVIDIA NIM, and "Show every
> provider" has more. Or use a **local GPU** running Ollama or llama.cpp, on this machine or
> elsewhere on your LAN ("Look on your network" finds one that is already running). Small models
> on a CPU with no GPU produce a token or two a second, which is not a conversation. The desktop
> itself is fully usable without a model; the agents are what need one.
>
> **Free models that need no account exist, and today you reach them in Settings, not in first
> run.** Choose "Not now" on the provider page, then:
> 1. Settings, AI & Intelligence: opt in to sharing with the free provider (the run used
>    OVHcloud). What you type goes to that provider.
> 2. Settings, Minds: press **Use Yantrik models** on the Yantrik Companion row, then Apply.
> 3. Pick a model in the ask bar.
>
> A fresh run answered "The capital of France is Paris." this way, with no key.
>
> <!-- SWAP-ON-#1167: the paragraph above ("Free models that need no account exist ...") describes
> first-run today. When the change under #1167 lands and puts the free models in first run,
> replace that one paragraph (and the same marked paragraph in docs/getting-started.md) and
> nothing else. -->
> A change that puts these free models in first run is in progress
> ([#1167](https://github.com/yantrikos/yantrik-os/issues/1167)). It is not in the image yet.

In the 16 Nov run the provider page was taller than the screen, and wheel events sent to the VM did
not scroll it; Tab did. That was not tried with a real mouse. Hermes, the default Mind, is set up
after a model is chosen; see "Minds" below.

| | Minimum the installer checks | Smallest it has been booted on | Booted in CI on every published image |
|--|---------|---------|------------------------|
| **CPU** | 2 cores | 4 cores | 4 |
| **RAM** | 4 GB | 4 GB | 4 GB |
| **GPU** | none | none (software rendering) | none (virtio-vga) |
| **Disk** | 6 GB, to install | 6 GB to install (the installer's own minimum) | none, boots live |
| **Firmware** | BIOS or UEFI | UEFI: installed and booted under UEFI firmware (OVMF) in a virtual machine | BIOS |

The installer's wizard checks for 2 cores; nothing under 4 cores or 4 GB has been booted, so no
smaller number is promised. x86-64
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
  OpenClaw (shipped as source in [`harnesses/`](harnesses)). Hermes is the default Mind. It is set
  up after you choose a model, and it downloads about 3 GB (from GitHub and Python's package
  index), so it needs the internet. Until it is ready the built-in Companion answers
  ([#1166](https://github.com/yantrikos/yantrik-os/issues/1166)). The first-run pages carry this
  note even when no model was chosen; the 16 Nov run chose none, so the download itself was not
  tested. The others are not started until you configure them. A Mind keeps its own endpoint and key; the one exception is "use a saved provider", which
  copies a key from the vault into that Mind's own settings, and can be reverted. The Lens names
  the Mind that is answering, and the Mind panel (hidden until you pin it) shows "Answering:" and
  what it runs on, the provider and model. The Yantrik Mind runs under its own system account
  (`yantrik-mind`), and the kernel holds that account to a local egress proxy
  ([design/mind-egress-2026-09-29.md](design/mind-egress-2026-09-29.md)). The default egress mode
  is `audit` (connections are recorded and shown in Settings); `guarded` and `enforce` restrict
  it to places you approve. The rules are on the Mind's account; your own apps are not blocked.
  Hermes runs under that account too
  ([#1076](https://github.com/yantrikos/yantrik-os/pull/1076), merged). Pi, OpenClaw and DeepSeek
  still run as you.
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
- **What the built-in companion does without asking.** At the setting the image ships with, the
  built-in companion runs its Safe, Standard and Sensitive tools with **no approval card**. Today
  that includes running shell commands and reading the screen. Minds and harnesses (Hermes, Pi and
  the others) are asked on cards, but "Allow for this session" and the bypass modes answer later
  calls unasked. This is a known gap, not a design promise. It is tracked in
  [#1247](https://github.com/yantrikos/yantrik-os/issues/1247) /
  [#1248](https://github.com/yantrikos/yantrik-os/pull/1248),
  [#1265](https://github.com/yantrikos/yantrik-os/issues/1265),
  [#1266](https://github.com/yantrikos/yantrik-os/issues/1266),
  [#1267](https://github.com/yantrikos/yantrik-os/issues/1267),
  [#542](https://github.com/yantrikos/yantrik-os/issues/542) and
  [#540](https://github.com/yantrikos/yantrik-os/issues/540). No date is promised. Details are in
  [SECURITY.md](SECURITY.md).
- **Other open security work**, tracked in public and listed one by one in
  [SECURITY.md](SECURITY.md): the Browser you open from the dock has no DevTools port, but Mind
  View's browser does ([#1200](https://github.com/yantrikos/yantrik-os/issues/1200)) and a job a
  Mind starts runs as you ([#1137](https://github.com/yantrikos/yantrik-os/issues/1137)); your own
  dotfiles start the session ([#548](https://github.com/yantrikos/yantrik-os/issues/548)); the
  built-in companion keeps its own mail password in `config.yaml` in plain text
  ([#1022](https://github.com/yantrikos/yantrik-os/issues/1022)); and the update manifest is
  signed, but the ISO you first download is checked only against the sha256 in `latest.json` from
  the same host. The full list is the issues labelled
  [`security`](https://github.com/yantrikos/yantrik-os/labels/security).
- **Agents are held by the gate, not by a sandbox.** The built-in Yantrik Mind and Hermes run as
  their own account. An agent you start yourself in a terminal, Pi, OpenClaw, DeepSeek and the
  jobs a Mind starts run as you, and the OS can only hold them where they go through `yos` or
  `yos-mcp`. A Mind's workshop is sealed on the program, not on the Mind: a sealed job sees only
  its workshop and has no network, but an unsealed job runs as you and can change your files,
  including what your session runs at login, and under a session rule it needs no card to do so. Applications are not sandboxed either: software
  installed through the package manager runs with your access, as on any Debian desktop.
- **Hardware.** Tested: QEMU (BIOS, software rendering) in CI and on the project's test VMs, and
  an install plus boot under UEFI firmware (OVMF) in a virtual machine; CI still boots BIOS only.
  Not measured: real hardware, UEFI on real firmware, any GPU, and the disk installer on real
  disks. x86-64 only; there is no ARM image.
- **Models.** The image ships none, and the quality of what the agents do depends on the model
  you give them.
- **Privacy.** The OS itself sends no telemetry. It does make some calls you did not type: once,
  when it does not know where it is, it asks ipinfo.io for a location and timezone (the installer
  asks ip-api.com over plain HTTP for a timezone), which tells that service your public IP;
  `YANTRIK_NO_GEOLOCATE=1` turns both off. After you choose a model it downloads about 3 GB for Hermes, the
  default Mind, from GitHub and Python's package index. Beyond those, it reaches out where you pointed it: the update
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
