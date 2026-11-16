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

In the default `ask` mode, a Mind or harness that runs a sensitive action or a shell command does
not do it quietly: a card says who is asking, what it will do and with which arguments, and you choose **Allow once**,
**Deny**, or (for anything recoverable) **Allow for this session**. Not every call raises a card.
A Mind's `read_screen` and `screenshot_screen` do, every time except in full bypass. Its `os_screen`, `os_perception`,
`read_mind_view` and `screenshot_mind_view` do not. The built-in companion runs Sensitive tools
with no card at the shipped setting. See the limits below.

## Try it

1. **Download** the newest nightly: <https://iso.yantrikos.com/latest.iso>
   (`https://iso.yantrikos.com/nightly/latest.json` names the current file, its sha256 and size).
   The image is about **1.85 GiB** as built (1,984,960,512 bytes for build
   `v0.1.0-2690-g92eec0bd`, 16 Nov). The size changes from build to build; `latest.json` has the
   exact `bytes` for the build you are about to download.

   ```bash
   sh install.sh --download     # fetches the image and checks its sha256; writes nothing else
   ```
2. **Boot it** in a VM or from a USB stick. A boot menu shows for five seconds, then the
   installer's Welcome page, where you pick the keyboard layout and choose **Try it first** (runs
   live; touches no disk) or **Install**. For VM settings (on Proxmox, give the VM a VirtIO
   display, or the screen stays black), see
   [docs/getting-started.md](docs/getting-started.md) sections 2 and 3.
3. **To install**, the installer's pages come first, in this order:
   1. **Welcome**: the keyboard layout, then "Try it first" or "Install".
   2. **About you**: your name, a username and a password.
   3. **Where to install**: the disk, and **"Encrypt with my password", which is ticked by
      default**. With it ticked the installed machine asks for that password at every start, on a
      screen that shows your name. Untick it on this page if you do not want that. Without
      encryption, anyone who takes the disk out can read it; with it, a forgotten password means an
      unreadable disk.
   4. **Ready to install**: every answer and the disk together, and a **Thinking** row where you
      can choose a model provider now (Choose or Change). "Erase disk and install" erases the
      disk you chose. It took about five minutes on a test VM.

   **Then take the installer disc out before the restart.** In a virtual machine, detach the ISO
   (and check the boot order): the page says the disc comes out by itself, but in a VM it does not,
   and the machine starts the installer's Welcome page again
   ([#1165](https://github.com/yantrikos/yantrik-os/issues/1165)). The install is fine; boot from
   the disk. On real hardware with a USB stick, the page says to take the stick out once the
   screen goes dark.
4. **First run** is on the installed machine's first boot, after you enter the disk password. It
   has four pages. If you choose a provider there it has five (a model on this computer skips the
   sharing page, so it stays at four), and if you chose one in the installer's Thinking row it has
   three. "Not now" on the second page skips choosing a model:
   1. **Welcome**: text size and read-aloud options, then Get started.
   2. **Who should do the thinking?**: where the AI runs. See "Point it at a model" below.
   3. **Getting your minds ready**: a check of each mind. With no model chosen it says it "is not
      set up to think yet".
   4. **Your minds**: one row per mind, with what was seen, and "Add another mind".

   First run does not ask about your interests or where you are. On a boot that did not come
   through the installer, Welcome also asks "What should your minds call you?". If you choose a
   provider, a page about what your minds share with it comes after the provider page (it is in
   the code; the 16 Nov run did not choose one, so did not see it).
   The sharing page is skipped when you start with **Start free, no account**: the consent is on
   that card.

   In the 16 Nov run the provider page (page 2) was taller than the screen, and wheel events sent
   to the VM did not scroll it; Tab did. That was not tried with a real mouse.
5. **Point it at a model.**

> **You need a model for the AI parts to be useful, and the image ships none.**
> The provider page leads with **Start free, no account**, when the image knows a free provider
> that needs no sign-up (today OVHcloud AI Endpoints). Nothing is sent, and nothing is chosen,
> until you press its button, and the card says what pressing it does, with the provider's name,
> before you do:
>
> > **What you tell your minds, and what they read on this computer, is sent to the provider, also when a mind works on its own.** That includes your messages and name, the time, what Yantrik remembers about you (a place you saved too), files, pages, command output and any saved password a mind opens for you, the screen and clipboard when a mind looks at them, and your network's name, open apps, battery, processor, memory, disk and how long the computer has been idle. It applies to every mind on Yantrik models. It is free and rate-limited, so it can be slow or busy. Switch it off in Settings → AI & Intelligence.
>
> Every other card on that page needs an account: Groq (marked Recommended, free, needs a free
> Groq account), Claude, OpenAI, Google Gemini, DeepSeek, OpenRouter and NVIDIA NIM, and "Show
> every provider" has more. Or use a **local GPU** running Ollama or llama.cpp, on this machine or
> elsewhere on your LAN ("Look on your network" finds one that is already running). Small models
> on a CPU with no GPU produce a token or two a second, which is not a conversation. The desktop
> itself is fully usable without a model; the agents are what need one. If you choose "Not now",
> the ask bar offers the same card as a "Start free" chip beside "Add key".

| | Stated minimum | Smallest it has been booted on | Booted in CI on every published image |
|--|---------|---------|------------------------|
| **CPU** | 2 cores | 4 cores | 4 |
| **RAM** | 4 GB | 4 GB | 4 GB |
| **GPU** | none | none (software rendering) | none (virtio-vga) |
| **Disk** | 6 GB, to install | 6 GB to install | none, boots live |
| **Firmware** | not checked | UEFI: installed and booted under UEFI firmware (OVMF) in a virtual machine | BIOS |

The first-boot hardware scan reports against 2 cores, 4 GB and 6 GB, but nothing refuses a
smaller machine or a disk under 6 GB, and nothing checks the firmware. Installing beside another
system needs 20 GB. Nothing under 4 cores or 4 GB has been booted, so no smaller number is
promised. x86-64 only. Real hardware and any GPU other than QEMU's have **not been measured**; see
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
        │  approval cards    mind-audit.jsonl     │  you decide; a Mind's unasked runs are logged
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
  its countdown). Its menu has "See what it did without asking", which lists what Minds and
  harnesses ran without a card; it does not list the built-in companion's own tool calls. Nothing
  graded above a Mind's ceiling runs or is even asked about, in any mode. The Settings control
  "Tool permission" sets that ceiling. The built-in companion has its own, set in `config.yaml`;
  see the limits below. No action on any surface can grant a permission, and `set_mind_mode` can
  only tighten.

  | mode | the Mind may... |
  |---|---|
  | `plan` | read only; every change is refused and it has to say what it *would* do |
  | `ask` | routine things run; sensitive ones put a card in front of you (the default) |
  | `auto` | sensitive things run; destructive ones still ask, and so does the first command of a session |
  | `bypass` / `bypass_all` | little or nothing is asked. Time-boxed, never written to disk, so no machine boots into either |

- **Minds.** A Mind is any agent that attaches to the desktop through the
  [harness protocol](docs/harness.md): the built-in Yantrik Mind, and Hermes, Pi, DeepSeek and
  OpenClaw (shipped as source in [`harnesses/`](harnesses)). Hermes is the default Mind. It is set
  up after you choose one of the account cards' models (not after **Start free**), and it downloads about 3 GB from GitHub and Python's package
  index, so it needs the internet. Until it is ready the built-in Companion answers
  ([#1166](https://github.com/yantrikos/yantrik-os/issues/1166)). The provider page says so before you choose, and it
  is not shown after Start free; the 16 Nov run chose none, so the download itself was not
  tested. The others are not started until you configure them.
  A Mind keeps its own endpoint and key; the one exception is "use a saved provider", which
  copies a key from the vault into that Mind's own settings, and can be reverted. The Lens names
  the Mind that is answering. The Minds panel, opened from the mind chip, shows "Answering:" and
  what it runs on, the provider and model. The Mind panel on the right edge (hidden until you
  pin it) says "answering · <account> · <model>". The Yantrik Mind runs under its own system account
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
  that includes running shell commands and reading the screen. Its mail still asks on a card, except in full bypass, and
  `app_action` goes through the shell's gate. Tools are graded safe, standard, sensitive or
  dangerous, and the companion has a ceiling above which none of its tools runs
  (`tools.max_permission`; the image ships `sensitive`). The built-in companion runs its own tools
  up to that ceiling without asking you each time. To restrict it, edit `tools.max_permission` in
  `/opt/yantrik/config.yaml`, then restart the session. No Settings control sets that key today. The Settings control
  "Tool permission" sets a different key, `tool_permission`, which is the ceiling for Minds and
  harnesses; it reaches the companion only through `app_action`. The companion's tool calls are
  recorded as `audit/tools` memories in YantrikDB with a seven-day half-life (they fade from
  recall), each keeping at most 200 characters of the result, and are not written in incognito.
  `mind-audit.jsonl` and "See what it did without asking" cover Minds and harnesses only, and
  [#1262](https://github.com/yantrikos/yantrik-os/issues/1262) says that log can stop writing.
  A mind you attach is asked about on your screen: in the default `ask` mode its sensitive and
  dangerous actions put a card in front of you (Hermes, Pi and the others work this way), but
  "Allow for this session", a standing rule and the bypass modes answer later calls unasked. This
  is a known gap, not a design promise. It is tracked in
  [#1247](https://github.com/yantrikos/yantrik-os/issues/1247) /
  [#1248](https://github.com/yantrikos/yantrik-os/pull/1248),
  [#1265](https://github.com/yantrikos/yantrik-os/issues/1265),
  [#1266](https://github.com/yantrikos/yantrik-os/issues/1266),
  [#1267](https://github.com/yantrikos/yantrik-os/issues/1267),
  [#542](https://github.com/yantrikos/yantrik-os/issues/542) and
  [#540](https://github.com/yantrikos/yantrik-os/issues/540). No date is promised. Details are in
  [SECURITY.md](SECURITY.md).
- **Other open security work**, tracked in public and listed one by one in
  [SECURITY.md](SECURITY.md): no browser has a DevTools port any more, but a job a Mind starts
  runs as you and can read your browser's files
  ([#1137](https://github.com/yantrikos/yantrik-os/issues/1137)); your own
  dotfiles start the session ([#548](https://github.com/yantrikos/yantrik-os/issues/548)); the
  built-in companion keeps its own mail password in `config.yaml` in plain text
  ([#1022](https://github.com/yantrikos/yantrik-os/issues/1022)); what a consent to share covers is
  less than what is sent ([#1275](https://github.com/yantrikos/yantrik-os/issues/1275)); and the
  update manifest is signed, but the ISO you first download is checked only against the sha256 in
  `latest.json` from the same host. The full list is the issues labelled
  [`security`](https://github.com/yantrikos/yantrik-os/labels/security).
- **Agents are held by the gate, not by a sandbox.** The built-in Yantrik Mind and Hermes run as
  their own account. An agent you start yourself in a terminal, Pi, OpenClaw, DeepSeek and the
  jobs a Mind starts run as you, and the OS can only hold them where they go through `yos` or
  `yos-mcp`. A Mind's workshop is sealed on the program, not on the Mind: a sealed job sees only
  its workshop and has no network, but an unsealed job runs as you and can change your files,
  including what your session runs at login, and under a session rule it needs no card to do so.
  Applications are not sandboxed either: software installed through the package manager runs
  with your access, as on any Debian desktop.
- **Hardware.** Tested: QEMU (BIOS, software rendering) in CI and on the project's test VMs, and
  an install plus boot under UEFI firmware (OVMF) in a virtual machine; CI still boots BIOS only.
  Not measured: real hardware, UEFI on real firmware, any GPU, and the disk installer on real
  disks. x86-64 only; there is no ARM image.
- **Models.** The image ships none, and the quality of what the agents do depends on the model
  you give them.
- **Privacy.** The OS itself sends no telemetry. It does make some calls you did not type.
  IP-based location lookups and background web research are off until you allow them on a card
  that says what is sent and to whom; the installers do not ask the network for a timezone.
  `YANTRIK_NO_GEOLOCATE=1` keeps lookups off even then. With nobody at the keyboard:
  - it checks for updates against `releases.yantrikos.com` and syncs the clock (NTP) to Debian's
    pool;
  - once a place is saved, the weather service sends the machine's coordinates to
    `api.open-meteo.com` at every start
    ([#1244](https://github.com/yantrikos/yantrik-os/issues/1244));
  - once Hermes is installed, its gateway contacts hosts of Nous Research and OpenRouter, and
    nothing turns that off yet ([#1228](https://github.com/yantrikos/yantrik-os/issues/1228));
  - `clawhub.ai` was seen contacted at boot on one test VM, from no Yantrik code; the source is
    not yet known ([#1243](https://github.com/yantrikos/yantrik-os/issues/1243)).

  After you choose a model it downloads Hermes, the default Mind: about 3 GB from GitHub and
  Python's package index. Other calls go where you pointed the OS: whatever your weather, mail and
  calendar are configured against. A problem report leaves only when you press Send, or Allow a
  mind's request to send it.

  When an app crashes or fails, the desktop writes a record locally with your home path, your user
  name and common key and token shapes removed; other things a log line happens to contain, such
  as host names, addresses or file names, are not removed. *Report a problem* shows you that
  record, and it leaves the machine only when you press Send there or Allow a mind's request to
  send it. It goes over HTTPS to `report.yantrikos.com`, which posts it as a public issue on this
  repository.

  A Mind you point at a cloud provider talks to that provider and sends it what that Mind reads
  on this computer. For the built-in companion on a Yantrik model, that is your conversation; what your minds remember about you, including notification text, file names and apps opened that the shell wrote by itself; your name and the local time; on every turn the computer's state (network name, running apps, battery, CPU, memory, disk, idle time); and the results of every tool the model uses (files, clipboard, window text, browser pages, command output, described screenshots); the same goes for every mind on Yantrik models
  ([#1275](https://github.com/yantrikos/yantrik-os/issues/1275)).

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
